# Shuffle and Repeat Playback

> Feature 07 · P2 · GitHub issue [#32](https://github.com/yicheng47/pulse/issues/32). Pulse renders Shuffle and Repeat in the playback bar and Shuffle on album and playlist surfaces, but those controls are deliberately inert.

## Motivation

Visible playback controls should have honest behavior. The queue already provides deterministic next, previous, jump, remove, and clear operations; shuffle and repeat are the remaining transport modes needed to make the existing affordances real.

## Design source

`design/pulse-desktop.pen`, read via the `pencil` MCP only: **`Spec — Shuffle & Repeat States`** (`GCs9J`). Fixed there: shuffle toggles between `shuffle` in `$text-secondary` (off) and `$accent` (on); repeat cycles OFF → ALL → ONE → OFF rendering `repeat-2` muted, `repeat-2` accent, and `repeat-1` accent respectively (glyphs at the transport's existing 19px). Semantics: shuffle randomizes only the upcoming order, keeps the current track stable, Previous walks true play history, and toggling off restores the original order; repeat ALL wraps the queue at both ends; repeat ONE loops the current track on natural end while manual Next still advances; album/playlist Shuffle actions start playback over that collection with shuffle ON; modes are per launch, never persisted.

## Scope

- ~~Design the inactive and active states~~ Done — see Design source.
- Add queue-level shuffle behavior that keeps the current track stable and preserves coherent Previous history.
- Add repeat modes with explicit queue-end behavior and a visible active state.
- Make Album Shuffle and Playlist Shuffle start playback from a shuffled version of the selected collection.
- Keep mode changes and shuffled order per launch; rebuilding the queue from a new user selection starts from the mode semantics fixed by the design pass.

## Non-Goals

- Smart or weighted shuffle, crossfade, gapless-transition changes, queue persistence, or syncing playback modes across devices.
- Changing the existing deterministic queue behavior while Shuffle and Repeat are off.

## Implementation Phases

1. Fix the exact active states and repeat-mode cycle in `design/pulse-desktop.pen`.
2. Extend `QueueState` with testable shuffle and repeat semantics, using deterministic randomness in tests.
3. Wire the playback bar, Album Detail, and playlist controls to the shared queue state.

## Verification

- Unit tests cover shuffle without duplicates, stable current-track behavior, coherent Previous history, repeat-one, repeat-all, queue rebuilds, and unchanged off-mode ordering.
- `make verify` is green.
- Manual: toggle each mode during playback, traverse both ends of the queue, start shuffled album and playlist playback, and confirm every active state matches Pencil.
