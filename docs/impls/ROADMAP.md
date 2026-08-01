# Implementation Roadmap

This is the canonical engineering stage plan for Pulse. Product direction lives in `docs/product/vision.md`; MVP scope lives in [`docs/product/mvp.md`](../product/mvp.md). This roadmap controls implementation order so we do not drift into random probing.

Each stage should have a focused implementation note once work starts. A stage is done only when its verification path is clear enough to repeat. Shipped stages' notes move to [`archive/`](archive/) keeping their number. Session-by-session progress is recorded in [`IMPL_LOG.md`](IMPL_LOG.md).

## Stage Rules

- Finish one stage before starting the next stage branch.
- Keep each stage small enough to review as one PR.
- Do not implement frontend surfaces before a Pencil design exists for that surface.
- Do not harden app-shell settings backends before the settings UX and storage model are designed.
- Keep `pulse-engine` UI-agnostic and drivable from `pulse-cli`.
- Keep `pulse-cli` deterministic and scriptable because it is the future agent/MCP harness boundary.
- Persist Core Audio device identity by UID, not by transient `AudioDeviceID`.
- Product design can create enough direction to unblock an engine/frontend slice without being globally complete. Keep unfinished design details explicit instead of marking the whole design stage done.

## Completed

| Stage | Impl Note | Outcome |
|-------|-----------|---------|
| 1 | [`archive/0001-engine-validation-cli.md`](archive/0001-engine-validation-cli.md) | Created the standalone engine validation CLI and proved file/device inspection. |
| 2 | [`archive/0002-hal-format-validation.md`](archive/0002-hal-format-validation.md) | Proved hog mode, nominal sample-rate switching, and physical-format diagnostics. |
| 3 | [`archive/0003-auhal-playback.md`](archive/0003-auhal-playback.md) | Switched playback to AUHAL and produced clean native-rate playback on the Matrix DAC. |
| 4 | [`archive/0004-cli-config.md`](archive/0004-cli-config.md) | Added UID-backed CLI default output config for repeatable harness use. |
| 6 | [`archive/0006-playback-controller.md`](archive/0006-playback-controller.md) | Added the UI-agnostic `PlaybackController` with play/pause/resume/seek/stop, events, and CLI smoke commands. Hardware smoke on the Matrix DAC still needs a manual pass. |
| 7 | [`0009-playback-row.md`](0009-playback-row.md) | Native GPUI playback row: theme, embedded assets, window-wide drop-to-play, play/pause, drag-to-seek. Matrix hardware smoke still open. |
| 7.5 | [`0010-app-shell.md`](0010-app-shell.md) | App chrome merged: sidebar, top bar, routed body, docked playback row, macOS menu. Includes the P1 `font-kit` fix. Nav/shortcut/drop manual checks still open. |
| 8 | [`0011-output-device-management.md`](0011-output-device-management.md) | Output Device Popover merged (PR #8): live enumeration, UID persistence, capability line, failure fallbacks. Device list verified against real hardware; by-ear and unplug/hog checks still open. |
| 9-10 | [`0012-library-scan-and-store.md`](0012-library-scan-and-store.md) | Merged as PR #9. Headless `pulse-app::library`: storage roots, incremental scanner with progress, Lofty tag/cover extraction, SQLite store. Measured on the real NAS: 4,904 tracks, 7.9× incremental speedup, zero errors. The Storage screen itself is stage 11. |
| 11 | [`0013-library-ui.md`](0013-library-ui.md) | Merged as PR #10. Albums, Album Detail, Tracks, Playlists-empty, and full Storage surfaces over schema v2 (year/genre, album and track queries, root rename, scan cancel). Manual acceptance pass still open — see Current. |

## In Progress

| Stage | Impl Note | Status |
|-------|-----------|--------|
| 5 | [`0005-product-design.md`](0005-product-design.md) | Partially complete. Cyberpunk desktop foundation exists in Pencil for albums, tracks, playlists, storage, sidebar, and playback row. More detailed product surfaces still need design passes. |

## Current

| Stage | Impl Note | Goal | Boundary |
|-------|-----------|------|----------|
| — | [`IMPL_LOG.md`](IMPL_LOG.md) (2026-07-31 entry) | Manual acceptance pass over stages 6-11. | Human-only: hardware smoke, shell shortcuts, device by-ear checks, the full stage 11 first-run/scan/browse/play flow, failure states, and the visual pass against the `.pen`. Stage 12 does not start until this is done; findings triage into fix-now vs stage 13. |

## MVP Path

| Stage | Impl Note | Goal | Notes |
|-------|-----------|------|-------|
| 12 | [`0014-queue-playlists-search.md`](0014-queue-playlists-search.md) | Wire queue, playlists, and search. | App-side transient queue with next/previous, auto-advance, and the badge count. Playlist CRUD/reorder over schema v3 with the name dialog (`Lr7A3`) and track context menu (`cYA3i`). LIKE-based search behind the designed popover (`A3JuUc`). Shuffle/Repeat and the queue drawer stay out. |
| 13 | — | Add MVP states and failure handling. | Loading, empty, offline storage, scan failure, decode failure, hogged device, unavailable device, and missing-file states. No silent failure. |
| 14 | — | MVP hardening and v0 release. | Hardware smoke matrix, scanner/library regression tests, CLI smoke tests, basic packaging, release notes, and final product-design cleanup for shipped surfaces. |

## Post-MVP / Parking Lot

- Raw HAL integer IOProc playback can return later as a specialized engine experiment, but it is not on the main product path right now.
- JSON output and stable exit codes for `pulse-cli` are important before agent/MCP integration, but they should be a dedicated harness stage, not mixed into app settings.
- A CLI TUI can help human debugging later, but it must not replace the boring scriptable command surface.
- Metadata enrichment from MusicBrainz, Cover Art Archive, Last.fm, Discogs, or Wikipedia is post-MVP.
- Smart Radio v1 is post-MVP.
- Synced lyrics, spectrum/VU visualizers, EQ, normalization, replay gain, crossfade, and editorial artist context are post-MVP.
