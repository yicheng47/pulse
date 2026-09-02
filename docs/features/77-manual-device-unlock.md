# Manual Device Unlock

Feature 77 · P2 · GitHub issue [#77](https://github.com/yicheng47/pulse/issues/77). Decided by Jason, 2026-09-02: "we really need a way to unlock the device — maybe add lock/unlock icon in the output selection popup." This resolves the pause-release question: hold-on-pause stays (feature 32), no auto-release timer; release becomes an explicit user action.

## Motivation

The bit-perfect engine hogs the device and holds it across pause by design — instant resume, DSD lock kept. But holding is invisible and irreversible short of switching outputs or quitting: when another app needs the DAC mid-pause, there is no handle. An explicit lock/unlock affordance on the active device makes the hold visible and gives it a release valve, matching how Audirvana models the same state.

## Scope

- **Indicator**: the output popover's active-device card shows the hold state — locked (engine holds hog + integer format) vs unlocked — via a lock icon; exact placement, iconography, and states are the Pencil pass's call.
- **Release**: clicking the lock while the engine holds the device releases it — unhog, restore the captured format/mixing state, drop the sink — through the same release path the quit fix built (`f5150fd`'s release handle), keeping the playback session intact: track, position, and queue survive; the transport shows paused/stopped state honestly.
- **Re-acquire**: the next play/resume re-acquires through the existing `start_backend` path. If the device was taken meanwhile, the existing hogged-refusal/error paths apply (toast, feature 74).
- **While playing**: whether the lock is disabled during active playback or acts as stop-then-release is a Pencil-pass decision.
- **Universal engine**: shared-mode devices are never held; the affordance renders only where holding actually happens (bit-perfect / exclusive with hog).

## Non-Goals

- No auto-release timer or idle-based release — explicitly rejected in favor of manual control.
- No change to pause semantics or the quit path.
- No menu-bar, dock, or global-shortcut affordance — the popover owns it.

## Implementation Phases

1. **Pencil pass**: lock states on the active-device card (locked / unlocked / releasing?), the while-playing behavior decision, copy for any confirm-or-toast moments. Jason approves before the mission.
2. **Engine + app**: a release command through the controller (reusing the shutdown-built release handle machinery), hold-state surfaced in playback state, popover wiring per the approved design.
3. **Validation**: headless seams for release/re-acquire ordering; Jason's hardware pass — unlock while paused frees the Matrix for another app, play re-acquires cleanly.

## Verification

- `make verify` green; fake-seam tests: release keeps session state, re-acquire after release works, double-release is a no-op, release while playing follows the decided behavior.
- Manual (Jason): pause DSD → unlock → another app plays through the Matrix → back in Pulse, play → device re-acquired, DSD re-locks.
