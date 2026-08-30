use crate::theme::rpx;

use gpui::{AnyElement, Context, FontWeight, IntoElement, Window, div, prelude::*, svg};

use super::{
    LibraryView,
    albums_logic::{ALBUM_GRID_GAP, album_grid_columns},
    artists::render_artist_avatar,
    artists_logic::{ArtistArtwork, format_artist_count, format_artist_duration},
};
use crate::{theme, ui};

impl LibraryView {
    pub(super) fn render_artist_detail(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let detail = self.artist_detail.as_ref().expect("artist detail exists");
        let artist = detail.artist.clone();
        let albums = detail.albums.clone();
        let has_tracks = !detail.tracks.is_empty();
        let scale = theme::ui_scale(window.rem_size());
        let columns = album_grid_columns(f32::from(window.viewport_size().width), scale);
        let count_meta = format!(
            "{} · {}",
            format_artist_count(artist.album_count, "album"),
            format_artist_count(artist.track_count, "track")
        );
        let added = artist
            .earliest_added_year
            .map(|year| format!("added {year}"))
            .unwrap_or_else(|| "added —".to_string());
        let duration_meta = format!(
            "{} · {added}",
            format_artist_duration(artist.total_duration_ms)
        );
        let artwork = artist.cover_art_path.clone();

        let mut album_grid = div()
            .grid()
            .grid_cols(columns)
            .gap(rpx(ALBUM_GRID_GAP))
            .w_full()
            .pb(rpx(24.));
        for (index, album) in albums.into_iter().enumerate() {
            album_grid = album_grid.child(self.render_album_card(index, album, cx));
        }

        div()
            .id("artist-detail-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap(rpx(18.))
            .px(rpx(28.))
            .pt(rpx(22.))
            .pb(rpx(24.))
            .overflow_y_scroll()
            .track_scroll(&self.artist_detail_scroll)
            .child(
                div()
                    .id("artist-detail-back")
                    .flex()
                    .items_center()
                    .gap(rpx(8.))
                    .h(rpx(17.))
                    .w(rpx(80.))
                    .flex_none()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.artist_route.back();
                        this.artist_detail = None;
                        this.persist_route(cx);
                        cx.notify();
                    }))
                    .child(
                        svg()
                            .path("icons/arrow-left.svg")
                            .size(rpx(15.))
                            .text_color(theme::text_muted()),
                    )
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::BOLD)
                            .text_size(theme::text::BODY_LARGE)
                            .text_color(theme::text_muted())
                            .child("Artists"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(rpx(24.))
                    .w_full()
                    .flex_none()
                    .child(render_artist_avatar(
                        ArtistArtwork {
                            photo: None,
                            album_cover: artwork.as_deref(),
                        },
                        190.,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(rpx(6.))
                            .child(
                                div()
                                    .font_family(theme::FONT_MONO)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(theme::text::CAPTION)
                                    .text_color(theme::text_muted())
                                    .child("ARTIST"),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(theme::text::DISPLAY_LARGE)
                                    .line_height(rpx(42.))
                                    .text_color(theme::text_primary())
                                    .child(artist.name.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(rpx(8.))
                                    .child(
                                        div()
                                            .font_family(theme::FONT_SANS)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_size(theme::text::LABEL)
                                            .text_color(theme::text_primary())
                                            .child(count_meta),
                                    )
                                    .child(
                                        div()
                                            .font_family(theme::FONT_SANS)
                                            .text_size(theme::text::BODY_LARGE)
                                            .text_color(theme::text_secondary())
                                            .child(duration_meta),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(rpx(10.))
                                    .mt(rpx(10.))
                                    .child(
                                        div()
                                            .id("play-artist")
                                            .flex()
                                            .items_center()
                                            .gap(rpx(8.))
                                            .h(rpx(36.))
                                            .px(rpx(16.))
                                            .rounded(rpx(theme::RADIUS_MD))
                                            .bg(theme::accent())
                                            .opacity(if has_tracks { 1.0 } else { 0.45 })
                                            .when(has_tracks, |button| {
                                                button.cursor_pointer().on_click(cx.listener(
                                                    |this, _, _, cx| {
                                                        this.play_artist(cx);
                                                    },
                                                ))
                                            })
                                            .child(
                                                svg()
                                                    .path("icons/play.svg")
                                                    .size(rpx(15.))
                                                    .text_color(theme::bg_inset()),
                                            )
                                            .child(
                                                div()
                                                    .font_family(theme::FONT_DISPLAY)
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_size(theme::text::LABEL)
                                                    .text_color(theme::bg_inset())
                                                    .child("Play all"),
                                            ),
                                    )
                                    .child(
                                        ui::Button::new("shuffle-artist", "Shuffle")
                                            .icon("icons/shuffle.svg")
                                            .corner_radius(theme::RADIUS_MD)
                                            .disabled(!has_tracks)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.shuffle_artist(cx);
                                            })),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rpx(12.))
                    .w_full()
                    .pt(rpx(6.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .w_full()
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(theme::text::HEADING)
                                    .text_color(theme::text_primary())
                                    .child("Albums"),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_MONO)
                                    .text_size(theme::text::SMALL)
                                    .text_color(theme::text_muted())
                                    .child(format_artist_count(artist.album_count, "album")),
                            ),
                    )
                    .child(album_grid),
            )
            .into_any_element()
    }
}
