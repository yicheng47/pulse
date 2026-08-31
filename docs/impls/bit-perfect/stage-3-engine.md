# Stage 3 — sink and engine (`pulse-engine`)

Entry gate: stage 1's findings table archived in feature 32, stage 2's guard merged.

Stage 1 probe inputs (see feature 32's findings and [`stage-1-probe-output.md`](stage-1-probe-output.md)):

- **Only non-mixable integer variants are accepted as virtual formats** — the Matrix takes 24-bit aligned-high `0x00000054` and packed 32-bit `0x0000004c`, and refuses the mixable twins. Format selection must prefer the `IsNonMixable` integer physical variant; non-mixability lives in the flags word, since `kAudioDevicePropertySupportsMixing` was absent on every probed device (the guard's mixing step no-ops there).
- **A refused virtual write returns `noErr` and then never settles** — acceptance is a matching readback within `FORMAT_SETTLE_TIMEOUT`, never the setter status; the start-time refusal backstop can take up to the 2s timeout before it can error.
- The probe set the target rate directly in the physical ASBD without `set_nominal_sample_rate` first and it worked; the engine keeps the explicit nominal-rate step below, and stage 5 validates that combined sequence on hardware.

- **`raw_sink.rs`** — `RawSink`, mirroring `AuhalSink`'s surface (`start(device, consumer, device_format, …)`, `position_frames`, `underrun_frames`, `stop`, `Drop` stops): `AudioDeviceCreateIOProcID` + `AudioDeviceStart`. The IOProc receives an `AudioBufferList` to fill: pop from the ring, zero-fill the remainder, count underrun bytes, bump position atomics. Realtime rules identical to the AUHAL callback. **No `GainProcessor`** — samples are never multiplied.
- **`integer_engine.rs`** — `IntegerEngine`, mirroring `Engine`'s surface so the backend wrapper is mechanical:
  - `open`: hog **mandatory** — `owns()` false or acquire failure is a hard `Hogged` error, no shared fallback; mixing off via the guard; hardware-volume probe as today.
  - `set_format`: set the nominal sample rate explicitly (the AUHAL path gets this from `set_matching_physical_format`; the integer path owns it); pick the integer physical format (≥ source bits, prefer exact width, then smallest sufficient container); set physical; set virtual = physical; build the `IntPacker`; size the ring at 4s × device bytes-per-frame.
  - `IntPacker` (replaces `FloatPacker`): source width → device container. 16→16 and 24→24-packed are copies; 24→32 and 16→32 shift per the container's alignment flags (`IsAlignedHigh` vs low-aligned — resolve against stage 1 findings, fixture-test byte-exactly). No arithmetic on sample values, ever — shifts and sign extension only.
  - `feed`: repack + `push_partial_slice`, same backpressure contract.
  - `play`/`pause`/`stop`: direct sink start/stop, **no fades** — clicks accepted per the spec.
  - **Pause holds the device** (Jason, 2026-08-31 — the Audirvana model). The controller today drops the backend box on pause (`Worker::pause` → `release_backend`, `controller.rs:1115`), which on this engine would cycle hog + formats every pause. Instead: the trait already splits park from release — `stop()` parks (for `IntegerBackend`: `AudioDeviceStop`, hog + formats + mixing state untouched, restore guard stays armed) and *dropping* the box releases. So the change is controller policy only: `PlaybackBackend` gains a retention hook (e.g. `retains_device_when_paused()` — `EngineBackend` false, `IntegerBackend` true); `Worker::pause` calls `stop()` and keeps the box for a retaining backend; `ensure_backend` reuses it on resume and `IntegerBackend::start` skips format setup when the format is unchanged, so resume is `AudioDeviceStart` — instant, no relay click, no re-lock. Explicit `Stop`, device/engine switch, and quit still drop the box (restore guard fires there). Device unplugged while paused-holding surfaces on resume as a start failure → clear error, same as today's reopen failure.
  - `set_volume`: hardware volume when present; otherwise a no-op (the app disables the slider per feature 31's indicator; the engine never multiplies).
- Controller: `EngineKind { Universal { exclusive_mode: bool }, BitPerfect }` — the kind absorbs today's `exclusive_mode` bool so `BitPerfect` + shared is unrepresentable (hog is part of the kind, not a flag to validate). `PlaybackCommand::SetOutputDevice` carries the kind in place of `exclusive_mode`, and `SetExclusiveMode` applies only under `Universal` (protocol change — update app dispatch and controller tests); `spawn` takes the initial kind; the factory signature changes from `(DeviceId, bool)` to `(DeviceId, EngineKind)` and selects `EngineBackend` vs `IntegerBackend`. Runtime start failure on a device that advertised integer support → clear `Error` event, no silent float fallback.
- Gapless: the controller's same-format splice is backend-agnostic and may simply work; it stays out of the promise. Format changes take the rebuild path, which also exercises the restore guard.
- Dropout reporting and the stall watchdog read the same counters — unchanged.

## Verification

- `make verify` green; `IntPacker` fixture tests (known bytes in, exact bytes out, all width/alignment combinations from the probe findings).
- Controller tests against a mock integer backend for the command/protocol change, and for pause retention: pause retains for a retaining backend and releases otherwise; resume reuses the retained backend; `Stop` and device switch always release.
