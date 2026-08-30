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

## M2 — Playback, session, app shell → v0.2.0

The audio path first, then session state, then the shell overhaul. Closed at feature 28 on 2026-08-30 (Jason: release 0.2.0 after the overhaul); metadata, now playing, and integrations moved to M3.

| Item | Priority | Status | Notes |
|---|---|---|---|
| [Feature 16 — gapless playback](features/archive/16-gapless-playback.md) | P1 | done | `4ae4fe4` engine, `7d61a32` app; validated on the Matrix DAC 2026-08-30. |
| [Bug — seek lands early](bugs/archive/seek-lands-early.md) | P2 | done | `a9dbef5`: accurate seek + decode-forward; FLAC fixture. |
| [Bug — flaky stall watchdog test](bugs/archive/flaky-stall-watchdog-test.md) | P2 | done | `a9dbef5`: injected clock, no sleep-paced stall tests. |
| [Bug — built-in speakers exclusive mode freezes volume](bugs/builtin-speakers-exclusive-volume.md) | P2 | review | `38734b8`: hardware volume while hogged, adopt-once-per-session; Jason to validate on the speakers and the Matrix DAC. |
| [Feature 17 — playback dropout reporting](features/archive/17-playback-dropout-reporting.md) | P2 | done | `5952c95`: underrun read path, `Dropout` events, sustained-dropout notice; diagnostic count surface deferred to a design pass. |
| [Bug — stale row selection after advance](bugs/archive/stale-row-selection-after-advance.md) | P2 | done | `cf02027`: selection cleared (playlist: re-anchored) when playback moves to another track. |
| [Feature 28 — app shell overhaul](features/archive/28-app-shell-overhaul.md) | P1 | done | `3edc358` (header, bands, player width), `d213219` (island sidebars, Settings sections with Output); both smoke-tested by Jason 2026-08-30. |
| [Feature 21 — launch state restore](features/archive/21-launch-state-restore.md) | P2 | done | `1c7880f`: session blob in `settings.json`, engine `Load` (paused, no sink), route restore; smoke: quit mid-album and relaunch. |

## M3 — Metadata, now playing, integrations → v0.3.0

Artist metadata first, then the now-playing page, then integrations and polish. Build order below.

| Item | Priority | Status | Notes |
|---|---|---|---|
| Feature 26 — metadata enrichment (spec to write) | P2 | planned | `artist_metadata` keyed by MBID (Lidarr's Artist/ArtistMetadata split), a `MetadataProvider` trait with a MusicBrainz + fanart.tv implementation, opt-in, cached, off the render path; artist photos fill the Artists grid/detail avatars; unidentified artists stay visible. Reference: [`docs/reference/lidarr-library-model.md`](reference/lidarr-library-model.md). Chromaprint/AcoustID is a licensing decision before any fingerprinting. |
| [Feature 20 — now playing page](features/20-now-playing-page.md) | P2 | planned | Design pinned. |
| [Feature 19 — MCP server](features/19-mcp-server.md) | P2 | planned | Needs 25's ops layer; the trigger for splitting `backend/` into a crate. |
| [Feature 09 — collapsible library sidebar](features/09-collapsible-library-sidebar.md) | P3 | planned | |
| [Feature 30 — interface scale](features/30-interface-scale.md) | P2 | designing | Settings ▸ General: Interface scale stepper 80–150% replacing the Default output row; phase 1 is the `px` → `rems` migration (folds in 29), phase 2 the setting + control + `Cmd-=`/`Cmd--`/`Cmd-0`. Design in Pencil awaiting Jason's approval. |
| [Feature 29 — type scale](features/29-type-scale.md) | P3 | planned | Refactor: named text sizes in `theme.rs` from the design's type styles, literal `text_size(px(N))` replaced, grep gate. Not urgent; delivered by 30's phase 1. |

## Parking lot

Ideas without a spec yet. Promote by writing the spec.

- Album-level metadata (release-group ids) once artist enrichment exists.
- Curation on the `artists` seams: display-name override, hide, merge aliases.
