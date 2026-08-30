use crate::theme::rpx;

use gpui::{
    AnyElement, Context, ElementInputHandler, FontWeight, IntoElement, ObjectFit, Window, canvas,
    div, img, prelude::*, svg,
};

use super::{
    LibraryView,
    artists_logic::{
        ARTIST_BODY_HORIZONTAL_PADDING, ARTIST_GRID_GAP, ArtistArtwork, ArtistRoute,
        artist_grid_columns, filter_artist_index, format_artist_count,
    },
};
use crate::{backend::Artist, text_input, theme};

const ARTIST_AVATAR_SIZE: f32 = 180.;

impl LibraryView {
    pub(super) fn render_artists(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if matches!(self.artist_route, ArtistRoute::Album { .. }) && self.album_detail.is_none() {
            self.artist_route.back();
        }
        if matches!(self.artist_route, ArtistRoute::Detail { .. }) && self.artist_detail.is_none() {
            self.artist_route = ArtistRoute::Index;
        }
        match self.artist_route {
            ArtistRoute::Index => self.render_artist_index(window, cx),
            ArtistRoute::Detail { .. } => self.render_artist_detail(window, cx),
            ArtistRoute::Album { .. } => self.render_album_detail(window, cx),
        }
    }

    fn render_artist_index(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let visible = filter_artist_index(&self.artist_index, self.text_input.text())
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let scale = theme::ui_scale(window.rem_size());
        let columns = artist_grid_columns(f32::from(window.viewport_size().width), scale);
        let row_count = visible.len().div_ceil(columns as usize);
        let no_matches = visible.is_empty();
        self.artist_scrollbar.update(cx, |scrollbar, _| {
            scrollbar.set_estimated_content_height(
                rpx(row_count as f32 * 252.).to_pixels(window.rem_size()),
            );
        });
        let meta = if self.artist_index.is_empty() && self.is_library_loading() {
            "Loading…".to_string()
        } else if self.artist_index.is_empty() {
            "No artists yet".to_string()
        } else {
            format!(
                "{} artists · {} albums · {} tracks",
                self.artist_index.len(),
                self.catalog_summary.album_count,
                self.catalog_summary.track_count
            )
        };

        let mut grid = div()
            .grid()
            .grid_cols(columns)
            .gap(rpx(ARTIST_GRID_GAP))
            .w_full()
            .pb(rpx(24.));
        for (index, artist) in visible.into_iter().enumerate() {
            grid = grid.child(self.render_artist_card(index, artist, cx));
        }

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .px(rpx(ARTIST_BODY_HORIZONTAL_PADDING))
            .pt(rpx(26.))
            .pb(rpx(24.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .w_full()
                    .min_h(rpx(63.))
                    .pb(rpx(4.))
                    .flex_none()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(theme::text::DISPLAY)
                                    .text_color(theme::text_primary())
                                    .child("Artists"),
                            )
                            .child(
                                div()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(theme::text::BODY_LARGE)
                                    .text_color(theme::text_secondary())
                                    .child(meta),
                            ),
                    ),
            )
            .child(
                div()
                    .w(rpx(280.))
                    .h(rpx(47.))
                    .flex_none()
                    .child(self.render_artist_index_filter(window, cx)),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("artists-grid-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.artists_scroll)
                            .child(if self.artist_index.is_empty() {
                                super::list_loading_placeholder(if self.is_library_loading() {
                                    "Loading artists…"
                                } else {
                                    "No artists yet"
                                })
                            } else if no_matches {
                                render_no_artist_matches()
                            } else {
                                grid.into_any_element()
                            }),
                    )
                    .child(self.artist_scrollbar.clone()),
            )
            .into_any_element()
    }

    fn render_artist_index_filter(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let empty = self.text_input.text().is_empty();
        let focused = self.input_focus.is_focused(window);
        let input_entity = cx.entity();
        div()
            .id("artist-index-filter")
            .relative()
            .flex()
            .items_center()
            .gap(rpx(8.))
            .cursor_text()
            .h(rpx(36.))
            .w_full()
            .px(rpx(10.))
            .rounded(rpx(theme::RADIUS_SM))
            .border_1()
            .border_color(if focused {
                theme::accent()
            } else {
                theme::border_strong()
            })
            .bg(theme::bg_inset())
            .track_focus(&self.input_focus)
            .on_click(cx.listener(|this, _, window, cx| {
                window.focus(&this.input_focus, cx);
            }))
            .on_key_down(cx.listener(|this, event, _, cx| {
                this.handle_text_input(event, cx);
            }))
            .child(
                svg()
                    .path("icons/search.svg")
                    .size(rpx(14.))
                    .text_color(theme::text_muted()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme::FONT_SANS)
                    .text_size(theme::text::BODY)
                    .text_color(if empty {
                        theme::text_muted()
                    } else {
                        theme::text_primary()
                    })
                    .when(empty, |text| text.child("Filter artists"))
                    .when(!empty, |text| {
                        text.child(text_input::render_text(&self.text_input, focused))
                    }),
            )
            .when(empty && focused, |input| {
                input.child(crate::ui::input_caret())
            })
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let focus = input_entity.read(cx).input_focus.clone();
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, input_entity.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .left_0(),
            )
    }

    fn render_artist_card(
        &self,
        index: usize,
        artist: Artist,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let artwork = artist.cover_art_path.clone();
        let name = artist.name.clone();
        let meta = format!(
            "{} · {}",
            format_artist_count(artist.album_count, "album"),
            format_artist_count(artist.track_count, "track")
        );
        div()
            .id(("artist-card", index))
            .flex()
            .flex_col()
            .items_center()
            .gap(rpx(12.))
            .w_full()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_artist(artist.clone(), cx);
            }))
            .child(render_artist_avatar(
                ArtistArtwork {
                    photo: None,
                    album_cover: artwork.as_deref(),
                },
                ARTIST_AVATAR_SIZE,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(rpx(4.))
                    .w_full()
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_center()
                            .font_family(theme::FONT_SANS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(theme::text::LABEL)
                            .text_color(theme::text_primary())
                            .child(name),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_center()
                            .font_family(theme::FONT_SANS)
                            .text_size(theme::text::BODY)
                            .text_color(theme::text_secondary())
                            .child(meta),
                    ),
            )
    }
}

pub(super) fn render_artist_avatar(artwork: ArtistArtwork<'_>, size: f32) -> gpui::Div {
    let base = div()
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(size))
        .flex_none()
        .overflow_hidden()
        .rounded(rpx(size / 2.))
        .border_1();
    match artwork.path() {
        Some(path) => base.border_color(theme::border_strong()).child(
            img(path.to_path_buf())
                .size_full()
                .object_fit(ObjectFit::Cover)
                .rounded(rpx(size / 2.)),
        ),
        None => base
            .border_color(theme::border())
            .bg(theme::bg_muted())
            .child(
                svg()
                    .path("icons/user.svg")
                    .size(rpx(size * 0.4))
                    .text_color(theme::text_muted()),
            ),
    }
}

fn render_no_artist_matches() -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w_full()
        .h(rpx(260.))
        .font_family(theme::FONT_SANS)
        .text_size(theme::text::BODY_LARGE)
        .text_color(theme::text_muted())
        .child("No artists match this filter")
        .into_any_element()
}
