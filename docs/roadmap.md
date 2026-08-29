# Roadmap

The active-development tracker for Pulse. Since 2026-08-29 this file — not GitHub issues — is where unfinished work lives (Jason: the repo is private and in active development; a local doc is the more efficient tracker). Features are specs in [`docs/features/`](features/README.md), bugs are notes in [`docs/bugs/`](bugs/README.md), and this board orders both into milestones. Durable product and architecture decisions go to `docs/product/` and `docs/arch/`; the MVP-era stage plan is [`docs/impls/ROADMAP.md`](impls/ROADMAP.md) and stays as history.

## How this board works

- A milestone is a release. When its rows are done, bump the version (`release` skill) and move the milestone to Shipped with the version and date.
- Row status is one of `planned` · `designing` · `in progress` · `review` · `done`. Priority is the spec's or note's P0–P3.
- New work is filed first (`feature` skill → spec, `bug` skill → note), then gets a row here. Crews never edit this file: a mission reports deferred work in its final Runner message and the human adds the row.
- Order within a milestone is build order. Reorder freely; the milestones themselves are the commitment.

## Shipped

- **v0.1.7** (2026-08-29) — Artists page with the v5 `artists` table ([11](features/archive/11-artists-page.md)); app architecture parity with Runner: `ui/` kit, `settings.json` + migration, `AppStore`, surfaces split ([24](features/archive/24-app-architecture-parity.md)).
- **v0.1.6** (2026-08-28) — Device management page ([22](features/archive/22-device-management-page.md)); updater parity with Runner ([23](features/archive/23-updater-parity.md)); click-free transport transitions; stalled-output detection.
- v0.1.0–v0.1.5 — MVP and hardening; see [`docs/impls/ROADMAP.md`](impls/ROADMAP.md) and the archived specs.

## M1 — Backend foundations → v0.1.8 (in progress)

Make the domain layer structural before more features land on it. All rows done 2026-08-29; awaiting Jason's smoke test, then the v0.1.8 release.

| Item | Priority | Status | Notes |
|---|---|---|---|
| [Feature 25 — backend layer + library repo layer](features/archive/25-library-repo-layer.md) | P2 | done | Four phases merged: `965965b` (backend boundary + gpui gate), `aa211ec` (repo layer), `49a6f00` (model + ops), `c1975cd` (facade retirement, SQL boundary gate, arch doc §4). |
| [Feature 27 — flatten the backend layout](features/archive/27-flatten-backend-layout.md) | P2 | done | `9b8205a`: `backend/{model.rs, repo/, ops/, scan/}`, moves only. |
| [Bug — app menu items disabled without focus](bugs/archive/app-menu-items-disabled.md) | P2 | done | `f51accf`: global handlers + `cx.defer`; also fixed Cmd-M / Cmd-W and Edit ▸ Search. |
| [Bug — macOS 12 deployment target](bugs/archive/macos-deployment-target.md) | P2 | done | `84cf129`: floor raised to 13.0 (arm64-only build; Monterey EOL). |
| [Bug — playback row transport feedback](bugs/archive/playback-row-transport-feedback.md) | P2 | done | `3d85c64`: kit-level hover/active, prev/next from the queue, play button stable across resume and next/previous. |
| [Bug — effective-artist SQL copies](bugs/archive/effective-artist-sql-copies.md) | P3 | done | Fixed by 25 phase 2 (`aa211ec`). |
| [Bug — album card image clipping](bugs/archive/album-card-image-clipping.md) | P3 | done | `84cf129`: radius on the cover `img`; Album Detail hero included. |
| [Bug — track-artist SQL copies](bugs/archive/track-artist-sql-copies.md) | P3 | done | `84cf129`: `EFFECTIVE_TRACK_ARTIST_SQL`, gated. |

## M2 — Playback, metadata, session, integrations → v0.2.0

Everything after the backend foundations, in build order: the audio path first, then artist metadata, then session state and the now-playing page, then integrations and polish.

| Item | Priority | Status | Notes |
|---|---|---|---|
| [Feature 16 — gapless playback](features/16-gapless-playback.md) | P1 | planned | Keep the sink and ring alive across same-format transitions. |
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
