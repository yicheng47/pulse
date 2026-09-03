# Volume lock lags the switch to Bit-perfect

P2 · filed 2026-09-03 from Jason's use of `main` at `0a2e3ad` during the feature 78 review session.

## Description

Switching the active device's output mode to Bit-perfect (the popover's Mode row, or Reset to Auto resolving to Bit-perfect) does not lock the volume control right away. The playback row and the volume popover keep the software slider until the next play starts; only then does the control read `Fixed · 100% — no gain stage`.

## Expected Behavior

The moment the switch is confirmed (`OutputDeviceChanged { kind: BitPerfect }`) the volume control shows the bit-perfect domain — locked, or `Device volume · DAC attenuator` when the device exposes a hardware volume control — regardless of whether a track is playing.

## Steps To Reproduce

1. With the Matrix selected in Shared or Exclusive, pause or stop playback (or launch fresh).
2. Choose audio output → Mode → Bit-perfect.
3. The volume slider is still live (software domain). Press play → it locks.

While playing, the switch restarts the track and the `Fixed` state lands in the same 16 ms poll, so the lag is invisible there; the bug shows only when no backend is started by the switch.

## Relevant Code

- `crates/pulse-engine/src/controller.rs` — `set_volume_state` is reached only from `start_backend` (on success), `start_shared_fallback`, and `release_backend` (reset to `VolumeState::default()`, which is `Software`). `set_output_device` / `set_exclusive_mode` while Paused call `release_backend` and start nothing; while Idle they touch neither. So the app sees `VolumeStateChanged(Software)` (or nothing) followed by `OutputDeviceChanged`, and the `Fixed` domain arrives only with the next `start_backend`.
- `crates/pulse-engine/src/integer_engine.rs` `volume_domain()` — `Device` when `hal::hardware_volume_control` finds a control, else `Fixed`; known only after `open`, which is why the engine cannot emit the final domain before a backend exists.
- `crates/pulse-app/src/backend/playback/controller.rs` — `volume_state` is written only by the `VolumeStateChanged` arm; `complete_output_device_change` does not touch it.
- `crates/pulse-app/src/surfaces/playback_row.rs`, `playback_popovers.rs` — the lock is purely `volume_state.domain == Fixed`.
- Fix direction, to settle in the mission: (a) the engine emits a provisional `VolumeStateChanged` alongside `OutputDeviceChanged`, derived from the kind — `BitPerfect → Fixed`, `Universal → Software` — and the eventual backend start corrects it to `Device` if a hardware control exists; or (b) the app derives the displayed lock from `playback_output_mode == BitPerfect` while no backend is running. Prefer (a): it keeps engine knowledge out of the app and matches how `bit_perfect_active` is already engine-owned.
- Not the lock feature 77 is about: this is the volume domain; 77 is the device hold (hog).

## Environment

- OS: macOS 26
- Device / DAC: Matrix Mini-i Pro 4 (USB)
- Input file format: any
- Pulse version: 0.3.0 (`main` at `0a2e3ad`)

## Verification

Mechanism confirmed by reading the paths above; not yet reproduced headless. The fix's fake-seam test: switch to `BitPerfect` while `Paused` and while `Idle` → `VolumeStateChanged(Fixed)` before any play; then a start on a device with a hardware control → `VolumeStateChanged(Device)`.
