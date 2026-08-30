// GPUI installs no macOS menu bar, and on macOS the standard shortcuts are
// delivered through it — without this module Cmd+Q, Cmd+W, Cmd+M and Cmd+H all
// do nothing.

use gpui::{App, KeyBinding, Menu, MenuItem, SystemMenuType, actions};

use crate::{settings::SettingsSection, surfaces::Shell};

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
        ZoomIn,
        ZoomOut,
        ActualSize,
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
    // Global action listeners run while GPUI holds the active window's update lease.
    cx.on_action(|_: &About, cx| {
        let Some(window) = cx
            .active_window()
            .and_then(|window| window.downcast::<Shell>())
        else {
            return;
        };
        cx.defer(move |cx| {
            if let Err(error) = window.update(cx, |shell, window, cx| {
                window.blur();
                shell.open_settings(SettingsSection::About, cx);
            }) {
                eprintln!("Pulse About failed: {error:#}");
            }
        });
    });
    cx.on_action(|_: &OpenSettings, cx| {
        let Some(window) = cx
            .active_window()
            .and_then(|window| window.downcast::<Shell>())
        else {
            return;
        };
        cx.defer(move |cx| {
            if let Err(error) = window.update(cx, |shell, window, cx| {
                window.blur();
                shell.open_settings(SettingsSection::General, cx);
            }) {
                eprintln!("Pulse settings failed: {error:#}");
            }
        });
    });
    cx.on_action(|_: &CheckForUpdates, cx| {
        let Some(window) = cx
            .active_window()
            .and_then(|window| window.downcast::<Shell>())
        else {
            return;
        };
        cx.defer(move |cx| {
            if let Err(error) = window.update(cx, |shell, _, cx| shell.check_for_updates(cx)) {
                eprintln!("Pulse update check failed: {error:#}");
            }
        });
    });
    cx.on_action(|_: &FocusSearch, cx| {
        let Some(window) = cx
            .active_window()
            .and_then(|window| window.downcast::<Shell>())
        else {
            return;
        };
        cx.defer(move |cx| {
            if let Err(error) = window.update(cx, |shell, window, cx| {
                if shell.can_focus_search() {
                    shell.focus_search(window, cx);
                }
            }) {
                eprintln!("Pulse focus search failed: {error:#}");
            }
        });
    });
    cx.on_action(|_: &ZoomIn, cx| {
        let Some(window) = cx
            .active_window()
            .and_then(|window| window.downcast::<Shell>())
        else {
            return;
        };
        cx.defer(move |cx| {
            if let Err(error) = window.update(cx, |shell, window, cx| {
                shell.increase_interface_scale(window, cx);
            }) {
                eprintln!("Pulse zoom in failed: {error:#}");
            }
        });
    });
    cx.on_action(|_: &ZoomOut, cx| {
        let Some(window) = cx
            .active_window()
            .and_then(|window| window.downcast::<Shell>())
        else {
            return;
        };
        cx.defer(move |cx| {
            if let Err(error) = window.update(cx, |shell, window, cx| {
                shell.decrease_interface_scale(window, cx);
            }) {
                eprintln!("Pulse zoom out failed: {error:#}");
            }
        });
    });
    cx.on_action(|_: &ActualSize, cx| {
        let Some(window) = cx
            .active_window()
            .and_then(|window| window.downcast::<Shell>())
        else {
            return;
        };
        cx.defer(move |cx| {
            if let Err(error) = window.update(cx, |shell, window, cx| {
                shell.reset_interface_scale(window, cx);
            }) {
                eprintln!("Pulse actual size failed: {error:#}");
            }
        });
    });
    cx.on_action(|_: &Minimize, cx| {
        if let Some(window) = cx.active_window() {
            cx.defer(move |cx| {
                if let Err(error) = window.update(cx, |_, window, _| window.minimize_window()) {
                    eprintln!("Pulse minimize failed: {error:#}");
                }
            });
        }
    });
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(window) = cx.active_window() {
            cx.defer(move |cx| {
                if let Err(error) = window.update(cx, |_, window, _| window.remove_window()) {
                    eprintln!("Pulse close window failed: {error:#}");
                }
            });
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
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("cmd-0", ActualSize, None),
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
        Menu::new("View").items([
            MenuItem::action("Zoom In", ZoomIn),
            MenuItem::action("Zoom Out", ZoomOut),
            MenuItem::action("Actual Size", ActualSize),
        ]),
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
