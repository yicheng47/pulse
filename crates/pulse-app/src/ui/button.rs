use crate::theme::rpx;

use std::rc::Rc;

use gpui::{
    App, ClickEvent, CursorStyle, ElementId, FontWeight, IntoElement, RenderOnce, SharedString,
    Window, div, prelude::*, svg,
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
        let variant = self.variant;
        let disabled = self.disabled;
        let content_color = if compact {
            theme::text_primary()
        } else if self.variant == ButtonVariant::Secondary {
            theme::text_secondary()
        } else {
            theme::bg_inset()
        };
        let mut button = div()
            .id(self.id)
            .group("button")
            .flex()
            .items_center()
            .justify_center()
            .gap(rpx(8.))
            .h(rpx(if compact { 24. } else { 34. }))
            .when(compact, |button| button.flex_none())
            .px(rpx(if compact {
                10.
            } else if self.variant == ButtonVariant::Secondary {
                13.
            } else {
                14.
            }))
            .rounded(rpx(self.corner_radius))
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
            .text_size(theme::text::BODY_LARGE)
            .when(compact, |button| button.text_size(theme::text::BODY))
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
                    .size(rpx(15.))
                    .flex_none()
                    .text_color(content_color)
                    .when(variant == ButtonVariant::Danger && !disabled, |icon| {
                        icon.group_hover("button", |style| style.text_color(theme::danger()))
                    })
            }))
            .child(self.label);
        if !self.disabled {
            button = button
                .hover(|style| match self.variant {
                    ButtonVariant::Primary => style.bg(theme::accent_bright()),
                    ButtonVariant::Danger => {
                        style.bg(theme::danger_soft()).text_color(theme::danger())
                    }
                    ButtonVariant::Secondary if compact => style.bg(theme::bg_elevated()),
                    ButtonVariant::Secondary => style.bg(theme::bg_muted()),
                })
                .active(|style| match self.variant {
                    ButtonVariant::Primary => style.bg(theme::accent()),
                    ButtonVariant::Danger => {
                        style.bg(theme::bg_elevated()).text_color(theme::danger())
                    }
                    ButtonVariant::Secondary if compact => style.bg(theme::bg_selected()),
                    ButtonVariant::Secondary => style.bg(theme::bg_elevated()),
                });
        }
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum IconButtonVariant {
    #[default]
    Muted,
    Secondary,
    Accent,
    AccentSoft,
    Primary,
}

#[derive(IntoElement)]
pub(crate) struct IconButton {
    id: ElementId,
    icon: SharedString,
    variant: IconButtonVariant,
    button_size: f32,
    icon_size: f32,
    corner_radius: f32,
    horizontal_margin: f32,
    framed: bool,
    selected: bool,
    disabled: bool,
    disabled_opacity: f32,
    tooltip: Option<SharedString>,
    on_click: Option<ClickHandler>,
}

impl IconButton {
    pub(crate) fn new(id: impl Into<ElementId>, icon: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            variant: IconButtonVariant::Muted,
            button_size: 28.,
            icon_size: 16.,
            corner_radius: theme::RADIUS_SM,
            horizontal_margin: 0.,
            framed: false,
            selected: false,
            disabled: false,
            disabled_opacity: 0.5,
            tooltip: None,
            on_click: None,
        }
    }

    pub(crate) fn variant(mut self, variant: IconButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub(crate) fn button_size(mut self, button_size: f32) -> Self {
        self.button_size = button_size;
        self
    }

    pub(crate) fn icon_size(mut self, icon_size: f32) -> Self {
        self.icon_size = icon_size;
        self
    }

    pub(crate) fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    pub(crate) fn horizontal_margin(mut self, horizontal_margin: f32) -> Self {
        self.horizontal_margin = horizontal_margin;
        self
    }

    pub(crate) fn framed(mut self, framed: bool) -> Self {
        self.framed = framed;
        self
    }

    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub(crate) fn disabled_opacity(mut self, disabled_opacity: f32) -> Self {
        self.disabled_opacity = disabled_opacity;
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
        let variant = self.variant;
        let framed = self.framed;
        let selected = self.selected;
        let disabled = self.disabled;
        let foreground = if self.selected {
            theme::accent()
        } else {
            match self.variant {
                IconButtonVariant::Muted => theme::text_muted(),
                IconButtonVariant::Secondary => theme::text_secondary(),
                IconButtonVariant::Accent | IconButtonVariant::AccentSoft => theme::accent(),
                IconButtonVariant::Primary => theme::bg_inset(),
            }
        };
        let mut button = div()
            .id(self.id)
            .group("icon-button")
            .flex()
            .items_center()
            .justify_center()
            .size(rpx(self.button_size))
            .mx(rpx(self.horizontal_margin))
            .rounded(rpx(self.corner_radius))
            .when(self.variant == IconButtonVariant::Primary, |button| {
                button.bg(theme::accent())
            })
            .when(self.variant == IconButtonVariant::AccentSoft, |button| {
                button.bg(theme::accent_soft())
            })
            .when(self.framed, |button| {
                button
                    .bg(theme::bg_muted())
                    .border_1()
                    .border_color(theme::border())
            })
            .when(self.selected, |button| button.bg(theme::bg_elevated()))
            .opacity(if self.disabled {
                self.disabled_opacity
            } else {
                1.
            })
            .cursor(if self.disabled {
                CursorStyle::Arrow
            } else {
                CursorStyle::PointingHand
            })
            .child(
                svg()
                    .path(self.icon)
                    .size(rpx(self.icon_size))
                    .text_color(foreground)
                    .when(
                        variant == IconButtonVariant::AccentSoft && !selected && !disabled,
                        |icon| {
                            icon.group_hover("icon-button", |style| {
                                style.text_color(theme::bg_inset())
                            })
                        },
                    ),
            );
        if !self.disabled {
            button = button
                .hover(|style| match (self.selected, self.variant) {
                    (true, _) => style.bg(theme::bg_elevated()),
                    (false, IconButtonVariant::Primary) => style.bg(theme::accent_bright()),
                    (false, IconButtonVariant::AccentSoft) => style.bg(theme::accent()),
                    (false, IconButtonVariant::Secondary) if framed => {
                        style.bg(theme::bg_elevated())
                    }
                    (false, IconButtonVariant::Muted)
                    | (false, IconButtonVariant::Secondary)
                    | (false, IconButtonVariant::Accent) => style.bg(theme::bg_muted()),
                })
                .active(|style| match self.variant {
                    IconButtonVariant::Primary => style.bg(theme::accent()),
                    IconButtonVariant::Muted
                    | IconButtonVariant::Secondary
                    | IconButtonVariant::Accent => style.bg(theme::bg_elevated()),
                    IconButtonVariant::AccentSoft => style.bg(theme::accent_bright()),
                });
        }
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

    #[test]
    fn icon_button_geometry_is_configurable_for_transport_controls() {
        let default = IconButton::new("default", "icons/x.svg");
        assert_eq!(default.variant, IconButtonVariant::Muted);
        assert_eq!(default.button_size, 28.);
        assert_eq!(default.icon_size, 16.);
        assert!(!default.framed);
        assert_eq!(default.corner_radius, theme::RADIUS_SM);
        assert_eq!(default.horizontal_margin, 0.);
        assert!(!default.selected);
        assert_eq!(default.disabled_opacity, 0.5);

        let transport = IconButton::new("transport", "icons/skip-forward.svg")
            .variant(IconButtonVariant::Secondary)
            .button_size(28.)
            .icon_size(19.)
            .horizontal_margin(-4.5)
            .disabled_opacity(0.35);
        assert_eq!(transport.variant, IconButtonVariant::Secondary);
        assert_eq!(transport.button_size, 28.);
        assert_eq!(transport.icon_size, 19.);
        assert_eq!(transport.horizontal_margin, -4.5);
        assert_eq!(transport.disabled_opacity, 0.35);
    }
}
