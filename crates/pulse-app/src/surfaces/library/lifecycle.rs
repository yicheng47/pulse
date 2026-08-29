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
                    LibraryStore::open_with_progress(&database_path, move |progress| {
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
            self.selected_album_track_id = None;
        }
        self.destination = destination;
        self.track_menu = None;
        self.playlist_menu = None;
        self.genre_popover_open = false;
        self.artist_popover_open = false;
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
        let album_page = store.album_page(
            self.album_sort,
            &self.album_filter.album_query_filter(current_time_ms()),
            self.albums.len().max(LIST_PAGE_SIZE),
            0,
        )?;
        let track_page = store.track_page(
            self.track_sort,
            &self.track_filter.track_query_filter(current_time_ms()),
            self.artist_filter.as_deref(),
            self.tracks.len().max(LIST_PAGE_SIZE),
            0,
        )?;
        let genres = store.genre_album_counts()?;
        let artists = store.artists()?;
        let playlists = store.playlists()?;
        let catalog_summary = store.catalog_summary()?;
        let mut roots = Vec::new();
        for root in store.storage_roots()? {
            roots.push(StorageRootView {
                summary: store.root_summary(root.id)?,
                latest_scan: store.recent_scans(root.id, 1)?.into_iter().next(),
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
                .albums
                .iter()
                .find(|album| (album.artist.as_str(), album.title.as_str()) == (&key.0, &key.1))
                .cloned()
            {
                self.album_detail = Some(AlbumDetail {
                    tracks: store.tracks_for_album(&album.artist, &album.title)?,
                    album,
                });
            } else {
                self.album_detail = None;
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
                    entries: store.playlist_tracks(playlist_id)?,
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
                            self.reload_or_show_error();
                        }
                        Err(message) => self.boot = LibraryBoot::Failed { message },
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
                        Err(error) => self.error = Some(error.clone()),
                    }
                    self.reload_or_show_error();
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
                            self.error = None;
                            self.reload_or_show_error();
                            let reload_error = self.error.take();
                            self.error = delete_album_notice(&outcome, reload_error);
                        }
                        Err(error) => self.error = Some(error),
                    }
                    changed = true;
                }
                Ok(WorkerEvent::DeleteAlbumPanicked) => {
                    self.album_delete_in_flight = false;
                    if matches!(self.modal, Some(Modal::DeleteAlbum { .. })) {
                        self.modal = None;
                    }
                    self.error =
                        Some("The album delete crashed. Reopening the library.".to_string());
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
                    self.error =
                        Some("The library scan crashed. Reopening the library.".to_string());
                    self.begin_open_store();
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.error = Some("Library worker disconnected.".to_string());
                    changed = true;
                    break;
                }
            }
        }
        if changed {
            cx.notify();
        }
    }

    pub(super) fn reload_or_show_error(&mut self) {
        if let Err(error) = self.reload() {
            self.error = Some(error.to_string());
        }
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
            .gap(px(10.))
            .w_full()
            .child(
                div()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_size(px(20.))
                    .text_color(theme::text_primary())
                    .child("Could not open your library"),
            )
            .child(
                div()
                    .max_w(px(560.))
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::danger())
                    .child(message),
            )
            .child(
                div()
                    .mt(px(6.))
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
}
