# Gapless Playback

> Feature 16 · P1 · GitHub issue [#46](https://github.com/yicheng47/pulse-src/issues/46). Every track transition tears down and rebuilds the playback path, so even same-format consecutive tracks get an audible gap of silence.

## Motivation

Track transitions currently run: track A's ring drains → the controller emits `Ended` → the app dispatches `PlayFile` for track B → the controller opens a new decoder, calls `set_format` (rebuilding the ring), and restarts the AUHAL sink. The engine only learns about track B after track A has fully drained, so a gap is structural, not a tuning problem.

A large class of albums is mastered as continuous audio — live recordings, classical works, concept albums, DJ mixes — where track boundaries fall mid-sound. Every serious player handles these seamlessly; Pulse cannot. This is the biggest audible gap between Pulse and the hi-fi players it competes with.

## Scope

- Consecutive tracks with the same `PcmFormat` play seamlessly: the AUHAL sink and rtrb ring stay alive across the transition, and the next track's PCM is fed into the live ring the moment the current decoder reaches EOF.
- A next-track lookahead surface on the controller (e.g. `SetNext { path }` / `ClearNext`), so the app can hand the engine the upcoming queue entry before the current track drains. The app keeps queue ownership; the engine holds at most one preloaded source.
- Per-track fed-frame accounting so the controller distinguishes the decode boundary (decoder EOF) from the audible boundary (callback consumed the last frame of the outgoing track). `NowPlaying`, position, and `Ended`/advance events flip at the audible boundary, not the decode boundary.
- Format-change transitions keep the existing stop/rebuild path (nominal-rate switch and ring rebuild are inherently audible), falling back automatically when the preloaded source's format differs or no next source is set.
- Lookahead invalidation: queue edits (skip, remove, reorder, clear) while a next source is preloaded must replace or clear the preload without disturbing the playing track. **Buffered window:** once the current decoder has hit EOF and the incoming track's PCM is already in the ring (up to the ring's ~4 s, in the last seconds of the outgoing track), the ring cannot be rewound — `ClearNext` / `SetNext` then act on the *incoming track's successor*, and the incoming track still plays. Removing the upcoming track in that window is therefore not honored; the app may skip explicitly (a `PlayFile`) if it must.

## Non-Goals

- Crossfading or any DSP between tracks.
- Gapless across format changes — a device sample-rate switch cannot be seamless.
- Moving the full queue model into `pulse-engine`; only single-track lookahead crosses the boundary.
- Cue-sheet or single-file multi-track support.
- Preloading more than one track ahead, or speculative decode of unqueued tracks.

**Validated** on the Matrix Mini-i Pro 4 by Jason on 2026-08-30 (phase 4): continuous album across boundaries, 44.1→96 kHz fallback, mid-track skips, queue removal outside and inside the buffered window, repeat One/All, pause across a boundary, unreadable next track.

## Implementation Phases

Phases 1–2 merged 2026-08-30 in `4ae4fe4`: `PlaybackCommand::SetNext { path }` / `ClearNext`; `PlaybackEvent::Advanced { attempt, source, format }` (same `attempt`, followed by `Position 0`) replaces `Ended` + `PlayFile` whenever a next source is set — seamless on matching formats, engine-driven stop/rebuild otherwise; `Ended` only when no next source is set. Phase 3 merged in `7d61a32`: the app keeps one `sync_next_source` behind every queue mutation, handles `Advanced` by reconciling the queue by path, and gets `NextRejected` for a failed preload.

1. Engine accounting: track the fed-frame boundary between the outgoing and incoming track so the controller can observe when the callback's position crosses it; verify the position/underrun atomics stay callback-safe.
2. Controller: add the next-source command surface; on decoder EOF with a matching preloaded format, swap decoders and keep feeding the live ring, emitting `NowPlaying`/position/advance events at the audible boundary; fall back to the rebuild path on format mismatch or missing preload.
3. App: `playback_row`/queue hand the controller the upcoming entry whenever the effective queue head changes, and invalidate the preload on queue edits; drop the app-side `Ended → PlayFile` dispatch for the gapless path while keeping it as the fallback.
4. Hardware validation on the Matrix DAC.

## Verification

- Controller tests with fake backend/decoder: a same-format transition never calls backend `stop`/`start` between tracks; `NowPlaying` for track B is emitted only when the backend position crosses track A's final fed frame; a format-mismatched preload falls back to the rebuild path; queue edits mid-track replace or clear the preload without affecting playback; pause/seek during a preloaded transition stays correct.
- `make verify` is green.
- Manual: a continuously mastered album (live or classical rip) plays across a track boundary with no audible gap on the Matrix DAC; a 44.1 kHz → 96 kHz transition still switches rates and plays cleanly via the fallback path; skipping mid-track and removing the upcoming track from the queue both behave.
