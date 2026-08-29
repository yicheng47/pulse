use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, IntoElement, MouseButton, MouseDownEvent,
    StatefulInteractiveElement, Window, div, prelude::*, px, svg,
};

use super::{
    LibraryView, TrackMenu, TrackSurface, current_time_ms,
    logic::{
        FilterChip, format_duration, format_label, is_hi_res, quality_label, track_album,
        track_artist, track_title,
    },
    tracks_logic::{format_relative_time, track_sort_label},
};
use crate::{library::Track, theme, ui};

const TRACK_ROW_HEIGHT_PX: f32 = 58.;

impl LibraryView {
    pub(super) fn render_tracks(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.should_load_more_tracks() && self.load_more_tracks() {
            cx.notify();
        }
        let now_ms = current_time_ms();
        let visible = self.tracks.clone();
        let meta = if self.catalog_summary.track_count == 0 && self.is_library_loading() {
            "Loading…".to_string()
        } else if self.catalog_summary.track_count == 0 {
            "No tracks yet".to_string()
        } else {
            format!(
                "{} tracks · {} albums{}",
                self.track_total,
                self.catalog_summary.album_count,
                self.artist_filter
                    .as_ref()
                    .map(|artist| format!(" · {artist} artist filter"))
                    .unwrap_or_default()
            )
        };
        let mut rows = div().flex().flex_col().w_full();
        for (index, track) in visible.iter().cloned().enumerate() {
            rows = rows.child(self.render_track_row(index, track, now_ms, cx));
        }
        self.track_scrollbar.update(cx, |scrollbar, _| {
            scrollbar
                .set_estimated_content_height(px(self.tracks.len() as f32 * TRACK_ROW_HEIGHT_PX));
        });

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
                                    .child("Tracks"),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(px(13.))
                                    .text_color(theme::text_secondary())
                                    .child(meta),
                            ),
                    )
                    .child(self.render_track_sort(cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .h(px(47.))
                    .flex_none()
                    .child(self.render_track_filters(cx))
                    .child(self.render_artist_filter_hint(cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(render_track_table_header())
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .overflow_hidden()
                            .child(
                                div()
                                    .id("tracks-table-scroll")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.tracks_scroll)
                                    .child(if visible.is_empty() {
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .h(px(220.))
                                            .font_family(theme::FONT_SANS)
                                            .text_size(px(13.))
                                            .text_color(theme::text_muted())
                                            .child(if self.is_library_loading() {
                                                "Loading tracks…"
                                            } else if self.catalog_summary.track_count == 0 {
                                                "No tracks yet"
                                            } else {
                                                "No tracks match this filter"
                                            })
                                            .into_any_element()
                                    } else {
                                        rows.into_any_element()
                                    }),
                            )
                            .child(self.track_scrollbar.clone()),
                    ),
            )
            .into_any_element()
    }

    fn render_track_filters(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut filters = div()
            .id("track-filter-scroll")
            .flex()
            .items_center()
            .gap(px(8.));
        for chip in [
            FilterChip::All,
            FilterChip::HiRes,
            FilterChip::RecentlyAdded,
        ] {
            filters = filters.child(self.render_track_filter(chip, cx));
        }
        filters = filters.child(self.render_genre_filter(cx));
        filters.into_any_element()
    }

    fn render_track_filter(&self, chip: FilterChip, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.track_filter == chip;
        let label = if chip == FilterChip::All {
            "All tracks".to_string()
        } else {
            chip.label().to_string()
        };
        div()
            .id(format!("track-filter-{label}"))
            .flex()
            .items_center()
            .h(px(29.))
            .flex_none()
            .px(px(10.))
            .rounded(px(theme::RADIUS_SM))
            .bg(if selected {
                theme::accent_soft()
            } else {
                theme::bg_muted()
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_track_filter(chip.clone(), cx);
            }))
            .font_family(theme::FONT_SANS)
            .text_size(px(12.))
            .text_color(if selected {
                theme::accent()
            } else {
                theme::text_secondary()
            })
            .child(label)
    }

    fn render_track_sort(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("track-sort")
            .flex()
            .items_center()
            .gap(px(8.))
            .mt(px(17.5))
            .h(px(28.))
            .px(px(10.))
            .rounded(px(theme::RADIUS_SM))
            .bg(theme::bg_muted())
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.cycle_track_sort(cx)))
            .child(
                div()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(11.))
                    .text_color(theme::text_muted())
                    .child(track_sort_label(self.track_sort)),
            )
            .child(
                svg()
                    .path("icons/arrow-up-down.svg")
                    .size(px(14.))
                    .text_color(theme::text_muted()),
            )
    }

    fn render_artist_filter_hint(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.artist_filter.is_some();
        let label = self
            .artist_filter
            .clone()
            .unwrap_or_else(|| "ARTIST FILTER".to_string());
        let mut hint = div()
            .id("artist-filter-hint")
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .px(px(10.))
            .rounded(px(theme::RADIUS_SM))
            .bg(if active {
                theme::primary_soft()
            } else {
                theme::bg_surface_alt()
            })
            .cursor_pointer()
            // Same press-closed guard as the output picker: when the popover
            // is open, the press lands on `on_mouse_down_out` first (the
            // trigger is outside the popover) and closes it; without the
            // guard, the click that follows would reopen it immediately.
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Left {
                    this.artist_hint_press_closed_popover = this.artist_popover_open;
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| {
                if std::mem::take(&mut this.artist_hint_press_closed_popover) {
                    cx.notify();
                    return;
                }
                this.genre_popover_open = false;
                this.artist_popover_open = true;
                this.text_input.reset("");
                window.focus(&this.input_focus, cx);
                cx.notify();
            }))
            .child(
                svg()
                    .path("icons/user-round-search.svg")
                    .size(px(14.))
                    .text_color(if active {
                        theme::primary()
                    } else {
                        theme::text_muted()
                    }),
            )
            .child(
                div()
                    .max_w(px(170.))
                    .truncate()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(11.))
                    .text_color(if active {
                        theme::primary()
                    } else {
                        theme::text_muted()
                    })
                    .child(label),
            );
        if self.artist_popover_open {
            hint = hint.child(self.render_artist_popover(cx));
        }
        hint
    }

    fn render_artist_popover(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filtered = self.filtered_artists();
        let mut rows = div().flex().flex_col().w_full();
        for (index, (artist, track_count)) in filtered.into_iter().enumerate() {
            let selected = self
                .artist_filter
                .as_deref()
                .is_some_and(|active| active.eq_ignore_ascii_case(&artist));

            let chosen = artist.clone();
            rows = rows.child(
                div()
                    .id(("artist-filter-option", index))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .w_full()
                    .px(px(12.))
                    .py(px(8.))
                    .when(selected, |row| row.bg(theme::accent_soft()))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choose_artist_filter(Some(chosen.clone()), cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(13.))
                            .text_color(if selected {
                                theme::accent()
                            } else {
                                theme::text_primary()
                            })
                            .child(artist),
                    )
                    .child(
                        div()
                            .font_family(theme::FONT_MONO)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(10.))
                            .text_color(theme::text_muted())
                            .child(track_count.to_string()),
                    ),
            );
        }

        div()
            .id("artist-filter-popover")
            .absolute()
            .right_0()
            .top(px(36.))
            .flex()
            .flex_col()
            .w(px(240.))
            .max_h(px(420.))
            .py(px(6.))
            .rounded(px(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::bg_surface())
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.artist_popover_open = false;
                cx.notify();
            }))
            .child(
                div()
                    .w_full()
                    .px(px(10.))
                    .py(px(6.))
                    .child(super::render_text_input(
                        "artist-search-input",
                        &self.text_input,
                        &self.input_focus,
                        cx,
                    )),
            )
            .child(
                div()
                    .id("artist-filter-options-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            )
            .child(div().w_full().h(px(1.)).my(px(4.)).bg(theme::border()))
            .child(
                div()
                    .id("artist-filter-clear")
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .w_full()
                    .px(px(12.))
                    .py(px(8.))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.choose_artist_filter(None, cx);
                    }))
                    .child(
                        svg()
                            .path("icons/x.svg")
                            .size(px(14.))
                            .text_color(theme::text_secondary()),
                    )
                    .child(
                        div()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(13.))
                            .text_color(theme::text_secondary())
                            .child("Clear filter"),
                    ),
            )
    }

    fn render_track_row(
        &self,
        index: usize,
        track: Track,
        now_ms: i64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let playing = self.is_now_playing(&track.path);
        let missing = self.is_track_missing(track.id);
        let selected = self.selected_track_id == Some(track.id);
        let track_id = track.id;
        let artist = track_artist(&track).to_string();
        let artist_for_filter = artist.clone();
        let format = quality_label(track.bit_depth, track.sample_rate_hz)
            .map(|quality| quality.replace('/', " / "))
            .map(|quality| format!("{} {quality}", format_label(&track.path)))
            .unwrap_or_else(|| format_label(&track.path).to_string());
        let added = format_relative_time(track.added_at_ms, now_ms);

        div()
            .id(format!("track-row-{index}"))
            .flex()
            .items_center()
            .w_full()
            .h(px(TRACK_ROW_HEIGHT_PX))
            .flex_none()
            .relative()
            .border_t_1()
            .border_color(theme::border())
            .when(selected || playing, |row| row.bg(theme::bg_selected()))
            .when(playing, |row| {
                row.child(ui::playing_row_glow())
                    .child(ui::playing_row_bar())
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                this.activate_visible_track(track_id, event.click_count() == 2, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.selected_track_id = Some(track_id);
                    this.playlist_menu = None;
                    this.track_menu = Some(TrackMenu {
                        track_id,
                        surface: TrackSurface::Tracks,
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
                    .items_center()
                    .w(px(456.))
                    .h_full()
                    .pl(px(14.))
                    .child(super::albums::render_cover(
                        track.cover_art_path.as_deref(),
                        42.,
                        42.,
                        18.,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .ml(px(12.))
                            .min_w_0()
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
                                            .text_size(px(15.))
                                            .text_color(if playing {
                                                theme::accent()
                                            } else if missing {
                                                theme::text_muted()
                                            } else {
                                                theme::text_primary()
                                            })
                                            .child(track_title(&track)),
                                    )
                                    .when(missing, |title| {
                                        title.child(super::missing_file_badge())
                                    }),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(px(12.))
                                    .text_color(theme::text_secondary())
                                    .child(artist.clone()),
                            ),
                    ),
            )
            .child(
                div()
                    .id(format!("track-artist-{index}"))
                    .w(px(162.))
                    .truncate()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_artist_filter(Some(artist_for_filter.clone()), cx);
                    }))
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_secondary())
                    .child(artist),
            )
            .child(
                div()
                    .w(px(232.))
                    .truncate()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_secondary())
                    .child(track_album(&track).to_string()),
            )
            .child(
                div()
                    .w(px(70.))
                    .font_family(theme::FONT_MONO)
                    .text_size(px(11.))
                    .text_color(theme::text_primary())
                    .child(format_duration(track.duration_ms)),
            )
            .child(
                div()
                    .w(px(132.))
                    .truncate()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(11.))
                    .text_color(if is_hi_res(track.bit_depth, track.sample_rate_hz) {
                        theme::primary()
                    } else {
                        theme::quality()
                    })
                    .child(format),
            )
            .child(
                div()
                    .w(px(82.))
                    .truncate()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(11.))
                    .text_color(theme::text_secondary())
                    .child(added),
            )
    }
}

fn render_track_table_header() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .w_full()
        .h(px(33.))
        .flex_none()
        .child(header_cell("TRACK", 456., 14.))
        .child(header_cell("ARTIST", 162., 0.))
        .child(header_cell("ALBUM", 232., 0.))
        .child(header_cell("TIME", 70., 0.))
        .child(header_cell("FORMAT", 132., 0.))
        .child(header_cell("ADDED", 82., 0.))
}

fn header_cell(label: &'static str, width: f32, left_padding: f32) -> impl IntoElement {
    div()
        .w(px(width))
        .pl(px(left_padding))
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(px(10.))
        .text_color(theme::text_muted())
        .child(label)
}
