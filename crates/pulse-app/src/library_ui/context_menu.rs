use gpui::{AnyElement, Context, FontWeight, IntoElement, KeyDownEvent, div, prelude::*, px, svg};

use super::{LibraryView, TrackSurface};
use crate::{
    shell::{SIDEBAR_WIDTH, TOP_BAR_HEIGHT},
    theme,
};

impl LibraryView {
    pub(super) fn render_track_context_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let menu = self
            .track_menu
            .as_ref()
            .expect("track menu is open")
            .clone();
        let track_id = menu.track_id;
        let surface = menu.surface;
        let flyout_open = menu.flyout_open;
        let playlist_move = match surface {
            TrackSurface::Playlist {
                playlist_id,
                position,
            } => self.playlist_detail.as_ref().and_then(|detail| {
                let index = detail
                    .entries
                    .iter()
                    .position(|entry| entry.position == position)?;
                Some((playlist_id, position, index, detail.entries.len()))
            }),
            TrackSurface::Album | TrackSurface::Tracks => None,
        };
        let mut items = div()
            .flex()
            .flex_col()
            .w(px(210.))
            .py(px(6.))
            .rounded(px(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::bg_surface())
            .child(
                context_item("context-play", "icons/play.svg", "Play")
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.play_context_track(track_id, surface, cx);
                    })),
            )
            .child(
                context_item(
                    "context-add-to-playlist",
                    "icons/list-music.svg",
                    "Add to Playlist",
                )
                .cursor_pointer()
                .on_hover(cx.listener(|this, &hovered, _, cx| {
                    if hovered
                        && let Some(menu) = &mut this.track_menu
                        && !menu.flyout_open
                    {
                        menu.flyout_open = true;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(menu) = &mut this.track_menu {
                        menu.flyout_open = true;
                    }
                    cx.notify();
                }))
                .child(div().flex_1())
                .child(
                    svg()
                        .path("icons/chevron-down.svg")
                        .size(px(13.))
                        .text_color(theme::text_muted()),
                ),
            );

        if let Some((playlist_id, position, index, count)) = playlist_move {
            items = items
                .child(context_separator())
                .child(
                    context_item(
                        "context-remove-from-playlist",
                        "icons/x.svg",
                        "Remove from Playlist",
                    )
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.remove_playlist_entry(playlist_id, position, cx);
                    })),
                )
                .child(
                    context_text_item("context-move-up", "Move Up", index > 0).when(
                        index > 0,
                        |item| {
                            item.cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.move_playlist_entry(playlist_id, position, index - 1, cx);
                                }))
                        },
                    ),
                )
                .child(
                    context_text_item("context-move-down", "Move Down", index + 1 < count).when(
                        index + 1 < count,
                        |item| {
                            item.cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.move_playlist_entry(playlist_id, position, index + 1, cx);
                                }))
                        },
                    ),
                );
        }

        let mut wrapper = div()
            .absolute()
            .left(menu.anchor.x - px(SIDEBAR_WIDTH))
            .top(menu.anchor.y - px(TOP_BAR_HEIGHT))
            .flex()
            .items_start()
            .w(px(if flyout_open { 412. } else { 210. }))
            .track_focus(&self.input_focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.track_menu = None;
                    cx.notify();
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.track_menu = None;
                cx.notify();
            }))
            .child(items);
        if flyout_open {
            wrapper = wrapper.child(self.render_playlist_flyout(track_id, cx));
        }
        wrapper.into_any_element()
    }

    fn render_playlist_flyout(
        &self,
        track_id: crate::library::TrackId,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut flyout = div()
            .flex()
            .flex_col()
            .w(px(200.))
            .mt(px(32.))
            .ml(px(2.))
            .py(px(6.))
            .rounded(px(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::bg_surface());
        for (index, summary) in self.playlists.iter().enumerate() {
            let playlist_id = summary.playlist.id;
            flyout = flyout.child(
                div()
                    .id(format!("add-to-playlist-{index}"))
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .h(px(32.))
                    .w_full()
                    .px(px(12.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.add_track_to_playlist(playlist_id, track_id, cx);
                    }))
                    .child(
                        svg()
                            .path("icons/list-music.svg")
                            .size(px(14.))
                            .text_color(theme::text_muted()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(11.))
                            .text_color(theme::text_secondary())
                            .child(summary.playlist.name.clone()),
                    ),
            );
        }
        flyout.child(context_separator()).child(
            div()
                .id("add-to-new-playlist")
                .flex()
                .items_center()
                .gap(px(9.))
                .h(px(32.))
                .w_full()
                .px(px(12.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.begin_create_playlist(Some(track_id), window, cx);
                }))
                .child(
                    svg()
                        .path("icons/plus.svg")
                        .size(px(14.))
                        .text_color(theme::accent()),
                )
                .child(
                    div()
                        .font_family(theme::FONT_SANS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(11.))
                        .text_color(theme::accent())
                        .child("New Playlist…"),
                ),
        )
    }

    pub(super) fn render_playlist_context_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let menu = self
            .playlist_menu
            .as_ref()
            .expect("playlist menu is open")
            .clone();
        let rename_id = menu.playlist_id;
        let delete_id = menu.playlist_id;
        div()
            .absolute()
            .left(menu.anchor.x - px(SIDEBAR_WIDTH))
            .top(menu.anchor.y - px(TOP_BAR_HEIGHT))
            .flex()
            .flex_col()
            .w(px(160.))
            .py(px(6.))
            .rounded(px(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::bg_surface())
            .track_focus(&self.input_focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.playlist_menu = None;
                    cx.notify();
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.playlist_menu = None;
                cx.notify();
            }))
            .child(
                context_text_item("rename-playlist", "Rename", true)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.begin_rename_playlist(rename_id, window, cx);
                    })),
            )
            .child(context_separator())
            .child(
                context_text_item("delete-playlist", "Delete", true)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.request_delete_playlist(delete_id, cx);
                    }))
                    .text_color(theme::danger()),
            )
            .into_any_element()
    }

    fn play_context_track(
        &mut self,
        track_id: crate::library::TrackId,
        surface: TrackSurface,
        cx: &mut Context<Self>,
    ) {
        self.track_menu = None;
        match surface {
            TrackSurface::Album => {
                if let Some(index) = self
                    .album_detail
                    .as_ref()
                    .and_then(|detail| detail.tracks.iter().position(|track| track.id == track_id))
                {
                    self.activate_album_track(index, true, cx);
                }
            }
            TrackSurface::Tracks => self.activate_visible_track(track_id, true, cx),
            TrackSurface::Playlist { position, .. } => {
                self.activate_playlist_entry(position, true, cx);
            }
        }
    }
}

fn context_item(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(32.))
        .w_full()
        .px(px(12.))
        .child(
            svg()
                .path(icon)
                .size(px(15.))
                .text_color(theme::text_muted()),
        )
        .child(
            div()
                .font_family(theme::FONT_SANS)
                .text_size(px(11.))
                .text_color(theme::text_secondary())
                .child(label),
        )
}

fn context_text_item(
    id: &'static str,
    label: &'static str,
    enabled: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .h(px(32.))
        .w_full()
        .px(px(37.))
        .opacity(if enabled { 1.0 } else { 0.4 })
        .font_family(theme::FONT_SANS)
        .text_size(px(11.))
        .text_color(theme::text_secondary())
        .child(label)
}

fn context_separator() -> impl IntoElement {
    div().w_full().h(px(1.)).bg(theme::border())
}
