# Per-Device Exclusive Mode

> Feature 08 · P2 · GitHub issue [#33](https://github.com/yicheng47/pulse/issues/33). Exclusive mode is currently one global on-by-default preference, even though a dedicated DAC and Bluetooth output usually need different choices.

## Motivation

The global toggle forces users who alternate between an exclusive-capable DAC and shared outputs such as AirPods to revisit Settings whenever the output changes. Pulse already persists output identity by Core Audio UID, so exclusive-mode preference should follow the same stable device identity.

## Scope

- Persist the exclusive-mode choice per Core Audio device UID.
- Reflect the selected device's saved value in Settings and apply it on the next backend open for that device.
- Changing the selected device during playback reopens the backend with that device's saved mode while preserving the existing logical-position behavior.
- Define a migration from the current global marker preference without silently changing an existing user's choice.

## Non-Goals

- Per-device volume, EQ, DSP, sample-rate policy, output presets, or automatic device classification.
- Changing hog-mode behavior inside `pulse-engine`; this feature only chooses the existing exclusive/shared path per device.

## Implementation Phases

1. Design how Settings identifies the device whose preference is being edited.
2. Replace the global marker with a small UID-keyed preference representation and migrate the existing value.
3. Wire device selection, Settings state, backend reopen behavior, and tests.

## Verification

- Preference tests cover two UIDs with different values, missing-device defaults, and migration from both states of the existing marker.
- Controller tests confirm a live device change preserves position and uses the target device's mode.
- `make verify` is green.
- Manual: save exclusive mode on for the Matrix DAC and off for AirPods, alternate between them across relaunches, and confirm each device restores its own value.
