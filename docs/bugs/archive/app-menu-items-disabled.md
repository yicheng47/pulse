# App menu: About / Settings… / Check for Updates… are disabled unless something in the window has focus

> Bug · P2 · filed 2026-08-29 as GitHub issue #69 (closed when tracking moved to docs, 2026-08-29). **Fixed** 2026-08-29, commit `f51accf`: the three actions (plus Edit ▸ Search) are global `cx.on_action` handlers in `menu::install`, routed to the active `Shell` window through `cx.defer` because GPUI holds the window lease during action dispatch. The same defer fixed the pre-existing `Minimize` / `Close Window` no-ops (Cmd-M / Cmd-W).

## Description
The Pulse app menu shows **About Pulse**, **Settings…**, and **Check for Updates…** greyed out (screenshot from Jason, 2026-08-29, v0.1.7 / `main`), while Hide / Hide Others / Show All / Quit work.

Cause: those three handlers are `on_action` listeners on the shell's root `div` (`surfaces/shell.rs` ~142–153), and GPUI validates a macOS menu item by walking the dispatch tree from the **focused** element up to the window root (`Window::available_actions`). When nothing in the window has focus, the walk starts at the window's own root node — which is *above* the shell div — so the listeners are never on the path and the items are disabled. Hide/Others/Quit stay enabled because `menu::install` registers them globally with `cx.on_action`. It gets worse after use: each of the three handlers calls `window.blur()`, so invoking one clears focus and disables all three for the next menu open. Same code shipped in v0.1.6 (`shell.rs` 1527–1540 at that tag) — not a regression from feature 24.

## Expected Behavior
The three items are always enabled while a Pulse window exists, regardless of focus, and keep working after use.

## Steps To Reproduce
1. Launch Pulse; do not click into the search box or any input.
2. Open the app menu → About / Settings… / Check for Updates… are disabled.
3. Click the search box (Cmd-F), open the menu again → enabled. Choose Settings… → menu re-opened → disabled again (the handler blurred the window).

## Relevant Code
- `crates/pulse-app/src/surfaces/shell.rs` ~142–153 — element-level `on_action` for `OpenSettings`, `CheckForUpdates`, `About`, each calling `window.blur()`.
- `crates/pulse-app/src/menu.rs` — `install` registers `Quit`/`Hide`/`HideOthers`/`ShowAll`/`Minimize`/`CloseWindow` globally; the three app items are only wired as `MenuItem::action`.
- Runner does it the focus-independent way: `runner-app/src/main.rs` ~1136 `cx.on_action(|_: &CheckForUpdates, cx| global_updater(cx).read(cx).check_for_updates())`.

## Fix
Register the three actions globally in `menu::install` (or `main.rs`) with `cx.on_action`, and route to the shell through the active window: `cx.active_window()` → `window.update(cx, |root, window, cx| …)` downcasting the root view to `Shell` (or a small global holding `WeakEntity<Shell>`, the `GlobalAppStore` pattern). `CheckForUpdates` can go straight to the updater entity like Runner. Drop the `window.blur()` calls or keep them as a no-op for menu validation. Keep `FocusSearch` element-level if desired, but `Cmd-,` (`OpenSettings`) should also go global so the shortcut works without focus. Verify by the repro above plus: choose Settings… twice in a row from the menu.

## Environment
- OS: macOS (Darwin 25.5)
- Pulse version: v0.1.7 (present since v0.1.6 at least)

## Verification
Code review of `surfaces/shell.rs` and `menu.rs` on `main` `63381a8`; GPUI dispatch semantics from `gpui-ce` `6c799b8` (`available_actions` walks from the focused node).

