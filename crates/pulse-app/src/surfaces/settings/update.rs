use super::*;

impl Shell {
    pub(super) fn render_update_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let (updater_available, available_version, automatically_checks, last_checked) = {
            let updater = self.updater.read(cx);
            (
                updater.is_available(),
                updater
                    .available()
                    .map(|update| update.version().to_owned()),
                updater.automatically_checks_for_updates(),
                format_last_checked(updater.last_check_at()),
            )
        };
        let (status, status_color) = match available_version.as_deref() {
            Some(version) => (format!("Update available: v{version}"), theme::quality()),
            None if updater_available => (
                "You're on the latest version.".to_owned(),
                theme::text_muted(),
            ),
            None => (
                "Update controls are disabled in development builds.".to_owned(),
                theme::text_muted(),
            ),
        };
        let action = div()
            .id("check-for-updates")
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(14.))
            .py(px(9.))
            .flex_none()
            .rounded(px(theme::RADIUS_MD))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_muted())
            .opacity(if updater_available { 1.0 } else { 0.45 })
            .when(updater_available, |button| {
                button
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.updater.read(cx).check_for_updates();
                    }))
            })
            .when(!updater_available, |button| button.cursor_default())
            .child(
                svg()
                    .path("icons/refresh-cw.svg")
                    .size(px(16.))
                    .flex_none()
                    .text_color(theme::text_secondary()),
            )
            .child(
                div()
                    .font_family(theme::FONT_SANS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(13.))
                    .text_color(theme::text_primary())
                    .child("Check for Updates"),
            );
        let hero = ui::SettingsCard::new()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .w_full()
                    .py(px(15.))
                    .child(settings_app_mark())
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .font_family(theme::FONT_SANS)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_size(px(14.))
                                            .text_color(theme::text_primary())
                                            .child("Pulse"),
                                    )
                                    .child(version_chip()),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(px(12.))
                                    .text_color(status_color)
                                    .child(status),
                            ),
                    )
                    .child(action),
            )
            .child(div().w_full().h(px(1.)).flex_none().bg(theme::border()))
            .child(ui::SettingsRow::new(
                "Last checked",
                last_checked.description,
                div()
                    .flex_none()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(11.))
                    .text_color(theme::text_secondary())
                    .child(last_checked.value),
            ));

        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .w_full()
            .max_w(px(820.))
            .child(settings_group("VERSION", hero))
            .child(settings_group(
                "PREFERENCES",
                ui::SettingsCard::new().child(ui::SettingsRow::new(
                    "Check for updates on launch",
                    "Let Sparkle check GitHub for a newer signed release when its schedule is due.",
                    ui::Toggle::new("update-check-on-launch-toggle", automatically_checks)
                        .disabled(!updater_available)
                        .disabled_opacity(0.45)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_check_updates_on_launch(cx);
                        })),
                )),
            ))
            .into_any_element()
    }

    fn toggle_check_updates_on_launch(&mut self, cx: &mut Context<Self>) {
        if !self.updater.read(cx).is_available() {
            return;
        }
        let enabled = !self.updater.read(cx).automatically_checks_for_updates();
        self.updater.update(cx, |updater, cx| {
            updater.set_automatically_checks_for_updates(enabled, cx)
        });
    }
}
