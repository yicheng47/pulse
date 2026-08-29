use gpui::{
    AnyElement, FontWeight, IntoElement, RenderOnce, SharedString, Window, div, prelude::*, px,
};

use crate::theme;

#[derive(IntoElement)]
pub(crate) struct EmptyStateCard {
    icon: AnyElement,
    title: SharedString,
    description: SharedString,
    action: AnyElement,
}

impl EmptyStateCard {
    pub(crate) fn new(
        icon: impl IntoElement,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        action: impl IntoElement,
    ) -> Self {
        Self {
            icon: icon.into_any_element(),
            title: title.into(),
            description: description.into(),
            action: action.into_any_element(),
        }
    }
}

impl RenderOnce for EmptyStateCard {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .items_center()
            .justify_center()
            .px(px(28.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(54.))
                    .rounded_full()
                    .bg(theme::bg_muted())
                    .child(self.icon),
            )
            .child(
                div()
                    .mt(px(15.))
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(18.))
                    .text_color(theme::text_primary())
                    .child(self.title),
            )
            .child(
                div()
                    .mt(px(5.))
                    .max_w(px(250.))
                    .text_center()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child(self.description),
            )
            .child(div().mt(px(18.)).child(self.action))
    }
}
