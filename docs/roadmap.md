# Roadmap

The active-development tracker for Pulse. Since 2026-08-29 this file — not GitHub issues — is where unfinished work lives (Jason: the repo is private and in active development; a local doc is the more efficient tracker). Features are specs in [`docs/features/`](features/README.md), bugs are notes in [`docs/bugs/`](bugs/README.md), and this board orders both into milestones. Durable product and architecture decisions go to `docs/product/` and `docs/arch/`; the MVP-era stage history lives in [`docs/impls/archive/`](impls/archive/).

## Now

The cursor for a fresh session — current state of `main` in one glance; details in the milestone tables below. Updated 2026-09-01.

- **Just landed**: feature 74 + feature 71 phase 3 (`ee09b0c`) — the toast primitive, error-surface migration, and the DSD refusal UI (typed gate errors, Switch-to-Bit-perfect retry, library dimming, queue skip). Jason's smoke test caught one design mismatch (action button), fixed before merge.
- **In flight**: [bug — device not released on quit](bugs/device-not-released-on-quit.md) (P1) — mission running in drive mode; merge on lead review.
- **Waiting on Jason**: feature 71 phase 4 remainder — pause-lock / seek-relock / refusal checks on the Matrix; closes [#71](https://github.com/yicheng47/pulse/issues/71). Open product pick: pause-release flavor (grace period vs immediate) for the exclusive device.
- **Up next**: feature 56 — now-playing page (design pinned, mission-ready).

## How this board works

- A milestone is a release. When its rows are done, bump the version (`release` skill) and move the milestone to Shipped with the version and date.
- Row status is one of `planned` · `designing` · `in progress` · `review` · `done`. Priority is the spec's or note's P0–P3.
- New work is filed first (`feature` skill → issue + spec, `bug` skill → note), then gets a row here. Crews never edit this file: a mission reports deferred work in its final Runner message and the human adds the row.
- The `## Now` section above is the cursor: refresh it whenever work lands on `main` or a new slice starts, so a fresh agent reads one section instead of reconstructing state from git and three indexes.
- Order within a milestone is build order. Reorder freely; the milestones themselves are the commitment.

## Shipped

- **v0.3.0** (2026-09-01) — The bit-perfect release: raw-HAL integer engine proven bit-exact by the DoP test (the Matrix read "DSD DoP 2.8MHz") ([32](features/archive/32-integer-engine.md)); one-axis Output mode control (Shared·Exclusive·Bit-perfect, AUTO-resolved, supersedes feature 08's toggle); pause holds the device; volume transparency — domains, the Signal Path popover, honest FIXED slider ([31](features/archive/31-volume-transparency.md)). Filed-to-shipped in ~30 hours across six crew missions; journey log in [`impls/archive/bit-perfect/`](impls/archive/bit-perfect/IMPL_LOG.md).
- **v0.2.0** (2026-08-30) — App shell overhaul: full-width header, island sidebars, Settings sections with Output ([28](features/archive/28-app-shell-overhaul.md)); interface scale 80–150% on a `px` → `rems` migration ([30](features/archive/30-interface-scale.md), [29](features/archive/29-type-scale.md)); gapless playback ([16](features/archive/16-gapless-playback.md)); launch state restore ([21](features/archive/21-launch-state-restore.md)); dropout reporting ([17](features/archive/17-playback-dropout-reporting.md)); hardware volume while hogged, accurate seek, stale-selection fix.
- **v0.1.8** (2026-08-29) — Backend foundations: `backend/{model.rs, repo/, ops/, scan/}` with gpui and SQL boundary gates ([25](features/archive/25-library-repo-layer.md), [27](features/archive/27-flatten-backend-layout.md)); transport buttons with hover/pressed feedback and a stable play button; app menu items and ⌘M / ⌘W working without focus; rounded album covers; one track-artist SQL fragment; macOS 13 floor.
- **v0.1.7** (2026-08-29) — Artists page with the v5 `artists` table ([11](features/archive/11-artists-page.md)); app architecture parity with Runner: `ui/` kit, `settings.json` + migration, `AppStore`, surfaces split ([24](features/archive/24-app-architecture-parity.md)).
- **v0.1.6** (2026-08-28) — Device management page ([22](features/archive/22-device-management-page.md)); updater parity with Runner ([23](features/archive/23-updater-parity.md)); click-free transport transitions; stalled-output detection.
- v0.1.0–v0.1.5 — MVP and hardening; see the notes in [`docs/impls/archive/`](impls/archive/) and the archived specs.

## M2 — Playback, session, app shell → v0.2.0 (shipped 2026-08-30)

Kept as the record of what v0.2.0 contained; every row is done.

| Item | Priority | Status | Notes |
|---|---|---|---|
| [Feature 16 — gapless playback](features/archive/16-gapless-playback.md) | P1 | done | `4ae4fe4` engine, `7d61a32` app; validated on the Matrix DAC 2026-08-30. |
| [Bug — seek lands early](bugs/archive/seek-lands-early.md) | P2 | done | `a9dbef5`: accurate seek + decode-forward; FLAC fixture. |
| [Bug — flaky stall watchdog test](bugs/archive/flaky-stall-watchdog-test.md) | P2 | done | `a9dbef5`: injected clock, no sleep-paced stall tests. |
| [Bug — built-in speakers exclusive mode freezes volume](bugs/archive/builtin-speakers-exclusive-volume.md) | P2 | done | `38734b8`: hardware volume while hogged, adopt-once-per-session; validated by Jason on hardware 2026-08-30. |
| [Feature 17 — playback dropout reporting](features/archive/17-playback-dropout-reporting.md) | P2 | done | `5952c95`: underrun read path, `Dropout` events, sustained-dropout notice; diagnostic count surface deferred to a design pass. |
| [Bug — stale row selection after advance](bugs/archive/stale-row-selection-after-advance.md) | P2 | done | `cf02027`: selection cleared (playlist: re-anchored) when playback moves to another track. |
| [Feature 28 — app shell overhaul](features/archive/28-app-shell-overhaul.md) | P1 | done | `3edc358` (header, bands, player width), `d213219` (island sidebars, Settings sections with Output); both smoke-tested by Jason 2026-08-30. |
| [Feature 30 — interface scale](features/archive/30-interface-scale.md) | P2 | done | `05b9d08` (px → rems, type scale), `56a120e` (setting, General stepper, View menu shortcuts); smoke-tested by Jason 2026-08-30. |
| [Feature 29 — type scale](features/archive/29-type-scale.md) | P3 | done | 05b9d08, with feature 30 phase 1. |
| [Feature 21 — launch state restore](features/archive/21-launch-state-restore.md) | P2 | done | `1c7880f`: session blob in `settings.json`, engine `Load` (paused, no sink), route restore; smoke: quit mid-album and relaunch. |

## M3 — Bit-perfect engine → v0.3.0 (shipped 2026-09-01)

A challenge milestone (Jason, 2026-08-31): prove a bit-exact delivery path exists, as a second engine the user can pick. Kept as the record; every row is done. The DoP acceptance passed 2026-09-01: the Matrix read "DSD DoP 2.8MHz".

| Item | Priority | Status | Notes |
|---|---|---|---|
| [Feature 32 — integer bit-perfect engine](features/archive/32-integer-engine.md) | P2 | done | Five staged missions 2026-08-31 (probe gate PASS, restore guard, engine `689bdbb`, app wiring `668e504`, DoP payload `de5bf37`); DoP acceptance passed on the Matrix 2026-09-01. |
| [Feature 31 — volume transparency](features/archive/31-volume-transparency.md) | P2 | done | `0c44e08`: volume domains, Signal Path popover, FIXED slider disable, Devices line. |


## M4 — DSD, now playing, integrations → v0.4.0

DSD playback first, then the now-playing page, then integrations and polish; artist metadata postponed (Jason, 2026-09-01) — it keeps its bottom row until re-prioritized. Build order below.

| Item | Priority | Status | Notes |
|---|---|---|---|
| [Feature 71 — DSD playback over DoP](features/71-dsd-over-dop.md) | P2 | in progress | Phases 1–2 `f50e040`; phase 3 (refusal UI on the toast primitive) `ee09b0c`. Remaining: phase 4 Matrix acceptance (Jason, hardware at home). Impl note: [`impls/71-dsd-over-dop.md`](impls/71-dsd-over-dop.md). |
| [Feature 74 — toast notifications](features/archive/74-toast-notifications.md) | P2 | done | `ee09b0c`, codex crew mission 2026-09-01: primitive + migration + 71's refusal UI; smoke-tested by Jason (one design fix round on the action button). |
| [Feature 56 — now playing page](features/56-now-playing-page.md) | P2 | planned | Design pinned. |
| [Feature 55 — MCP server](features/55-mcp-server.md) | P2 | planned | Needs 25's ops layer; the trigger for splitting `backend/` into a crate. |
| [Feature 75 — scan performance](features/75-scan-performance.md) | P2 | planned | Folder-art memo, chunked writer transactions, bounded extractor pool; single-connection ownership model unchanged. Backend-only, no design gate; filed 2026-09-01. |
| [Feature 73 — scan error detail](features/73-scan-error-detail.md) | P3 | planned | Persist per-file scan errors; ISSUES badge opens path + plain-language reason + Reveal in Finder. Pencil pass before the UI phase. |
| [Feature 72 — metadata enrichment](features/72-metadata-enrichment.md) | P2 | designing | Postponed (Jason, 2026-09-01). Spec + Settings ▸ Metadata design drafted 2026-08-30; awaiting approval. Phase 1 backend + enrich dry-run flag, phase 2 settings page, triggers, photos. Carries the placeholder-artist bug. |
| [Bug — device not released on quit](bugs/device-not-released-on-quit.md) | P1 | planned | Quit must deterministically unhog + restore format before exit; investigate which drop in the teardown chain gets skipped. Pairs naturally with the pause-release decision when it's made. |
| [Bug — output popover mode row crowding](bugs/output-popover-mode-row-crowding.md) | P2 | planned | `Reset to Auto` collides with the mode segments in the output popover when an override is set; scale-sensitive. Layout fix in `ui::output_mode_control`; Pencil pass only if the row wraps to two lines. |
| [Bug — placeholder album artist becomes an artist](bugs/placeholder-album-artist.md) | P3 | planned | `######` and similar name-less album artists fall back to the track artist in the scanner; files already fixed at the source. Small; rides with feature 72 when it un-postpones. |

## Parking lot

Ideas without a spec yet. Promote by writing the spec.

- Album-level metadata (release-group ids) once artist enrichment exists.
- Curation on the `artists` seams: display-name override, hide, merge aliases.
