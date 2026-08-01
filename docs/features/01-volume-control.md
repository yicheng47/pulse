# Volume Control

> Feature 01 · P1 · discovered missing during the stage 11-12 acceptance passes. Not in `mvp.md`'s Now Playing list — an omission, not a decision: only volume *leveling* (automatic loudness normalization) was excluded, and that exclusion stands.

## Motivation

The app has no way to adjust loudness. With a desktop DAC the hardware knob covers it, but Pulse now supports AirPods-class Bluetooth outputs (#11), where the app is the natural volume surface. A player that requires macOS system controls to change volume feels broken, and during exclusive/hog playback the system volume UI does not reliably govern the hogged device anyway.

## Scope

- **Software gain in the engine's existing float32 render path.** One multiply per sample inside the fill loop the AUHAL callback already runs — no allocation, no locks, no syscalls, per the standing IOProc constraints. The target gain crosses threads as an atomic (f32 bits in an `AtomicU32`); the callback ramps toward it over ~10 ms per buffer to avoid zipper noise.
- **Unity is pass-through.** Default is 100%, where the multiply is skippable and today's sample-for-sample behavior is preserved — purists leave the slider alone and lose nothing. This does not weaken any claims: `AGENTS.md` already forbids hard bit-perfect claims for the AUHAL float path.
- **Perceptual taper.** Slider position maps to gain with a cubic (or dB) curve, not linear, with a floor around −60 dB before hard mute.
- **UI in the playback row's Output Status cluster**: volume icon (click = mute toggle, icon reflects level/muted) plus a compact slider, using the seek bar's existing drag interaction language. Requires a Pencil pass on the Playback Bar component (`qKkw7`) first — the project rule stands: no UI before design.
- **Persistence**: one global level + mute flag in the existing app preferences alongside the output-device UID, restored on launch.
- **Controller surface**: a `SetVolume` command on the existing engine command path; volume survives pause/resume/seek/track changes and device switches.

## Non-Goals

- Volume leveling, replay gain, loudness normalization — excluded by `mvp.md`, stays excluded.
- Hardware/device volume via Core Audio (`kAudioDevicePropertyVolumeScalar`) and a purist "fixed output" mode that disables attenuation entirely — the audiophile-correct end state, but it adds a per-device HAL capability surface with real device variance. Future feature; the unity-default software gain is the honest interim.
- Per-device volume memory (AirPods at 60%, DAC at 100%) — worth doing once device volume exists; global level for v1.
- Media-key / system volume-key integration.

## Implementation Phases

1. **Design**: add the volume cluster to `qKkw7` in `design/pulse-desktop.pen`. Gated until the stage 12 mission lands — it is actively reading that component.
2. **Engine**: atomic gain + ramp in the render path, `SetVolume` on the controller, unity default. Tests with synthesized buffers: unity is sample-identical, ramp stays monotonic and bounded within a buffer, mute reaches silence.
3. **App**: slider + mute UI per the design, preference persistence, wire to the controller. View-model tests for taper mapping and icon states.

## Verification

- `make verify` green; the unity pass-through equality test is the load-bearing one.
- Manual: audible level tracks the slider on both the Matrix mini-i Pro 4 and AirPods; no zipper noise while dragging during playback; mute/unmute instant; level and mute survive relaunch; unity behaves identically to pre-feature builds.
