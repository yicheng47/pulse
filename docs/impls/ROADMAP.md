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

## In Progress

| Stage | Impl Note | Status |
|-------|-----------|--------|
| 5 | [`0005-product-design.md`](0005-product-design.md) | Partially complete. Cyberpunk desktop foundation exists in Pencil for albums, tracks, playlists, storage, sidebar, and playback row. More detailed product surfaces still need design passes. |

## Current

| Stage | Impl Note | Goal | Boundary |
|-------|-----------|------|----------|
| 7 | [`0009-playback-row.md`](0009-playback-row.md) | Implemented: native GPUI app shell, Pencil-derived theme and embedded assets, window-wide file drop, playback row, app-owned controller state, play/pause, and drag-to-seek. Remaining: manual validation on the physical Matrix Mini-i Pro 4. | No command/IPC layer. No library scanner, SQLite, or other library surfaces in this stage. |

## MVP Path

| Stage | Impl Note | Goal | Notes |
|-------|-----------|------|-------|
| 7.5 | [`0010-app-shell.md`](0010-app-shell.md) | Build the shared app chrome. | Sidebar navigation, top bar, routed body, docked playback row. Prerequisite for stages 8-13 — device management and Storage have nowhere to live until it exists. Must not regress drop-to-play. |
| 8 | [`0011-output-device-management.md`](0011-output-device-management.md) | Implement output-device settings. | List devices, select the Pulse output device, persist by Core Audio UID, surface unavailable/hogged-device errors, and show active output in the playback row. Delivered through the Output Device Popover; the Devices page is not designed yet. Needs a narrow capability query added to `pulse-engine`. |
| 9 | — | Add storage roots and the library scanner. | Local folders and mounted NAS folders. PCM music files only: FLAC, ALAC, AIFF, WAV. No streaming and no video playback or video library support. Blocked on design: the Add Storage flow, scan progress/loading states, and offline-root/scan-failure states. |
| 10 | — | Add SQLite library store and search. | Store scanned metadata, cover-art cache paths, storage-root status, and FTS/search indexes. Artist remains metadata/filter context, not a primary destination. |
| 11 | — | Implement MVP library UI. | Build Albums, Tracks, Playlists, and Storage from `design/pulse-desktop.pen`. Add required missing design passes immediately before each UI slice. Storage panels carry off-token colors (`#151514`, `#111110`) that need a re-tokenize pass first. |
| 12 | — | Wire now-playing, queue, and playlists. | Playback row uses Stage 6 controller state. Manual playlists support create/rename/delete/add/remove/reorder/play. Queue supports next/previous and queue count. |
| 13 | — | Add MVP states and failure handling. | Loading, empty, offline storage, scan failure, decode failure, hogged device, unavailable device, and missing-file states. No silent failure. |
| 14 | — | MVP hardening and v0 release. | Hardware smoke matrix, scanner/library regression tests, CLI smoke tests, basic packaging, release notes, and final product-design cleanup for shipped surfaces. |

## Post-MVP / Parking Lot

- Raw HAL integer IOProc playback can return later as a specialized engine experiment, but it is not on the main product path right now.
- JSON output and stable exit codes for `pulse-cli` are important before agent/MCP integration, but they should be a dedicated harness stage, not mixed into app settings.
- A CLI TUI can help human debugging later, but it must not replace the boring scriptable command surface.
- Metadata enrichment from MusicBrainz, Cover Art Archive, Last.fm, Discogs, or Wikipedia is post-MVP.
- Smart Radio v1 is post-MVP.
- Synced lyrics, spectrum/VU visualizers, EQ, normalization, replay gain, crossfade, and editorial artist context are post-MVP.
