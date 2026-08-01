# 0014 — Queue, playlists, and search

> Stage 12, the last feature stage of the MVP. Depends on 0013 (merged, PR #10) and the #16 acceptance fixes. Wires next/previous and the queue count, makes playlists real end to end, and implements search against the already-designed popover. After this, stage 13 is failure hardening and stage 14 is the condensed acceptance pass and v0.

## Goal

Playing an album, playlist, or filtered track list plays *through* it: next/previous work, end-of-track advances, the queue badge counts what remains. Playlists support the full mvp.md verb set (create, rename, delete, add, remove, reorder, play). The top-bar search field stops being inert and resolves to playable or browsable objects.

## Design sources

Read via the `pencil` MCP; `.pen` files are encrypted, never Read or Grep them. All verified at 0 layout problems.

| Node | Surface | Notes |
|------|---------|-------|
| `qKkw7` | Playback Bar component | Previous/Play/Next/Shuffle/Repeat, progress row, queue button `cOeyi` with count badge `eCWdc`. The row already renders these icons — this stage wires Previous/Next and the badge. Shuffle and Repeat stay inert (below). |
| `MHrLm` | Library / Playlists (populated) | The stage 12 contract for list + detail; stage 11 shipped only the empty state `LNdSq`. Detail actions `vsnDp`: Play works, Shuffle inert. |
| `Lr7A3` | Playlists — Name Dialog | One dialog for Create and Rename: title and confirm label swap ("New Playlist"/"Create Playlist" vs "Rename Playlist"/"Rename"). Reuses the Add Storage modal chrome. |
| `cYA3i` | Tracks — Context Menu | Right-click menu on track rows: Play / Add to Playlist ▸ flyout (existing playlists + "New Playlist…" in accent) / Remove from Playlist. The flyout and menu are `bg-surface` on `border-strong`, `radius-lg`, 210/200 wide. |
| `A3JuUc` | Search Results popover | Grouped ALBUMS/TRACKS/PLAYLISTS results, selected-row treatment, footer with no-match group line and `↵ OPEN · ESC DISMISS` hints. |

## Backend (library module, schema v3)

- `playlists` (id, name, created_at_ms, updated_at_ms) and `playlist_tracks` (playlist_id, track_id, position) with `ON DELETE CASCADE` from both parents — a track removed from the library drops out of playlists; offline roots keep their rows (0012), so no cascade happens from a mere unplug.
- CRUD + ordering: create, rename, delete, append tracks (multi-insert for future multi-select, single-use today), remove entry, move entry to a new position with contiguous renumbering. Duplicate tracks in one playlist are allowed (position is the identity, standard player behavior).
- Playlist listing query with per-playlist track count, total duration, and a cover path (first track with art) for `MHrLm`'s rows and detail header.
- Search query, LIKE-based, case-insensitive, no FTS (the 0012 deferral stands — this query's shape must not preclude the FTS5 swap later): tracks on title/artist/album/album_artist, albums as the grouped query filtered the same way, playlists on name. Substring match works for CJK without tokenization. Caps: 3 albums, 5 tracks, 3 playlists, matching the popover's proportions.

## Queue semantics (app-side, transient)

The engine controller stays single-file; the queue is plain app state — a `Vec<TrackRef>` plus index, rebuilt on every play-from-list action. Not persisted.

- Any play-from-a-list sets the queue to that list in its **current visible order** (album detail order, playlist order, or the Tracks page's active sort/filter) with the index at the clicked track. Play Album/Play Playlist = same with index 0.
- End-of-track (`Ended` event) advances to the next queue entry; at the end of the queue, stop as today.
- Next = play queue[i+1]; disabled styling at queue end. Previous = restart current track if >3s elapsed, else queue[i-1]; at queue start, restart.
- Queue badge shows remaining count (current excluded); hidden at zero. The queue *button* stays inert — a queue drawer is designed nowhere and deliberately post-MVP.
- Shuffle and Repeat icons remain inert this stage. mvp.md does not require their logic; wiring them without design for their states would be guesswork.
- A single-click selection (loads the row, Play arms it — the #16 behavior) also carries its list context, so pressing Play starts the queue exactly as double-click would.

## UI wiring

- **Playlists page**: real list from the store; selection drives the detail panel per `MHrLm`. New Playlist opens `Lr7A3`; empty name or Escape cancels; duplicate names allowed (id is identity). Rename via the context menu on a playlist row and a detail-panel action; Delete asks with the confirm-modal pattern (#16's remove-root chrome) and states the track-entry count. Play plays the playlist from the top; from a selected row, play starts there (mvp: "play playlist from the first selected item").
- **Context menu** (`cYA3i`): right-click on track rows in Album Detail, Tracks, and playlist detail. Items: Play; Add to Playlist ▸ flyout listing playlists plus "New Playlist…" (creates via `Lr7A3`, then adds); Remove from Playlist only when the row lives in a playlist detail. Anchored overlay with the stage-8 popover dismiss pattern (outside click, Escape); flyout opens on hover/click of the parent item. Keep menu state per-view like row selection.
- **Reorder**: drag a playlist-detail row to a new position with a drop-indicator line, via gpui's drag APIs. If the pinned gpui-ce rev makes internal list drag genuinely fiddly, the sanctioned fallback is Move Up / Move Down items appended to the playlist-row context menu — decide in-branch and log which shipped and why.
- **Search**: the top-bar field takes focus (click and `Cmd+F` via the existing menu/shortcut plumbing... verify; if no shortcut exists, click-focus suffices this stage), queries debounced ~150 ms, renders `A3JuUc` under the field. Interactions: click or Enter — album opens Album Detail, track plays (queue = that track's album, standard expectation), playlist opens Playlists with it selected; arrow keys move the selected row; Escape dismisses and blurs. Empty query closes the popover; no matches renders the group-line treatment ("PLAYLISTS — NO MATCHES") for each empty group with a plain "No matches for “query”" line when all groups are empty (text-only, per the 0013 deferral note).
- **Playback row**: secondary line must read `artist - album` (mvp wording) — verify it survived #16's artwork changes.

## Testing

- Store: playlist CRUD, reorder renumbering (move first→last, last→first, middle), cascade on track removal, duplicate entries, listing query counts/durations/cover fallback; search matching including CJK substrings, album_artist hits, caps, and empty results.
- View-models: queue build from each surface (respecting sort/filter order), advance on end, next-at-end disabled, previous 3-second rule, badge count, search grouping and keyboard selection movement.
- No binary fixtures; existing generated-WAV/temp-dir patterns cover anything new.
- GUI cannot be verified by agents in this sandbox. Manual checklist for Jason: play 菲靡靡之音 through a track boundary (auto-advance), Next/Previous including the 3-second rule, queue badge counting down, create/rename/delete a playlist, right-click → Add to Playlist from all three surfaces, reorder (drag or fallback), play from a selected middle row, search for 王菲 / "frank" / gibberish, Enter and Escape in the popover.

## Verification

- `make verify` green (currently 81 passing + 2 ignored).
- Agent-runnable proof: extend the ignored harness to create a playlist over the scanned temp library, reorder it, and print the queue order a play-from-middle would produce.

## Non-goals

- Queue drawer/detail UI, drag-out or M3U import/export, smart playlists, Liked/favorites.
- Shuffle and Repeat behavior.
- FTS5 index (additive later; the LIKE query is the MVP implementation of mvp.md's search requirement).
- Any engine/controller changes beyond consuming existing events. Gapless playback is post-MVP.

## Open questions

- Whether `Cmd+F` focus wiring exists in the current menu plumbing — cheap if yes, skip if not.
- Drag-reorder feasibility at the pinned gpui-ce rev (fallback sanctioned above).
