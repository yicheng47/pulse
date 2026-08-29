use super::*;

impl Shell {
    pub(super) fn render_about_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let application = ui::SettingsCard::new().child(
            div()
                .flex()
                .items_center()
                .gap(px(14.))
                .w_full()
                .py(px(16.))
                .child(settings_app_mark())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .font_family(theme::FONT_DISPLAY)
                                        .font_weight(FontWeight::BOLD)
                                        .text_size(px(18.))
                                        .text_color(theme::text_primary())
                                        .child("Pulse"),
                                )
                                .child(version_chip()),
                        )
                        .child(
                            div()
                                .font_family(theme::FONT_SANS)
                                .text_size(px(12.))
                                .text_color(theme::text_muted())
                                .child("Local music player for macOS."),
                        ),
                ),
        );

        let mut links = ui::SettingsCard::new();
        for (index, link) in AboutLink::ALL.into_iter().enumerate() {
            links = links.child(
                ui::SettingsRow::new(
                    link.label(),
                    link.description(),
                    svg()
                        .path("icons/external-link.svg")
                        .size(px(15.))
                        .flex_none()
                        .text_color(theme::text_muted()),
                )
                .divider(index + 1 < AboutLink::ALL.len())
                .id(("about-link", index))
                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(link.url()))),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .w_full()
            .max_w(px(820.))
            .child(settings_group("APPLICATION", application))
            .child(settings_group("LINKS", links))
            .into_any_element()
    }
}
