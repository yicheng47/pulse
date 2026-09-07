# Shared output volume does not follow the selected device

P2 · reported by Jason 2026-09-06 while testing the engine refactor: switching to Mac Studio Speakers leaves Pulse's slider unrelated to the macOS volume. Investigated after feature 93 merged in PR #94 (`1d9468f`). This is existing Shared-mode policy, not a refactor regression; the proposed behavior below is a product recommendation awaiting Jason's decision.

## Description

Mac Studio Speakers has no explicit output-mode override in the inspected release/dev settings. Its stored capability has no integer bit width, so it resolves to Shared. `AuhalEngine::open` probes device volume only when Pulse owns the hog; Shared always uses Pulse's software gain. The slider retains the app's global level, independently of the selected device's output volume. At 100% it supplies unity gain, not maximum device volume; below 100% it further attenuates the samples using the existing cubic curve.

The device-volume path is not continuously synchronized either: it reads the level at backend open, adopts it only once per device per worker session, and later reapplies the app level. No volume/mute property listener or polling updates the slider after changes made outside Pulse. The adopt-once rule came from the earlier built-in-speaker Exclusive fix, because that device's hogged level could revert on release; changing it globally without considering Exclusive would undo that behavior.

## Expected Behavior — Proposed

For Mac Studio Speakers in Shared mode, use the selected device's volume as the slider's source of truth: read level/mute on selection, update on external macOS changes, and write that device's control only when the user adjusts the slider. Keep steady-state app gain at unity on this path, including when paused/idle. A device switch should adopt the new device's level rather than write a persisted app level into it.

This changes the output volume for every app using that device. It is a deliberate product tradeoff, not a requirement imposed by macOS. Apple documents that ordinary app volume controls can remain independent and below the computer output level ([Sound settings](https://support.apple.com/en-gb/guide/mac-help/mchlp2256/mac)).

Any broader device-first policy should preserve Software gain when a Shared output has no usable volume control and Fixed for the integer path without one. Control the device selected by Pulse, which can differ from macOS's default output; do not change the default output as a side effect. Existing Exclusive adoption/release behavior and mute semantics need explicit treatment. A settable Core Audio scalar proves that a control exists, not that its implementation is a DAC-side analog attenuator ([Apple QA1016](https://developer.apple.com/library/archive/qa/qa1016/_index.html)).

## Steps To Reproduce

1. Use a PCM track and select Mac Studio Speakers in Shared mode.
2. Compare Pulse's slider with the macOS output-volume control for the same speakers; change the latter.
3. Pulse retains its own volume value. Adjusting Pulse changes its software gain rather than the speakers' volume property.

The report supplies the UI symptom; this investigation confirmed the code path and hardware capability without playing audio or changing device state.

## Relevant Code

- `crates/pulse-app/src/backend/playback/logic.rs:368` — default Shared resolution for built-in speakers without an integer bit-width capability.
- `crates/pulse-engine/src/auhal_engine.rs:62` — device-volume discovery requires an owned hog; `:186` selects hardware writes or software gain; `:197` consumes a one-shot volume snapshot.
- `crates/pulse-engine/src/hal/volume.rs:23` and `:50` — device writes and initial readback; no external-change subscription.
- `crates/pulse-engine/src/controller/worker.rs:632` — first device adoption versus later app-level reapplication; `:719` handles Shared fallback with app volume.
- `crates/pulse-app/src/backend/playback/controller.rs:41`, `:467` — slider state and engine hardware-volume events feed the single app volume setting.
- `crates/pulse-engine/src/controller/tests/volume.rs:254`, `:495` — explicit tests of Shared software gain and adopt-once device round trips.
- [`archive/builtin-speakers-exclusive-volume.md`](archive/builtin-speakers-exclusive-volume.md) and [`features/archive/31-volume-transparency.md`](../features/archive/31-volume-transparency.md) — historical policy and its disclosure.

## Environment

- macOS; Mac Studio Speakers, built-in output, `BuiltInSpeakerDevice`.
- Pulse code: `main` at `1d9468f`, package version 0.3.3. Runtime app/output at the original report was not instrumented.
- Release and dev settings both have the built-in speaker mode unset and app volume saved at 1.0, unmuted. Saved preferred output was Matrix when inspected; these preferences are not a live playback-state snapshot.

## Verification

- Read-only Core Audio probe on 2026-09-06: Mac Studio Speakers returned volume scalar `0.5471949`, mute `0`, and a settable main output-volume property. At that probe instant it was both the default audio and system-alert output. Device ids/defaults are transient. No setters, hog acquisition, format changes, or playback were invoked.
- Existing focused tests `shared_mode_uses_software_gain_even_when_the_device_has_hardware_volume` and `device_round_trip_reapplies_the_adopted_level_without_readopting` passed on the merged code. No production or test source was changed.
- A future change should cover initial device adoption without writes, bidirectional level/mute sync while playing and idle, switches away/back without stale-volume writes, Shared fallback, and the existing Exclusive adoption behavior. Hardware acceptance must compare controls on the same selected output and verify other-device volumes stay untouched.
