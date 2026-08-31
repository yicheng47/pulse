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

- **Per-device engine selection** (Jason, 2026-08-31 — the Roon/JRiver model): each device on the Devices page gets an Engine control — `Universal (default)` vs `Bit-perfect (experimental)` — following feature 08's per-device exclusive-mode pattern. Capability-gated: the Bit-perfect option is disabled, with a capability note, on devices that cannot take the engine (built-in speakers, Bluetooth class). The gating signal is provisional until the phase-1 probe reports: built-in speakers likely *do* report integer physical formats — what they refuse is the integer virtual format, which can only be probed under hog, too intrusive for a background capability probe — so the expected gate is transport type combined with integer physical formats present, with start-time failure + clear error covering the remainder. Either way the capability probe and stored capabilities must be extended (today they reduce to max bits/rate). Stored per device UID in `settings.json`; changing the active device's engine restarts playback. Hog is mandatory on this engine, so the existing per-device Exclusive control is implied-on under Bit-perfect — the design pass decides whether it locks on or hides, but it must not remain a dead toggle. The controller's `BackendFactory` seam (`controller.rs::spawn`) already abstracts the engine behind `Box<dyn PlaybackBackend>`; the per-device setting picks the factory. Universal stays the default on every device while the engine is experimental; flipping the default for integer-capable DACs is a follow-up decision gated on the phase-4 DoP pass.
- **The integer engine**: raw HAL `AudioDeviceCreateIOProcID` path. Requirements, in order: hog (mandatory — refuse to start without it), disable mixing where the device supports the property, set integer physical format matching the source (wider container allowed, e.g. 24-in-32), set virtual format = physical format, feed source integers with no intermediate float stage.
- **Purity rules**: no software gain in this engine — volume is hardware volume (feature 31's domain-2 path) when the device offers it, otherwise fixed at 100% with the limitation disclosed in the UI. No fades: pause/seek/stop may click; that is the accepted cost of an untouched sample path and is documented in the engine picker's description.
- **Pause holds the device** (Jason, 2026-08-31 — the Audirvana model): hog mode means the user dedicated the device to Pulse, so pause keeps the hog, formats, and mixing state and only stops the IO — resume restarts it, instant, with no relay click or re-lock. The flip side: other apps cannot use the device while Pulse is paused on this engine; that is the point, and the picker copy discloses it alongside the no-fade clicks. Explicit Stop, switching device or engine, and quit release the device and restore its prior formats. The Universal engine keeps its existing release-on-pause behavior — the cost is invisible there, and by the same hog-implies-hold principle AUHAL-exclusive could also hold, but that shipped path stays untouched in this milestone.
- **Fallbacks**: capability gating keeps unsupported devices from ever offering the engine, so runtime fallback is the rare case (a device's formats changed since the probe): fall back to Universal with a notice. If the virtual-format-integer write is refused at start on a device that advertised integer formats, report a clear error rather than silently reverting to float.
- **Feature 31 integration**: the signal-path indicator reads "bit-perfect" only on this engine with hog held and integer virtual format confirmed; the AUHAL engine keeps its existing "untouched samples leave Pulse" ceiling.

## Non-Goals

- No change to the default engine; Universal (AUHAL) stays unless the user opts in.
- No DSD playback in this feature (the DoP test file is a verification payload here); DSD-over-DoP support is [feature 33](33-dsd-over-dop.md), layered on this engine after the phase-4 acceptance pass.
- No gapless guarantees in v1 of this engine (same-format splice may come later; format changes rebuild as today).
- No attempt to keep software volume, EQ, or fades working in bit-perfect mode.

## Implementation Phases

Detailed tactical plan: [`docs/impls/0016-integer-engine.md`](../impls/0016-integer-engine.md).

1. **Probe spike**: a dev-only example binary (`cargo run -p pulse-engine --example integer_probe`) that hogs the Matrix, walks physical formats, attempts virtual = physical (integer), and reports what the device/OS accepts. Findings recorded in the spec before any engine code. Reading pass over the references first (clone the missing ones).
2. **Engine**: `pulse-engine` gains the integer backend behind the existing `PlaybackBackend` trait — hog, mixing off, format setup, integer ring (bytes at source width, no `FloatPacker`), IOProc callback honoring the realtime rules. Reuses `hal.rs` and extends it with the virtual-format and mixing properties.
3. **Settings + wiring**: per-device Engine control on the Devices page (UID-keyed persistence, capability gating with the disabled-state note), controller factory selection, runtime-fallback notice, feature 31 indicator states.
4. **Hardware validation**: the DoP test — a DSD64-in-DoP FLAC played through the integer engine on the Matrix Mini-i Pro 4 must flip its display to DSD64. That single observation proves bit-exact delivery through engine, Core Audio, driver, and USB. Repeat at 44.1/16 and 96/24 with the rate display; record results in the spec.

## Verification

- `make verify` green; the integer packer and format-selection logic get the same fixture-test treatment as `decode.rs` (known bytes in, exact bytes out).
- Phase 1 probe output archived in this spec (what the Matrix accepts for virtual formats).
- Phase 4 DoP pass on the Matrix is the acceptance gate for the bit-perfect claim; without it the engine ships labeled experimental with no claim.
- Manual: engine switch on the active device restarts playback cleanly; built-in speakers and Bluetooth show the Bit-perfect option disabled with the capability note; volume slider disabled or hardware-routed per device, per feature 31's indicator; while paused the device stays hogged (another app cannot open it) and resume is instant; after Stop or quit the device's prior formats are restored in Audio MIDI Setup.
