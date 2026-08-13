// GPUI installs no macOS menu bar, and on macOS the standard shortcuts are
// delivered through it — without this module Cmd+Q, Cmd+W, Cmd+M and Cmd+H all
// do nothing.

use gpui::{App, KeyBinding, Menu, MenuItem, SystemMenuType, actions};

actions!(
    pulse,
    [
        About,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        CloseWindow,
        FocusSearch,
        CheckForUpdates,
        OpenSettings,
        Quit
    ]
);

pub fn install(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| {
        if let Some(window) = cx.active_window() {
            window
                .update(cx, |_, window, _| window.minimize_window())
                .ok();
        }
    });
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(window) = cx.active_window() {
            window
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
    });

    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-f", FocusSearch, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
    ]);

    cx.set_menus([
        Menu::new("Pulse").items([
            MenuItem::action("About Pulse", About),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Pulse", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Pulse", Quit),
        ]),
        Menu::new("Edit").items([MenuItem::action("Search", FocusSearch)]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Close Window", CloseWindow),
        ]),
    ]);

    // Closing the only window would otherwise leave Pulse running with no way
    // to get a window back.
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}
