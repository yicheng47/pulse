use gpui::{Context, FontWeight, IntoElement, SharedString, div, prelude::*, px, svg};

use crate::{
    app_store::global_app_store,
    backend::PlaybackAction,
    settings::SettingsSection,
    surfaces::{
        Destination, NAV_GROUPS, SIDEBAR_TOP_PADDING, SIDEBAR_WIDTH, Shell, TOP_BAR_HEIGHT,
    },
    theme, ui,
};

impl Shell {
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .gap(px(22.))
            .pt(px(SIDEBAR_TOP_PADDING))
            .pr(px(14.))
            .pb(px(16.))
            .pl(px(14.))
            .bg(theme::bg_surface())
            .border_r_1()
            .border_color(theme::border())
            .child(render_brand())
            .child(self.render_navigation(cx))
            .child(div().flex_1())
            .child(self.render_settings_footer(cx))
            .child(
                self.render_titlebar_drag_area(
                    "sidebar-titlebar-drag",
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h(px(TOP_BAR_HEIGHT)),
                    cx,
                ),
            )
    }

    fn render_navigation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut navigation = div().flex().flex_col().gap(px(20.)).w_full();

        for (header, destinations) in NAV_GROUPS {
            let mut items = div().flex().flex_col().gap(px(4.)).w_full();
            for destination in *destinations {
                items = items.child(self.render_nav_item(*destination, cx));
            }

            navigation = navigation.child(
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
                                .child(*header),
                        ),
                    )
                    .child(items),
            );
        }

        navigation
    }

    fn render_nav_item(
        &self,
        destination: Destination,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.destination == destination;
        let hover_group = SharedString::from(format!("sidebar-nav-{}", destination.label()));

        div()
            .id(destination.label())
            .group(hover_group.clone())
            .flex()
            .items_center()
            .gap(px(10.))
            .w_full()
            .px(px(10.))
            .py(px(9.))
            .rounded(px(theme::RADIUS_MD))
            .when(active, |item| item.bg(theme::accent_soft()))
            .when(!active, |item| {
                item.hover(|style| style.bg(theme::accent_soft()))
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx: &mut Context<Shell>| {
                this.destination = destination;
                if destination == Destination::Devices {
                    global_app_store(cx).update(cx, |store, store_cx| {
                        store.send_command(PlaybackAction::RefreshOutputDevices, store_cx);
                    });
                }
                this.library
                    .update(cx, |library, cx| library.set_destination(destination, cx));
                cx.notify();
            }))
            .child(
                svg()
                    .path(destination.icon())
                    .size(px(17.))
                    .flex_none()
                    .text_color(if active {
                        theme::accent()
                    } else {
                        theme::text_muted()
                    })
                    .when(!active, |icon| {
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
                    .text_color(if active {
                        theme::text_primary()
                    } else {
                        theme::text_secondary()
                    })
                    .when(!active, |label| {
                        label.group_hover(hover_group.clone(), |style| {
                            style.text_color(theme::text_primary())
                        })
                    })
                    .child(destination.label()),
            )
            .when(destination == Destination::Storage, |item| {
                item.child(div().flex_1()).child(render_storage_badge(
                    self.library.read(cx).storage_root_count(),
                ))
            })
    }

    fn render_settings_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let updater = self.updater.clone();
        let update_version = updater
            .read(cx)
            .available()
            .map(|update| update.version().to_owned());
        let update_hint = update_version.map(|version| {
            let click_updater = updater.clone();
            let tooltip = SharedString::from(format!("Pulse {version} is ready to install"));
            let trigger = div()
                .id("sidebar-update")
                .flex()
                .items_center()
                .justify_center()
                .size(px(36.))
                .flex_none()
                .rounded(px(theme::RADIUS_MD))
                .cursor_pointer()
                .hover(|style| style.bg(theme::accent_soft()))
                .on_click(move |_, _, cx| {
                    click_updater.read(cx).check_for_updates();
                })
                .child(
                    svg()
                        .path("icons/circle-arrow-down.svg")
                        .size(px(16.))
                        .flex_none()
                        .text_color(theme::accent()),
                );
            ui::Tooltip::new("sidebar-update-tooltip", tooltip, trigger)
        });

        div()
            .w_full()
            .pt(px(12.))
            .border_t_1()
            .border_color(theme::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .w_full()
                    .child(
                        div()
                            .id("open-settings")
                            .group("sidebar-settings")
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .min_w_0()
                            .flex_1()
                            .px(px(10.))
                            .py(px(9.))
                            .rounded(px(theme::RADIUS_MD))
                            .cursor_pointer()
                            .hover(|style| style.bg(theme::accent_soft()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                window.blur();
                                this.open_settings(SettingsSection::General, cx);
                            }))
                            .child(
                                svg()
                                    .path("icons/settings.svg")
                                    .size(px(18.))
                                    .flex_none()
                                    .text_color(theme::text_muted())
                                    .group_hover("sidebar-settings", |style| {
                                        style.text_color(theme::accent())
                                    }),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(14.))
                                    .text_color(theme::text_secondary())
                                    .group_hover("sidebar-settings", |style| {
                                        style.text_color(theme::text_primary())
                                    })
                                    .child("Settings"),
                            ),
                    )
                    .children(update_hint),
            )
    }
}

fn render_brand() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .w_full()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(32.))
                .flex_none()
                .child(
                    svg()
                        .path("icons/activity.svg")
                        .size(px(24.))
                        .text_color(theme::accent()),
                ),
        )
        .child(
            div()
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::BOLD)
                .text_size(px(22.))
                .text_color(theme::text_primary())
                .child("Pulse"),
        )
}

fn render_storage_badge(count: usize) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .px(px(6.))
        .py(px(2.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg_muted())
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::BOLD)
                .text_size(px(10.))
                .text_color(theme::text_muted())
                .child(count.to_string()),
        )
}

#[cfg(test)]
mod tests {
    use gpui::AssetSource;

    use super::*;

    #[test]
    fn each_destination_maps_to_a_bundled_icon() {
        for destination in [
            Destination::Albums,
            Destination::Artists,
            Destination::Tracks,
            Destination::Playlists,
            Destination::Storage,
            Destination::Devices,
        ] {
            assert!(
                crate::assets::Assets
                    .load(destination.icon())
                    .unwrap()
                    .is_some(),
                "{}",
                destination.icon()
            );
        }
    }
}
