use crate::theme::rpx;

use super::*;

impl LibraryView {
    pub(super) fn begin_open_store(&mut self) {
        self.boot = LibraryBoot::Opening { backfill: None };
        let sender = self.worker_tx.clone();
        let database_path = self.database_path.clone();
        thread::Builder::new()
            .name("pulse-library-open".to_string())
            .spawn(move || {
                let progress_sender = sender.clone();
                let result = catch_unwind(AssertUnwindSafe(|| {
                    ops::open_with_progress(&database_path, move |progress| {
                        let _ = progress_sender.send(WorkerEvent::BootProgress(progress));
                    })
                    .map_err(|error| error.to_string())
                }))
                .unwrap_or_else(|_| Err("The library open worker crashed.".to_string()));
                let _ = sender.send(WorkerEvent::BootFinished(result));
            })
            .expect("failed to spawn library open worker");
    }

    pub(super) fn is_library_loading(&self) -> bool {
        matches!(self.boot, LibraryBoot::Opening { .. })
    }

    pub(super) fn store_busy_message(&self) -> String {
        if self.is_library_loading() {
            "Wait for the library to finish opening.".to_string()
        } else if self.album_delete_in_flight {
            "Wait for the album delete to finish.".to_string()
        } else {
            "Wait for the active library scan to finish.".to_string()
        }
    }

    pub(crate) fn set_destination(&mut self, destination: Destination, cx: &mut Context<Self>) {
        if destination != self.destination {
            self.album_menu_open = false;
        }
        if destination != self.destination && destination == Destination::Albums {
            self.album_detail = None;
            self.album_artist = None;
            self.selected_album_track_id = None;
        }
        if destination != self.destination && destination == Destination::Artists {
            self.album_detail = None;
            self.album_artist = None;
            self.artist_detail = None;
            self.artist_route = ArtistRoute::Index;
            self.text_input.reset("");
            self.artists_scroll = ScrollHandle::new();
            self.artist_scrollbar.update(cx, |scrollbar, _| {
                scrollbar.set_scroll_handle(self.artists_scroll.clone());
            });
        }
        self.destination = destination;
        self.track_menu = None;
        self.playlist_menu = None;
        self.genre_popover_open = false;
        self.artist_popover_open = false;
        self.persist_route(cx);
        cx.notify();
    }

    pub(crate) fn storage_root_count(&self) -> usize {
        self.roots.len()
    }

    pub(super) fn reload(&mut self) -> Result<(), LibraryError> {
        let Some(store) = self.store.as_ref() else {
            return Ok(());
        };
        // Refresh the already-loaded portions so infinite-scroll positions
        // survive reloads; at least one page always loads.
        let album_page = ops::catalog::album_page(
            store,
            self.album_sort,
            &self.album_filter.album_query_filter(current_time_ms()),
            None,
            self.albums.len().max(LIST_PAGE_SIZE),
            0,
        )?;
        let track_page = ops::catalog::track_page(
            store,
            self.track_sort,
            &self.track_filter.track_query_filter(current_time_ms()),
            self.artist_filter.as_deref(),
            self.tracks.len().max(LIST_PAGE_SIZE),
            0,
        )?;
        let genres = ops::catalog::genre_album_counts(store)?;
        let artists = ops::catalog::artist_filter_counts(store)?;
        let artist_index = ops::catalog::artist_index(store)?;
        let artist_detail = self
            .artist_route
            .artist()
            .and_then(|name| artist_index.iter().find(|artist| artist.name == name))
            .cloned()
            .map(|artist| ops::catalog::artist_detail(store, artist, self.album_sort))
            .transpose()?;
        let playlists = ops::playlists::list(store)?;
        let catalog_summary = ops::catalog::summary(store)?;
        let mut roots = Vec::new();
        for root in ops::storage::list(store)? {
            roots.push(StorageRootView {
                summary: ops::storage::summary(store, root.id)?,
                latest_scan: ops::storage::recent_scans(store, root.id, 1)?
                    .into_iter()
                    .next(),
                root,
            });
        }
        self.albums = album_page.albums;
        self.album_total = album_page.total_count;
        self.album_load_stalled = false;
        self.track_total = track_page.total_count;
        self.track_load_stalled = false;
        self.tracks = track_page.tracks;
        self.genres = genres;
        self.artists = artists;
        self.artist_index = artist_index;
        self.artist_detail = artist_detail;
        if self.artist_route.artist().is_some() && self.artist_detail.is_none() {
            self.artist_route = ArtistRoute::Index;
            self.album_detail = None;
            self.album_artist = None;
        }
        self.playlists = playlists;
        self.catalog_summary = catalog_summary;
        self.roots = roots;
        if self
            .selected_root_id
            .is_none_or(|selected| !self.roots.iter().any(|root| root.root.id == selected))
        {
            self.selected_root_id = self.roots.first().map(|root| root.root.id);
        }
        if let Some(detail) = &self.album_detail {
            let key = (detail.album.artist.clone(), detail.album.title.clone());
            if let Some(album) = self
                .artist_detail
                .as_ref()
                .into_iter()
                .flat_map(|artist| artist.albums.iter())
                .chain(self.albums.iter())
                .find(|album| (album.artist.as_str(), album.title.as_str()) == (&key.0, &key.1))
                .cloned()
            {
                let album_artist = ops::catalog::artist_by_name(store, &album.artist)?;
                self.album_detail = Some(AlbumDetail {
                    tracks: ops::catalog::album_tracks(store, &album.artist, &album.title)?,
                    album,
                });
                self.album_artist = album_artist;
            } else {
                self.album_detail = None;
                self.album_artist = None;
                if self.destination == Destination::Artists {
                    self.artist_route.back();
                }
            }
        }
        if self
            .selected_track_id
            .is_some_and(|selected| !self.tracks.iter().any(|track| track.id == selected))
        {
            self.selected_track_id = None;
        }
        if self.selected_album_track_id.is_some_and(|selected| {
            self.album_detail
                .as_ref()
                .is_none_or(|detail| !detail.tracks.iter().any(|track| track.id == selected))
        }) {
            self.selected_album_track_id = None;
        }
        if self.selected_playlist_id.is_none_or(|selected| {
            !self
                .playlists
                .iter()
                .any(|playlist| playlist.playlist.id == selected)
        }) {
            self.selected_playlist_id = self.playlists.first().map(|playlist| playlist.playlist.id);
            self.selected_playlist_position = None;
        }
        self.playlist_detail = if let Some(playlist_id) = self.selected_playlist_id {
            match self
                .playlists
                .iter()
                .find(|playlist| playlist.playlist.id == playlist_id)
                .cloned()
            {
                Some(summary) => Some(PlaylistDetail {
                    entries: ops::playlists::tracks(store, playlist_id)?,
                    summary,
                }),
                None => None,
            }
        } else {
            None
        };
        if self.selected_playlist_position.is_some_and(|selected| {
            self.playlist_detail.as_ref().is_none_or(|detail| {
                !detail
                    .entries
                    .iter()
                    .any(|entry| entry.position == selected)
            })
        }) {
            self.selected_playlist_position = None;
        }
        Ok(())
    }

    pub(super) fn drain_worker_events(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        loop {
            match self.worker_rx.try_recv() {
                Ok(WorkerEvent::BootProgress(progress)) => {
                    if let LibraryBoot::Opening { backfill } = &mut self.boot {
                        *backfill = Some(progress);
                        changed = true;
                    }
                }
                Ok(WorkerEvent::BootFinished(result)) => {
                    match result {
                        Ok(store) => {
                            self.store = Some(store);
                            self.boot = LibraryBoot::Ready;
                            self.reload_or_show_error(cx);
                            self.restore_launch_state(cx);
                        }
                        Err(message) => {
                            self.boot = LibraryBoot::Failed { message };
                            self.app_store.update(cx, |store, store_cx| {
                                store.abandon_launch_session_restore(store_cx);
                            });
                        }
                    }
                    changed = true;
                }
                Ok(WorkerEvent::ScanProgress { root_id, progress }) => {
                    if let Some(scan) = &mut self.scan
                        && scan.root_id == root_id
                    {
                        scan.progress = Some(progress);
                        changed = true;
                    }
                }
                Ok(WorkerEvent::ScanFinished {
                    root_id,
                    store,
                    result,
                }) => {
                    if self
                        .scan
                        .as_ref()
                        .is_some_and(|scan| scan.root_id == root_id)
                    {
                        self.scan = None;
                    }
                    self.store = Some(store);
                    match &result {
                        Ok(completion) if scan_verified_presence(completion) => {
                            self.app_store.update(cx, |store, store_cx| {
                                store.send_command(PlaybackAction::ClearMissingMarks, store_cx);
                            });
                        }
                        Ok(_) => {}
                        Err(error) => self.show_error(error.clone(), cx),
                    }
                    self.reload_or_show_error(cx);
                    changed = true;
                }
                Ok(WorkerEvent::DeleteAlbumFinished { store, result }) => {
                    self.store = Some(store);
                    self.album_delete_in_flight = false;
                    if matches!(self.modal, Some(Modal::DeleteAlbum { .. })) {
                        self.modal = None;
                    }
                    match result {
                        Ok(outcome) => {
                            // Marks may be dropped only for rows that left
                            // the library: with the commit failed, the files
                            // are gone but the rows — and their now-correct
                            // missing marks — remain. Ids are recyclable, so
                            // marks for committed deletions must go.
                            if outcome.db_error.is_none() && !outcome.deleted_ids.is_empty() {
                                self.app_store.update(cx, |store, store_cx| {
                                    store.send_command(
                                        PlaybackAction::RemoveMissingMarks(
                                            outcome.deleted_ids.clone(),
                                        ),
                                        store_cx,
                                    );
                                });
                            }
                            let reload_error = self.reload_error(cx);
                            if let Some(notice) = delete_album_notice(&outcome, reload_error) {
                                self.show_error(notice, cx);
                            }
                        }
                        Err(error) => self.show_error(error, cx),
                    }
                    changed = true;
                }
                Ok(WorkerEvent::DeleteAlbumPanicked) => {
                    self.album_delete_in_flight = false;
                    if matches!(self.modal, Some(Modal::DeleteAlbum { .. })) {
                        self.modal = None;
                    }
                    self.show_error(
                        "The album delete crashed. Reopening the library.".to_string(),
                        cx,
                    );
                    self.begin_open_store();
                    changed = true;
                }
                Ok(WorkerEvent::ScanPanicked { root_id }) => {
                    if self
                        .scan
                        .as_ref()
                        .is_some_and(|scan| scan.root_id == root_id)
                    {
                        self.scan = None;
                    }
                    self.show_error(
                        "The library scan crashed. Reopening the library.".to_string(),
                        cx,
                    );
                    self.begin_open_store();
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.show_error("Library worker disconnected.".to_string(), cx);
                    changed = true;
                    break;
                }
            }
        }
        if changed {
            cx.notify();
        }
    }

    pub(super) fn reload_or_show_error(&mut self, cx: &mut Context<Self>) {
        if let Some(error) = self.reload_error(cx) {
            self.show_error(error, cx);
        }
    }

    fn reload_error(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let route = self.session_route();
        let error = self.reload().err().map(|error| error.to_string());
        if self.session_route() != route {
            self.persist_route(cx);
        }
        error
    }

    pub(super) fn select_root(&mut self, root_id: StorageRootId, cx: &mut Context<Self>) {
        self.selected_root_id = Some(root_id);
        self.rename_draft = None;
        cx.notify();
    }

    pub(super) fn selected_root(&self) -> Option<&StorageRootView> {
        let selected = self.selected_root_id?;
        self.roots.iter().find(|root| root.root.id == selected)
    }

    pub(super) fn handle_text_input(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "enter" => {
                if self.genre_popover_open {
                    if let Some((genre, _)) = self.filtered_genres().into_iter().next() {
                        self.choose_genre_filter(Some(genre), cx);
                    }
                    return;
                }
                if self.artist_popover_open {
                    if let Some((artist, _)) = self.filtered_artists().into_iter().next() {
                        self.choose_artist_filter(Some(artist), cx);
                    }
                    return;
                }
                if matches!(self.modal, Some(Modal::PlaylistName { .. })) {
                    self.confirm_playlist_name(cx);
                } else if matches!(self.modal, Some(Modal::AddStorage(_))) {
                    self.confirm_add_storage(cx);
                } else if self.rename_draft.is_some() {
                    self.commit_rename_storage(cx);
                }
                return;
            }
            "escape" => {
                self.text_input.unmark_text();
                self.rename_draft = None;
                self.modal = None;
                self.genre_popover_open = false;
                self.artist_popover_open = false;
            }
            _ => {
                let outcome = text_input::handle_key_down(&mut self.text_input, event, cx);
                if !outcome.handled {
                    return;
                }
            }
        }
        cx.notify();
    }

    pub(super) fn is_now_playing(&self, path: &Path) -> bool {
        self.playback_source_path.as_deref() == Some(path)
            && matches!(
                self.playback_state,
                PlaybackState::Loading | PlaybackState::Playing | PlaybackState::Paused
            )
    }

    pub(super) fn reconcile_selection_for_playback(&mut self, new_source_path: Option<&Path>) {
        let selected_album_track_path = self.selected_album_track_id.and_then(|id| {
            self.album_detail
                .as_ref()?
                .tracks
                .iter()
                .find(|track| track.id == id)
                .map(|track| track.path.as_path())
        });
        if !view_model::selection_survives_new_playing_track(
            new_source_path,
            selected_album_track_path,
        ) {
            self.selected_album_track_id = None;
        }

        let selected_track_path = self.selected_track_id.and_then(|id| {
            self.tracks
                .iter()
                .find(|track| track.id == id)
                .map(|track| track.path.as_path())
        });
        if !view_model::selection_survives_new_playing_track(new_source_path, selected_track_path) {
            self.selected_track_id = None;
        }

        let selected_playlist_track_path = self.selected_playlist_position.and_then(|position| {
            self.playlist_detail
                .as_ref()?
                .entries
                .iter()
                .find(|entry| entry.position == position)
                .map(|entry| entry.track.path.as_path())
        });
        if !view_model::selection_survives_new_playing_track(
            new_source_path,
            selected_playlist_track_path,
        ) {
            self.selected_playlist_position = self.playlist_detail.as_ref().and_then(|detail| {
                view_model::playlist_position_for_playing_track(
                    &detail.entries,
                    self.selected_playlist_position,
                    new_source_path,
                )
            });
        }
    }

    pub(super) fn render_library_failed(
        &self,
        message: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap(rpx(10.))
            .w_full()
            .child(
                div()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_size(theme::text::HEADING_SMALL)
                    .text_color(theme::text_primary())
                    .child("Could not open your library"),
            )
            .child(
                div()
                    .max_w(rpx(560.))
                    .font_family(theme::FONT_SANS)
                    .text_size(theme::text::BODY)
                    .text_color(theme::danger())
                    .child(message),
            )
            .child(
                div()
                    .mt(rpx(6.))
                    .child(
                        Button::new("library-open-retry", "Retry").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.begin_open_store();
                                cx.notify();
                            },
                        )),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn filtered_artists(&self) -> Vec<(String, u64)> {
        filter_artists(&self.artists, self.text_input.text())
    }

    pub(super) fn filtered_genres(&self) -> Vec<(String, u64)> {
        filter_genres(&self.genres, self.text_input.text())
    }

    pub(super) fn is_track_missing(&self, track_id: TrackId) -> bool {
        self.missing_track_ids.contains(&track_id)
    }

    pub(super) fn is_dsd_unplayable(&self, track: &Track) -> bool {
        let is_dsd = track
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("dsf") || extension.eq_ignore_ascii_case("dff")
            });
        if !is_dsd {
            return false;
        }
        // Scanned DSD rows without a rate stay gated; rendering must never reopen audio files.
        let Some(sample_rate_hz) = track.sample_rate_hz else {
            return true;
        };
        dsd_playback_error_with_sample_rate(
            &track.path,
            Some(sample_rate_hz),
            self.dsd_engine_kind,
            self.dsd_device_capabilities,
        )
        .is_some()
    }
}
