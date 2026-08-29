use gpui::{
    AnyElement, Div, ElementId, FontWeight, IntoElement, RenderOnce, Rgba, SharedString, Stateful,
    Window, div, linear_color_stop, linear_gradient, prelude::*, px,
};

use crate::theme;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum BadgeSize {
    #[default]
    Small,
    Large,
}

#[derive(IntoElement)]
pub(crate) struct Badge {
    label: SharedString,
    size: BadgeSize,
}

impl Badge {
    pub(crate) fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            size: BadgeSize::Small,
        }
    }

    pub(crate) fn size(mut self, size: BadgeSize) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for Badge {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        let large = self.size == BadgeSize::Large;
        div()
            .flex()
            .items_center()
            .h(px(if large { 21. } else { 18. }))
            .px(px(if large { 7. } else { 6. }))
            .when(!large, |badge| badge.flex_none())
            .rounded(px(theme::RADIUS_SM))
            .border_1()
            .border_color(theme::quality_border())
            .bg(theme::quality_soft())
            .font_family(theme::FONT_MONO)
            .font_weight(FontWeight::BOLD)
            .text_size(px(if large { 10. } else { 9. }))
            .text_color(theme::quality())
            .child(self.label)
    }
}

pub(crate) fn pill(label: impl Into<SharedString>, active: bool) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(9.))
        .py(px(5.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(if active {
            theme::accent()
        } else {
            theme::border_strong()
        })
        .bg(if active {
            theme::accent_soft()
        } else {
            theme::bg_elevated()
        })
        .when(active, |pill| {
            pill.child(div().size(px(6.)).rounded_full().bg(theme::accent()))
        })
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(11.))
                .text_color(if active {
                    theme::accent()
                } else {
                    theme::text_secondary()
                })
                .child(label.into()),
        )
}

pub(crate) fn exclusive_mode_reset_link(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .ml(px(8.))
        .cursor_pointer()
        .font_family(theme::FONT_SANS)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(11.))
        .text_color(theme::accent())
        .child("Reset to Auto")
}

pub(crate) fn exclusive_mode_control(
    automatic: bool,
    reset_link: AnyElement,
    toggle: AnyElement,
) -> Div {
    div()
        .flex()
        .items_center()
        .w_full()
        .child(
            div()
                .font_family(theme::FONT_SANS)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(12.))
                .text_color(theme::text_primary())
                .child("Exclusive mode"),
        )
        .child(if automatic {
            div()
                .ml(px(8.))
                .px(px(5.))
                .py(px(2.))
                .rounded(px(theme::RADIUS_SM))
                .border_1()
                .border_color(theme::border_strong())
                .bg(theme::bg_elevated())
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::BOLD)
                .text_size(px(9.))
                .text_color(theme::text_secondary())
                .child("AUTO")
                .into_any_element()
        } else {
            reset_link
        })
        .child(div().flex_1())
        .child(toggle)
}

pub(crate) fn playing_row_glow() -> AnyElement {
    let from = Rgba {
        a: 0.15,
        ..theme::accent()
    };
    let to = Rgba {
        a: 0.,
        ..theme::accent()
    };
    div()
        .absolute()
        .inset_0()
        .bg(linear_gradient(
            90.,
            linear_color_stop(from, 0.),
            linear_color_stop(to, 0.5),
        ))
        .into_any_element()
}

pub(crate) fn playing_row_bar() -> AnyElement {
    div()
        .absolute()
        .left_0()
        .top_0()
        .bottom_0()
        .w(px(4.))
        .bg(theme::accent())
        .into_any_element()
}

pub(crate) fn input_caret() -> AnyElement {
    div()
        .w(px(1.5))
        .h(px(14.))
        .flex_none()
        .bg(theme::accent())
        .into_any_element()
}
