use crate::theme::rpx;

use gpui::{
    AnyElement, CursorStyle, Div, ElementId, FontWeight, IntoElement, RenderOnce, Rgba,
    SharedString, Stateful, Window, div, linear_color_stop, linear_gradient, prelude::*,
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
    warning: bool,
}

impl Badge {
    pub(crate) fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            size: BadgeSize::Small,
            warning: false,
        }
    }

    pub(crate) fn size(mut self, size: BadgeSize) -> Self {
        self.size = size;
        self
    }

    pub(crate) fn warning(mut self, warning: bool) -> Self {
        self.warning = warning;
        self
    }
}

impl RenderOnce for Badge {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        let large = self.size == BadgeSize::Large;
        div()
            .flex()
            .items_center()
            .h(rpx(if large { 21. } else { 18. }))
            .px(rpx(if large { 7. } else { 6. }))
            .when(!large, |badge| badge.flex_none())
            .rounded(rpx(theme::RADIUS_SM))
            .border_1()
            .border_color(if self.warning {
                theme::warning()
            } else {
                theme::quality_border()
            })
            .bg(if self.warning {
                theme::bg_elevated()
            } else {
                theme::quality_soft()
            })
            .font_family(theme::FONT_MONO)
            .font_weight(FontWeight::BOLD)
            .text_size(theme::text::CAPTION_XS)
            .when(large, |badge| badge.text_size(theme::text::CAPTION))
            .text_color(if self.warning {
                theme::warning()
            } else {
                theme::quality()
            })
            .child(self.label)
    }
}

pub(crate) fn pill(label: impl Into<SharedString>, active: bool) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(rpx(6.))
        .px(rpx(9.))
        .py(rpx(5.))
        .rounded(rpx(theme::RADIUS_SM))
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
            pill.child(div().size(rpx(6.)).rounded_full().bg(theme::accent()))
        })
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(theme::text::SMALL)
                .text_color(if active {
                    theme::accent()
                } else {
                    theme::text_secondary()
                })
                .child(label.into()),
        )
}

pub(crate) fn output_mode_reset_link(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .ml(rpx(8.))
        .cursor_pointer()
        .font_family(theme::FONT_SANS)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(theme::text::SMALL)
        .text_color(theme::accent())
        .child("Reset to Auto")
}

pub(crate) fn output_mode_control(
    label: &'static str,
    automatic: bool,
    integer_path_available: bool,
    reset_link: AnyElement,
    segments: AnyElement,
    stacked: bool,
) -> Div {
    let label = div()
        .font_family(theme::FONT_SANS)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(theme::text::BODY)
        .text_color(theme::text_primary())
        .child(label);
    let state = if !automatic {
        reset_link
    } else if !integer_path_available {
        div()
            .ml(rpx(8.))
            .px(rpx(5.))
            .py(rpx(2.))
            .rounded(rpx(theme::RADIUS_SM))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::bg_elevated())
            .font_family(theme::FONT_MONO)
            .font_weight(FontWeight::BOLD)
            .text_size(theme::text::CAPTION_XS)
            .text_color(theme::text_muted())
            .child("NO INTEGER PATH")
            .into_any_element()
    } else {
        div()
            .ml(rpx(8.))
            .px(rpx(5.))
            .py(rpx(2.))
            .rounded(rpx(theme::RADIUS_SM))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::bg_elevated())
            .font_family(theme::FONT_MONO)
            .font_weight(FontWeight::BOLD)
            .text_size(theme::text::CAPTION_XS)
            .text_color(theme::text_secondary())
            .child("AUTO")
            .into_any_element()
    };

    if stacked {
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(rpx(8.))
            .w_full()
            .child(div().flex().items_center().child(label).child(state))
            .child(segments)
    } else {
        div()
            .flex()
            .items_center()
            .w_full()
            .child(label)
            .child(state)
            .child(div().flex_1())
            .child(segments)
    }
}

pub(crate) fn output_mode_segments(shared: AnyElement, exclusive: AnyElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rpx(2.))
        .p(rpx(2.))
        .rounded(rpx(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border_strong())
        .child(shared)
        .child(exclusive)
}

pub(crate) fn output_mode_segment(
    id: impl Into<ElementId>,
    label: &'static str,
    selected: bool,
    quality: bool,
    disabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .px(rpx(8.))
        .py(rpx(2.))
        .rounded(rpx(3.))
        .border_1()
        .border_color(if selected && quality {
            theme::quality_border()
        } else if selected {
            theme::border_strong()
        } else {
            theme::bg_inset()
        })
        .bg(if selected && quality {
            theme::quality_soft()
        } else if selected {
            theme::bg_elevated()
        } else {
            theme::bg_inset()
        })
        .opacity(if disabled { 0.5 } else { 1.0 })
        .cursor(if disabled {
            CursorStyle::Arrow
        } else {
            CursorStyle::PointingHand
        })
        .font_family(theme::FONT_SANS)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(theme::text::CAPTION)
        .text_color(if selected && quality {
            theme::quality()
        } else if selected {
            theme::text_primary()
        } else if disabled {
            theme::text_muted()
        } else {
            theme::text_secondary()
        })
        .child(label)
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
        .w(rpx(4.))
        .bg(theme::accent())
        .into_any_element()
}

pub(crate) fn input_caret() -> AnyElement {
    div()
        .w(rpx(1.5))
        .h(rpx(14.))
        .flex_none()
        .bg(theme::accent())
        .into_any_element()
}
