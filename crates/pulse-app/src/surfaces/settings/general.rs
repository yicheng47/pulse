use crate::{
    settings::{
        DEFAULT_INTERFACE_SCALE, interface_scale_label, next_interface_scale,
        previous_interface_scale,
    },
    theme::rpx,
    ui::IconButtonVariant,
};
use gpui::Window;

use super::*;

impl Shell {
    pub(super) fn render_general_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let interface_scale = global_app_store(cx).read(cx).interface_scale();
        let previous_scale = previous_interface_scale(interface_scale);
        let next_scale = next_interface_scale(interface_scale);
        let stepper = div()
            .flex()
            .items_center()
            .gap(rpx(8.))
            .flex_none()
            .child(
                ui::IconButton::new("decrease-interface-scale", "icons/minus.svg")
                    .variant(IconButtonVariant::Secondary)
                    .button_size(34.)
                    .icon_size(17.)
                    .corner_radius(theme::RADIUS_MD)
                    .framed(true)
                    .disabled(previous_scale == interface_scale)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.decrease_interface_scale(window, cx);
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(rpx(56.))
                    .h(rpx(34.))
                    .flex_none()
                    .rounded(rpx(theme::RADIUS_MD))
                    .bg(theme::bg_inset())
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(theme::text::BODY_LARGE)
                    .text_color(theme::text_primary())
                    .child(interface_scale_label(interface_scale)),
            )
            .child(
                ui::IconButton::new("increase-interface-scale", "icons/plus.svg")
                    .variant(IconButtonVariant::Secondary)
                    .button_size(34.)
                    .icon_size(17.)
                    .corner_radius(theme::RADIUS_MD)
                    .framed(true)
                    .disabled(next_scale == interface_scale)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.increase_interface_scale(window, cx);
                    })),
            );

        settings_group(
            "INTERFACE",
            ui::SettingsCard::new().child(ui::SettingsRow::new(
                "Interface scale",
                "Scales every part of the window. 100% is the default.",
                stepper,
            )),
        )
        .max_w(rpx(820.))
        .into_any_element()
    }

    pub(crate) fn increase_interface_scale(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let interface_scale = global_app_store(cx).read(cx).interface_scale();
        self.set_interface_scale(next_interface_scale(interface_scale), window, cx);
    }

    pub(crate) fn decrease_interface_scale(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let interface_scale = global_app_store(cx).read(cx).interface_scale();
        self.set_interface_scale(previous_interface_scale(interface_scale), window, cx);
    }

    pub(crate) fn reset_interface_scale(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_interface_scale(DEFAULT_INTERFACE_SCALE, window, cx);
    }

    fn set_interface_scale(
        &mut self,
        interface_scale: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let interface_scale = global_app_store(cx).update(cx, |store, store_cx| {
            store.set_interface_scale(interface_scale, store_cx)
        });
        if let Some(interface_scale) = interface_scale {
            window.set_rem_size(gpui::px(theme::REM_BASE_PX * interface_scale)); // physical
            cx.notify();
        }
    }
}
