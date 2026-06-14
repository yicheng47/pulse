# Implementation Roadmap

This is the canonical engineering stage plan for Pulse. Product direction lives in `docs/product/vision.md`; this roadmap controls implementation order so we do not drift into random probing.

Each stage should have a focused implementation note once work starts. A stage is done only when its verification path is clear enough to repeat.

## Stage Rules

- Finish one stage before starting the next stage branch.
- Keep each stage small enough to review as one PR.
- Do not implement frontend surfaces before a Pencil design exists for that surface.
- Do not harden app-specific Tauri settings backends before the settings UX and storage model are designed.
- Keep `pulse-engine` UI-agnostic and drivable from `pulse-cli`.
- Keep `pulse-cli` deterministic and scriptable because it is the future agent/MCP harness boundary.
- Persist Core Audio device identity by UID, not by transient `AudioDeviceID`.

## Completed

| Stage | Impl Note | Outcome |
|-------|-----------|---------|
| 1 | [`0001-engine-validation-cli.md`](0001-engine-validation-cli.md) | Created the standalone engine validation CLI and proved file/device inspection. |
| 2 | [`0002-hal-format-validation.md`](0002-hal-format-validation.md) | Proved hog mode, nominal sample-rate switching, and physical-format diagnostics. |
| 3 | [`0003-auhal-playback.md`](0003-auhal-playback.md) | Switched playback to AUHAL and produced clean native-rate playback on the Matrix DAC. |
| 4 | [`0004-cli-config.md`](0004-cli-config.md) | Added UID-backed CLI default output config for repeatable harness use. |

## Current

| Stage | Impl Note | Goal | Boundary |
|-------|-----------|------|----------|
| 5 | [`0005-product-design.md`](0005-product-design.md) | Design the desktop settings/library shell in Pencil. | No React UI or app-settings Tauri backend until the design defines the contract. |

## Next

| Stage | Goal | Notes |
|-------|------|-------|
| 6 | Implement the designed app shell and settings surface. | Add Tauri backend only where the Stage 5 design needs concrete data or persistence. App settings probably belong in SQLite once the app store exists. |
| 7 | Add local library scanner and SQLite store. | PCM music files only: FLAC, ALAC, AIFF, WAV. No streaming and no video. |
| 8 | Add fast library browsing and search. | Build around local library UX, not enrichment or radio yet. |
| 9 | Add playback queue and now-playing foundation. | Use the existing AUHAL engine path; avoid bit-perfect claims. |
| 10 | Add metadata enrichment. | MusicBrainz / Cover Art Archive / Last.fm / Discogs / Wikipedia can be evaluated here. |
| 11 | Add Smart Radio v1. | Use the user's own library only. |
| 12 | Polish and package. | Packaging, release notes, app icon pass, and smoke matrix. |

## Parking Lot

- Raw HAL integer IOProc playback can return later as a specialized engine experiment, but it is not on the main product path right now.
- JSON output and stable exit codes for `pulse-cli` are important before agent/MCP integration, but they should be a dedicated harness stage, not mixed into app settings.
- A CLI TUI can help human debugging later, but it must not replace the boring scriptable command surface.
