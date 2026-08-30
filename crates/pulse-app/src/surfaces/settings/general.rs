use crate::theme::rpx;

use super::*;

impl Shell {
    pub(super) fn render_general_settings(
        &self,
        model: &SettingsViewModel,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let output_picker = div()
            .id("settings-output-picker")
            .relative()
            .flex()
            .items_center()
            .gap(rpx(8.))
            .max_w(rpx(360.))
            .cursor_pointer()
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                if event.button == MouseButton::Left {
                    this.settings_output_toggle_press_closed =
                        this.settings_output_picker.read(cx).output_popover_open();
                }
            }))
            .on_click(cx.listener(|this, _, _, cx| {
                if std::mem::take(&mut this.settings_output_toggle_press_closed) {
                    this.settings_output_picker
                        .update(cx, |picker, cx| picker.close_output_popover(cx));
                    return;
                }
                this.row.update(cx, |row, cx| row.close_output_popover(cx));
                this.settings_output_picker
                    .update(cx, |picker, cx| picker.toggle_settings_output_popover(cx));
            }))
            .child(
                div()
                    .truncate()
                    .font_family(theme::FONT_SANS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(theme::text::BODY_LARGE)
                    .text_color(theme::text_secondary())
                    .child(model.output_device_name.clone()),
            )
            .child(
                svg()
                    .path("icons/chevron-right.svg")
                    .size(rpx(16.))
                    .flex_none()
                    .text_color(theme::text_muted()),
            )
            .child(self.settings_output_picker.clone());

        settings_group(
            "PLAYBACK",
            ui::SettingsCard::new().child(ui::SettingsRow::new(
                "Default output device",
                "Where Pulse sends audio.",
                output_picker,
            )),
        )
        .max_w(rpx(820.))
        .into_any_element()
    }
}
