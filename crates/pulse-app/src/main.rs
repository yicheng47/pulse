mod theme;

use gpui::{
    App, Application, Bounds, Context, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, size,
};

struct PulseApp;

impl Render for PulseApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(theme::bg_page())
            .text_color(theme::text_primary())
            .child("Pulse")
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
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
