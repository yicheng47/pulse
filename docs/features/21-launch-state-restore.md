# Launch State Restore

> Feature 21 · P2 · GitHub issue [#57](https://github.com/yicheng47/pulse-src/issues/57). Every launch starts cold — empty playback row, default library page. The app should reopen where it left off: the last track ready to resume and the last page open.

## Motivation

Pulse forgets everything between launches: the queue and current track vanish, and the library always opens on the default page. Every session starts with re-navigating and re-queueing, which is friction no established player has — the expected behavior is to come back exactly where you stopped, with the last track loaded and paused.

## Scope

- **Playback restore.** Persist the queue (track identities), current index, playback position, and the shuffle/repeat state; on launch, rebuild the queue and load the current track **paused** at the saved position. Launch never auto-plays.
- **Navigation restore.** Persist the active library route (page plus its parameter — the open album, playlist, or artist filter); on launch, reopen it if the target still exists in the library, else fall back to the default page.
- **Persistence points.** Save navigation on route change and playback state on pause, on track change, on quit, and coarsely during playback (position every few seconds — cheap enough to be invisible, fresh enough that a crash loses seconds, not the session).
- **Missing files degrade softly.** The library is NAS-backed, so restored paths may be unreachable at launch. Restore keeps the queue and metadata visible; unavailability surfaces only when the user presses play (the existing playback error path), never as a launch-time error or a silently emptied queue.
- **Deleted tracks.** Queue entries whose library records are gone are dropped at restore; if the current track itself is gone, restore lands on the queue's next surviving entry, paused at zero.

## Non-Goals

- Auto-play on launch, under any setting.
- Window geometry/size restore — separate polish if ever wanted.
- Cross-device or cloud state sync.
- Restoring transient UI state: open popovers, scroll positions, search text, selection.
- A "resume playback" prompt or notification — the row simply sits ready.

## Implementation Phases

1. Persistence model: a small versioned state blob in the existing preferences store (queue as track identities, index, position, modes, route); tolerate missing/corrupt state by starting cold.
2. Playback restore: rebuild the queue through the existing queue model, load-paused-at-position through the controller without starting the sink.
3. Navigation restore: route serialization and the exists-check fallback.
4. Wire the save points; verify save cost is imperceptible during playback.

## Verification

- Unit tests: round-trip the state blob; restore with all tracks present, with some deleted, with the current track deleted, and with a corrupt blob (cold start, no crash); route restore falls back when the album/playlist is gone.
- `make verify` is green.
- Manual: play mid-album, navigate into a playlist, quit, relaunch — same page opens, same track sits paused at the same position, next/prev walk the restored queue, shuffle/repeat toggles match; with the NAS unmounted, relaunch shows the queue intact and play produces the normal unavailable error; a fresh install starts cold.
