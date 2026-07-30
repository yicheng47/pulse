# 0013 — MVP library UI

> Stage 11. Depends on the 0012 library backend (merged, PR #9) and the Storage/library design passes in `design/pulse-desktop.pen`. Builds Albums, Album Detail, Tracks, Playlists, and Storage as GPUI surfaces over the SQLite store. Playback-queue behavior and playlist CRUD are stage 12; loading and failure-hardening states are stage 13.

## Goal

Point the app at the library the 0012 backend builds and make it browsable: real albums and tracks from SQLite, storage-root management end to end (add, scan with live progress, remove, offline), and every designed state reachable. A user with zero roots sees a first-run path that leads to a scanned library without touching a manual.

## Design sources

Read via the `pencil` MCP; `.pen` files are encrypted, never Read or Grep them. All screens verified at 0 layout problems.

| Node | Screen | Notes |
|------|--------|-------|
| `E3N1P` | Library / Main (Albums grid) | chips, sort control, card grid |
| `XaIfA` | Library / Album Detail | back link, header, track table with playing-row state |
| `KfJr9` | Library / Tracks | dense table, filter chips, artist filter hint |
| `MHrLm` | Library / Playlists | list panel + detail panel (stage 12 populates) |
| `C6IrDC` | Library / Storage | summary tiles, roots table, detail panel |
| `cJCbr` | Storage — Add Storage modal | scrim + modal, `Scan this root now` |
| `NK6lk` | Storage — Scanning | progress bars, SCANNING pills, Cancel Scan |
| `e2xOQ` | Storage — Offline & Scan Failure | OFFLINE detail panel, FAILED row with inline Retry |
| `VPPvT` | Storage — Empty | zero roots |
| `lTVhl` | Albums — Empty | first-run state, Add Storage CTA |
| `LNdSq` | Playlists — Empty | stage 11 ships this state; populated `MHrLm` is stage 12 |
| `A3JuUc` | Search Results popover | **design only — implementation deferred, see Non-goals** |

Token additions the theme needs: `bg-surface-alt #111110`, `bg-selected #1A1117`, `quality-soft #111A0B`, `quality-border #4F7B22`, `danger-soft #2A1016`, `scrim #0A0A0ACC`. `#151514` was collapsed into `bg-surface`. Selected rows use `bg-selected`, not `accent-soft` — `accent-soft` is button/pill fill.

## Backend additions (small, additive, in `library/`)

The 0012 store exposes roots, `tracks_for_root`, per-root and catalog summaries, and scan history. Stage 11 needs, all as plain additive queries or columns:

- **`year` and `genre` columns on tracks.** Not a design nicety — `mvp.md` requires sort by release year and a genre filter, and Album Detail/search show year. Lofty exposes both tags. Additive migration plus extraction; bump schema, backfill on rescan (mtime-unchanged files will not re-read — either a one-shot backfill pass or accept nulls until files change; pick one and log it).
- **Album listing query**: group by (`album_artist` falling back to `artist`, `album`), returning title, artist, year, track count, total duration, max sample rate/bit depth, and a cover path (first track with `cover_art_path`). Untagged tracks group under an explicit Unknown Album bucket; do not vanish them (`mvp.md`: missing tags must not break browsing).
- **All-tracks query** with the sort orders `mvp.md` names (title, artist, album, date added, release year, duration).
- **Tracks-for-album query** ordered by disc then track number.
- **`rename_storage_root`** — the Storage detail panel has Edit; the store has add/remove only.
- Format label per track derives from the path extension (`flac`, ALAC-in-`.m4a`, `aiff`, `wav`) — no new column. The quality pill is `bit_depth`/`sample_rate` (`24/96`); Hi-Res means above 16/48.

## Placement and threading

Screens live in `crates/pulse-app` as modules routed by the 0010 shell body — same pattern as the existing placeholder bodies. Keep view-models (state derivation, label formatting, sort/filter logic) as plain testable structs separate from GPUI views, the way stage 8 split the preferences parser.

The app owns one `LibraryStore` plus a scan worker: scans run on a `std::thread` (the 0012 scan API is deliberately synchronous), progress crosses to the UI over a channel, and the UI polls/notifies on its executor. One scan at a time; Rescan on a second root queues or disables — pick disable, it is simpler and honest.

## Screen behavior

**Sidebar**: Storage badge binds live root count. Devices stays the 0010 placeholder (popover from stage 8 remains the device surface).

**Albums** (`E3N1P`): grid from the album query. Chips: All Albums, Hi-Res, Recently Added, plus genre chips from distinct genres once the column lands. Sort control per the mvp orders. Card click pushes Album Detail. Cover fallback when no art: `bg-muted` well + `disc-3` icon (the Playlists artwork-placeholder pattern). Zero albums → `lTVhl`; its Add Storage CTA opens the Add Storage modal directly.

**Album Detail** (`XaIfA`): back returns to Albums preserving scroll/filter state. Header: cover, title, artist, `year · n tracks · total min`, format + HI-RES badges from the album's maxima. Double-click a row plays that file through the existing controller path (single-file play works today); Play Album plays the first track. Full queue semantics are stage 12 — do not fake next/previous here. The `audio-lines` playing indicator binds to controller now-playing state matched by path.

**Tracks** (`KfJr9`): dense table over the all-tracks query; same chips plus the artist filter hint (clicking an artist name filters, per mvp "artist as facet"). The **Liked chip in the design has no backing** — `mvp.md` does not list favorites; skip the chip at implementation and leave the design as a post-MVP hook. Double-click plays.

**Playlists** (`MHrLm`/`LNdSq`): stage 11 renders the empty state only; New Playlist is present but inert until stage 12 (tooltip or plain disabled, do not half-wire create). The populated design is the stage 12 contract.

**Storage** (`C6IrDC` + states): roots table binds `storage_roots()` + `root_summary` + `recent_scans`; summary tiles bind `catalog_summary` and the latest scan. Selection drives the detail panel. Add Storage opens the native folder picker (macOS `NSOpenPanel` via gpui's paths prompt — confirm the pinned gpui-ce rev exposes it; if not, this is the one place a small platform call is justified), then the `cJCbr` modal confirms with display name prefilled from the folder name and `Scan this root now` checked. Rescan runs the worker and swaps the row + detail + SCAN tile into the `NK6lk` scanning forms; percent is `processed/total` from `ScanProgress::Processing`, count line `n of ~total files`, and Cancel stops the run (0012's per-file commits make cancel safe mid-scan). `is_reachable == false` renders the `e2xOQ` OFFLINE treatment (purple, Reconnect re-checks and rescans, Remove removes); `ScanOutcome::Failed` renders the FAILED row with `error_message` inline and Retry. `CompletedWithErrors` shows the error count as the Issues line in the detail panel — plain text, no new design needed. Remove uses a confirm modal reusing the `cJCbr` chrome (title, body copy, Cancel/Remove-in-danger); this dialog is deliberately undesigned — keep it to existing tokens, same precedent as stage 8's plain error text.

State mapping is a pure function worth unit-testing: `(StorageRoot, latest ScanHistoryEntry, live scan?) -> RowState` covering online/offline/scanning/failed/completed-with-errors.

## Testing

- Store: album grouping (including Unknown Album and cover fallback), all-tracks sorts, tracks-for-album disc/track order, rename root, year/genre extraction (extend the generated in-test WAV; if ALAC/FLAC year-tag paths need real files, say so rather than committing binaries).
- View-models: row-state mapping, format/quality labels (`24/96`, Hi-Res threshold), scan percent/count lines, chip filtering.
- GPUI rendering cannot be verified by agents in this sandbox — the visual pass, click-throughs, and drop-to-play regression stay Jason's. Keep the manual checklist in the PR description.

## Verification

- `make verify` green (59 tests today; expect meaningful growth).
- Agent-runnable proof: a `#[ignore]`d or temporary harness that opens the real library DB scanned in 0012 and prints album count, a sample album's tracks in order, and each root's derived row state.
- Jason: first-run flow on a scratch config (empty → add root → watch scan → browse albums → open album → play a track), offline unplug of the NAS root, a forced scan failure (chmod a subfolder), and the drop-to-play regression.

## Non-goals

- **Search**: the popover is designed (`A3JuUc`, grouped results, no-match group line) but implementation waits for the search query/FTS decision — proposed home is stage 12 alongside queue wiring, since result rows resolve to playable objects. Until then the top-bar field stays inert.
- Playlist CRUD, queue, next/previous — stage 12.
- Loading skeletons, decode/hogged/unavailable-device errors, missing-file states — stage 13.
- Devices page, metadata enrichment, favorites/Liked, artist destination pages.

## Open questions

- Year/genre backfill choice (one-shot re-read vs. wait-for-mtime) — decide in-branch and log it.
- Rajdhani SemiBold vendoring (carried from 0010) — nav and label weights still approximate.
- Whether `gpui-ce` at the pinned rev exposes a folder-picker prompt; if not, smallest possible AppKit call.
