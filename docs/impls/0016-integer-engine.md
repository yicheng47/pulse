# 0016 — Integer bit-perfect engine

Tactical plan for [feature 32](../features/32-integer-engine.md) (milestone M3 → v0.3.0). Reference sources are read-only clones in `~/repos/audio/`: `mpv`, `MPD`, and `SFBAudioEngine` still need cloning (not present as of 2026-08-31 — clone before the stage 1 reading pass); `hummingbird/`, already there, is a fourth reference — a native macOS bit-perfect player. GPL sources are ideas only, no code.

## Goal

A second playback sink: raw HAL `AudioDeviceCreateIOProcID` with hog, mixing disabled, integer physical format, and — the step 0003 missed — **virtual format set equal to the integer physical format**, so the IOProc buffer takes source integers directly. Selected per device (feature 08 pattern), proven by the DoP test on the Matrix.

## Prior art in this repo

- `archive/0002-hal-format-validation.md` proved hog + physical-format switching; that machinery is today's `hal.rs` (`HogGuard`, `set_nominal_sample_rate`, `set_matching_physical_format`) and is reused wholesale.
- `archive/0003-auhal-playback.md` records the raw-HAL noise failure that forced the AUHAL pivot. Root cause in hindsight: integer bytes written into a float32 **virtual** format buffer. This note's whole premise is fixing that properly instead of avoiding it.
- The controller already abstracts the engine: `trait PlaybackBackend { start, feed, position, underrun_frames, take_hardware_volume, set_volume, stop }` behind a `BackendFactory` (`controller.rs:127`). The integer engine is a second implementor; the state machine, queue, and event logic are untouched.

## Stage 1 — probe (`crates/pulse-engine/examples/integer_probe.rs`)

Prerequisite hal additions (needed by the probe itself):

- `kAudioStreamPropertyVirtualFormat` + `kAudioStreamPropertyAvailableVirtualFormats` get/set wrappers (stream-scoped, same shape as the physical pair).
- `kAudioDevicePropertySupportsMixing` get/set (device-scoped; not every device has it — probe with `AudioObjectHasProperty` first).

Probe algorithm, per output device: capabilities → output streams → available physical formats; hog; **save** current physical format, virtual format, and mixing state; for each signed-integer physical candidate at 44.1/48/96/192kHz: set physical, attempt virtual = same ASBD, read both back, record accepted/refused with the full flags word (`IsSignedInteger`, `IsPacked`, `IsAlignedHigh` matter for the packer); **restore** everything; drop the hog. Print a table.

Run on: the Matrix Mini-i Pro 4 (the target), built-in speakers (expected refusal — exercises the gating story), the Lotoo if docked. Paste findings into feature 32's spec. **This stage is the gate**: if no device accepts an integer virtual format on current macOS, the milestone pivots (see Risks) before any engine code exists.

## Stage 2 — hal guards

- `FormatRestoreGuard` (RAII, `HogGuard` style): captures physical + virtual + mixing at construction, restores in `Drop`. mpv restores formats on teardown; Pulse must too — the device outlives the app, and leaving a DAC in a 192k integer state with mixing off breaks every other app. Held by the integer engine for its lifetime, dropped on stop/teardown — and the backend is retained across pause (stage 3), so the guard spans pauses and fires only on `Stop`, device/engine switch, or quit.
- Unit-testable pieces (format matching, flag checks) get tests; property calls themselves are hardware-only.

## Stage 3 — sink and engine (`pulse-engine`)

- **`raw_sink.rs`** — `RawSink`, mirroring `AuhalSink`'s surface (`start(device, consumer, device_format, …)`, `position_frames`, `underrun_frames`, `stop`, `Drop` stops): `AudioDeviceCreateIOProcID` + `AudioDeviceStart`. The IOProc receives an `AudioBufferList` to fill: pop from the ring, zero-fill the remainder, count underrun bytes, bump position atomics. Realtime rules identical to the AUHAL callback. **No `GainProcessor`** — samples are never multiplied.
- **`integer_engine.rs`** — `IntegerEngine`, mirroring `Engine`'s surface so the backend wrapper is mechanical:
  - `open`: hog **mandatory** — `owns()` false or acquire failure is a hard `Hogged` error, no shared fallback; mixing off via the guard; hardware-volume probe as today.
  - `set_format`: set the nominal sample rate explicitly (the AUHAL path gets this from `set_matching_physical_format`; the integer path owns it); pick the integer physical format (≥ source bits, prefer exact width, then smallest sufficient container); set physical; set virtual = physical; build the `IntPacker`; size the ring at 4s × device bytes-per-frame.
  - `IntPacker` (replaces `FloatPacker`): source width → device container. 16→16 and 24→24-packed are copies; 24→32 and 16→32 shift per the container's alignment flags (`IsAlignedHigh` vs low-aligned — resolve against stage 1 findings, fixture-test byte-exactly). No arithmetic on sample values, ever — shifts and sign extension only.
  - `feed`: repack + `push_partial_slice`, same backpressure contract.
  - `play`/`pause`/`stop`: direct sink start/stop, **no fades** — clicks accepted per the spec.
  - **Pause holds the device** (Jason, 2026-08-31 — the Audirvana model). The controller today drops the backend box on pause (`Worker::pause` → `release_backend`, `controller.rs:1115`), which on this engine would cycle hog + formats every pause. Instead: the trait already splits park from release — `stop()` parks (for `IntegerBackend`: `AudioDeviceStop`, hog + formats + mixing state untouched, restore guard stays armed) and *dropping* the box releases. So the change is controller policy only: `PlaybackBackend` gains a retention hook (e.g. `retains_device_when_paused()` — `EngineBackend` false, `IntegerBackend` true); `Worker::pause` calls `stop()` and keeps the box for a retaining backend; `ensure_backend` reuses it on resume and `IntegerBackend::start` skips format setup when the format is unchanged, so resume is `AudioDeviceStart` — instant, no relay click, no re-lock. Explicit `Stop`, device/engine switch, and quit still drop the box (restore guard fires there). Device unplugged while paused-holding surfaces on resume as a start failure → clear error, same as today's reopen failure. Controller tests: pause retains for a retaining backend and releases otherwise; resume reuses the retained backend; `Stop` and device switch always release.
  - `set_volume`: hardware volume when present; otherwise a no-op (the app disables the slider per feature 31's indicator; the engine never multiplies).
- Controller: `EngineKind { Universal { exclusive_mode: bool }, BitPerfect }` — the kind absorbs today's `exclusive_mode` bool so `BitPerfect` + shared is unrepresentable (hog is part of the kind, not a flag to validate). `PlaybackCommand::SetOutputDevice` carries the kind in place of `exclusive_mode`, and `SetExclusiveMode` applies only under `Universal` (protocol change — update app dispatch and controller tests); `spawn` takes the initial kind; the factory signature changes from `(DeviceId, bool)` to `(DeviceId, EngineKind)` and selects `EngineBackend` vs `IntegerBackend`. Runtime start failure on a device that advertised integer support → clear `Error` event, no silent float fallback.
- Gapless: the controller's same-format splice is backend-agnostic and may simply work; it stays out of the promise. Format changes take the rebuild path, which also exercises the restore guard.
- Dropout reporting and the stall watchdog read the same counters — unchanged.

## Stage 4 — app (`pulse-app`)

- `settings.rs`: `StoredDevicePreferences` gains `engine: Option<StoredEngineKind>` (serde, `None` = Universal), accessors beside `exclusive_mode_override`.
- Capability plumbing: the probe today collapses to `(max_bits_per_channel, max_sample_rate)` (`hal.rs::output_device_capabilities`) and discards the format list. It must additionally report the gating signal, and `StoredDeviceCapabilities` gains the matching field; stored entries without it re-probe on next sight of the device.
- **The gating criterion is provisional until stage 1 reports.** The spec's "no integer physical formats" test likely does not gate built-in speakers — they typically do report integer physical formats; what they refuse (if anything) is the integer *virtual* format, and that can only be probed under hog, which is too intrusive for a background capability probe. If stage 1 confirms, gate on transport type (built-in/Bluetooth classes excluded) combined with integer physical formats present, and let start-time failure with a clear error cover the remainder.
- Devices page: an Engine control per device row — enabled only when the stored gating signal allows it; disabled state carries the capability note. **Pencil pass first** (small: one control + one disabled state). The pass also decides the existing Exclusive control's interplay when Engine = Bit-perfect: hog is mandatory, so exclusive is implied — lock the control on or hide it, but don't leave a dead toggle.
- Session/app_store: thread the kind through boot and `SetOutputDevice`; engine change on the active device restarts playback.
- Feature 31 indicator: "bit-perfect" state = integer engine + hog held + virtual format confirmed. If 31 isn't built yet, land the state plumbing and let 31 render it.

## Stage 5 — acceptance

Payload prep: source material is `~/qobuz/04 - 暧昧.dff` (DSD64, ~4½ min). A dev-only script (outside the product — Pulse stays PCM-only) parses the DFF stream, DoP-packs it (16 DSD bits per 24-bit sample at 176.4kHz, `0x05`/`0xFA` markers in the top byte), writes 24-bit/176.4k WAV, and `flac` encodes it losslessly. To Pulse the result is an ordinary PCM FLAC.

DoP test on the Matrix: the DoP FLAC through the integer engine → display flips to DSD64 (any bit corruption destroys the markers, so the flip is the proof). Control: the same file through the AUHAL engine at unity — if the display stays at 176.4kHz PCM, that demonstrates the exact gap this engine closes. Repeat plain PCM at 44.1/16 and 96/24 watching the rate display. Record results in the spec; only a pass permits any bit-perfect wording in the UI. Also verify teardown: after quitting Pulse, Audio MIDI Setup shows the device's prior formats restored.

## Verification

- `make verify` green per stage; `IntPacker` fixture tests (known bytes in, exact bytes out, all width/alignment combinations from the probe findings); controller tests against a mock integer backend for the command/protocol change.
- Stage 1 probe table archived in feature 32's spec before stage 3 starts.
- Manual: engine switch restarts cleanly; speakers/Bluetooth show the disabled control; volume slider hardware-routed or disabled; pause clicks are expected and documented. Pause holds the device (see stage 3): while paused, the hog is still held (another app cannot open the device) and resume is instant with no relay click; formats restore only after `Stop`, device/engine switch, or quit — that is when the Audio MIDI Setup check applies.

## Risks

- **Modern macOS may refuse integer virtual formats** (Apple has narrowed hog-mode privileges before). Stage 1 answers cheaply; if refused everywhere, the milestone records the finding in the spec and stops — the honest outcome is "not possible on this OS," not a float engine wearing the label.
- IOProc buffer-list semantics (multiple streams, non-interleaved layouts, variable frame counts) — mpv's callback is the reference for the edge cases.
- Restore-on-kill: the guard cannot run on SIGKILL/crash. macOS releases hog on process death; formats may persist — document, and consider a launch-time restore of any device Pulse left configured.
- Alignment flags vary by device; the packer must be driven by the probed flags, not assumptions.
- The `SetOutputDevice` protocol change ripples through app dispatch and the controller test suite; budget for test updates.
