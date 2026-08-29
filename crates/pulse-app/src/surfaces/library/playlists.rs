use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, IntoElement, MouseButton, MouseDownEvent, div,
    prelude::*, px, svg,
};

use super::{
    LibraryView, PlaylistMenu, PlaylistNameMode, TrackMenu, TrackSurface, current_time_ms,
    logic::{format_duration, format_label, quality_label, track_album, track_artist, track_title},
    playlists_logic::{format_playlist_duration, format_updated},
};
use crate::{
    backend::{PlaylistId, PlaylistSummary, PlaylistTrack},
    theme, ui,
};

impl LibraryView {
    pub(super) fn render_playlists(&self, cx: &mut Context<Self>) -> AnyElement {
        let total_tracks = self
            .playlists
            .iter()
            .map(|playlist| playlist.track_count)
            .sum::<u64>();
        let meta = if self.playlists.is_empty() && self.is_library_loading() {
            "Loading…".to_string()
        } else if self.playlists.is_empty() {
            "No playlists yet".to_string()
        } else {
            format!(
                "{} playlists · {total_tracks} track entries",
                self.playlists.len()
            )
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .px(px(28.))
            .pt(px(26.))
            .pb(px(24.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .w_full()
                    .min_h(px(63.))
                    .pb(px(10.))
                    .flex_none()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(34.))
                                    .text_color(theme::text_primary())
                                    .child("Playlists"),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(px(13.))
                                    .text_color(theme::text_secondary())
                                    .child(meta),
                            ),
                    )
                    .child(render_new_playlist_button("new-playlist-header", cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .gap(px(18.))
                    .child(self.render_playlist_list(cx))
                    .child(self.render_playlist_detail(cx)),
            )
            .into_any_element()
    }

    fn render_playlist_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = div().flex().flex_col().w_full();
        for (index, playlist) in self.playlists.iter().cloned().enumerate() {
            rows = rows.child(self.render_playlist_row(index, playlist, cx));
        }
        div()
            .flex()
            .flex_col()
            .w(px(390.))
            .h_full()
            .flex_none()
            .overflow_hidden()
            .border_r_1()
            .border_color(theme::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(45.))
                    .flex_none()
                    .px(px(16.))
                    .border_b_1()
                    .border_color(theme::border())
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(17.))
                            .text_color(theme::text_primary())
                            .child("Library Playlists"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .min_w(px(22.))
                            .h(px(18.))
                            .px(px(5.))
                            .rounded(px(theme::RADIUS_SM))
                            .bg(theme::bg_muted())
                            .font_family(theme::FONT_MONO)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(10.))
                            .text_color(theme::text_muted())
                            .child(self.playlists.len().to_string()),
                    ),
            )
            .child(if self.playlists.is_empty() && self.is_library_loading() {
                super::list_loading_placeholder("Loading playlists…")
            } else if self.playlists.is_empty() {
                render_playlist_empty(cx).into_any_element()
            } else {
                div()
                    .id("playlist-list-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.playlists_scroll)
                    .child(rows)
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_playlist_row(
        &self,
        index: usize,
        summary: PlaylistSummary,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let playlist_id = summary.playlist.id;
        let selected = self.selected_playlist_id == Some(playlist_id);
        let updated = format_updated(summary.playlist.updated_at_ms, current_time_ms());
        let meta = format!(
            "{} entries · {}",
            summary.track_count,
            format_playlist_duration(summary.total_duration_ms)
        );
        div()
            .id(format!("playlist-row-{index}"))
            .flex()
            .items_center()
            .gap(px(10.))
            .w_full()
            .h(px(64.))
            .flex_none()
            .px(px(10.))
            .relative()
            .border_b_1()
            .border_color(theme::border())
            .when(selected, |row| {
                row.bg(theme::bg_selected())
                    .child(ui::playing_row_glow())
                    .child(ui::playing_row_bar())
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_playlist(playlist_id, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.track_menu = None;
                    this.playlist_menu = Some(PlaylistMenu {
                        playlist_id,
                        anchor: event.position,
                    });
                    window.focus(&this.input_focus, cx);
                    cx.notify();
                }),
            )
            .child(super::albums::render_cover(
                summary.cover_art_path.as_deref(),
                44.,
                44.,
                20.,
                theme::RADIUS_SM,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.))
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(13.))
                            .text_color(if selected {
                                theme::accent()
                            } else {
                                theme::text_primary()
                            })
                            .child(summary.playlist.name),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(11.))
                            .text_color(theme::text_muted())
                            .child(meta),
                    ),
            )
            .child(
                div()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(9.))
                    .text_color(theme::text_muted())
                    .child(updated),
            )
    }

    fn render_playlist_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(detail) = &self.playlist_detail else {
            return render_no_playlist_selected();
        };
        let summary = detail.summary.clone();
        let entries = detail.entries.clone();
        let playlist_id = summary.playlist.id;
        let selected_start = self
            .selected_playlist_position
            .and_then(|selected| entries.iter().position(|entry| entry.position == selected))
            .map(|index| index + 1)
            .unwrap_or(1);
        let meta = format!(
            "{} entries · {} · Updated {}",
            summary.track_count,
            format_playlist_duration(summary.total_duration_ms),
            format_updated(summary.playlist.updated_at_ms, current_time_ms())
        );
        let mut rows = div().flex().flex_col().w_full();
        for (index, entry) in entries.iter().cloned().enumerate() {
            rows = rows.child(self.render_playlist_track_row(index, entry, playlist_id, cx));
        }

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(18.))
                    .w_full()
                    .h(px(132.))
                    .flex_none()
                    .p(px(18.))
                    .border_b_1()
                    .border_color(theme::border())
                    .child(super::albums::render_cover(
                        summary.cover_art_path.as_deref(),
                        92.,
                        92.,
                        34.,
                        theme::RADIUS_SM,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(6.))
                            .child(
                                div()
                                    .font_family(theme::FONT_MONO)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(9.))
                                    .text_color(theme::text_muted())
                                    .child("SELECTED PLAYLIST"),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(30.))
                                    .text_color(theme::text_primary())
                                    .child(summary.playlist.name),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(px(12.))
                                    .text_color(theme::text_secondary())
                                    .child(meta),
                            )
                            .child(render_manual_badge()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .id("play-playlist")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(34.))
                                    .rounded(px(theme::RADIUS_MD))
                                    .bg(theme::accent())
                                    .opacity(if entries.is_empty() { 0.45 } else { 1.0 })
                                    .when(!entries.is_empty(), |button| {
                                        button.cursor_pointer().on_click(
                                            cx.listener(|this, _, _, cx| this.play_playlist(cx)),
                                        )
                                    })
                                    .child(
                                        svg()
                                            .path("icons/play.svg")
                                            .size(px(15.))
                                            .text_color(theme::bg_inset()),
                                    ),
                            )
                            .child(
                                div()
                                    .id("shuffle-playlist")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(34.))
                                    .rounded(px(theme::RADIUS_MD))
                                    .border_1()
                                    .border_color(theme::border())
                                    .bg(theme::bg_muted())
                                    .opacity(if entries.is_empty() { 0.45 } else { 1.0 })
                                    .when(!entries.is_empty(), |button| {
                                        button.cursor_pointer().on_click(cx.listener(
                                            |this, _, _, cx| {
                                                this.shuffle_playlist(cx);
                                            },
                                        ))
                                    })
                                    .child(
                                        svg()
                                            .path("icons/shuffle.svg")
                                            .size(px(15.))
                                            .text_color(theme::text_secondary()),
                                    ),
                            )
                            .child(
                                div()
                                    .id("playlist-actions")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(34.))
                                    .rounded(px(theme::RADIUS_MD))
                                    .border_1()
                                    .border_color(theme::border())
                                    .bg(theme::bg_muted())
                                    .cursor_pointer()
                                    .on_click(cx.listener(
                                        move |this, event: &ClickEvent, window, cx| {
                                            this.track_menu = None;
                                            this.playlist_menu = Some(PlaylistMenu {
                                                playlist_id,
                                                anchor: match event {
                                                    ClickEvent::Mouse(event) => event.up.position,
                                                    ClickEvent::Keyboard(event) => {
                                                        event.bounds.origin
                                                    }
                                                },
                                            });
                                            window.focus(&this.input_focus, cx);
                                            cx.notify();
                                        },
                                    ))
                                    .child(
                                        svg()
                                            .path("icons/ellipsis.svg")
                                            .size(px(16.))
                                            .text_color(theme::text_secondary()),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(43.))
                    .flex_none()
                    .px(px(12.))
                    .border_b_1()
                    .border_color(theme::border())
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(17.))
                    .text_color(theme::text_primary())
                    .child("Queue Preview")
                    .child(
                        div()
                            .font_family(theme::FONT_MONO)
                            .font_weight(FontWeight::NORMAL)
                            .text_size(px(10.))
                            .text_color(theme::text_muted())
                            .child(if entries.is_empty() {
                                "0 entries".to_string()
                            } else {
                                format!("{selected_start} of {}", entries.len())
                            }),
                    ),
            )
            .child(if entries.is_empty() {
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child("Add tracks from Albums or Tracks")
                    .into_any_element()
            } else {
                div()
                    .id("playlist-detail-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.playlist_detail_scroll)
                    .child(rows)
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_playlist_track_row(
        &self,
        index: usize,
        entry: PlaylistTrack,
        playlist_id: PlaylistId,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let track = entry.track;
        let position = entry.position;
        let track_id = track.id;
        let playing = self.is_now_playing(&track.path);
        let missing = self.is_track_missing(track.id);
        let selected = self.selected_playlist_position == Some(position);
        let quality = quality_label(track.bit_depth, track.sample_rate_hz)
            .unwrap_or_else(|| format_label(&track.path).to_string());
        div()
            .id(format!("playlist-track-{index}"))
            .flex()
            .items_center()
            .w_full()
            .h(px(50.))
            .flex_none()
            .px(px(12.))
            .relative()
            .border_b_1()
            .border_color(theme::border())
            .when(selected || playing, |row| row.bg(theme::bg_selected()))
            .when(playing, |row| {
                row.child(ui::playing_row_glow())
                    .child(ui::playing_row_bar())
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                this.activate_playlist_entry(position, event.click_count() == 2, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.selected_playlist_position = Some(position);
                    this.playlist_menu = None;
                    this.track_menu = Some(TrackMenu {
                        track_id,
                        surface: TrackSurface::Playlist {
                            playlist_id,
                            position,
                        },
                        anchor: event.position,
                        flyout_open: false,
                    });
                    window.focus(&this.input_focus, cx);
                    cx.notify();
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .w_full()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(theme::FONT_SANS)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(13.))
                                    .text_color(if playing {
                                        theme::accent()
                                    } else if missing {
                                        theme::text_muted()
                                    } else {
                                        theme::text_primary()
                                    })
                                    .child(track_title(&track)),
                            )
                            .when(missing, |title| title.child(super::missing_file_badge())),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(11.))
                            .text_color(theme::text_muted())
                            .child(track_album(&track).to_string()),
                    ),
            )
            .child(
                div()
                    .w(px(150.))
                    .truncate()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(11.))
                    .text_color(theme::text_secondary())
                    .child(track_artist(&track).to_string()),
            )
            .child(
                div()
                    .w(px(52.))
                    .font_family(theme::FONT_MONO)
                    .text_size(px(10.))
                    .text_color(theme::text_secondary())
                    .child(format_duration(track.duration_ms)),
            )
            .child(crate::ui::Badge::new(quality))
    }

    pub(super) fn render_playlist_name_modal(
        &self,
        mode: PlaylistNameMode,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (title, confirm) = match mode {
            PlaylistNameMode::Create { .. } => ("New Playlist", "Create Playlist"),
            PlaylistNameMode::Rename { .. } => ("Rename Playlist", "Rename"),
        };
        super::storage_modals::render_modal_scrim(
            div()
                .flex()
                .flex_col()
                .w(px(440.))
                .overflow_hidden()
                .rounded(px(theme::RADIUS_LG))
                .border_1()
                .border_color(theme::border_strong())
                .bg(theme::bg_surface())
                .child(super::storage_modals::render_modal_header(title, cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(14.))
                        .p(px(22.))
                        .child(super::storage_modals::render_field_label("NAME"))
                        .child(super::render_text_input(
                            "playlist-name-input",
                            &self.text_input,
                            &self.input_focus,
                            cx,
                        ))
                        .child(
                            div()
                                .font_family(theme::FONT_SANS)
                                .text_size(px(11.))
                                .text_color(theme::text_muted())
                                .child("Names do not need to be unique."),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(9.))
                        .h(px(62.))
                        .flex_none()
                        .px(px(22.))
                        .border_t_1()
                        .border_color(theme::border())
                        .child(super::storage_modals::render_cancel_modal_button(cx))
                        .child(
                            ui::Button::new("confirm-playlist-name", confirm)
                                .variant(ui::ButtonVariant::Primary)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_playlist_name(cx);
                                })),
                        ),
                ),
        )
    }

    pub(super) fn render_delete_playlist_modal(
        &self,
        playlist_id: PlaylistId,
        name: String,
        entry_count: u64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entry_word = if entry_count == 1 { "entry" } else { "entries" };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(9.))
            .p(px(22.))
            .child(
                div()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(17.))
                    .text_color(theme::text_primary())
                    .child(format!("Delete “{name}”?")),
            )
            .child(
                div()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_secondary())
                    .child(format!(
                        "This removes {entry_count} track {entry_word} from the playlist. Your music files stay in the library."
                    )),
            );
        ui::ConfirmDialog::new("delete-playlist-dialog", "Delete Playlist", body)
            .cancel_id("cancel-delete-playlist")
            .confirm_id(format!("confirm-delete-playlist-{playlist_id}"))
            .close_id("close-delete-playlist")
            .confirm_label("Delete Playlist")
            .on_cancel(cx.listener(|this, _, _, cx| {
                this.modal = None;
                cx.notify();
            }))
            .on_confirm(cx.listener(|this, _, _, cx| {
                this.confirm_delete_playlist(cx);
            }))
            .into_any_element()
    }
}

fn render_playlist_empty(cx: &mut Context<LibraryView>) -> impl IntoElement {
    ui::EmptyStateCard::new(
        svg()
            .path("icons/list-music.svg")
            .size(px(25.))
            .text_color(theme::text_muted()),
        "No playlists yet",
        "Build a collection from tracks in your library.",
        render_new_playlist_button("new-playlist-empty", cx),
    )
}

fn render_new_playlist_button(id: &'static str, cx: &mut Context<LibraryView>) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(7.))
        .h(px(34.))
        .px(px(12.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::accent())
        .bg(theme::accent_soft())
        .cursor_pointer()
        .on_click(cx.listener(|this, _, window, cx| {
            this.begin_create_playlist(None, window, cx);
        }))
        .child(
            svg()
                .path("icons/plus.svg")
                .size(px(15.))
                .text_color(theme::accent()),
        )
        .child(
            div()
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(13.))
                .text_color(theme::text_primary())
                .child("New Playlist"),
        )
}

fn render_no_playlist_selected() -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .child(
                    svg()
                        .path("icons/list-music.svg")
                        .size(px(34.))
                        .text_color(theme::text_muted()),
                )
                .child(
                    div()
                        .mt(px(13.))
                        .font_family(theme::FONT_DISPLAY)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(18.))
                        .text_color(theme::text_secondary())
                        .child("Select a playlist"),
                ),
        )
        .into_any_element()
}

fn render_manual_badge() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .h(px(19.))
        .w(px(56.))
        .px(px(7.))
        .rounded(px(theme::RADIUS_SM))
        .bg(theme::bg_muted())
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(px(9.))
        .text_color(theme::text_muted())
        .child("MANUAL")
}
