mod app_store;
mod assets;
pub mod backend;
mod menu;
mod settings;
mod surfaces;
mod text_input;
mod theme;
mod toast;
mod ui;

use app_store::{AppStore, GlobalAppStore};
use assets::Assets;
use backend::{AppSettings, app_data_directory, load_or_migrate_app_settings, settings_path};
use gpui::{
    App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};
use surfaces::Shell;

#[cfg(target_os = "macos")]
fn install_app_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let main_thread = MainThreadMarker::new().expect("Pulse must start on the main thread");
    // dock.png bakes in the Apple icon-grid margin (content at 824/1024).
    let data = NSData::with_bytes(include_bytes!("../assets/app-icon/dock.png"));
    let image = NSImage::initWithData(NSImage::alloc(), &data).expect("invalid app icon");
    unsafe {
        NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&image));
    }
}

#[cfg(target_os = "macos")]
fn install_titlebar_toolbar() {
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSTitlebarSeparatorStyle, NSToolbar, NSWindowToolbarStyle};
    use objc2_foundation::NSString;

    let main_thread = MainThreadMarker::new().expect("Pulse must start on the main thread");
    let app = NSApplication::sharedApplication(main_thread);
    let window = app.windows().firstObject().expect("Pulse window not found");
    let identifier = NSString::from_str("pulse-titlebar");
    let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(main_thread), &identifier);

    // The native band must contain the traffic lights for them to remain clickable.
    window.setToolbar(Some(&toolbar));
    window.setToolbarStyle(NSWindowToolbarStyle::Unified);
    window.setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::None);
}

#[cfg(target_os = "macos")]
fn update_titlebar_toolbar_for_fullscreen(is_fullscreen: bool) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSWindowButton};

    let main_thread = MainThreadMarker::new().expect("Pulse must run on the main thread");
    let app = NSApplication::sharedApplication(main_thread);
    let window = app.windows().firstObject().expect("Pulse window not found");
    let toolbar = window.toolbar().expect("Pulse titlebar toolbar not found");
    if toolbar.isVisible() == is_fullscreen {
        toolbar.setVisible(!is_fullscreen);
    }

    if !is_fullscreen
        && let Some(button_group) = window
            .standardWindowButton(NSWindowButton::CloseButton)
            .and_then(|button| unsafe { button.superview() })
    {
        button_group.updateTrackingAreas();
    }
}

fn main() {
    gpui_platform::application()
        .with_assets(Assets)
        .run(|cx: &mut App| {
            #[cfg(target_os = "macos")]
            install_app_icon();

            cx.text_system()
                .add_fonts(assets::fonts())
                .expect("failed to load bundled fonts");
            menu::install(cx);

            let app_data_directory =
                app_data_directory().expect("failed to resolve app data directory");
            let settings_path = settings_path(&app_data_directory);
            let settings = load_or_migrate_app_settings().unwrap_or_else(|error| {
                eprintln!("Could not load app settings: {error}");
                AppSettings::default()
            });
            let app_store = cx.new(|cx| AppStore::new(settings_path, settings, cx));
            let interface_scale = app_store.read(cx).interface_scale();
            cx.set_global(GlobalAppStore(app_store.clone()));
            cx.on_app_quit(move |cx| {
                app_store.update(cx, |store, _| store.shutdown());
                std::future::ready(())
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx); // physical
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: None,
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(24.), px(30.))), // physical
                    }),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_rem_size(px(theme::REM_BASE_PX * interface_scale)); // physical
                    cx.new(|cx| Shell::new(window, cx))
                },
            )
            .expect("failed to open window");
            #[cfg(target_os = "macos")]
            {
                install_titlebar_toolbar();
                cx.defer(|_| update_titlebar_toolbar_for_fullscreen(false));
            }
            cx.activate(true);
        });
}
