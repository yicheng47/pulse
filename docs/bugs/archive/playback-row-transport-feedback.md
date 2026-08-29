# Playback row transport buttons give no click feedback and prev/next blink on resume

> Bug · P2 · filed 2026-08-29 from Jason's in-app report. **Fixed** 2026-08-29, commit `3d85c64` — kit-level hover/active, availability from the queue, optimistic play/pause reconciled by settled states, `Loading` shows pause.

## Description

Two related problems in the playback row's transport cluster (`surfaces/playback_row.rs`, `render_transport`):

1. **Clicks have no visible effect.** Shuffle, previous, play/pause, next, and repeat are plain `div()`s with a static color and `cursor_pointer()`; none has a hover, pressed, or active style (`.hover(` is used once in the whole app, on the scrollbar thumb; `.active(` never). The only acknowledgement of a click is the eventual state change — for play/pause that is the icon flip, which waits for the engine to report `Playing`, so a resume click feels dead for the sink-start latency.
2. **Previous / next blink on resume.** The engine's `resume()` (`crates/pulse-engine/src/controller.rs:347-356`) sets `PlaybackState::Loading` before it rebuilds the sink and starts the path, then `Playing`. The row treats `Loading` as "transport unavailable": `previous_enabled` / `next_enabled` exclude `Loading | Stopping` and render the buttons at 0.35 opacity with no click handler. So every resume dims both buttons for the sink-start window and restores them — a visible flicker. The same `Loading` round-trip happens on output-device change and exclusive-mode toggle while playing (`controller.rs:433`, `:476`).

## Expected Behavior

- Every transport control shows hover and pressed states like the rest of the kit should (Runner's transport row is the reference), and play/pause reflects the click immediately — an optimistic icon flip, or a pressed state that persists until the engine confirms.
- Previous / next availability is a function of the queue and the loaded track, not of the transient engine state. Their enabled look must not change across pause → resume; if a command must be dropped while the engine is between states, drop it in the controller (`toggle_command` already returns `None` for `Loading | Stopping`) rather than repainting the buttons.

## Steps To Reproduce

1. Play any track from Albums.
2. Click pause, then click play again. Watch the previous / next icons: they dim to 35% and come back within ~100–300 ms.
3. Click any transport button: no hover highlight before the click and no pressed state during it.
4. Optional: with playback running, switch output device in the popover — the same dim-and-restore.

## Relevant Code

- `crates/pulse-app/src/surfaces/playback_row.rs` — `render_transport`: `previous_enabled` / `next_enabled` gates on `Loading | Stopping`; plain `div()` buttons with `.opacity(0.35)` for the disabled look.
- `crates/pulse-engine/src/controller.rs` — `resume()` sets `Loading` then `start_path`; `pause()` releases the backend (the sink is torn down on pause, so resume is a full restart, which is why `Loading` is honest from the engine's side).
- `crates/pulse-app/src/backend/playback/controller.rs:101-108` — `toggle_command` already drops toggles during `Loading | Stopping`.
- `crates/pulse-app/src/ui/button.rs` — `IconButton` has disabled/opacity but no hover or active styling either; the fix should add hover/active to the kit and have the transport row use it (or a `TransportButton` variant), not another page-local button.

## Environment

- OS: macOS 26.5.2 (Darwin 25.5.0)
- Device / DAC: any output; reproduced on the built-in output
- Input file format: any supported PCM file
- Pulse version: 0.1.7 (`main` at `2461590`)

## Verification

Code read only: the `Loading` gate in `render_transport` and the `Loading` entry in `resume()` were confirmed in source; the missing hover/active styling was confirmed by grep (`.hover(` ×1 in `ui/scrollbar.rs`, `.active(` ×0). No fix attempted.
