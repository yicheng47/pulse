use crate::theme::rpx;

use gpui::{Context, FontWeight, IntoElement, Window, div, prelude::*, svg};

use crate::{
    settings::SettingsSection,
    surfaces::{Shell, TOP_BAR_HEIGHT},
    theme,
    ui::{self, IconButtonVariant},
};

use super::header_logic::HeaderState;

impl Shell {
    pub(super) fn render_header(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let update_version = self
            .updater
            .read(cx)
            .available()
            .map(|update| update.version().to_owned());
        let state = HeaderState::new(self.settings_section.is_some(), update_version);
        let update_hint = state.update_hint_version().map(|version| {
            let updater = self.updater.clone();
            ui::IconButton::new("header-update", "icons/arrow-down.svg")
                .variant(IconButtonVariant::AccentSoft)
                .button_size(22.)
                .icon_size(13.)
                .corner_radius(11.)
                .tooltip(format!("Pulse {version} is ready to install"))
                .on_click(move |_, _, cx| updater.read(cx).check_for_updates())
        });
        let settings = ui::IconButton::new("open-settings", "icons/settings.svg")
            .variant(IconButtonVariant::Secondary)
            .selected(state.settings_active)
            .button_size(32.)
            .icon_size(18.)
            .corner_radius(theme::RADIUS_MD)
            .tooltip("Settings")
            .on_click(cx.listener(|this, _, window, cx| {
                window.blur();
                this.open_settings(SettingsSection::General, cx);
            }));

        let header = div()
            .flex()
            .items_center()
            .gap(rpx(16.))
            .w_full()
            .h(rpx(TOP_BAR_HEIGHT))
            .flex_none()
            .pt_0()
            .pr(rpx(28.))
            .pb_0()
            .pl(rpx(24.))
            .border_b_1()
            .border_color(theme::border())
            .bg(theme::bg_page())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(10.))
                    .h_full()
                    .flex_none()
                    .child(div().w(rpx(52.)).h(rpx(12.)).flex_none())
                    .child(div().w(rpx(2.)).h(rpx(1.)).flex_none())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(rpx(32.))
                            .flex_none()
                            .child(
                                svg()
                                    .path("icons/activity.svg")
                                    .size(rpx(24.))
                                    .text_color(theme::accent()),
                            ),
                    )
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::BOLD)
                            .text_size(theme::text::HEADING)
                            .text_color(theme::text_primary())
                            .child("Pulse"),
                    ),
            )
            .child(
                div()
                    .w(gpui::px(1.)) // physical
                    .h(rpx(28.))
                    .flex_none()
                    .bg(theme::border()),
            )
            .child(self.render_search_input(window, cx))
            .child(div().flex_1().min_w_0().h(rpx(1.)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(rpx(12.))
                    .h_full()
                    .flex_none()
                    .occlude()
                    .children(update_hint)
                    .child(settings),
            );

        self.render_titlebar_drag_area("app-header-drag", header, cx)
    }
}
