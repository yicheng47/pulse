use super::*;

impl LibraryView {
    pub(super) fn play_tracks(&mut self, tracks: Vec<Track>, index: usize, cx: &mut Context<Self>) {
        self.app_store.update(cx, |store, store_cx| {
            store.send_command(
                PlaybackAction::PlayLibraryTracks {
                    tracks,
                    start_index: index,
                },
                store_cx,
            );
        });
    }

    pub(super) fn select_tracks(
        &mut self,
        tracks: Vec<Track>,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        self.app_store.update(cx, |store, store_cx| {
            store.send_command(
                PlaybackAction::SelectLibraryTracks {
                    tracks,
                    start_index: index,
                },
                store_cx,
            );
        });
    }

    pub(super) fn activate_album_track(
        &mut self,
        index: usize,
        play: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = &self.album_detail else {
            return;
        };
        let Some(track) = detail.tracks.get(index) else {
            return;
        };
        self.selected_album_track_id = Some(track.id);
        let tracks = detail.tracks.clone();
        if play {
            self.play_tracks(tracks, index, cx);
        } else {
            self.select_tracks(tracks, index, cx);
        }
        cx.notify();
    }

    pub(super) fn matching_tracks(&self) -> Result<Vec<Track>, LibraryError> {
        let fallback_store = if self.store.is_none() {
            Some(LibraryStore::open(&self.database_path)?)
        } else {
            None
        };
        let store = self
            .store
            .as_ref()
            .or(fallback_store.as_ref())
            .expect("library store is available");
        store.matching_tracks(
            self.track_sort,
            &self.track_filter.track_query_filter(current_time_ms()),
            self.artist_filter.as_deref(),
        )
    }

    pub(super) fn activate_visible_track(
        &mut self,
        track_id: TrackId,
        play: bool,
        cx: &mut Context<Self>,
    ) {
        let tracks = match self.matching_tracks() {
            Ok(tracks) => tracks,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let Some(index) = tracks.iter().position(|track| track.id == track_id) else {
            return;
        };
        self.selected_track_id = Some(track_id);
        if play {
            self.play_tracks(tracks, index, cx);
        } else {
            self.select_tracks(tracks, index, cx);
        }
        cx.notify();
    }

    pub(super) fn activate_playlist_entry(
        &mut self,
        position: usize,
        play: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = &self.playlist_detail else {
            return;
        };
        let tracks = detail
            .entries
            .iter()
            .map(|entry| entry.track.clone())
            .collect::<Vec<_>>();
        let Some(index) = detail
            .entries
            .iter()
            .position(|entry| entry.position == position)
        else {
            return;
        };
        self.selected_playlist_position = Some(position);
        if play {
            self.play_tracks(tracks, index, cx);
        } else {
            self.select_tracks(tracks, index, cx);
        }
        cx.notify();
    }

    pub(super) fn play_album(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &self.album_detail else {
            return;
        };
        if !detail.tracks.is_empty() {
            self.play_tracks(detail.tracks.clone(), 0, cx);
        }
    }

    pub(super) fn play_artist(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &self.artist_detail else {
            return;
        };
        if !detail.tracks.is_empty() {
            self.play_tracks(detail.tracks.clone(), 0, cx);
        }
    }

    pub(super) fn shuffle_artist(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &self.artist_detail else {
            return;
        };
        if detail.tracks.is_empty() {
            return;
        }
        let tracks = detail.tracks.clone();
        self.app_store.update(cx, |store, store_cx| {
            store.send_command(PlaybackAction::PlayLibraryTracksShuffled(tracks), store_cx);
        });
    }

    pub(super) fn play_playlist(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &self.playlist_detail else {
            return;
        };
        if detail.entries.is_empty() {
            return;
        }
        let index = self
            .selected_playlist_position
            .and_then(|selected| {
                detail
                    .entries
                    .iter()
                    .position(|entry| entry.position == selected)
            })
            .unwrap_or(0);
        let tracks = detail
            .entries
            .iter()
            .map(|entry| entry.track.clone())
            .collect();
        self.play_tracks(tracks, index, cx);
    }

    pub(super) fn shuffle_playlist(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &self.playlist_detail else {
            return;
        };
        if detail.entries.is_empty() {
            return;
        }
        let tracks = detail
            .entries
            .iter()
            .map(|entry| entry.track.clone())
            .collect::<Vec<_>>();
        self.app_store.update(cx, |store, store_cx| {
            store.send_command(PlaybackAction::PlayLibraryTracksShuffled(tracks), store_cx);
        });
    }

    pub(crate) fn search_library(&self, query: &str) -> Result<LibrarySearchResults, LibraryError> {
        match &self.store {
            Some(store) => store.search(query),
            None => Ok(LibrarySearchResults::default()),
        }
    }

    pub(crate) fn show_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        cx.notify();
    }

    pub(crate) fn open_search_album(&mut self, album: Album, cx: &mut Context<Self>) {
        self.open_album(album, cx);
    }

    pub(crate) fn play_search_track(&mut self, track: Track, cx: &mut Context<Self>) {
        let album_artist = track
            .album_artist
            .as_deref()
            .map(str::trim)
            .filter(|artist| !artist.is_empty())
            .unwrap_or_else(|| view_model::track_artist(&track))
            .to_string();
        let album = view_model::track_album(&track).to_string();
        let tracks = if let Some(store) = &self.store {
            match store.tracks_for_album(&album_artist, &album) {
                Ok(tracks) => tracks,
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            self.tracks
                .iter()
                .filter(|candidate| {
                    view_model::track_album(candidate) == album
                        && candidate
                            .album_artist
                            .as_deref()
                            .map(str::trim)
                            .filter(|artist| !artist.is_empty())
                            .unwrap_or_else(|| view_model::track_artist(candidate))
                            == album_artist
                })
                .cloned()
                .collect()
        };
        let index = tracks
            .iter()
            .position(|candidate| candidate.id == track.id)
            .unwrap_or(0);
        self.play_tracks(tracks, index, cx);
    }

    pub(crate) fn open_search_playlist(&mut self, playlist_id: PlaylistId, cx: &mut Context<Self>) {
        self.select_playlist(playlist_id, cx);
    }
}
