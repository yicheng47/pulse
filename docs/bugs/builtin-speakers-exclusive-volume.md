# Exclusive mode on the built-in speakers disables the macOS volume control and freezes the hardware level

> Bug · P2 · filed 2026-08-22 as GitHub issue #59 (closed when tracking moved to docs, 2026-08-29). **Fixed in code** 2026-08-30, commit `38734b8` — awaiting Jason's hardware check on the built-in speakers and the Matrix DAC before archiving. Note: the "release the hog while paused" item was already true (pause releases the backend since the click-free work); the hardware level Pulse writes does not survive hog release on the speakers, so the device level is adopted only on the first hog per session and Pulse's slider is the source of truth afterwards.

## Description

With exclusive mode pinned on for the built-in **MacBook Pro Speakers**, macOS's own volume control goes dead for as long as Pulse holds the hog: the volume keys do nothing, the volume HUD / menu-bar slider show the output as unavailable, and `osascript -e 'output volume of (get volume settings)'` returns `missing value`. The speakers' hardware volume is frozen at whatever level it had when Pulse took the hog.

Pulse's own slider keeps working, but it is a software gain stage (`gain.rs`) that tops out at unity — so if the system volume was at 50% when the hog was taken, nothing in Pulse can get louder than 50% until the hog is released. Pulse never reads or writes the device's volume control (`kAudioDevicePropertyVolumeScalar` / `kAudioDevicePropertyMute`), even though the hog owner is the only process that can.

The hog is held from the first `play` through **pause** and is only released at track end, explicit stop, or a device / mode switch — so the system volume stays dead while Pulse sits paused.

The built-in speakers are a float-only device (no integer physical formats), so `default_exclusive_mode` classifies them as shared on Auto; this only bites once the user flips the exclusive toggle for them. But the same applies to any hogged device that exposes a volume control (most USB DACs, AirPods).

## Expected Behavior

While Pulse owns the hog on a device that exposes a volume control:

- Pulse's volume slider drives the device's hardware volume (`kAudioDevicePropertyVolumeScalar`, output scope, main element), keeping the software gain at unity. This is the path that stays bit-perfect-friendly and gets the full level range back.
- On taking the hog, Pulse reads the current hardware level / mute state so the slider reflects reality instead of the persisted software level.

Worth considering alongside:

- Release the hog while paused (hold it only while the IOProc runs) so the system control returns whenever Pulse is not actually playing.
- Exclusive mode on a float-only device buys nothing; the output popover could say that exclusive mode takes over the system volume for this device, or leave the toggle off for float-only outputs.

## Steps To Reproduce

1. Output device: MacBook Pro Speakers. Set the system volume to ~50%.
2. Open the output popover and pin exclusive mode on for the speakers (Auto is shared for this device).
3. Play any track.
4. Press the volume keys / open the menu-bar Sound slider → the control is greyed / shows the prohibited icon; `osascript -e 'output volume of (get volume settings)'` prints `missing value`.
5. Pause Pulse → still dead. Stop (or let the track end) → the system volume control comes back at the pre-hog level.
6. Drag Pulse's slider to 100% → playback never exceeds the pre-hog hardware level.

## Relevant Code

- `crates/pulse-engine/src/hal.rs:45-80` — `HogGuard::acquire` toggles `kAudioDevicePropertyHogMode` only; nothing in `hal.rs` touches the volume / mute controls.
- `crates/pulse-engine/src/engine.rs:41,56-58` — `_hog` is acquired in `Engine::open` and lives as long as the `Engine`.
- `crates/pulse-engine/src/controller.rs:703-722` — `stop_active` (pause) keeps the backend and therefore the hog; `release_backend` runs only from `stop`, `finish_playback`, and device / mode switches.
- `crates/pulse-engine/src/gain.rs` — volume is a software gain on the render path, `UNITY_GAIN` max.
- `crates/pulse-app/src/playback_row.rs:2921` — `default_exclusive_mode` is `false` for float-only devices; the speakers reach hog mode only via the per-device override (feature 08).
- `docs/features/archive/01-volume-control.md:7` — the volume spec already noted "during exclusive/hog playback the system volume UI does not reliably govern the hogged device" and scoped media-key integration out; it did not cover the hogged device's hardware control.

## Environment

- OS: macOS 26.5.1 (25F80)
- Device / DAC: MacBook Pro Speakers (built-in). Physical formats: float 32-bit only, 44.1 / 48 / 88.2 / 96 kHz, 2 ch. `VolumeScalar` present and settable on the output main element only (no per-channel controls).
- Input file format: any (not format-dependent)
- Pulse version: v0.1.4 (`6eef35f`)

## Verification

- Code inspection of the hog, engine, controller, and gain paths above.
- Standalone Swift probe that takes the hog exactly like `HogGuard::acquire` (toggle `kAudioDevicePropertyHogMode`) on the speakers:

```
before hog: main scalar=0.49999997 applescript=50
hogged by 30470 (me=30470): owner reads main scalar=0.49999997; applescript=missing value
while hogged: osascript 'set volume output volume 30' -> silently ignored, reads back 'missing value'
owner sets main scalar to 0.2: status=0 readback=0.19999999
released (owner=-1): main scalar=0.49999997 applescript=50
```

  So while hogged, other processes can neither read nor set the output volume (what the HUD shows), and the hog owner can set the hardware control without error. Pulse itself was not instrumented for this; the behavior reported in the UI matches the HAL result.

