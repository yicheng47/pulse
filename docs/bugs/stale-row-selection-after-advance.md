# Clicked track row stays highlighted after playback advances to the next track

> Bug · P2 · filed 2026-08-30 from Jason's in-app report (screenshot: Beethoven 5, row 2 still highlighted while row 4 plays). Local note only — no GitHub issue.

## Description

Clicking a row in Album Detail, Tracks, or a Playlist sets a page-local selection (`selected_album_track_id`, `selected_track_id`, and the playlist equivalent in `surfaces/library/mod.rs`). Each track row paints `theme::bg_selected()` when `selected || playing` (`surfaces/library/albums.rs` `render_album_track_row` ~line 669; `surfaces/library/tracks.rs` ~line 450; the playlist row likewise), and the now-playing row additionally gets the glow and accent bar. The selection is cleared only when the selected track leaves the list (`surfaces/library/lifecycle.rs` ~157–166). So when playback advances on its own — end of track, and now gapless `Advanced` — the row the user clicked minutes ago stays highlighted next to the row that is actually playing: two lit rows, one of them meaningless.

## Expected Behavior

Selection means "the row I just acted on". When the now-playing track changes to a track the user did not click (auto-advance, gapless advance, next/previous from the transport row, queue jump), the page-local selection is cleared, leaving only the playing row highlighted. Clicking a row still selects it as today.

## Steps To Reproduce

1. Open an album; click track 2 (single click, or play it).
2. Let it play to the end, or press next.
3. Track 3 plays with the glow and bar; track 2 keeps the `bg_selected` background.

## Relevant Code

- `crates/pulse-app/src/surfaces/library/mod.rs` — the three selection fields; the `reactions.playback` branch of the store observer (~line 360) is where the now-playing path change is observed and where the selection should be dropped when the new path is not the selected track's.
- `crates/pulse-app/src/surfaces/library/lifecycle.rs` ~157–166 — existing selection-clearing when the track leaves the list; `is_now_playing` ~388.
- `crates/pulse-app/src/surfaces/library/albums.rs`, `tracks.rs`, `playlists.rs` — the `selected || playing` row paint.

## Environment

- OS: macOS 26.5.2
- Device / DAC: any
- Input file format: any
- Pulse version: v0.1.8 + gapless (`main` at `66bd066`)

## Verification

Code read only. Fix shape: on a playback reaction where the now-playing path changed and does not equal the selected track's path, set all three selection fields to `None`; a `*_logic.rs` unit test for the rule (selected A, playing A → A; playing becomes B → None; click B → B).
