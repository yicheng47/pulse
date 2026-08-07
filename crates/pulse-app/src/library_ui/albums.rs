use std::{collections::BTreeSet, path::Path};

use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, IntoElement, MouseButton, MouseDownEvent,
    ObjectFit, StatefulInteractiveElement, Window, div, img, prelude::*, px, svg,
};

use super::{
    LibraryView, TrackMenu, TrackSurface,
    view_model::{
        FilterChip, format_duration, format_label, is_hi_res, quality_label, track_artist,
        track_title,
    },
};
use crate::{
    library::{Album, AlbumSortOrder, Track},
    shell::SIDEBAR_WIDTH,
    theme,
};

const ALBUM_BODY_HORIZONTAL_PADDING: f32 = 28.;
const ALBUM_GRID_GAP: f32 = 14.;
const ALBUM_CARD_MIN_WIDTH: f32 = 200.;

impl LibraryView {
    pub(super) fn render_albums(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.album_detail.is_some() {
            self.render_album_detail(window, cx)
        } else {
            self.render_album_grid(window, cx)
        }
    }

    fn render_album_grid(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        // Infinite scroll: fetch the next backend page when the grid is
        // scrolled near its bottom (or does not fill the viewport yet).
        if self.should_load_more_albums() && self.load_more_albums() {
            cx.notify();
        }
        let library_empty = self.album_total == 0 && self.album_filter == FilterChip::All;
        let meta = if self.albums.is_empty() && self.is_library_loading() {
            "Loading…".to_string()
        } else if library_empty {
            "No albums yet".to_string()
        } else {
            format!(
                "{} albums · {} tracks{}",
                self.album_total,
                self.catalog_summary.track_count,
                if self.album_filter == FilterChip::All {
                    String::new()
                } else {
                    format!(" · {} filter active", self.album_filter.label())
                }
            )
        };

        let mut body = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .px(px(ALBUM_BODY_HORIZONTAL_PADDING))
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
                    .child(render_page_title("Albums", meta)),
            );

        if self.albums.is_empty() && self.is_library_loading() {
            return body
                .child(super::list_loading_placeholder("Loading albums…"))
                .into_any_element();
        }
        if library_empty {
            return body.child(self.render_albums_empty(cx)).into_any_element();
        }

        body = body.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .w_full()
                .h(px(47.))
                .flex_none()
                .child(self.render_album_filters(cx))
                .child(self.render_album_sort(cx)),
        );

        let browser = if self.albums.is_empty() {
            render_no_filter_matches("No albums match this filter").into_any_element()
        } else {
            let visible = self.albums.clone();
            let columns = album_grid_columns(f32::from(window.viewport_size().width));
            let mut grid = div()
                .grid()
                .grid_cols(columns)
                .gap(px(ALBUM_GRID_GAP))
                .w_full()
                .pb(px(24.));
            for (index, album) in visible.into_iter().enumerate() {
                grid = grid.child(self.render_album_card(index, album, cx));
            }
            div()
                .id("albums-grid-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&self.albums_scroll)
                .child(grid)
                .into_any_element()
        };

        body.child(browser).into_any_element()
    }

    fn render_albums_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .w_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(64.))
                    .rounded(px(theme::RADIUS_LG))
                    .bg(theme::bg_muted())
                    .child(
                        svg()
                            .path("icons/disc-3.svg")
                            .size(px(30.))
                            .text_color(theme::text_muted()),
                    ),
            )
            .child(
                div()
                    .mt(px(14.))
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(24.))
                    .text_color(theme::text_primary())
                    .child("Your library is empty"),
            )
            .child(
                div()
                    .mt(px(12.))
                    .w(px(400.))
                    .text_center()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(13.))
                    .text_color(theme::text_secondary())
                    .child(
                        "Add a storage folder and Pulse will index your FLAC, ALAC, AIFF and WAV files into one library.",
                    ),
            )
            .child(
                div()
                    .id("albums-add-storage")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .mt(px(14.))
                    .px(px(14.))
                    .h(px(36.))
                    .rounded(px(theme::RADIUS_MD))
                    .border_1()
                    .border_color(theme::accent())
                    .bg(theme::accent_soft())
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.choose_storage_folder(cx)))
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
                            .text_size(px(14.))
                            .text_color(theme::text_primary())
                            .child("Add Storage"),
                    ),
            )
            .child(
                div()
                    .mt(px(14.))
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(10.))
                    .text_color(theme::text_muted())
                    .child("LOCAL FOLDERS AND NAS MOUNTS SUPPORTED"),
            )
            .into_any_element()
    }

    fn render_album_filters(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut filters = div()
            .id("album-filter-scroll")
            .flex()
            .items_center()
            .gap(px(8.))
            .max_w(px(900.))
            .overflow_x_scroll();
        for chip in [
            FilterChip::All,
            FilterChip::HiRes,
            FilterChip::RecentlyAdded,
        ] {
            filters = filters.child(self.render_album_filter(chip, cx));
        }
        for genre in &self.genres {
            filters = filters.child(self.render_album_filter(FilterChip::Genre(genre.clone()), cx));
        }
        filters.into_any_element()
    }

    fn render_album_filter(&self, chip: FilterChip, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.album_filter == chip;
        let label = chip.label().to_string();
        div()
            .id(format!("album-filter-{label}"))
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
                this.album_filter = chip.clone();
                this.reset_albums();
                this.reload_or_show_error();
                cx.notify();
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

    fn render_album_sort(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("album-sort")
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .px(px(10.))
            .rounded(px(theme::RADIUS_SM))
            .bg(theme::bg_muted())
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.cycle_album_sort(cx)))
            .child(
                div()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(11.))
                    .text_color(theme::text_muted())
                    .child(album_sort_label(self.album_sort)),
            )
            .child(
                svg()
                    .path("icons/arrow-up-down.svg")
                    .size(px(14.))
                    .text_color(theme::text_muted()),
            )
    }

    fn render_album_card(
        &self,
        index: usize,
        album: Album,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let cover = album.cover_art_path.clone();
        let title = album.title.clone();
        let artist = album.artist.clone();
        let meta = match quality_label(album.max_bit_depth, album.max_sample_rate_hz) {
            Some(quality) => format!("{quality} · {} tracks", album.track_count),
            None => format!("{} tracks", album.track_count),
        };
        div()
            .id(format!("album-card-{index}"))
            .flex()
            .flex_col()
            .w_full()
            .min_h(px(226.))
            .p(px(12.))
            .rounded(px(theme::RADIUS_MD))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_surface())
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_album(album.clone(), cx);
            }))
            .child(render_cover(cover.as_deref(), 194.4, 140., 30.).w_full())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .mt(px(10.))
                    .w_full()
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(13.))
                            .text_color(theme::text_primary())
                            .child(title),
                    )
                    .child(
                        div()
                            .mt(px(3.))
                            .w_full()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(12.))
                            .text_color(theme::text_secondary())
                            .child(artist),
                    )
                    .child(
                        div()
                            .mt(px(3.))
                            .font_family(theme::FONT_MONO)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(10.))
                            .text_color(theme::quality())
                            .child(meta),
                    ),
            )
    }

    fn render_album_detail(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let detail = self.album_detail.as_ref().expect("album detail exists");
        let album = detail.album.clone();
        let tracks = detail.tracks.clone();
        let has_tracks = !tracks.is_empty();
        let total_minutes = album.total_duration_ms.div_ceil(60_000);
        let formats = tracks
            .iter()
            .map(|track| format_label(&track.path))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(" / ");
        let quality = quality_label(album.max_bit_depth, album.max_sample_rate_hz);

        let mut table = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .rounded(px(theme::RADIUS_MD))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_surface())
            .overflow_hidden()
            .child(render_album_track_header());
        let mut rows = div()
            .id("album-detail-track-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .track_scroll(&self.album_detail_scroll)
            .overflow_y_scroll();
        for (index, track) in tracks.into_iter().enumerate() {
            rows = rows.child(self.render_album_track_row(index, track, cx));
        }
        table = table.child(rows);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap(px(18.))
            .px(px(28.))
            .pt(px(22.))
            .pb(px(24.))
            .child(
                div()
                    .id("album-detail-back")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(17.))
                    .w(px(80.))
                    .flex_none()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.album_detail = None;
                        this.selected_album_track_id = None;
                        cx.notify();
                    }))
                    .child(
                        svg()
                            .path("icons/arrow-left.svg")
                            .size(px(15.))
                            .text_color(theme::text_muted()),
                    )
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(13.))
                            .text_color(theme::text_muted())
                            .child("Albums"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(24.))
                    .w_full()
                    .flex_none()
                    .child(
                        div()
                            .flex()
                            .size(px(190.))
                            .flex_none()
                            .overflow_hidden()
                            .rounded(px(theme::RADIUS_MD))
                            .border_1()
                            .border_color(theme::border())
                            .child(render_cover(
                                album.cover_art_path.as_deref(),
                                190.,
                                190.,
                                42.,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(8.))
                            .child(
                                div()
                                    .font_family(theme::FONT_MONO)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(10.))
                                    .text_color(theme::text_muted())
                                    .child("ALBUM"),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(38.))
                                    .text_color(theme::text_primary())
                                    .child(album.title.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .w_full()
                                    .min_w_0()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .flex_none()
                                            .font_family(theme::FONT_SANS)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_size(px(14.))
                                            .text_color(theme::text_primary())
                                            .child(album.artist.clone()),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .font_family(theme::FONT_SANS)
                                            .text_size(px(13.))
                                            .text_color(theme::text_secondary())
                                            .child(format!(
                                                "{} · {} tracks · {} min",
                                                album
                                                    .year
                                                    .map(|year| year.to_string())
                                                    .unwrap_or_else(|| "Unknown year".to_string()),
                                                album.track_count,
                                                total_minutes
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .when_some(quality.clone(), |row, quality| {
                                        row.child(render_quality_badge(format!(
                                            "{formats} {quality}"
                                        )))
                                    })
                                    .when(
                                        is_hi_res(album.max_bit_depth, album.max_sample_rate_hz),
                                        |row| row.child(render_hi_res_badge()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .child(
                                        div()
                                            .id("play-album")
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .h(px(36.))
                                            .px(px(16.))
                                            .rounded(px(theme::RADIUS_MD))
                                            .bg(theme::accent())
                                            .opacity(if has_tracks { 1.0 } else { 0.45 })
                                            .when(has_tracks, |button| {
                                                button.cursor_pointer().on_click(cx.listener(
                                                    |this, _, _, cx| {
                                                        this.play_album(cx);
                                                    },
                                                ))
                                            })
                                            .child(
                                                svg()
                                                    .path("icons/play.svg")
                                                    .size(px(15.))
                                                    .text_color(theme::bg_inset()),
                                            )
                                            .child(
                                                div()
                                                    .font_family(theme::FONT_DISPLAY)
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_size(px(14.))
                                                    .text_color(theme::bg_inset())
                                                    .child("Play Album"),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .h(px(36.))
                                            .px(px(14.))
                                            .rounded(px(theme::RADIUS_MD))
                                            .border_1()
                                            .border_color(theme::border())
                                            .bg(theme::bg_muted())
                                            .opacity(0.5)
                                            .child(
                                                svg()
                                                    .path("icons/shuffle.svg")
                                                    .size(px(15.))
                                                    .text_color(theme::text_secondary()),
                                            )
                                            .child(
                                                div()
                                                    .font_family(theme::FONT_DISPLAY)
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_size(px(14.))
                                                    .text_color(theme::text_secondary())
                                                    .child("Shuffle"),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(36.))
                                            .rounded(px(theme::RADIUS_MD))
                                            .border_1()
                                            .border_color(theme::border())
                                            .bg(theme::bg_muted())
                                            .opacity(0.5)
                                            .child(
                                                svg()
                                                    .path("icons/ellipsis.svg")
                                                    .size(px(16.))
                                                    .text_color(theme::text_secondary()),
                                            ),
                                    ),
                            ),
                    ),
            )
            .child(table)
            .into_any_element()
    }

    fn render_album_track_row(
        &self,
        index: usize,
        track: Track,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let playing = self.is_now_playing(&track.path, cx);
        let missing = self.is_track_missing(track.id, cx);
        let selected = self.selected_album_track_id == Some(track.id);
        let track_id = track.id;
        let number = track
            .track_number
            .map(|number| number.to_string())
            .unwrap_or_else(|| (index + 1).to_string());
        let quality = quality_label(track.bit_depth, track.sample_rate_hz)
            .unwrap_or_else(|| format_label(&track.path).to_string());
        div()
            .id(format!("album-track-{index}"))
            .flex()
            .items_center()
            .w_full()
            .h(px(38.))
            .flex_none()
            .border_b_1()
            .border_color(if playing {
                theme::accent()
            } else {
                theme::border()
            })
            .when(playing, |row| row.border_l_2())
            .when(selected || playing, |row| row.bg(theme::bg_selected()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                this.activate_album_track(index, event.click_count() == 2, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.selected_album_track_id = Some(track_id);
                    this.playlist_menu = None;
                    this.track_menu = Some(TrackMenu {
                        track_id,
                        surface: TrackSurface::Album,
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
                    .w(px(56.))
                    .pl(px(16.))
                    .when(playing, |cell| {
                        cell.child(
                            svg()
                                .path("icons/audio-lines.svg")
                                .size(px(13.))
                                .text_color(theme::accent()),
                        )
                    })
                    .when(!playing, |cell| {
                        cell.child(
                            div()
                                .font_family(theme::FONT_MONO)
                                .text_size(px(11.))
                                .text_color(theme::text_muted())
                                .child(number),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .w(px(700.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(theme::FONT_SANS)
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
                    .w(px(234.))
                    .truncate()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_secondary())
                    .child(track_artist(&track).to_string()),
            )
            .child(
                div()
                    .w(px(66.))
                    .font_family(theme::FONT_MONO)
                    .text_size(px(11.))
                    .text_color(theme::text_secondary())
                    .child(format_duration(track.duration_ms)),
            )
            .child(crate::components::quality_badge(quality))
    }
}

pub(super) fn render_cover(
    cover_path: Option<&Path>,
    width: f32,
    height: f32,
    icon_size: f32,
) -> gpui::Div {
    let base = div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(width))
        .h(px(height))
        .flex_none()
        .overflow_hidden()
        .rounded(px(theme::RADIUS_SM))
        .bg(theme::bg_muted());
    match cover_path {
        Some(path) => base.child(
            img(path.to_path_buf())
                .size_full()
                .object_fit(ObjectFit::Cover),
        ),
        None => base.child(
            svg()
                .path("icons/disc-3.svg")
                .size(px(icon_size))
                .text_color(theme::text_muted()),
        ),
    }
}

fn render_page_title(title: &'static str, meta: String) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::BOLD)
                .text_size(px(34.))
                .text_color(theme::text_primary())
                .child(title),
        )
        .child(
            div()
                .font_family(theme::FONT_SANS)
                .text_size(px(13.))
                .text_color(theme::text_secondary())
                .child(meta),
        )
}

fn album_grid_columns(viewport_width: f32) -> u16 {
    let content_width =
        (viewport_width - SIDEBAR_WIDTH - ALBUM_BODY_HORIZONTAL_PADDING * 2.).max(0.);
    (((content_width + ALBUM_GRID_GAP) / (ALBUM_CARD_MIN_WIDTH + ALBUM_GRID_GAP)).floor() as u16)
        .max(1)
}

fn render_no_filter_matches(message: &'static str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w_full()
        .h(px(260.))
        .font_family(theme::FONT_SANS)
        .text_size(px(13.))
        .text_color(theme::text_muted())
        .child(message)
}

fn render_quality_badge(label: String) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .h(px(21.))
        .px(px(7.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::quality_border())
        .bg(theme::quality_soft())
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(px(10.))
        .text_color(theme::quality())
        .child(label)
}

fn render_hi_res_badge() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .h(px(21.))
        .px(px(7.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::primary())
        .bg(theme::primary_soft())
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(px(10.))
        .text_color(theme::primary())
        .child("HI-RES")
}

fn render_album_track_header() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .w_full()
        .h(px(30.))
        .flex_none()
        .bg(theme::bg_surface_alt())
        .child(table_header_cell("#", 56.).pl(px(16.)))
        .child(table_header_cell("TITLE", 700.))
        .child(table_header_cell("ARTIST", 234.))
        .child(table_header_cell("TIME", 66.))
        .child(table_header_cell("FORMAT", 76.))
}

fn table_header_cell(label: &'static str, width: f32) -> gpui::Div {
    div()
        .w(px(width))
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(px(9.))
        .text_color(theme::text_muted())
        .child(label)
}

fn album_sort_label(sort: AlbumSortOrder) -> &'static str {
    match sort {
        AlbumSortOrder::Title => "TITLE",
        AlbumSortOrder::Artist => "ARTIST",
        AlbumSortOrder::DateAdded => "DATE ADDED",
        AlbumSortOrder::ReleaseYear => "YEAR",
        AlbumSortOrder::Duration => "DURATION",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ALBUM_BODY_HORIZONTAL_PADDING, ALBUM_CARD_MIN_WIDTH, ALBUM_GRID_GAP, album_grid_columns,
    };
    use crate::shell::SIDEBAR_WIDTH;

    #[test]
    fn album_grid_respects_the_card_minimum_across_window_sizes() {
        assert_eq!(album_grid_columns(1440.), 5);
        assert_eq!(album_grid_columns(1600.), 6);

        let five_column_threshold = SIDEBAR_WIDTH
            + ALBUM_BODY_HORIZONTAL_PADDING * 2.
            + ALBUM_CARD_MIN_WIDTH * 5.
            + ALBUM_GRID_GAP * 4.;
        assert_eq!(album_grid_columns(five_column_threshold - 1.), 4);
        assert_eq!(album_grid_columns(five_column_threshold), 5);
        assert_eq!(album_grid_columns(0.), 1);
    }
}
