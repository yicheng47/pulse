use super::*;

impl LibraryView {
    pub(super) fn cycle_album_sort(&mut self, cx: &mut Context<Self>) {
        self.album_sort = match self.album_sort {
            AlbumSortOrder::Title => AlbumSortOrder::Artist,
            AlbumSortOrder::Artist => AlbumSortOrder::DateAdded,
            AlbumSortOrder::DateAdded => AlbumSortOrder::ReleaseYear,
            AlbumSortOrder::ReleaseYear => AlbumSortOrder::Duration,
            AlbumSortOrder::Duration => AlbumSortOrder::Title,
        };
        self.reset_albums();
        self.reload_or_show_error();
        cx.notify();
    }

    pub(super) fn reset_albums(&mut self) {
        self.albums.clear();
        self.album_total = 0;
        self.album_load_stalled = false;
        self.albums_scroll = ScrollHandle::new();
    }

    /// The album grid loads more when scrolled within a prefetch margin of
    /// the bottom (or when the loaded set does not fill the viewport yet).
    pub(super) fn should_load_more_albums(&self) -> bool {
        if self.store.is_none() || self.album_load_stalled || self.albums.len() >= self.album_total
        {
            return false;
        }
        let offset = self.albums_scroll.offset();
        let max_offset = self.albums_scroll.max_offset();
        -offset.y >= max_offset.y - px(ALBUM_PREFETCH_PX)
    }

    /// Returns true when a page was appended; false on failure (which latches
    /// the stall flag) or with no store.
    pub(super) fn load_more_albums(&mut self) -> bool {
        let Some(store) = self.store.as_ref() else {
            return false;
        };
        match store.album_page(
            self.album_sort,
            &self.album_filter.album_query_filter(current_time_ms()),
            LIST_PAGE_SIZE,
            self.albums.len(),
        ) {
            Ok(page) => {
                self.album_total = page.total_count;
                self.albums.extend(page.albums);
                true
            }
            Err(error) => {
                self.album_load_stalled = true;
                self.error = Some(error.to_string());
                false
            }
        }
    }

    pub(super) fn cycle_track_sort(&mut self, cx: &mut Context<Self>) {
        self.track_sort = match self.track_sort {
            TrackSortOrder::Title => TrackSortOrder::Artist,
            TrackSortOrder::Artist => TrackSortOrder::Album,
            TrackSortOrder::Album => TrackSortOrder::DateAdded,
            TrackSortOrder::DateAdded => TrackSortOrder::ReleaseYear,
            TrackSortOrder::ReleaseYear => TrackSortOrder::Duration,
            TrackSortOrder::Duration => TrackSortOrder::Title,
        };
        self.reset_tracks(cx);
        self.reload_or_show_error();
        cx.notify();
    }

    pub(super) fn set_track_filter(&mut self, filter: FilterChip, cx: &mut Context<Self>) {
        if self.track_filter == filter {
            return;
        }
        self.track_filter = filter;
        self.reset_tracks(cx);
        self.reload_or_show_error();
        cx.notify();
    }

    pub(super) fn set_album_filter(&mut self, filter: FilterChip, cx: &mut Context<Self>) {
        if self.album_filter == filter {
            return;
        }
        self.album_filter = filter;
        self.reset_albums();
        self.reload_or_show_error();
        cx.notify();
    }

    pub(super) fn choose_genre_filter(&mut self, genre: Option<String>, cx: &mut Context<Self>) {
        self.genre_popover_open = false;
        cx.notify();
        let filter = genre.map_or(FilterChip::All, FilterChip::Genre);
        match self.destination {
            Destination::Albums => self.set_album_filter(filter, cx),
            Destination::Tracks => self.set_track_filter(filter, cx),
            _ => {}
        }
    }

    pub(super) fn set_artist_filter(&mut self, artist: Option<String>, cx: &mut Context<Self>) {
        if self.artist_filter == artist {
            return;
        }
        self.artist_filter = artist;
        self.reset_tracks(cx);
        self.reload_or_show_error();
        cx.notify();
    }

    /// Closes the popover before delegating, notifying unconditionally —
    /// `set_artist_filter` early-returns without a repaint when the value is
    /// unchanged, which must not leave a dismissed popover on screen.
    pub(super) fn choose_artist_filter(&mut self, artist: Option<String>, cx: &mut Context<Self>) {
        self.artist_popover_open = false;
        cx.notify();
        self.set_artist_filter(artist, cx);
    }

    pub(super) fn should_load_more_tracks(&self) -> bool {
        if self.store.is_none() || self.track_load_stalled || self.tracks.len() >= self.track_total
        {
            return false;
        }
        let offset = self.tracks_scroll.offset();
        let max_offset = self.tracks_scroll.max_offset();
        -offset.y >= max_offset.y - px(TRACK_PREFETCH_PX)
    }

    pub(super) fn load_more_tracks(&mut self) -> bool {
        let Some(store) = self.store.as_ref() else {
            return false;
        };
        match store.track_page(
            self.track_sort,
            &self.track_filter.track_query_filter(current_time_ms()),
            self.artist_filter.as_deref(),
            LIST_PAGE_SIZE,
            self.tracks.len(),
        ) {
            Ok(page) => {
                self.track_total = page.total_count;
                self.tracks.extend(page.tracks);
                true
            }
            Err(error) => {
                self.track_load_stalled = true;
                self.error = Some(error.to_string());
                false
            }
        }
    }

    pub(super) fn reset_tracks(&mut self, cx: &mut Context<Self>) {
        self.tracks.clear();
        self.track_total = 0;
        self.track_load_stalled = false;
        self.tracks_scroll = ScrollHandle::new();
        self.track_scrollbar.update(cx, |scrollbar, _| {
            scrollbar.set_scroll_handle(self.tracks_scroll.clone());
        });
    }

    pub(super) fn open_album(&mut self, album: Album, cx: &mut Context<Self>) {
        let tracks = if let Some(store) = &self.store {
            match store.tracks_for_album(&album.artist, &album.title) {
                Ok(tracks) => tracks,
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            let mut tracks = self
                .tracks
                .iter()
                .filter(|track| {
                    view_model::track_album(track) == album.title
                        && track
                            .album_artist
                            .as_deref()
                            .map(str::trim)
                            .filter(|artist| !artist.trim().is_empty())
                            .unwrap_or_else(|| view_model::track_artist(track))
                            == album.artist
                })
                .cloned()
                .collect::<Vec<_>>();
            tracks.sort_by_key(|track| {
                (
                    track.disc_number.unwrap_or(1),
                    track.track_number.unwrap_or(u32::MAX),
                    track.path.clone(),
                )
            });
            tracks
        };
        self.album_detail_scroll = ScrollHandle::new();
        self.album_detail = Some(AlbumDetail { album, tracks });
        self.selected_album_track_id = None;
        self.album_menu_open = false;
        cx.notify();
    }
}
