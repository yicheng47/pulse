# Volume Control

> Feature 01 · P1 · discovered missing during the stage 11-12 acceptance passes. Not in `mvp.md`'s Now Playing list — an omission, not a decision: only volume *leveling* (automatic loudness normalization) was excluded, and that exclusion stands.

## Motivation

The app has no way to adjust loudness. With a desktop DAC the hardware knob covers it, but during exclusive/hog playback the system volume UI does not reliably govern the hogged device, and shared-mode playback (the exclusive-mode toggle shipped in v0.1.1) makes the app the natural volume surface. That gets more acute when AirPods-class Bluetooth outputs become playable — issue #11 fixed only their capability copy; actual BT playback support is still ahead. A player that requires macOS system controls to change volume feels broken.

## Design source

`design/pulse-desktop.pen`, read via the `pencil` MCP only. The bar carries an **icon only** — the Volume node (`g0e4eO`) in the Playback Bar's Output Status cluster (`MvhQm`, width 300), ordered `Format | divider | volume icon | divider | output icon | divider | queue` with 1×24 `$border` dividers. Clicking the icon opens the **Volume Popover** (`IChZh`): output-popover chrome (`$bg-surface`, `radius-lg`, `$border`, padding 14), a `%` readout in mono, a **vertical** 4×120 slider in the seek bar's track/fill idiom (`$bg-inset` track, `$accent` fill rising from the bottom), and a `volume-x` mute button at the foot. Anchored above the icon, opening upward, with the queue popover's dismissal pattern (outside click, Escape, press-closed guard). Bar-icon thresholds per **`Spec — Volume States`** (`IAPbC`): `volume-2` ≥50%, `volume-1` <50%, `volume-x` muted or zero. Icon thresholds and the muted rendering are fixed in **`Spec — Volume States`** (`IAPbC`): `volume-2` at ≥50%, `volume-1` below 50%, `volume-x` with muted-tint icon and empty fill when muted or at zero, level remembered across mute.

## Scope

- **Software gain in the engine's existing float32 render path.** One multiply per sample inside the fill loop the AUHAL callback already runs — no allocation, no locks, no syscalls, per the standing IOProc constraints. The target gain crosses threads as an atomic (f32 bits in an `AtomicU32`); the callback ramps toward it over ~10 ms per buffer to avoid zipper noise.
- **Unity is pass-through.** Default is 100%, where the multiply is skippable and today's sample-for-sample behavior is preserved — purists leave the slider alone and lose nothing. This does not weaken any claims: `AGENTS.md` already forbids hard bit-perfect claims for the AUHAL float path.
- **Perceptual taper.** Slider position maps to gain with a cubic (or dB) curve, not linear, with a floor around −60 dB before hard mute.
- **UI: a volume icon in the playback row's Output Status cluster that opens a popover** with a vertical slider and a mute button — matching the bar's established icon-opens-popover language (output device, queue). The icon reflects level and muted state; the slider uses the seek bar's drag interaction, vertically. Designed — see Design source.
- **Persistence**: one global level + mute flag in the existing app preferences alongside the output-device UID, restored on launch.
- **Controller surface**: a `SetVolume` command on the existing engine command path; volume survives pause/resume/seek/track changes and device switches.

## Non-Goals

- Volume leveling, replay gain, loudness normalization — excluded by `mvp.md`, stays excluded.
- Hardware/device volume via Core Audio (`kAudioDevicePropertyVolumeScalar`) and a purist "fixed output" mode that disables attenuation entirely — the audiophile-correct end state, but it adds a per-device HAL capability surface with real device variance. Future feature; the unity-default software gain is the honest interim.
- Per-device volume memory (AirPods at 60%, DAC at 100%) — worth doing once device volume exists; global level for v1.
- Media-key / system volume-key integration.

## Implementation Phases

1. **Design**: add the volume cluster to `qKkw7` in `design/pulse-desktop.pen`. (The old stage-12 gate on this component is long lifted.)
2. **Engine**: atomic gain + ramp in the render path, `SetVolume` on the controller, unity default. Tests with synthesized buffers: unity is sample-identical, ramp stays monotonic and bounded within a buffer, mute reaches silence.
3. **App**: slider + mute UI per the design, preference persistence, wire to the controller. View-model tests for taper mapping and icon states.

## Verification

- `make verify` green; the unity pass-through equality test is the load-bearing one.
- Manual: audible level tracks the slider on both the Matrix mini-i Pro 4 and AirPods; no zipper noise while dragging during playback; mute/unmute instant; level and mute survive relaunch; unity behaves identically to pre-feature builds.
