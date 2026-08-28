# Updater Parity with Runner

> Feature 23 · P2 · GitHub issue [#60](https://github.com/yicheng47/pulse-src/issues/60). Pulse's Sparkle updater catches up with Runner's: silent background checks, delegate-driven state the app can see, a quiet sidebar hint when a release is ready, and a Last checked row in Settings ▸ Update.

## Motivation

Pulse and Runner both embed Sparkle through `objc2`, but Runner's updater has moved on since Pulse copied the pattern: it observes Sparkle through a delegate, keeps update state in the app, checks silently in the background, and surfaces a found update as a quiet hint instead of Sparkle's alert popping over the app at launch. Pulse still runs the bare `SPUStandardUpdaterController` with no delegate, so a scheduled check that finds a release opens Sparkle's window on its own, the app has no idea an update exists, and Settings ▸ Update cannot show "update available" or "last checked". Bring Pulse's updater to parity with Runner's current behavior (Runner `main` as of 2026-08-28, `54d1c29`).

Current behavior (Pulse v0.1.5): `crates/pulse-app/src/updater.rs` creates `SPUStandardUpdaterController` with `updaterDelegate: None` and exposes only `check_for_updates` and the auto-check getter/setter; launch and scheduled checks are Sparkle's own scheduler driving the standard user driver. `shell.rs` `render_update_settings` shows the version chip, **Check for Updates**, and the "Check for updates on launch" toggle — no available-version line, no last-checked row. `menu.rs` has **Check for Updates…** (already at parity). No sidebar hint; no dev escape hatch, so the update UI cannot be exercised outside a signed bundle.

## Scope

Reference implementation: `runner/crates/runner-app/src/updater.rs`, `runner/crates/runner-app/src/surfaces/settings/updates.rs`, and the Settings-row hint in `runner/crates/runner-app/src/surfaces/app_shell.rs` (`settings_update_hint_version`). Design intent is recorded in Runner's briefs `docs/impls/archive/gpui-rewrite/briefs/m6-9-update-hint.md` and `m6-18-silent-update-checks.md`.

1. **Delegate-driven state.** Implement `SPUUpdaterDelegate` in Rust (`define_class!`, `NSObject` superclass, `MainThreadOnly`, retained for the controller's lifetime) with `updater:shouldProceedWithUpdate:updateCheck:error:`, `updater:didFindValidUpdate:`, `updaterDidNotFindUpdate:`, `updater:userDidMakeChoice:forUpdate:state:` (Skip clears), and `updater:didAbortWithError:`. Keep the decisions in pure functions the objc glue calls (`should_proceed`, `transition_for_abort`, `apply_available_transition`) so they are unit-testable without Sparkle. State: `available: Option<UpdateInfo { version }>` on the updater, transitions `Found / NotFound / Aborted / UserSkipped`, observers notified only on change. Version comes from `SUAppcastItem.displayVersionString`, falling back to `versionString`.
2. **Silent background checks.** On start: `startUpdater`, apply the auto-check setting, then `checkForUpdatesInBackground` immediately when enabled. In `shouldProceedWithUpdate`, record `Found(version)` and, for `SPUUpdateCheckUpdatesInBackground` (= 1), return NO with a populated `NSError` in Pulse's own domain (`com.wycstudios.pulse.updater`, code 1) so Sparkle neither shows nor downloads it; `didAbortWithError` ignores that domain so the decline does not clear `available`. Sparkle's sheet appears only from explicit user actions (`checkForUpdates:` via the menu item, the Settings button, or the hint).
3. **Sidebar hint.** An accent-colored `circle-arrow-down` icon at the right end of the sidebar's Settings row while `available` is `Some`; tooltip `Pulse {version} is ready to install`; clicking it calls `check_for_updates()` (Sparkle resumes at its install prompt). As shipped, the hint is its own 36px hit target beside the Settings row rather than a child of it, so the two clicks never overlap and opening Settings is unaffected. Hidden otherwise; the row looks exactly as today.
4. **Settings ▸ Update.** Add `Update available: v{version}` under the version block when `available` is `Some`, and a **Last checked** row from `SPUUpdater.lastUpdateCheckDate` (formatted local time, "Never" fallback) that refreshes while the page is visible (Runner polls every 30 s). Keep the existing button and toggle; the toggle keeps its Sparkle-backed persistence.
5. **Dev escape hatch.** `PULSE_DEV_UPDATE_AVAILABLE=<version>` seeds `available` in debug builds only (`cfg!(debug_assertions)`), so the hint and Settings copy can be exercised from `cargo run` where Sparkle never finds an update — including non-`updater` builds, whose Sparkle stub otherwise never reports an update and keeps its controls disabled with the existing note.

## Non-Goals

- Custom `SPUUserDriver` — Sparkle's standard sheet stays for the explicit path (same as Runner).
- Automatic downloads/installs, delta updates, channels.
- Any change to the release pipeline, appcast, or feed URL.

## Design Source

`design/pulse-desktop.pen`, read via the `pencil` MCP only. Applied 2026-08-28, mirroring Runner's `runner/design/runner.pen` (`cmp/SidebarC` → `sbUpdateIcon`):

- **Settings / Update** (`D9PDB`): the Version card's status line (`ZpZ9k`) reads `Update available: v0.2.0` in the positive green when a release is ready, and `You're on the latest version.` otherwise. Below the hero, a divider (`jOtnp`) and a **Last checked** Settings Row (`O5aSj`): title `Last checked`, description `Today at 9:41 PM — refreshes while this page is open`, mono value at the right (`2 min ago`); `Never` when Sparkle has no date.
- **Spec — Update States** (`ADNMS`) → **Sidebar Hint** (`eR34c`): the library sidebar's Settings row with a flex spacer and a 16px accent-pink lucide `circle-arrow-down` at the right end (`v2nkV`); tooltip `Pulse 0.2.0 is ready to install`; click opens Sparkle's sheet at the install prompt without opening Settings. The row is unchanged when nothing is available, which is why the hint lives in the spec frame and not in the `Sidebar / Library` component.
- Both sidebar components now draw the window's traffic lights (`Traffic Lights` component `QQFSz`, placed at 18, 24 — matching `main.rs` `traffic_light_position`); artwork only, no implementation change.

## Implementation Phases

1. `updater.rs`: delegate + state model per Scope 1–2, pure transition functions with unit tests; dev escape hatch (Scope 5).
2. `shell.rs`: sidebar Settings-row hint (Scope 3) and Settings ▸ Update rows (Scope 4), observing the updater so the hint appears without an unrelated relayout; register `icons/circle-arrow-down.svg` in `assets.rs`. The legacy `check-updates.disabled` migration in `preferences.rs` stays as is.
3. Tests and manual validation.

## Verification

- Unit tests on the pure transitions: `Found` sets, `NotFound` / `Aborted` / `UserSkipped` clear, a duplicate `Found` of the same version does not re-notify, `should_proceed` is false only for check kind 1, an abort in Pulse's own domain leaves `available` set.
- `cargo run` with `PULSE_DEV_UPDATE_AVAILABLE=0.9.9`: hint visible with tooltip, Settings shows the available line; without the env var nothing renders.
- Signed bundle against a newer published release: launch → no alert; hint lights; clicking it opens Sparkle's sheet at the install prompt; **Check for Updates…** and the Settings button open the sheet directly; Skip clears the hint; Last checked updates.
- `make verify` is green.
