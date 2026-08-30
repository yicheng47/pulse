mod about;
mod general;
mod update;
mod update_logic;

use crate::theme::rpx;

use std::time::Duration;

use gpui::{
    AnyElement, Context, FontWeight, IntoElement, MouseButton, MouseDownEvent, div,
    linear_color_stop, linear_gradient, prelude::*, svg,
};

use crate::{
    app_store::global_app_store,
    backend::PlaybackAction,
    settings::{AboutLink, SETTINGS_GROUPS, SettingsSection, SettingsViewModel},
    surfaces::Shell,
    theme, ui,
};

use update_logic::format_last_checked;

const UPDATE_CHECK_POLL_INTERVAL: Duration = Duration::from_secs(30);

impl Shell {
    pub(crate) fn open_settings(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        self.settings_section = Some(section);
        self.search_open = false;
        self.search_input.unmark_text();
        self.row.update(cx, |row, cx| row.enter_settings(cx));
        self.settings_output_picker
            .update(cx, |picker, cx| picker.close_output_popover(cx));
        if section == SettingsSection::Output {
            global_app_store(cx).update(cx, |store, store_cx| {
                store.send_command(PlaybackAction::RefreshOutputDevices, store_cx);
            });
        }
        self.sync_update_check_polling(cx);
        cx.notify();
    }

    pub(crate) fn check_for_updates(&self, cx: &gpui::App) {
        self.updater.read(cx).check_for_updates();
    }

    fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_section = None;
        self.row.update(cx, |row, cx| row.leave_settings(cx));
        self.settings_output_picker
            .update(cx, |picker, cx| picker.close_output_popover(cx));
        self.sync_update_check_polling(cx);
        cx.notify();
    }

    fn select_settings_section(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        self.settings_section = Some(section);
        self.settings_output_picker
            .update(cx, |picker, cx| picker.close_output_popover(cx));
        if section == SettingsSection::Output {
            global_app_store(cx).update(cx, |store, store_cx| {
                store.send_command(PlaybackAction::RefreshOutputDevices, store_cx);
            });
        }
        self.sync_update_check_polling(cx);
        cx.notify();
    }

    fn sync_update_check_polling(&mut self, cx: &mut Context<Self>) {
        self.update_check_poll_generation = self.update_check_poll_generation.wrapping_add(1);
        if self.settings_section != Some(SettingsSection::Update) {
            return;
        }

        let generation = self.update_check_poll_generation;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(UPDATE_CHECK_POLL_INTERVAL)
                    .await;
                let keep_polling = this
                    .update(cx, |this, cx| {
                        if this.settings_section != Some(SettingsSection::Update)
                            || this.update_check_poll_generation != generation
                        {
                            return false;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep_polling {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn render_settings_shell(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let section = self
            .settings_section
            .expect("settings shell is only rendered for a settings section");
        let active_output = global_app_store(cx)
            .read(cx)
            .active_output_device()
            .cloned();
        let model = SettingsViewModel::new(section, active_output.as_ref());

        div()
            .flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(self.render_settings_sidebar(&model, cx))
            .child(self.render_settings_content(&model, cx))
    }

    fn render_settings_sidebar(
        &self,
        model: &SettingsViewModel,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut navigation = div().flex().flex_col().gap(rpx(32.)).w_full().child(
            ui::SidebarItem::new(
                "back-to-library",
                "Back to library",
                "icons/chevron-left.svg",
            )
            .on_click(cx.listener(|this, _, window, cx| {
                window.blur();
                this.close_settings(cx);
            })),
        );
        for (header, sections) in SETTINGS_GROUPS {
            let mut group = ui::SidebarSection::new(*header);
            for section in *sections {
                group = group.child(self.render_settings_nav_item(model, *section, cx));
            }
            navigation = navigation.child(group);
        }

        ui::SidebarIsland::new()
            .child(navigation)
            .child(div().flex_1())
    }

    fn render_settings_nav_item(
        &self,
        model: &SettingsViewModel,
        section: SettingsSection,
        cx: &mut Context<Self>,
    ) -> ui::SidebarItem {
        let selected = model.is_selected(section);
        ui::SidebarItem::new(
            format!("settings-section-{}", section.label()),
            section.label(),
            section.icon(),
        )
        .selected(selected)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.select_settings_section(section, cx);
        }))
    }

    fn render_settings_content(
        &self,
        model: &SettingsViewModel,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let content = match model.section {
            SettingsSection::General => self.render_general_settings(model, cx),
            SettingsSection::Output => {
                return div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.devices.clone())
                    .into_any_element();
            }
            SettingsSection::Update => self.render_update_settings(cx),
            SettingsSection::About => self.render_about_settings(cx),
        };

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .gap(rpx(22.))
            .pt(rpx(30.))
            .pr(rpx(36.))
            .pb(rpx(24.))
            .pl(rpx(36.))
            .child(
                div()
                    .w_full()
                    .max_w(rpx(820.))
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::BOLD)
                    .text_size(theme::text::PAGE_TITLE)
                    .text_color(theme::text_primary())
                    .child(model.section.label()),
            )
            .child(content)
            .into_any_element()
    }
}

pub(super) fn settings_group(label: &'static str, card: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(rpx(9.))
        .w_full()
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::BOLD)
                .text_size(theme::text::SMALL)
                .text_color(theme::text_muted())
                .child(label),
        )
        .child(card)
}

pub(super) fn settings_app_mark() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(32.))
        .flex_none()
        .rounded(rpx(7.))
        .overflow_hidden()
        .border_1()
        .border_color(theme::border())
        .bg(linear_gradient(
            180.,
            linear_color_stop(theme::bg_surface(), 0.),
            linear_color_stop(theme::bg_inset(), 1.),
        ))
        .child(
            svg()
                .path("icons/activity.svg")
                .size(rpx(22.))
                .text_color(theme::accent()),
        )
}

pub(super) fn version_chip() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .px(rpx(7.))
        .py(rpx(3.))
        .flex_none()
        .rounded(rpx(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg_muted())
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(theme::text::SMALL)
        .text_color(theme::text_secondary())
        .child(format!("v{}", env!("CARGO_PKG_VERSION")))
}
