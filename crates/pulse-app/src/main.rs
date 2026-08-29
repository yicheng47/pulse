mod app_settings;
mod app_store;
mod assets;
pub mod library;
mod menu;
mod playback;
mod preferences;
mod queue;
mod settings;
mod surfaces;
mod text_input;
mod theme;
mod ui;
mod updater;

use app_store::{AppStore, GlobalAppStore};
use assets::Assets;
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
                preferences::app_data_directory().expect("failed to resolve app data directory");
            let settings_path = app_settings::settings_path(&app_data_directory);
            let settings = preferences::load_or_migrate_app_settings().unwrap_or_else(|error| {
                eprintln!("Could not load app settings: {error}");
                app_settings::AppSettings::default()
            });
            let app_store = cx.new(|cx| AppStore::new(settings_path, settings, cx));
            cx.set_global(GlobalAppStore(app_store.clone()));
            cx.on_app_quit(move |cx| {
                app_store.update(cx, |store, _| store.shutdown());
                std::future::ready(())
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: None,
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(18.), px(24.))),
                    }),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(Shell::new),
            )
            .expect("failed to open window");
            cx.activate(true);
        });
}
