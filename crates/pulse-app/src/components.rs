//! Shared UI components. One definition per control so a styling change
//! lands in one place. Callers attach behavior (`on_click`) at the call
//! site; compound controls (icon buttons, enabled-state buttons) stay local
//! to their views.

use gpui::{
    AnyElement, Div, ElementId, FontWeight, Rgba, SharedString, Stateful, div, linear_color_stop,
    linear_gradient, prelude::*, px,
};

use crate::theme;

const TOGGLE_WIDTH: f32 = 40.0;
const TOGGLE_HEIGHT: f32 = 22.0;
const TOGGLE_KNOB_SIZE: f32 = 18.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum ToggleAlignment {
    Start,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ToggleAppearance {
    alignment: ToggleAlignment,
    track: Rgba,
    border: Option<Rgba>,
    knob: Rgba,
}

fn toggle_appearance(on: bool) -> ToggleAppearance {
    if on {
        ToggleAppearance {
            alignment: ToggleAlignment::End,
            track: theme::accent(),
            border: None,
            knob: theme::bg_inset(),
        }
    } else {
        ToggleAppearance {
            alignment: ToggleAlignment::Start,
            track: theme::bg_elevated(),
            border: Some(theme::border_strong()),
            knob: theme::text_muted(),
        }
    }
}

pub(crate) fn toggle(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    let appearance = toggle_appearance(on);
    let mut toggle = div()
        .id(id)
        .flex()
        .items_center()
        .w(px(TOGGLE_WIDTH))
        .h(px(TOGGLE_HEIGHT))
        .flex_none()
        .p(px(2.))
        .rounded(px(TOGGLE_HEIGHT / 2.0))
        .bg(appearance.track)
        .cursor_pointer();
    toggle = match appearance.alignment {
        ToggleAlignment::Start => toggle.justify_start(),
        ToggleAlignment::End => toggle.justify_end(),
    };
    if let Some(border) = appearance.border {
        toggle = toggle.border_1().border_color(border);
    }
    toggle.child(
        div()
            .size(px(TOGGLE_KNOB_SIZE))
            .flex_none()
            .rounded_full()
            .bg(appearance.knob),
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

/// Filled accent button for a modal's confirming action.
pub(crate) fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(34.))
        .px(px(14.))
        .rounded(px(theme::RADIUS_SM))
        .bg(theme::accent())
        .cursor_pointer()
        .font_family(theme::FONT_DISPLAY)
        .font_weight(FontWeight::BOLD)
        .text_size(px(13.))
        .text_color(theme::bg_inset())
        .child(label.into())
}

/// Filled danger button for a destructive confirming action.
pub(crate) fn danger_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(34.))
        .px(px(14.))
        .rounded(px(theme::RADIUS_SM))
        .bg(theme::danger())
        .cursor_pointer()
        .font_family(theme::FONT_DISPLAY)
        .font_weight(FontWeight::BOLD)
        .text_size(px(13.))
        .text_color(theme::bg_inset())
        .child(label.into())
}

/// Bordered neutral button for a dismissing or recovery action.
pub(crate) fn secondary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(34.))
        .px(px(13.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border())
        .cursor_pointer()
        .font_family(theme::FONT_DISPLAY)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(13.))
        .text_color(theme::text_secondary())
        .child(label.into())
}

/// Compact bordered button for dense strips like the playback notice banner.
pub(crate) fn compact_secondary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .h(px(24.))
        .flex_none()
        .px(px(10.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border_strong())
        .bg(theme::bg_muted())
        .cursor_pointer()
        .font_family(theme::FONT_SANS)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(12.))
        .text_color(theme::text_primary())
        .child(label.into())
}

/// Bordered mono badge for a track/album quality label (e.g. "24/96").
pub(crate) fn quality_badge(label: impl Into<SharedString>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .h(px(18.))
        .px(px(6.))
        .flex_none()
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::quality_border())
        .bg(theme::quality_soft())
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(px(9.))
        .text_color(theme::quality())
        .child(label.into())
        .into_any_element()
}

/// Neon glow that bleeds rightward from a playing row's accent bar, fading
/// out by the row's midpoint. Painted over the row background, under content.
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

/// The 4px accent bar on a playing row's left edge. Absolute insets resolve
/// against the parent's content box, which already excludes a border-side
/// separator — so a full span lands exactly against the hairline.
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

/// Text caret for the hand-rolled single-line inputs. These inputs edit only
/// at the end of the string, so the caret always sits after the text.
pub(crate) fn input_caret() -> AnyElement {
    div()
        .w(px(1.5))
        .h(px(14.))
        .flex_none()
        .bg(theme::accent())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_rendering_matches_the_on_and_off_components() {
        assert_eq!(TOGGLE_WIDTH, 40.0);
        assert_eq!(TOGGLE_HEIGHT, 22.0);
        assert_eq!(TOGGLE_KNOB_SIZE, 18.0);
        assert_eq!(
            toggle_appearance(true),
            ToggleAppearance {
                alignment: ToggleAlignment::End,
                track: theme::accent(),
                border: None,
                knob: theme::bg_inset(),
            }
        );
        assert_eq!(
            toggle_appearance(false),
            ToggleAppearance {
                alignment: ToggleAlignment::Start,
                track: theme::bg_elevated(),
                border: Some(theme::border_strong()),
                knob: theme::text_muted(),
            }
        );
    }
}
