# Hogged device not released cleanly on app quit

> Bug · P1 · filed 2026-09-01 from Jason's report (Matrix mini-i Pro 4 left claimed/mis-formatted after quitting Pulse). Local note only — no GitHub issue.

## Description

After quitting Pulse while the bit-perfect engine holds the device, the DAC does not come back cleanly: hog mode and/or the device format state linger past the app's exit. The teardown design is RAII and correctly ordered — `on_app_quit` (`main.rs:97`) → `AppStore::shutdown` → `Playback::shutdown` drops the `PlaybackController` (`backend/playback/controller.rs:730`), whose `Drop` joins the engine worker; the engine's `Drop` pauses and drops the sink, which releases `HogGuard` (unhog, `pulse-engine/src/hal.rs:118`) and `FormatRestoreGuard` (restore physical format + mixing, `hal.rs:77`). The suspect is everything that runs before and around that chain: `Playback::shutdown` flushes settings and persists the session first, then the controller `Drop` blocks on a thread join — all inside the quit callback. If the process exits before the chain completes (quit-deadline, force quit, crash), coreaudiod eventually releases hog for the dead pid, but nothing restores the physical format or re-enables mixing — the device stays in the integer/DoP state.

## Expected Behavior

Quit releases the device deterministically: hog dropped and the captured format state (physical format, virtual format, mixing) restored before the process exits, on every quit path (⌘Q, window close if it quits, app termination). A crash may leave state behind — but launch-time recovery could then restore it (out of scope unless cheap).

## Steps To Reproduce

1. Bit-perfect mode on the Matrix, play a track (hog held, integer format set — DSD DoP makes it most visible).
2. Quit Pulse (⌘Q); also try quit while paused and quit mid-playback.
3. Observe the DAC/device state: whether another app can claim it immediately, what rate/format the device reports, whether the DAC display stays locked to the last rate.

## Relevant Code

- `crates/pulse-app/src/main.rs:97` — `on_app_quit` hook.
- `crates/pulse-app/src/backend/playback/controller.rs:717` — `shutdown()` does settings/session work before dropping the controller; the drop then joins the worker.
- `crates/pulse-engine/src/controller.rs:132` — `Drop for PlaybackController`: shutdown flag + join.
- `crates/pulse-engine/src/integer_engine.rs:302` — `Drop for IntegerEngine`: `pause()` then `sink = None`; hold-on-pause semantics interact here — verify the sink drop path actually stops the IOProc and releases both guards.
- `crates/pulse-engine/src/hal.rs:77,118` — the two guards whose `Drop` must run.

## Environment

- OS: macOS (Darwin 25.5)
- Device / DAC: Matrix Audio mini-i Pro 4, bit-perfect (hog) mode
- Input file format: any on the integer engine; DSD/DoP most visible
- Pulse version: main post-`f50e040`, 2026-09-01

## Verification

Reported by Jason 2026-09-01; teardown chain read and mapped (above), root cause not yet isolated — first step of the fix mission is instrumenting the quit path to see which drop (if any) is skipped and measuring how long `shutdown()` + join take against GPUI's quit window.

## Fixed

`f5150fd`, codex crew mission 2026-09-02. Phase-1 trace showed the quit callback does complete (flushes in microseconds, controller join ~107ms) — the risk was ordering and unboundedness, not a skipped drop. The fix releases the device first (sink stop, format+mixing restore, unhog) under a 1s try-lock deadline before any persistence, bounds the worker join with its own 1s budget, and makes both `Playback::shutdown` and the controller `Drop` idempotent. Hardware ⌘Q check pending: quit during bit-perfect DSD playback, then confirm another app can claim the Matrix and Audio MIDI Setup shows the pre-Pulse rate.
