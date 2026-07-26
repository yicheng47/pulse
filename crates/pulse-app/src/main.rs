mod assets;
mod playback_row;
mod theme;

use assets::Assets;
use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use playback_row::PlaybackRow;

fn main() {
    gpui_platform::application()
        .with_assets(Assets)
        .run(|cx: &mut App| {
            cx.text_system()
                .add_fonts(assets::fonts())
                .expect("failed to load bundled fonts");

            let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Pulse".into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(PlaybackRow::new),
            )
            .expect("failed to open window");
            cx.activate(true);
        });
}
