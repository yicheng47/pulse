mod assets;
pub mod library;
mod library_ui;
mod menu;
mod playback_row;
mod preferences;
mod queue;
mod shell;
mod theme;

use assets::Assets;
use gpui::{
    App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};
use shell::Shell;

#[cfg(target_os = "macos")]
fn install_app_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let main_thread = MainThreadMarker::new().expect("Pulse must start on the main thread");
    let data = NSData::with_bytes(include_bytes!("../assets/app-icon/v06cM3.png"));
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
