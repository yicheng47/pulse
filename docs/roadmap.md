# Roadmap

The active-development tracker for Pulse. Since 2026-08-29 this file — not GitHub issues — is where unfinished work lives (Jason: the repo is private and in active development; a local doc is the more efficient tracker). Features are specs in [`docs/features/`](features/README.md), bugs are notes in [`docs/bugs/`](bugs/README.md), and this board orders both into milestones. Durable product and architecture decisions go to `docs/product/` and `docs/arch/`; the MVP-era stage plan is [`docs/impls/ROADMAP.md`](impls/ROADMAP.md) and stays as history.

## How this board works

- A milestone is a release. When its rows are done, bump the version (`release` skill) and move the milestone to Shipped with the version and date.
- Row status is one of `planned` · `designing` · `in progress` · `review` · `done`. Priority is the spec's or note's P0–P3.
- New work is filed first (`feature` skill → spec, `bug` skill → note), then gets a row here. Crews never edit this file: a mission reports deferred work in its final Runner message and the human adds the row.
- Order within a milestone is build order. Reorder freely; the milestones themselves are the commitment.

## Shipped

- **v0.1.8** (2026-08-29) — Backend foundations: `backend/{model.rs, repo/, ops/, scan/}` with gpui and SQL boundary gates ([25](features/archive/25-library-repo-layer.md), [27](features/archive/27-flatten-backend-layout.md)); transport buttons with hover/pressed feedback and a stable play button; app menu items and ⌘M / ⌘W working without focus; rounded album covers; one track-artist SQL fragment; macOS 13 floor.
- **v0.1.7** (2026-08-29) — Artists page with the v5 `artists` table ([11](features/archive/11-artists-page.md)); app architecture parity with Runner: `ui/` kit, `settings.json` + migration, `AppStore`, surfaces split ([24](features/archive/24-app-architecture-parity.md)).
- **v0.1.6** (2026-08-28) — Device management page ([22](features/archive/22-device-management-page.md)); updater parity with Runner ([23](features/archive/23-updater-parity.md)); click-free transport transitions; stalled-output detection.
- v0.1.0–v0.1.5 — MVP and hardening; see [`docs/impls/ROADMAP.md`](impls/ROADMAP.md) and the archived specs.

## M2 — Playback, metadata, session, integrations → v0.2.0

Everything after the backend foundations, in build order: the audio path first, then artist metadata, then session state and the now-playing page, then integrations and polish.

| Item | Priority | Status | Notes |
|---|---|---|---|
| [Feature 16 — gapless playback](features/16-gapless-playback.md) | P1 | review | Phases 1–3 merged (`4ae4fe4` engine, `7d61a32` app wiring); phase 4 = Jason's Matrix DAC validation, then archive. |
| [Bug — seek lands early](bugs/seek-lands-early.md) | P2 | planned | Coarse seek, no decode-forward. |
| [Bug — built-in speakers exclusive mode freezes volume](bugs/builtin-speakers-exclusive-volume.md) | P2 | planned | Probably: never take exclusive on the built-in output, or restore the hardware level on release. |
| [Feature 17 — playback dropout reporting](features/17-playback-dropout-reporting.md) | P2 | planned | Read side of the underrun counter. |
| Feature 26 — metadata enrichment (spec to write) | P2 | planned | `artist_metadata` keyed by MBID (Lidarr's Artist/ArtistMetadata split), a `MetadataProvider` trait with a MusicBrainz + fanart.tv implementation, opt-in, cached, off the render path; artist photos fill the Artists grid/detail avatars; unidentified artists stay visible. Reference: [`docs/reference/lidarr-library-model.md`](reference/lidarr-library-model.md). Chromaprint/AcoustID is a licensing decision before any fingerprinting. |
| [Feature 21 — launch state restore](features/21-launch-state-restore.md) | P2 | planned | Persists into `settings.json`; never auto-plays. |
| [Feature 20 — now playing page](features/20-now-playing-page.md) | P2 | planned | Design pinned. |
| [Feature 19 — MCP server](features/19-mcp-server.md) | P2 | planned | Needs 25's ops layer; the trigger for splitting `backend/` into a crate. |
| [Feature 09 — collapsible library sidebar](features/09-collapsible-library-sidebar.md) | P3 | planned | |

## Parking lot

Ideas without a spec yet. Promote by writing the spec.

- Album-level metadata (release-group ids) once artist enrichment exists.
- Curation on the `artists` seams: display-name override, hide, merge aliases.
