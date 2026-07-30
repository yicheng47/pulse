mod assets;
pub mod library;
mod library_ui;
mod menu;
mod playback_row;
mod preferences;
mod shell;
mod theme;

use assets::Assets;
use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use shell::Shell;

fn main() {
    gpui_platform::application()
        .with_assets(Assets)
        .run(|cx: &mut App| {
            cx.text_system()
                .add_fonts(assets::fonts())
                .expect("failed to load bundled fonts");
            menu::install(cx);

            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Pulse".into()),
                        ..Default::default()
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
