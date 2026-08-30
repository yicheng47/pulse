mod about;
mod general;
mod update;
mod update_logic;

use std::time::Duration;

use gpui::{
    AnyElement, Context, FontWeight, IntoElement, MouseButton, MouseDownEvent, SharedString, div,
    linear_color_stop, linear_gradient, prelude::*, px, svg,
};

use crate::{
    app_store::global_app_store,
    settings::{AboutLink, SettingsSection, SettingsViewModel},
    surfaces::{SIDEBAR_WIDTH, Shell},
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
        let mut navigation = div().flex().flex_col().gap(px(4.)).w_full();
        for section in SettingsSection::ALL {
            navigation = navigation.child(self.render_settings_nav_item(model, section, cx));
        }

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .gap(px(22.))
            .pt(px(20.))
            .pr(px(14.))
            .pb(px(16.))
            .pl(px(14.))
            .bg(theme::bg_surface())
            .border_r_1()
            .border_color(theme::border())
            .child(
                div()
                    .id("back-to-library")
                    .group("settings-back")
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .w_full()
                    .px(px(10.))
                    .py(px(9.))
                    .rounded(px(theme::RADIUS_MD))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::accent_soft()))
                    .on_click(cx.listener(|this, _, window, cx| {
                        window.blur();
                        this.close_settings(cx);
                    }))
                    .child(
                        svg()
                            .path("icons/chevron-left.svg")
                            .size(px(17.))
                            .flex_none()
                            .text_color(theme::text_muted())
                            .group_hover("settings-back", |style| {
                                style.text_color(theme::accent())
                            }),
                    )
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(15.))
                            .text_color(theme::text_secondary())
                            .group_hover("settings-back", |style| {
                                style.text_color(theme::text_primary())
                            })
                            .child("Back to library"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .w_full()
                    .child(
                        div().flex().px(px(10.)).w_full().child(
                            div()
                                .font_family(theme::FONT_MONO)
                                .font_weight(FontWeight::BOLD)
                                .text_size(px(10.))
                                .text_color(theme::text_muted())
                                .child("SETTINGS"),
                        ),
                    )
                    .child(navigation),
            )
            .child(div().flex_1())
    }

    fn render_settings_nav_item(
        &self,
        model: &SettingsViewModel,
        section: SettingsSection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = model.is_selected(section);
        let hover_group = SharedString::from(format!("settings-nav-{}", section.label()));

        div()
            .id(SharedString::from(format!(
                "settings-section-{}",
                section.label()
            )))
            .group(hover_group.clone())
            .flex()
            .items_center()
            .gap(px(10.))
            .w_full()
            .px(px(10.))
            .py(px(9.))
            .rounded(px(theme::RADIUS_MD))
            .when(selected, |item| item.bg(theme::accent_soft()))
            .when(!selected, |item| {
                item.hover(|style| style.bg(theme::accent_soft()))
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_settings_section(section, cx);
            }))
            .child(
                svg()
                    .path(section.icon())
                    .size(px(17.))
                    .flex_none()
                    .text_color(if selected {
                        theme::accent()
                    } else {
                        theme::text_muted()
                    })
                    .when(!selected, |icon| {
                        icon.group_hover(hover_group.clone(), |style| {
                            style.text_color(theme::accent())
                        })
                    }),
            )
            .child(
                div()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(15.))
                    .text_color(if selected {
                        theme::text_primary()
                    } else {
                        theme::text_secondary()
                    })
                    .when(!selected, |label| {
                        label.group_hover(hover_group.clone(), |style| {
                            style.text_color(theme::text_primary())
                        })
                    })
                    .child(section.label()),
            )
    }

    fn render_settings_content(
        &self,
        model: &SettingsViewModel,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let content = match model.section {
            SettingsSection::General => self.render_general_settings(model, cx),
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
            .gap(px(22.))
            .pt(px(30.))
            .pr(px(36.))
            .pb(px(24.))
            .pl(px(36.))
            .child(
                div()
                    .w_full()
                    .max_w(px(820.))
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(28.))
                    .text_color(theme::text_primary())
                    .child(model.section.label()),
            )
            .child(content)
    }
}

pub(super) fn settings_group(label: &'static str, card: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(9.))
        .w_full()
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::BOLD)
                .text_size(px(11.))
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
        .size(px(32.))
        .flex_none()
        .rounded(px(7.))
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
                .size(px(22.))
                .text_color(theme::accent()),
        )
}

pub(super) fn version_chip() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .px(px(7.))
        .py(px(3.))
        .flex_none()
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg_muted())
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(11.))
        .text_color(theme::text_secondary())
        .child(format!("v{}", env!("CARGO_PKG_VERSION")))
}
