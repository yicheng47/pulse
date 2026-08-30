use crate::theme::rpx;

use gpui::{
    AnyElement, App, ElementId, FontWeight, IntoElement, Render, RenderOnce, SharedString, Window,
    div, prelude::*,
};

use crate::theme;

struct TooltipView {
    content: SharedString,
}

impl Render for TooltipView {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        tooltip_content(self.content.clone())
    }
}

#[derive(IntoElement)]
pub(crate) struct Tooltip {
    id: ElementId,
    content: SharedString,
    child: AnyElement,
}

impl Tooltip {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        content: impl Into<SharedString>,
        child: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            content: content.into(),
            child: child.into_any_element(),
        }
    }
}

impl RenderOnce for Tooltip {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let content = self.content;
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .relative()
            .tooltip(move |_, cx| {
                cx.new(|_| TooltipView {
                    content: content.clone(),
                })
                .into()
            })
            .child(self.child)
    }
}

fn tooltip_content(content: SharedString) -> impl IntoElement {
    div()
        .max_w(rpx(320.))
        .px(rpx(8.))
        .py(rpx(4.))
        .rounded(rpx(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border_strong())
        .bg(theme::bg_elevated())
        .font_family(theme::FONT_SANS)
        .font_weight(FontWeight::NORMAL)
        .text_size(theme::text::SMALL)
        .text_color(theme::text_secondary())
        .child(content)
}
