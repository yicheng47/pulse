use super::*;

impl LibraryView {
    pub(super) fn select_playlist(&mut self, playlist_id: PlaylistId, cx: &mut Context<Self>) {
        if !self
            .playlists
            .iter()
            .any(|playlist| playlist.playlist.id == playlist_id)
        {
            return;
        }
        self.selected_playlist_id = Some(playlist_id);
        self.selected_playlist_position = None;
        self.reload_or_show_error(cx);
        self.persist_route(cx);
        cx.notify();
    }

    pub(super) fn begin_create_playlist(
        &mut self,
        add_track_id: Option<TrackId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.track_menu = None;
        self.playlist_menu = None;
        self.text_input.reset("");
        self.modal = Some(Modal::PlaylistName {
            mode: PlaylistNameMode::Create { add_track_id },
        });
        window.focus(&self.input_focus, cx);
        cx.notify();
    }

    pub(super) fn begin_rename_playlist(
        &mut self,
        playlist_id: PlaylistId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(playlist) = self
            .playlists
            .iter()
            .find(|playlist| playlist.playlist.id == playlist_id)
        else {
            return;
        };
        let name = playlist.playlist.name.clone();
        self.track_menu = None;
        self.playlist_menu = None;
        self.text_input.reset(name);
        self.modal = Some(Modal::PlaylistName {
            mode: PlaylistNameMode::Rename { playlist_id },
        });
        window.focus(&self.input_focus, cx);
        cx.notify();
    }

    pub(super) fn confirm_playlist_name(&mut self, cx: &mut Context<Self>) {
        let Some(Modal::PlaylistName { mode }) = self.modal.take() else {
            return;
        };
        let name = self.text_input.text().trim().to_string();
        if name.is_empty() {
            cx.notify();
            return;
        }
        let Some(store) = self.store.as_mut() else {
            self.show_error(self.store_busy_message(), cx);
            self.modal = Some(Modal::PlaylistName { mode });
            cx.notify();
            return;
        };
        let result = match mode {
            PlaylistNameMode::Create { add_track_id } => {
                ops::playlists::create(store, &name, add_track_id.as_slice())
                    .map(|playlist| playlist.id)
            }
            PlaylistNameMode::Rename { playlist_id } => {
                ops::playlists::rename(store, playlist_id, &name).map(|playlist| playlist.id)
            }
        };
        match result {
            Ok(playlist_id) => {
                self.selected_playlist_id = Some(playlist_id);
                self.selected_playlist_position = None;
                self.reload_or_show_error(cx);
                self.persist_route(cx);
            }
            Err(error) => {
                self.show_error(error.to_string(), cx);
                self.modal = Some(Modal::PlaylistName { mode });
            }
        }
        cx.notify();
    }

    pub(super) fn request_delete_playlist(
        &mut self,
        playlist_id: PlaylistId,
        cx: &mut Context<Self>,
    ) {
        let Some(playlist) = self
            .playlists
            .iter()
            .find(|playlist| playlist.playlist.id == playlist_id)
        else {
            return;
        };
        self.playlist_menu = None;
        self.modal = Some(Modal::DeletePlaylist {
            playlist_id,
            name: playlist.playlist.name.clone(),
            entry_count: playlist.track_count,
        });
        cx.notify();
    }

    pub(super) fn confirm_delete_playlist(&mut self, cx: &mut Context<Self>) {
        let Some(Modal::DeletePlaylist {
            playlist_id,
            name,
            entry_count,
        }) = self.modal.take()
        else {
            return;
        };
        let Some(store) = self.store.as_mut() else {
            self.show_error(self.store_busy_message(), cx);
            self.modal = Some(Modal::DeletePlaylist {
                playlist_id,
                name,
                entry_count,
            });
            cx.notify();
            return;
        };
        match ops::playlists::delete(store, playlist_id) {
            Ok(()) => {
                if self.selected_playlist_id == Some(playlist_id) {
                    self.selected_playlist_id = None;
                    self.selected_playlist_position = None;
                }
                self.reload_or_show_error(cx);
                self.persist_route(cx);
            }
            Err(error) => self.show_error(error.to_string(), cx),
        }
        cx.notify();
    }

    pub(super) fn request_delete_album(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &self.album_detail else {
            return;
        };
        self.album_menu_open = false;
        self.modal = Some(Modal::DeleteAlbum {
            album: detail.album.clone(),
        });
        cx.notify();
    }

    /// Spawns the delete worker. The modal stays open in an in-flight state
    /// until the worker reports back; the store moves into the worker, so a
    /// scan or second delete cannot start while the job runs. All file I/O
    /// happens off the UI thread — album audio can live on a slow NAS.
    pub(super) fn confirm_delete_album(&mut self, cx: &mut Context<Self>) {
        if self.album_delete_in_flight {
            return;
        }
        let Some(Modal::DeleteAlbum { album }) = self.modal.as_ref() else {
            return;
        };
        let album = album.clone();
        let Some(mut store) = self.store.take() else {
            self.show_error(self.store_busy_message(), cx);
            cx.notify();
            return;
        };
        self.album_delete_in_flight = true;
        let sender = self.worker_tx.clone();
        thread::Builder::new()
            .name("pulse-album-delete".to_string())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    let result = ops::delete::album_tracks(&mut store, &album.artist, &album.title)
                        .map_err(|error| error.to_string());
                    (store, result)
                }));
                let _ = match outcome {
                    Ok((store, result)) => {
                        sender.send(WorkerEvent::DeleteAlbumFinished { store, result })
                    }
                    // The store was consumed by the unwind; the UI reopens it.
                    Err(_) => sender.send(WorkerEvent::DeleteAlbumPanicked),
                };
            })
            .expect("failed to spawn album delete worker");
        cx.notify();
    }

    pub(super) fn add_track_to_playlist(
        &mut self,
        playlist_id: PlaylistId,
        track_id: TrackId,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.store.as_mut() else {
            self.show_error(self.store_busy_message(), cx);
            cx.notify();
            return;
        };
        if let Err(error) = ops::playlists::append_tracks(store, playlist_id, &[track_id]) {
            self.show_error(error.to_string(), cx);
        } else {
            self.reload_or_show_error(cx);
        }
        self.track_menu = None;
        cx.notify();
    }

    pub(super) fn remove_playlist_entry(
        &mut self,
        playlist_id: PlaylistId,
        position: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.store.as_mut() else {
            self.show_error(self.store_busy_message(), cx);
            cx.notify();
            return;
        };
        if let Err(error) = ops::playlists::remove_entry(store, playlist_id, position) {
            self.show_error(error.to_string(), cx);
        } else {
            self.selected_playlist_position = None;
            self.reload_or_show_error(cx);
        }
        self.track_menu = None;
        cx.notify();
    }

    pub(super) fn move_playlist_entry(
        &mut self,
        playlist_id: PlaylistId,
        from_position: usize,
        to_position: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.store.as_mut() else {
            self.show_error(self.store_busy_message(), cx);
            cx.notify();
            return;
        };
        if let Err(error) =
            ops::playlists::move_entry(store, playlist_id, from_position, to_position)
        {
            self.show_error(error.to_string(), cx);
        } else {
            self.selected_playlist_position = Some(to_position);
            self.reload_or_show_error(cx);
        }
        self.track_menu = None;
        cx.notify();
    }
}
