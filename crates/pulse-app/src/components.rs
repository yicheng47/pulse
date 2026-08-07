//! Shared UI components. One definition per control so a styling change
//! lands in one place. Callers attach behavior (`on_click`) at the call
//! site; compound controls (icon buttons, enabled-state buttons) stay local
//! to their views.

use gpui::{AnyElement, Div, ElementId, FontWeight, SharedString, Stateful, div, prelude::*, px};

use crate::theme;

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
