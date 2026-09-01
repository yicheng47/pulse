# Integer Bit-Perfect Engine

P2. A challenge milestone (Jason, 2026-08-31): a second, user-selectable playback engine that delivers source samples to the DAC bit-exactly, alongside the existing AUHAL engine. Promotes the parked raw-HAL experiment from the MVP roadmap's parking lot.

## Motivation

The AUHAL engine's honest claim stops at the float32 client boundary: Core Audio owns the f32 → physical conversion, so Pulse cannot claim bit-perfect delivery even at unity gain (`docs/arch/pulse-engine.md`). The raw HAL path can close that gap: hog the device, disable mixing, set an integer physical format, and — the decisive step the 0003-era attempt missed — set the stream's **virtual format equal to the integer physical format** while hogged, so the IOProc buffer accepts source integers directly. This is the documented approach of the 2010s "Integer Mode" players and of mpv's exclusive Core Audio output.

Audience: Jason. This is a correctness challenge and a learning vehicle, not a mass-market feature. The AUHAL engine remains the default.

## References

Reference clones live read-only in `~/repos/audio/`, kept outside pulse-src so GPL code never enters the project tree. `mpv/`, `MPD/`, and `SFBAudioEngine/` still need cloning (not present as of 2026-08-31); `hummingbird/` is already there.

- mpv `audio/out/ao_coreaudio_exclusive.c` — hog, `kAudioDevicePropertySupportsMixing`, physical format, raw IOProc (GPL: ideas only, no code).
- MPD `src/output/plugins/OSXOutputPlugin.cxx` — hog + physical format + DoP fallback ladders (GPLv2: ideas only).
- sbooth `SFBAudioEngine` and `Play` — permissively licensed macOS engines; Play records the original Integer Mode era.
- hummingbird — a native macOS bit-perfect player, already cloned; a fourth reference for the hog + format-switch flow.

## Scope

- **Per-device Output mode control** (Jason, 2026-08-31, design pass — supersedes feature 08's separate Exclusive toggle): one 3-option segmented control per device — `Shared · Exclusive · Bit-perfect` — one axis: shared AUHAL, hogged AUHAL, hogged integer. It lives in each device card's settings line on the Devices page and (as `Mode`) in the output-device popover's current-device card. Default is **Auto**: stored mode `None` resolves down the ladder — transport + format gate passes → Bit-perfect; integer physical formats but gated transport or refused virtual → Exclusive; float-only → Shared (feature 08's probe defaults carry over). Pinning any segment drops the AUTO tag (`Reset to Auto` returns); one axis means no cross-control interaction rules — hog is implied by the mode itself. Gating, resolved by the Stage 1 probe: the Bit-perfect segment is disabled with a `NO INTEGER PATH` tag unless the device reports signed-integer physical formats and its transport is neither display class (DisplayPort/HDMI) nor Bluetooth — built-in speakers fall out via float-only formats, while all three DELL displays advertise integer physical formats but refuse the matching integer virtual format. The existing `max_bits_per_channel: Some(_)` capability answers the format half; Stage 4 adds transport class to the probed and stored capabilities. Start-time virtual-format failure with a clear error remains the backstop for devices whose formats changed since probing. Storage: per device UID in `settings.json`; the legacy `exclusive_mode_override` and any engine key migrate into the one stored mode. The mode maps 1:1 onto the engine protocol — Shared → `Universal{shared}`, Exclusive → `Universal{exclusive}`, Bit-perfect → `BitPerfect` — resolution app-side, the controller always receives a resolved kind, the `BackendFactory` seam (`controller.rs::spawn`) picks the backend. Changing the active device's mode restarts playback. The bit-perfect *wording* in the UI remains gated on the phase-4 DoP pass.
- **The integer engine**: raw HAL `AudioDeviceCreateIOProcID` path. Requirements, in order: hog (mandatory — refuse to start without it), disable mixing where the device supports the property, set integer physical format matching the source (wider container allowed, e.g. 24-in-32), set virtual format = physical format, feed source integers with no intermediate float stage.
- **Purity rules**: no software gain in this engine — volume is hardware volume (feature 31's domain-2 path) when the device offers it, otherwise fixed at 100% with the limitation disclosed in the UI. No fades: pause/seek/stop may click; that is the accepted cost of an untouched sample path and is documented in the engine picker's description.
- **Pause holds the device** (Jason, 2026-08-31 — the Audirvana model): hog mode means the user dedicated the device to Pulse, so pause keeps the hog, formats, and mixing state and only stops the IO — resume restarts it, instant, with no relay click or re-lock. The flip side: other apps cannot use the device while Pulse is paused on this engine; that is the point, and the picker copy discloses it alongside the no-fade clicks. Explicit Stop, switching device or engine, and quit release the device and restore its prior formats. The Universal engine keeps its existing release-on-pause behavior — the cost is invisible there, and by the same hog-implies-hold principle AUHAL-exclusive could also hold, but that shipped path stays untouched in this milestone.
- **Fallbacks**: capability gating keeps unsupported devices from ever offering the engine, so runtime fallback is the rare case (a device's formats changed since the probe): fall back to Universal with a notice. If the virtual-format-integer write is refused at start on a device that advertised integer formats, report a clear error rather than silently reverting to float.
- **Feature 31 integration**: the signal-path indicator reads "bit-perfect" only on this engine with hog held and integer virtual format confirmed; the AUHAL engine keeps its existing "untouched samples leave Pulse" ceiling.

## Non-Goals

- No bit-perfect claims in the UI before the phase-4 DoP pass — Auto may select the engine, the wording still waits for the proof.
- No DSD playback in this feature (the DoP test file is a verification payload here); DSD-over-DoP support is [feature 33](33-dsd-over-dop.md), layered on this engine after the phase-4 acceptance pass.
- No gapless guarantees in v1 of this engine (same-format splice may come later; format changes rebuild as today).
- No attempt to keep software volume, EQ, or fades working in bit-perfect mode.

## Implementation Phases

Detailed tactical plan: [`docs/impls/bit-perfect/`](../impls/bit-perfect/README.md) — five staged mission docs with a journey log ([`IMPL_LOG.md`](../impls/bit-perfect/IMPL_LOG.md)).

1. **Probe spike**: a dev-only example binary (`cargo run -p pulse-engine --example integer_probe`) that hogs the Matrix, walks physical formats, attempts virtual = physical (integer), and reports what the device/OS accepts. Findings recorded in the spec before any engine code. Reading pass over the references first (clone the missing ones).
2. **Engine**: `pulse-engine` gains the integer backend behind the existing `PlaybackBackend` trait — hog, mixing off, format setup, integer ring (bytes at source width, no `FloatPacker`), IOProc callback honoring the realtime rules. Reuses `hal.rs` and extends it with the virtual-format and mixing properties.
3. **Settings + wiring**: per-device Engine control on the Devices page (UID-keyed persistence, capability gating with the disabled-state note), controller factory selection, runtime-fallback notice, feature 31 indicator states.
4. **Hardware validation**: the DoP test — a DSD64-in-DoP FLAC played through the integer engine on the Matrix Mini-i Pro 4 must flip its display to DSD64. That single observation proves bit-exact delivery through engine, Core Audio, driver, and USB. Repeat at 44.1/16 and 96/24 with the rate display; record results in the spec.

## Verification

- `make verify` green; the integer packer and format-selection logic get the same fixture-test treatment as `decode.rs` (known bytes in, exact bytes out).
- Phase 1 probe output archived in this spec (what the Matrix accepts for virtual formats).
- Phase 4 DoP pass on the Matrix is the acceptance gate for the bit-perfect claim; without it the engine ships labeled experimental with no claim.
- Manual: engine switch on the active device restarts playback cleanly; built-in speakers and Bluetooth show the Bit-perfect option disabled with the capability note; volume slider disabled or hardware-routed per device, per feature 31's indicator; while paused the device stays hogged (another app cannot open it) and resume is instant; after Stop or quit the device's prior formats are restored in Audio MIDI Setup.

## Stage 5 acceptance results

- **2026-09-01 — THE DoP TEST PASSES.** `04 - 暧昧 (DoP).flac` (DSD64-in-DoP, structurally verified source-exact by `script/dop_pack.py --verify`) played through the integer engine on the Matrix Mini-i Pro 4 (Auto-resolved Bit-perfect): the display reads **"DSD DoP 2.8MHz"** — the DAC detected intact DoP markers and unwrapped DSD64, proving bit-exact delivery through decode, `IntPacker`, ring, IOProc, Core Audio HAL, driver, and USB. Any single-bit corruption would have destroyed the markers. The bit-perfect claim in the UI is hereby legitimate.
- Also verified 2026-08-31 during first hardware contact: plain FLAC plays clean through the integer engine; the Matrix has no settable volume scalar while hogged (feature 31's `FIXED` camp).
- Remaining manual sweep (pending): Shared-mode control test on the same file (display must stay 176.4 kHz PCM), 44.1/16 and 96/24 rate checks, pause-hold observation, format restore in Audio MIDI Setup after quit, feature 31 UX pass.

## Stage 1 probe findings

Run on macOS 26.5.2 arm64 on 2026-08-31 with `cargo run -p pulse-engine --example integer_probe`. The probe saved and restored each stream's physical and virtual ASBD plus the device mixing state when that property existed, verified every readback, released each hog, and exited successfully. The Matrix Mini-i Pro 4 appeared as `mini-i Series`; built-in output appeared as `Mac Studio Speakers`; the Lotoo was not docked. The exact per-candidate stdout, including device UIDs, full ASBD readbacks, flags words, and setter errors, is archived verbatim in [`stage-1-probe-output.md`](../impls/bit-perfect/stage-1-probe-output.md).

| Device | Capabilities | Streams | Physical formats | Virtual formats | Mixing property | Restore |
|---|---:|---:|---:|---:|---|---|
| DELL U2725QE | 24-bit integer / 192.0 kHz | 1 | 21 | 7 | unavailable | verified |
| DELL U2725QE | 24-bit integer / 192.0 kHz | 1 | 21 | 7 | unavailable | verified |
| DELL U2718Q | 24-bit integer / 96.0 kHz | 1 | 15 | 5 | unavailable | verified |
| Scarlett 2i2 4th Gen | 24-bit integer / 192.0 kHz | 1 | 12 | 12 | unavailable | verified |
| Matrix Mini-i Pro 4 (`mini-i Series`) | 32-bit integer / 768.0 kHz | 1 | 40 | 30 | unavailable | verified |
| Mac Studio Speakers | mixable float / 96.0 kHz | 1 | 4 | 4 | unavailable | verified |
| OrayVirtualAudioDevice | mixable float / 768.0 kHz | 1 | 12 | 12 | unavailable | verified |

| Device | Rates probed | Signed-integer physical result | Integer virtual result | Full relevant flags words |
|---|---|---|---|---|
| DELL U2725QE (two devices) | 44.1 / 48 / 96 / 192 kHz | 16/20/24-bit candidates accepted | All refused; virtual stayed float32; the refused virtual write changed 16/20-bit physical formats to 24-bit | Physical `0x0000000c` or `0x00000004`; virtual readback `0x00000009` |
| DELL U2718Q | 44.1 / 48 / 96 kHz | 16/20/24-bit candidates accepted | All refused; virtual stayed float32; the refused virtual write changed 16/20-bit physical formats to 24-bit | Physical `0x0000000c` or `0x00000004`; virtual readback `0x00000009` |
| Scarlett 2i2 4th Gen | 44.1 / 48 / 96 / 192 kHz | Both 24-bit candidates accepted | Non-mixable candidate accepted at every rate; mixable candidate refused | Accepted `0x00000054`; refused `0x00000014`; float readback `0x00000009` |
| Matrix Mini-i Pro 4 (`mini-i Series`) | 44.1 / 48 / 96 / 192 kHz | All 24-bit and 32-bit candidates accepted | Non-mixable 24-bit and 32-bit candidates accepted at every rate; mixable candidates refused; the refused mixable 24-bit virtual write changed physical to packed 32-bit | Accepted 24-bit `0x00000054`; accepted 32-bit `0x0000004c`; refused 24-bit `0x00000014`; refused 32-bit `0x0000000c`; float readback `0x00000009` |
| Mac Studio Speakers | 44.1 / 48 / 96 / 192 kHz | No signed-integer physical candidate | Not attempted | No integer flags word |
| OrayVirtualAudioDevice | 44.1 / 48 / 96 / 192 kHz | No signed-integer physical candidate | Not attempted | No integer flags word |

The accepted Matrix ASBDs are stereo interleaved LPCM with 8 bytes per frame: 24 valid bits aligned high in a 32-bit container at flags `0x00000054` (`IsSignedInteger | IsAlignedHigh | IsNonMixable`), and packed 32-bit at flags `0x0000004c` (`IsSignedInteger | IsPacked | IsNonMixable`). These exact flags drive Stage 3's packer. Stage 4 gates on both transport and format: display-class and Bluetooth devices are disabled even if they advertise integer physical formats, while built-in output also falls out from the absence of signed-integer physical formats; start-time virtual refusal remains a backstop for changed or misreported capabilities.

`kAudioDevicePropertySupportsMixing` was absent on every connected output, so disabling/restoring mixing is a no-op on this hardware and the setter reaches Stage 3 without hardware coverage. The probe followed the Stage 1 stream-property algorithm and mpv's raw path by putting each target rate directly in the physical ASBD; unlike Pulse's current AUHAL exclusive sequence, it did not set `kAudioDevicePropertyNominalSampleRate` first. Stage 5 hardware acceptance must therefore validate the final engine's nominal-rate-plus-stream-format sequence.

Every refused virtual-format write returned `noErr` from `AudioObjectSetPropertyData` but failed to settle before the two-second `FORMAT_SETTLE_TIMEOUT`. Stage 3 must treat matching readback as acceptance rather than trusting setter status, and Stage 4's start-time backstop may take two seconds before it can surface the error.
