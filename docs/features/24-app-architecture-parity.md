# App Architecture Parity with Runner

> Feature 24 · P1 · GitHub issue [#64](https://github.com/yicheng47/pulse-src/issues/64). Bring `pulse-app`'s structure up to Runner's current GPUI conventions — a UI kit, one settings file, an app-level store, and surfaces with their pure logic beside them — before more pages are built on the old shape. Behavior-preserving; no product change.

## Motivation

`pulse-app` grew page by page and it shows. The structural facts, measured 2026-08-29 on `main` (`160ff50`, v0.1.6):

- **Three god files.** `playback_row.rs` is 4,787 lines and owns the engine bridge, the queue, the volume popover, the output popover, the per-device store (managed devices, saved output, exclusive overrides), notices, and 54 tests. `library_ui/mod.rs` (2,140) and `shell.rs` (1,939 — sidebar, three settings pages, search, the IME text-input impl) are the same pattern. New surfaces bolt onto them: the Devices page (feature 22) had to take an `Entity<PlaybackRow>` just to read device state, and there are 39 `row.update`/`row.read` call sites outside `playback_row.rs`.
- **No app-level state.** Nothing is a `Global`; state lives in whichever view first needed it and other views reach in through entity handles. There is no single place a page can observe for "devices changed" or "playback changed", so each cross-view dependency is a bespoke handle plus a bespoke notify.
- **A preference file zoo.** `preferences.rs` (912 lines) hand-parses `app-output-device.uid`, `exclusive-modes.tsv`, `exclusive-mode.disabled`, `check-updates.disabled`, and volume level/mute files. Every new stored value means a new path, a new parser, and new round-trip tests; features 21 (launch state restore) and the Devices page each added to it.
- **No component kit.** `components.rs` is 315 lines of free functions (toggle, three buttons, badges). Everything else is re-derived per page: the hand-rolled scrollbar exists twice with byte-identical metrics math (`library_ui/tracks.rs` and `device_management.rs`), the destructive-confirm dialog is copy-pasted three times (delete album, delete playlist, forget device), and the update-hint tooltip is a one-off struct in `shell.rs`.

Runner solved each of these in its own GPUI app (`runner/crates/runner-app`, 53k lines) and the shapes are directly reusable: `ui/` (18 builder-style components), `app_settings.rs` (one serde struct, JSON), `app_store.rs` (one `Entity<AppStore>` registered as a `Global`, with revision counters that let surfaces react selectively), and `surfaces/` with `*_logic.rs` pure modules beside each surface. Pulse should adopt them now, while the app is 25k lines and five pages, rather than after Now Playing, Artists, and launch-state restore land on the current shape.

## Scope

Reference implementation: the Runner repo at `runner/crates/runner-app/src` on `main` as of 2026-08-29. Read it as a template; Pulse keeps its own theme, names, and domain. The spec deliberately excludes Runner machinery Pulse has no use for (multi-window `window_state.rs` layouts, terminal bridge, event bus coalescing).

1. **`ui/` component kit** (`crates/pulse-app/src/ui/`). Builder-style structs with the same API shape as Runner's `ui/` — `new(id, …)` then chained setters then `IntoElement` — for: `Button` / `IconButton` (variants primary, secondary, danger, compact; disabled; tooltip), `Toggle`, `Tooltip`, `Scrollbar` (with a `ScrollbarMetrics` value type carrying the one metrics function), `ConfirmDialog` / `Modal` (title, body, footer, busy), `SettingsRow` / `SettingsCard`, `PopoverMenu` / `ContextMenu`, `EmptyStateCard`, `Badge`/`pill`. `components.rs` is dissolved into it. Every existing use is migrated: the two scrollbars become one `Scrollbar`, the three confirm flows become `ConfirmDialog`, `UpdateHintTooltip` becomes `Tooltip`. The kit renders pixel-identically to today — the Pencil design is the acceptance reference, not a redesign target.
2. **`AppSettings`** (`app_settings.rs`). One `serde` struct saved as pretty JSON at `settings.json` in the app data directory, replacing every flat preference file: saved output device UID, per-device exclusive overrides plus the stored device table (name, capabilities, last seen), auto-update-check, volume level and mute, and whatever feature 21 needs (last route, queue, position) so that spec builds on this file rather than on new flat files. Runner's `AppSettings::load` / `save` / `normalize` and `#[serde(default)]` forward-compat are the template. A one-shot migration reads the legacy files on first launch, writes `settings.json`, and deletes them; the Sparkle-backed `check-updates.disabled` migration and Sparkle's own `SUEnableAutomaticChecks` persistence stay as they are today. Dev/prod isolation (feature 04) is unchanged: debug builds write `pulse-dev/settings.json`.
3. **`AppStore`** (`app_store.rs`). One `Entity<AppStore>` created at startup and registered as a `Global` (`GlobalAppStore(Entity<AppStore>)`, `global_app_store(cx)`), owning what more than one surface reads: `AppSettings`, the device table (`ManagedDevice`s, saved default, overrides, capability probes, sightings), the playback snapshot the sidebar/library/devices pages already consume (state, now-playing identity, active device, notice), and the queue view. It publishes `StoreRevisions` counters per domain (`settings`, `devices`, `playback`, `queue`, `library`) and surfaces `cx.observe` it and diff revisions to decide what to re-derive — Runner's `reactions_since` pattern, sized down to Pulse's five domains. The engine bridge (controller thread, `drain_events`) moves out of `PlaybackRow` into a `playback` module the store owns; `PlaybackRow` becomes the transport-bar surface and nothing else. `DeviceManagementPage` and `LibraryView` stop taking `Entity<PlaybackRow>` and read the store; playback commands go through one store method rather than `row.update` calls from other views.
4. **Surfaces and pure logic.** `surfaces/` for shell, sidebar, settings (split from `shell.rs` into `settings/{general,update,about}.rs`), playback_row, devices, and the library pages; each surface's tested pure functions move to a sibling `*_logic.rs` (Runner's `sidebar_logic.rs` / `chat_lifecycle.rs` pattern) so files stay in the hundreds of lines and tests target logic without a window. `library_ui/view_model.rs` already is this pattern and becomes `surfaces/library/logic.rs`. Target: no file in `pulse-app` over ~1,500 lines when done, none of the current three over 1,000.
5. **Architecture doc.** `docs/arch/pulse-app.md` records the result the way `docs/arch/pulse-engine.md` records the engine: module map, the store/observe contract, the settings file and migration, the `ui/` kit rules. Read `AGENTS.md`'s UI-agnostic-engine rule while writing it; nothing in this feature touches `pulse-engine`.

## Non-Goals

- Any engine change. `pulse-engine` is untouched; the controller's command/event protocol is not redesigned.
- Any visible change. No redesign, no theme work, no new pages; the Pencil design is the regression reference.
- Runner's multi-window `window_state.rs` / `window_layout.json`, terminal bridge, event bus, keymap system, or zoom. Pulse is single-window.
- Rewriting the library store (`library/store/*`, SQLite repo layer). It already has the right shape; only its UI consumers move.
- Feature 21 itself (launch state restore) — this feature only provides the settings file it should use.

## Design Source

None — no UI change. `design/pulse-desktop.pen` is the acceptance reference: every screen must render the same before and after each phase.

## Implementation Phases

Each phase is its own mission, merged and released independently; the app must be shippable after every one.

1. **`ui/` kit** — extract components, dedupe the scrollbar and confirm dialog, dissolve `components.rs`. Lowest risk, immediate payoff.
2. **`AppSettings` + migration** — one JSON file, legacy files migrated and removed, `preferences.rs` shrinks to path helpers.
3. **`AppStore`** — store entity as Global; device table and playback snapshot move in; engine bridge leaves `PlaybackRow`; `DeviceManagementPage` and `LibraryView` drop their `Entity<PlaybackRow>`.
4. **Surfaces + logic split** and `docs/arch/pulse-app.md`.

## Verification

- `make verify` green after every phase; the existing test count does not go down (tests move, they are not dropped).
- Phase 1: `library_ui/tracks.rs` and `device_management.rs` share one `Scrollbar` and one metrics function; grep finds a single `ConfirmDialog` implementation and zero `fn primary_button`-style free functions.
- Phase 2: a fixture directory with every legacy file migrates into one `settings.json` with identical values (unit test); a fresh install writes defaults; a settings file with unknown keys loads (forward compat); debug builds still write under `pulse-dev`. Manual: upgrade a real v0.1.6 data directory and confirm saved output, exclusive overrides, volume, and last-seen devices survive.
- Phase 3: `grep -rn "Entity<PlaybackRow>" crates/pulse-app/src` matches only the shell; store revision tests mirror Runner's (`refresh kinds preserve each data dependency`, `coalesced refreshes cannot drop a domain`); the Devices page and output popover render from the same store rows (existing feature 22 tests re-pointed, not rewritten).
- Phase 4: `wc -l` on `pulse-app` shows no file over ~1,500 lines; every `*_logic.rs` compiles without `gpui` imports.
- Manual after every phase: play a track, pause/seek, switch output device, toggle exclusive mode, open Devices, delete an album (confirm dialog), check Settings ▸ Update — all behave and look exactly as on v0.1.6.
