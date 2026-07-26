mod assets;
mod theme;

use assets::Assets;
use gpui::{
    App, Bounds, Context, FontWeight, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, size, svg,
};

struct PulseApp;

impl Render for PulseApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .bg(theme::bg_page())
            .child(
                svg()
                    .path("icons/play.svg")
                    .size(px(28.))
                    .text_color(theme::accent()),
            )
            .child(
                div()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(28.))
                    .text_color(theme::text_primary())
                    .child("Pulse"),
            )
    }
}

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
                |_, cx| cx.new(|_| PulseApp),
            )
            .expect("failed to open window");
            cx.activate(true);
        });
}
