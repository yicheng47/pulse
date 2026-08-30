use crate::theme::rpx;

use std::rc::Rc;

use gpui::{
    App, ClickEvent, CursorStyle, Div, ElementId, IntoElement, RenderOnce, Rgba, Stateful, Window,
    div, prelude::*,
};

use crate::theme;

const TOGGLE_WIDTH: f32 = 40.0;
const TOGGLE_HEIGHT: f32 = 22.0;
const TOGGLE_KNOB_SIZE: f32 = 18.0;

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

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

#[derive(IntoElement)]
pub(crate) struct Toggle {
    id: ElementId,
    on: bool,
    disabled: bool,
    disabled_opacity: f32,
    on_click: Option<ClickHandler>,
}

impl Toggle {
    pub(crate) fn new(id: impl Into<ElementId>, on: bool) -> Self {
        Self {
            id: id.into(),
            on,
            disabled: false,
            disabled_opacity: 0.5,
            on_click: None,
        }
    }

    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub(crate) fn disabled_opacity(mut self, opacity: f32) -> Self {
        self.disabled_opacity = opacity;
        self
    }

    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Toggle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let appearance = toggle_appearance(self.on);
        let mut toggle: Stateful<Div> = div()
            .id(self.id)
            .flex()
            .items_center()
            .w(rpx(TOGGLE_WIDTH))
            .h(rpx(TOGGLE_HEIGHT))
            .flex_none()
            .p(rpx(2.))
            .rounded(rpx(TOGGLE_HEIGHT / 2.0))
            .bg(appearance.track)
            .opacity(if self.disabled {
                self.disabled_opacity
            } else {
                1.0
            })
            .cursor(if self.disabled {
                CursorStyle::Arrow
            } else {
                CursorStyle::PointingHand
            });
        toggle = match appearance.alignment {
            ToggleAlignment::Start => toggle.justify_start(),
            ToggleAlignment::End => toggle.justify_end(),
        };
        if let Some(border) = appearance.border {
            toggle = toggle.border_1().border_color(border);
        }
        toggle = toggle.child(
            div()
                .size(rpx(TOGGLE_KNOB_SIZE))
                .flex_none()
                .rounded_full()
                .bg(appearance.knob),
        );
        if !self.disabled
            && let Some(on_click) = self.on_click
        {
            toggle = toggle.on_click(move |event, window, cx| on_click(event, window, cx));
        }
        toggle
    }
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
