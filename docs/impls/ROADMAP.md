# Implementation Roadmap

This is the canonical engineering stage plan for Pulse. Product direction lives in `docs/product/vision.md`; this roadmap controls implementation order so we do not drift into random probing.

Each stage should have a focused implementation note once work starts. A stage is done only when its verification path is clear enough to repeat.

## Stage Rules

- Finish one stage before starting the next stage branch.
- Keep each stage small enough to review as one PR.
- Do not implement frontend surfaces before a Pencil design exists for that surface.
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
| 5 | [`0005-desktop-device-settings.md`](0005-desktop-device-settings.md) | Add backend-only Tauri output-device settings commands and app config persistence. | No React settings UI until the settings surface is designed in Pencil. |

## Next

| Stage | Goal | Notes |
|-------|------|-------|
| 6 | Design desktop settings/library shell in Pencil. | Settings, library, now-playing, and search need a coherent app shape before React implementation. |
| 7 | Implement the designed app shell and settings surface. | Use Stage 5 backend commands; keep playback UI out unless the design explicitly covers it. |
| 8 | Add local library scanner and SQLite store. | PCM music files only: FLAC, ALAC, AIFF, WAV. No streaming and no video. |
| 9 | Add fast library browsing and search. | Build around local library UX, not enrichment or radio yet. |
| 10 | Add playback queue and now-playing foundation. | Use the existing AUHAL engine path; avoid bit-perfect claims. |
| 11 | Add metadata enrichment. | MusicBrainz / Cover Art Archive / Last.fm / Discogs / Wikipedia can be evaluated here. |
| 12 | Add Smart Radio v1. | Use the user's own library only. |
| 13 | Polish and package. | Packaging, release notes, app icon pass, and smoke matrix. |

## Parking Lot

- Raw HAL integer IOProc playback can return later as a specialized engine experiment, but it is not on the main product path right now.
- JSON output and stable exit codes for `pulse-cli` are important before agent/MCP integration, but they should be a dedicated harness stage, not mixed into app settings.
- A CLI TUI can help human debugging later, but it must not replace the boring scriptable command surface.
