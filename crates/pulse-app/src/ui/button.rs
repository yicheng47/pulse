use std::rc::Rc;

use gpui::{
    App, ClickEvent, CursorStyle, ElementId, FontWeight, IntoElement, RenderOnce, SharedString,
    Window, div, prelude::*, px, svg,
};

use crate::{theme, ui::Tooltip};

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ButtonVariant {
    Primary,
    Danger,
    #[default]
    Secondary,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ButtonSize {
    #[default]
    Regular,
    Compact,
}

#[derive(IntoElement)]
pub(crate) struct Button {
    id: ElementId,
    label: SharedString,
    icon: Option<SharedString>,
    variant: ButtonVariant,
    size: ButtonSize,
    corner_radius: f32,
    disabled: bool,
    tooltip: Option<SharedString>,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub(crate) fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            variant: ButtonVariant::Secondary,
            size: ButtonSize::Regular,
            corner_radius: theme::RADIUS_SM,
            disabled: false,
            tooltip: None,
            on_click: None,
        }
    }

    pub(crate) fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub(crate) fn icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub(crate) fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub(crate) fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    #[allow(dead_code)]
    pub(crate) fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
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

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tooltip_id = (self.id.clone(), "tooltip");
        let compact = self.size == ButtonSize::Compact;
        let content_color = if compact {
            theme::text_primary()
        } else if self.variant == ButtonVariant::Secondary {
            theme::text_secondary()
        } else {
            theme::bg_inset()
        };
        let mut button = div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .h(px(if compact { 24. } else { 34. }))
            .when(compact, |button| button.flex_none())
            .px(px(if compact {
                10.
            } else if self.variant == ButtonVariant::Secondary {
                13.
            } else {
                14.
            }))
            .rounded(px(self.corner_radius))
            .when(
                compact || self.variant == ButtonVariant::Secondary,
                |button| {
                    button.border_1().border_color(if compact {
                        theme::border_strong()
                    } else {
                        theme::border()
                    })
                },
            )
            .when(compact, |button| button.bg(theme::bg_muted()))
            .when(self.variant == ButtonVariant::Primary, |button| {
                button.bg(theme::accent())
            })
            .when(self.variant == ButtonVariant::Danger, |button| {
                button.bg(theme::danger())
            })
            .cursor_pointer()
            .font_family(if compact {
                theme::FONT_SANS
            } else {
                theme::FONT_DISPLAY
            })
            .font_weight(if self.variant == ButtonVariant::Secondary {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::BOLD
            })
            .text_size(px(if compact { 12. } else { 13. }))
            .text_color(content_color)
            .opacity(if self.disabled {
                if self.variant == ButtonVariant::Danger {
                    0.6
                } else {
                    0.5
                }
            } else {
                1.0
            })
            .children(self.icon.map(|icon| {
                svg()
                    .path(icon)
                    .size(px(15.))
                    .flex_none()
                    .text_color(content_color)
            }))
            .child(self.label);
        if !self.disabled
            && let Some(on_click) = self.on_click
        {
            button = button.on_click(move |event, window, cx| on_click(event, window, cx));
        }
        if let Some(tooltip) = self.tooltip {
            Tooltip::new(tooltip_id, tooltip, button).into_any_element()
        } else {
            button.into_any_element()
        }
    }
}

#[derive(IntoElement)]
pub(crate) struct IconButton {
    id: ElementId,
    icon: SharedString,
    disabled: bool,
    tooltip: Option<SharedString>,
    on_click: Option<ClickHandler>,
}

impl IconButton {
    pub(crate) fn new(id: impl Into<ElementId>, icon: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            disabled: false,
            tooltip: None,
            on_click: None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    #[allow(dead_code)]
    pub(crate) fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
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

impl RenderOnce for IconButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tooltip_id = (self.id.clone(), "tooltip");
        let mut button = div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .size(px(28.))
            .rounded(px(theme::RADIUS_SM))
            .opacity(if self.disabled { 0.5 } else { 1. })
            .cursor(if self.disabled {
                CursorStyle::Arrow
            } else {
                CursorStyle::PointingHand
            })
            .child(
                svg()
                    .path(self.icon)
                    .size(px(16.))
                    .text_color(theme::text_muted()),
            );
        if !self.disabled
            && let Some(on_click) = self.on_click
        {
            button = button.on_click(move |event, window, cx| on_click(event, window, cx));
        }
        if let Some(tooltip) = self.tooltip {
            Tooltip::new(tooltip_id, tooltip, button).into_any_element()
        } else {
            button.into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_defaults_match_the_existing_secondary_control() {
        assert_eq!(ButtonVariant::default(), ButtonVariant::Secondary);
        assert_eq!(ButtonSize::default(), ButtonSize::Regular);
    }
}
