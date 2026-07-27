use gpui::{
    Context, Entity, ExternalPaths, FontWeight, IntoElement, MouseButton, MouseMoveEvent,
    MouseUpEvent, Render, Window, div, linear_color_stop, linear_gradient, prelude::*, px, svg,
};

use crate::{playback_row::PlaybackRow, theme};

const SIDEBAR_WIDTH: f32 = 236.0;
const TOP_BAR_HEIGHT: f32 = 74.0;
const SEARCH_WIDTH: f32 = 420.0;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Destination {
    Albums,
    Tracks,
    Playlists,
    Storage,
    Devices,
}

impl Destination {
    fn label(self) -> &'static str {
        match self {
            Self::Albums => "Albums",
            Self::Tracks => "Tracks",
            Self::Playlists => "Playlists",
            Self::Storage => "Storage",
            Self::Devices => "Devices",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Albums => "icons/library.svg",
            Self::Tracks => "icons/music.svg",
            Self::Playlists => "icons/list-music.svg",
            Self::Storage => "icons/database.svg",
            Self::Devices => "icons/speaker.svg",
        }
    }
}

const NAV_GROUPS: &[(&str, &[Destination])] = &[
    (
        "LIBRARY",
        &[
            Destination::Albums,
            Destination::Tracks,
            Destination::Playlists,
        ],
    ),
    ("MANAGE", &[Destination::Storage]),
    ("OUTPUT", &[Destination::Devices]),
];

pub struct Shell {
    destination: Destination,
    row: Entity<PlaybackRow>,
}

impl Shell {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            destination: Destination::Albums,
            row: cx.new(PlaybackRow::new),
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .gap(px(22.))
            .pt(px(24.))
            .pr(px(14.))
            .pb(px(16.))
            .pl(px(14.))
            .bg(theme::bg_surface())
            .border_r_1()
            .border_color(theme::border())
            .child(render_brand())
            .child(self.render_navigation(cx))
            .child(div().flex_1())
            .child(render_settings_footer())
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

        div()
            .id(destination.label())
            .flex()
            .items_center()
            .gap(px(10.))
            .w_full()
            .px(px(10.))
            .py(px(9.))
            .rounded(px(theme::RADIUS_MD))
            .when(active, |item| item.bg(theme::accent_soft()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.destination = destination;
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
                    .child(destination.label()),
            )
            .when(destination == Destination::Storage, |item| {
                item.child(div().flex_1()).child(render_storage_badge())
            })
    }

    fn render_body(&self, cx: &Context<Self>) -> impl IntoElement {
        let row = self.row.read(cx);
        let error = row.error().map(str::to_string);
        let hint = match &error {
            Some(error) => error.clone(),
            None if !row.has_track() => "Drop an audio file anywhere in the window".to_string(),
            None => String::new(),
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap(px(18.))
            .pt(px(26.))
            .pr(px(28.))
            .pb(px(24.))
            .pl(px(28.))
            .child(
                div()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(26.))
                    .text_color(theme::text_primary())
                    .child(self.destination.label()),
            )
            .child(
                div()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(10.))
                    .text_color(theme::text_muted())
                    .child("NOT BUILT YET"),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(13.))
                    .text_color(if error.is_some() {
                        theme::danger()
                    } else {
                        theme::text_muted()
                    })
                    .child(hint),
            )
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("window-drop-target")
            .flex()
            .size_full()
            .bg(theme::bg_page())
            .drag_over::<ExternalPaths>(|style, _, _, _| style.bg(theme::accent_soft()))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.row.update(cx, |row, cx| row.handle_drop(paths, cx));
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.row.update(cx, |row, cx| row.update_scrub(event, cx));
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.row.update(cx, |row, cx| row.finish_scrub(event, cx));
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.row.update(cx, |row, cx| row.finish_scrub(event, cx));
                }),
            )
            .child(self.render_sidebar(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(render_top_bar())
                    .child(self.render_body(cx))
                    .child(self.row.clone()),
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
                .overflow_hidden()
                .rounded(px(7.))
                .border_1()
                .border_color(theme::border())
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(theme::bg_surface(), 0.),
                    linear_color_stop(theme::bg_inset(), 1.),
                ))
                .child(
                    svg()
                        .path("icons/pulse-mark.svg")
                        .size(px(22.))
                        .text_color(theme::accent()),
                ),
        )
        .child(
            div()
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::BOLD)
                .text_size(px(18.))
                .text_color(theme::text_primary())
                .child("Pulse"),
        )
}

fn render_storage_badge() -> impl IntoElement {
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
                .child("0"),
        )
}

fn render_settings_footer() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .w_full()
        .px(px(10.))
        .py(px(9.))
        .rounded(px(theme::RADIUS_MD))
        .child(
            svg()
                .path("icons/settings.svg")
                .size(px(18.))
                .flex_none()
                .text_color(theme::text_muted()),
        )
        .child(
            div()
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(14.))
                .text_color(theme::text_secondary())
                .child("Settings"),
        )
        .child(div().flex_1())
        .child(
            svg()
                .path("icons/chevrons-left.svg")
                .size(px(17.))
                .flex_none()
                .text_color(theme::text_muted()),
        )
}

fn render_top_bar() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .h(px(TOP_BAR_HEIGHT))
        .flex_none()
        .px(px(28.))
        .border_b_1()
        .border_color(theme::border())
        .child(render_search_input())
}

fn render_search_input() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .flex_none()
        .w(px(SEARCH_WIDTH))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(theme::RADIUS_MD))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg_inset())
        .child(
            svg()
                .path("icons/search.svg")
                .size(px(16.))
                .flex_none()
                .text_color(theme::text_muted()),
        )
        .child(
            div()
                .font_family(theme::FONT_SANS)
                .text_size(px(13.))
                .text_color(theme::text_muted())
                .child("Search library"),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_destination_is_reachable_from_exactly_one_nav_group() {
        let listed: Vec<Destination> = NAV_GROUPS
            .iter()
            .flat_map(|(_, destinations)| destinations.iter().copied())
            .collect();

        assert_eq!(listed.len(), 5);
        for destination in [
            Destination::Albums,
            Destination::Tracks,
            Destination::Playlists,
            Destination::Storage,
            Destination::Devices,
        ] {
            assert_eq!(
                listed
                    .iter()
                    .filter(|listed| **listed == destination)
                    .count(),
                1,
                "{}",
                destination.label()
            );
        }
    }

    #[test]
    fn each_destination_maps_to_a_bundled_icon() {
        use gpui::AssetSource;

        for destination in [
            Destination::Albums,
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
