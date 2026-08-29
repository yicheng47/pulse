# Implementation Roadmap

This is the canonical engineering stage plan for Pulse. Product direction lives in `docs/product/vision.md`; MVP scope lives in [`docs/product/mvp.md`](../product/mvp.md). This roadmap controls implementation order so we do not drift into random probing.

Each stage should have a focused implementation note once work starts. A stage is done only when its verification path is clear enough to repeat. Shipped stages' notes move to [`archive/`](archive/) keeping their number. Post-MVP work is tracked in [`docs/roadmap.md`](../roadmap.md) (milestones), [`docs/features/`](../features/) (specs), and [`docs/bugs/`](../bugs/) (bug notes) — not in GitHub issues since 2026-08-29.

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
| 5 | [`archive/0005-product-design.md`](archive/0005-product-design.md) | Design baseline delivered and then some: every surface the note listed now exists in `design/pulse-desktop.pen`, plus album detail, empty/first-run states, storage state variants, search, playlist dialog, and context menu. Retired as too broad to ever close; remaining undesigned surfaces are tracked concretely in `docs/product/mvp.md` and the parking lot. |
| 6 | [`archive/0006-playback-controller.md`](archive/0006-playback-controller.md) | Added the UI-agnostic `PlaybackController` with play/pause/resume/seek/stop, events, and CLI smoke commands. Hardware smoke on the Matrix DAC still needs a manual pass. |
| 7 | [`archive/0009-playback-row.md`](archive/0009-playback-row.md) | Native GPUI playback row: theme, embedded assets, window-wide drop-to-play, play/pause, drag-to-seek. Matrix hardware smoke still open. |
| 7.5 | [`archive/0010-app-shell.md`](archive/0010-app-shell.md) | App chrome merged: sidebar, top bar, routed body, docked playback row, macOS menu. Includes the P1 `font-kit` fix. Nav/shortcut/drop manual checks still open. |
| 8 | [`archive/0011-output-device-management.md`](archive/0011-output-device-management.md) | Output Device Popover merged (PR #8): live enumeration, UID persistence, capability line, failure fallbacks. Device list verified against real hardware; by-ear and unplug/hog checks still open. |
| 9-10 | [`archive/0012-library-scan-and-store.md`](archive/0012-library-scan-and-store.md) | Merged as PR #9. Headless `pulse-app::library`: storage roots, incremental scanner with progress, Lofty tag/cover extraction, SQLite store. Measured on the real NAS: 4,904 tracks, 7.9× incremental speedup, zero errors. The Storage screen itself is stage 11. |
| 11 | [`archive/0013-library-ui.md`](archive/0013-library-ui.md) | Merged as PR #10. Albums, Album Detail, Tracks, Playlists-empty, and full Storage surfaces over schema v2 (year/genre, album and track queries, root rename, scan cancel). Manual acceptance pass still open — see Current. |
| 12 | [`archive/0014-queue-playlists-search.md`](archive/0014-queue-playlists-search.md) | Merged as PR #18. Schema v3 playlists with CRUD/reorder, transient queue with auto-advance and badge, LIKE search with `Cmd+F` and CJK IME, context menus, plus live-pass polish (Tracks pagination, responsive album cards, native chrome). Last MVP feature stage. |

## Current

| Stage | Impl Note | Goal | Boundary |
|-------|-----------|------|----------|
| — | [`MANUAL_ACCEPTANCE_CHECKLIST.md`](MANUAL_ACCEPTANCE_CHECKLIST.md) | Condensed manual acceptance pass over the whole MVP. | 96 human-only checks across 11 groups: hardware smoke, shell commands, device lifecycle, first-run/browsing, offline and scan-failure states, queue and transport, playlists, search, and the visual comparison. All features are now built — findings triage into fix-now vs stage 13 hardening. |

## MVP Path

| Stage | Impl Note | Goal | Notes |
|-------|-----------|------|-------|
| 13 | [`0015-mvp-states-and-hardening.md`](0015-mvp-states-and-hardening.md) | Add remaining MVP states and failure handling. | Narrower than it looks — empty, offline-storage and scan-failure states already shipped in stage 11. Remaining: loading states, decode failure, hogged/unavailable device during playback, and missing-file. Each needs a Pencil pass first. |
| 14 | — | v0 release. | Hardware smoke matrix, packaging the `.app`, release notes, and final design-vs-implementation cleanup. [`MANUAL_ACCEPTANCE_CHECKLIST.md`](MANUAL_ACCEPTANCE_CHECKLIST.md) is the gate for both 13 and 14. |

## Post-MVP / Parking Lot

- Raw HAL integer IOProc playback can return later as a specialized engine experiment, but it is not on the main product path right now.
- JSON output and stable exit codes for `pulse-cli` are important before agent/MCP integration, but they should be a dedicated harness stage, not mixed into app settings.
- A CLI TUI can help human debugging later, but it must not replace the boring scriptable command surface.
- Metadata enrichment from MusicBrainz, Cover Art Archive, Last.fm, Discogs, or Wikipedia is post-MVP.
- Smart Radio v1 is post-MVP.
- Synced lyrics, spectrum/VU visualizers, EQ, normalization, replay gain, crossfade, and editorial artist context are post-MVP.
