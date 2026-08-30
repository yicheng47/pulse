use crate::theme::rpx;

use std::{ops::Range, path::Path, time::Duration};

use gpui::{
    AnyElement, Bounds, Context, ElementInputHandler, EntityInputHandler, FontWeight, IntoElement,
    KeyDownEvent, ObjectFit, Pixels, ScrollHandle, UTF16Selection, Window, canvas, deferred, div,
    img, prelude::*, svg,
};

use crate::{
    backend::{Album, PlaylistSummary, Track},
    surfaces::{
        Destination, Shell,
        library::logic::{self as view_model, SearchSelection},
    },
    text_input::{self},
    theme, ui,
};

const SEARCH_WIDTH: f32 = 420.0;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);

impl Shell {
    pub(crate) fn can_focus_search(&self) -> bool {
        self.settings_section.is_none()
    }

    pub(crate) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_input.move_to_end();
        window.focus(&self.search_focus, cx);
        if !self.search_input.text().is_empty() {
            self.search_open = true;
        }
        cx.notify();
    }

    fn handle_search_input(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "down" => {
                if self.search_open {
                    self.search.move_next();
                    self.scroll_to_search_selection();
                    cx.notify();
                }
            }
            "up" => {
                if self.search_open {
                    self.search.move_previous();
                    self.scroll_to_search_selection();
                    cx.notify();
                }
            }
            "enter" => {
                if self.search_open
                    && let Some(selection) = self.search.selected()
                {
                    self.activate_search_selection(selection, window, cx);
                }
            }
            "escape" => {
                self.search_open = false;
                self.search_input.unmark_text();
                window.blur();
                cx.notify();
            }
            _ => {
                let outcome = text_input::handle_key_down(&mut self.search_input, event, cx);
                if outcome.content_changed {
                    self.search_query_changed(cx);
                } else if outcome.handled {
                    cx.notify();
                }
            }
        }
    }

    fn search_query_changed(&mut self, cx: &mut Context<Self>) {
        self.search_revision = self.search_revision.wrapping_add(1);
        let revision = self.search_revision;
        let query = self.search_input.text().trim().to_string();
        if query.is_empty() {
            self.search.clear();
            self.search_open = false;
            self.search_loading = false;
            cx.notify();
            return;
        }
        self.search.clear();
        self.search_open = true;
        self.search_loading = true;
        self.search_scroll = ScrollHandle::new();
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                if this.search_revision != revision {
                    return;
                }
                match this.library.read(cx).search_library(&query) {
                    Ok(results) => {
                        this.search.set_results(results);
                        this.search_loading = false;
                    }
                    Err(error) => {
                        this.search.clear();
                        this.search_loading = false;
                        this.search_open = false;
                        this.library.update(cx, |library, cx| {
                            library.show_error(error.to_string(), cx);
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn scroll_to_search_selection(&self) {
        let Some(selection) = self.search.selected() else {
            return;
        };
        let child_index = match selection {
            SearchSelection::Album(index) => 1 + index,
            SearchSelection::Track(index) => 2 + self.search.results.albums.len() + index,
            SearchSelection::Playlist(index) => {
                3 + self.search.results.albums.len() + self.search.results.tracks.len() + index
            }
        };
        self.search_scroll.scroll_to_item(child_index);
    }

    fn activate_search_selection(
        &mut self,
        selection: SearchSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match selection {
            SearchSelection::Album(index) => {
                let Some(album) = self.search.results.albums.get(index).cloned() else {
                    return;
                };
                self.destination = Destination::Albums;
                self.library.update(cx, |library, cx| {
                    library.set_destination(Destination::Albums, cx);
                    library.open_search_album(album, cx);
                });
            }
            SearchSelection::Track(index) => {
                let Some(track) = self.search.results.tracks.get(index).cloned() else {
                    return;
                };
                self.library
                    .update(cx, |library, cx| library.play_search_track(track, cx));
            }
            SearchSelection::Playlist(index) => {
                let Some(playlist) = self.search.results.playlists.get(index) else {
                    return;
                };
                let playlist_id = playlist.playlist.id;
                self.destination = Destination::Playlists;
                self.library.update(cx, |library, cx| {
                    library.set_destination(Destination::Playlists, cx);
                    library.open_search_playlist(playlist_id, cx);
                });
            }
        }
        self.search_open = false;
        self.search_input.unmark_text();
        window.blur();
        cx.notify();
    }

    pub(super) fn render_search_input(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let query = self.search_input.text().to_string();
        let focused = self.search_focus.is_focused(window);
        let input_entity = cx.entity();
        let mut search = div()
            .relative()
            .occlude()
            .w(rpx(SEARCH_WIDTH))
            .h(rpx(36.))
            .flex_none()
            .track_focus(&self.search_focus)
            .on_key_down(cx.listener(|this, event, window, cx| {
                this.handle_search_input(event, window, cx);
            }))
            .child(
                div()
                    .id("library-search-input")
                    .relative()
                    .flex()
                    .items_center()
                    .gap(rpx(10.))
                    .w_full()
                    .h(rpx(36.))
                    .px(rpx(12.))
                    .rounded(rpx(theme::RADIUS_MD))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg_inset())
                    .cursor_text()
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.can_focus_search() {
                            this.focus_search(window, cx);
                        }
                    }))
                    .child(
                        svg()
                            .path("icons/search.svg")
                            .size(rpx(16.))
                            .flex_none()
                            .text_color(theme::text_muted()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_1()
                            .min_w_0()
                            .when(query.is_empty() && focused, |text| {
                                text.child(ui::input_caret())
                            })
                            .when(query.is_empty(), |text| {
                                text.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(theme::FONT_SANS)
                                        .text_size(theme::text::BODY_LARGE)
                                        .text_color(theme::text_muted())
                                        .child("Search library"),
                                )
                            })
                            .when(!query.is_empty(), |text| {
                                text.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(theme::FONT_SANS)
                                        .text_size(theme::text::BODY_LARGE)
                                        .text_color(theme::text_primary())
                                        .child(text_input::render_text(
                                            &self.search_input,
                                            focused,
                                        )),
                                )
                            }),
                    )
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, cx| {
                                let focus = input_entity.read(cx).search_focus.clone();
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
                    ),
            );
        if self.search_open {
            search = search.child(deferred(self.render_search_popover(cx)).with_priority(1));
        }
        search.into_any_element()
    }

    fn render_search_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let no_results = !self.search_loading && self.search.result_count() == 0;
        let mut content = div().flex().flex_col().w_full();

        content = content.child(search_group_header(
            "ALBUMS",
            !self.search_loading && self.search.results.albums.is_empty(),
        ));
        for (index, album) in self.search.results.albums.iter().cloned().enumerate() {
            content = content.child(self.render_search_album(index, album, cx));
        }

        content = content.child(search_group_header(
            "TRACKS",
            !self.search_loading && self.search.results.tracks.is_empty(),
        ));
        for (index, track) in self.search.results.tracks.iter().cloned().enumerate() {
            content = content.child(self.render_search_track(index, track, cx));
        }

        content = content.child(search_group_header(
            "PLAYLISTS",
            !self.search_loading && self.search.results.playlists.is_empty(),
        ));
        for (index, playlist) in self.search.results.playlists.iter().cloned().enumerate() {
            content = content.child(self.render_search_playlist(index, playlist, cx));
        }

        if no_results {
            content = content.child(
                div()
                    .w_full()
                    .px(rpx(14.))
                    .py(rpx(10.))
                    .font_family(theme::FONT_SANS)
                    .text_size(theme::text::SMALL)
                    .text_color(theme::text_secondary())
                    .child(format!(
                        "No matches for “{}”",
                        self.search_input.text().trim()
                    )),
            );
        }

        content = content.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .h(rpx(28.))
                .w_full()
                .flex_none()
                .px(rpx(14.))
                .border_t_1()
                .border_color(theme::border())
                .font_family(theme::FONT_MONO)
                .font_weight(FontWeight::BOLD)
                .text_size(theme::text::CAPTION_XS)
                .text_color(theme::text_muted())
                .child("↵ OPEN · ESC DISMISS"),
        );

        content
            .id("search-results-popover")
            .absolute()
            .left_0()
            .top(rpx(41.))
            .w_full()
            .max_h(rpx(540.))
            .overflow_y_scroll()
            .track_scroll(&self.search_scroll)
            .rounded(rpx(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::bg_surface())
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                this.search_open = false;
                this.search_input.unmark_text();
                window.blur();
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_search_album(
        &self,
        index: usize,
        album: Album,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.search.selected_index() == Some(self.search.album_index(index));
        let meta = format!(
            "{} · {} · {} tracks",
            album.artist,
            album
                .year
                .map(|year| year.to_string())
                .unwrap_or_else(|| "Unknown year".to_string()),
            album.track_count
        );
        div()
            .id(format!("search-album-{index}"))
            .flex()
            .items_center()
            .gap(rpx(10.))
            .h(rpx(46.))
            .w_full()
            .px(rpx(14.))
            .relative()
            .when(selected, |row| {
                row.bg(theme::bg_selected())
                    .child(ui::playing_row_glow())
                    .child(ui::playing_row_bar())
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_search_selection(SearchSelection::Album(index), window, cx);
            }))
            .child(search_cover(album.cover_art_path.as_deref()))
            .child(search_copy(album.title, meta))
    }

    fn render_search_track(
        &self,
        index: usize,
        track: Track,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.search.selected_index() == Some(self.search.track_index(index));
        let quality = view_model::quality_label(track.bit_depth, track.sample_rate_hz);
        div()
            .id(format!("search-track-{index}"))
            .flex()
            .items_center()
            .gap(rpx(10.))
            .h(rpx(44.))
            .w_full()
            .px(rpx(14.))
            .relative()
            .when(selected, |row| {
                row.bg(theme::bg_selected())
                    .child(ui::playing_row_glow())
                    .child(ui::playing_row_bar())
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_search_selection(SearchSelection::Track(index), window, cx);
            }))
            .child(search_copy(
                view_model::track_title(&track),
                format!(
                    "{} · {}",
                    view_model::track_artist(&track),
                    view_model::track_album(&track)
                ),
            ))
            .child(div().flex_1())
            .when_some(quality, |row, quality| row.child(ui::Badge::new(quality)))
    }

    fn render_search_playlist(
        &self,
        index: usize,
        playlist: PlaylistSummary,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.search.selected_index() == Some(self.search.playlist_index(index));
        div()
            .id(format!("search-playlist-{index}"))
            .flex()
            .items_center()
            .gap(rpx(10.))
            .h(rpx(44.))
            .w_full()
            .px(rpx(14.))
            .relative()
            .when(selected, |row| {
                row.bg(theme::bg_selected())
                    .child(ui::playing_row_glow())
                    .child(ui::playing_row_bar())
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_search_selection(SearchSelection::Playlist(index), window, cx);
            }))
            .child(search_cover(playlist.cover_art_path.as_deref()))
            .child(search_copy(
                playlist.playlist.name,
                format!("{} track entries", playlist.track_count),
            ))
    }
}

impl EntityInputHandler for Shell {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        Some(self.search_input.text_for_range(range, adjusted_range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(self.search_input.selected_text_range())
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.search_input.marked_text_range()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.search_input.unmark_text();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search_input.replace_text_in_range(range, text) {
            self.search_query_changed(cx);
        } else {
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .search_input
            .replace_and_mark_text_in_range(range, new_text, new_selected_range)
        {
            self.search_query_changed(cx);
        } else {
            cx.notify();
        }
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.search_input.character_index_utf16())
    }
}

fn search_group_header(label: &'static str, empty: bool) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .h(rpx(27.))
        .w_full()
        .px(rpx(14.))
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(theme::text::CAPTION_XS)
        .text_color(theme::text_muted())
        .child(if empty {
            format!("{label} — NO MATCHES")
        } else {
            label.to_string()
        })
}

fn search_cover(path: Option<&Path>) -> AnyElement {
    let content = match path {
        Some(path) => img(path.to_path_buf())
            .size_full()
            .object_fit(ObjectFit::Cover)
            .into_any_element(),
        None => svg()
            .path("icons/list-music.svg")
            .size(rpx(14.))
            .text_color(theme::text_muted())
            .into_any_element(),
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(28.))
        .flex_none()
        .overflow_hidden()
        .rounded(rpx(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg_muted())
        .child(content)
        .into_any_element()
}

fn search_copy(title: String, meta: String) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(rpx(1.))
        .child(
            div()
                .w_full()
                .truncate()
                .font_family(theme::FONT_SANS)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(theme::text::SMALL)
                .text_color(theme::text_primary())
                .child(title),
        )
        .child(
            div()
                .w_full()
                .truncate()
                .font_family(theme::FONT_SANS)
                .text_size(theme::text::CAPTION_XS)
                .text_color(theme::text_muted())
                .child(meta),
        )
}
