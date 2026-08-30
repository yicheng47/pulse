use super::*;

enum RestoredRoute {
    Albums(Option<(Album, Vec<Track>)>),
    Artists {
        detail: Option<Box<ArtistDetail>>,
        album: Option<(Album, Vec<Track>)>,
    },
    Tracks,
    Playlists(Option<(PlaylistSummary, Vec<PlaylistTrack>)>),
    Storage,
    Devices,
}

impl LibraryView {
    pub(crate) fn destination(&self) -> Destination {
        self.destination
    }

    pub(super) fn restore_launch_state(&mut self, cx: &mut Context<Self>) {
        if self.launch_state_restored {
            return;
        }
        let Some(session) = self.app_store.read(cx).launch_session() else {
            self.app_store.update(cx, |store, store_cx| {
                store.abandon_launch_session_restore(store_cx);
            });
            return;
        };
        let (route, tracks) = {
            let Some(store) = self.store.as_ref() else {
                self.app_store.update(cx, |store, store_cx| {
                    store.abandon_launch_session_restore(store_cx);
                });
                return;
            };
            (
                resolve_session_route(store, &session.route, self.album_sort),
                ops::catalog::tracks_by_ids(store, &session.queue_track_ids),
            )
        };
        self.launch_state_restored = true;
        let restore_devices = matches!(&route, Ok(RestoredRoute::Devices));

        match route {
            Ok(route) => self.apply_restored_route(route),
            Err(error) => {
                self.apply_restored_route(RestoredRoute::Albums(None));
                self.error = Some(error.to_string());
            }
        }
        if restore_devices {
            self.app_store.update(cx, |store, store_cx| {
                store.send_command(PlaybackAction::RefreshOutputDevices, store_cx);
            });
        }

        match tracks {
            Ok(tracks) => {
                self.app_store.update(cx, |store, store_cx| {
                    store.restore_session(&session, tracks, store_cx);
                });
                self.persist_route(cx);
            }
            Err(error) => {
                self.app_store.update(cx, |store, store_cx| {
                    store.abandon_launch_session_restore(store_cx);
                });
                self.error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    pub(super) fn persist_route(&mut self, cx: &mut Context<Self>) {
        let route = self.session_route();
        self.app_store.update(cx, |store, store_cx| {
            store.set_session_route(route, store_cx);
        });
    }

    pub(super) fn session_route(&self) -> SessionRoute {
        match self.destination {
            Destination::Albums => SessionRoute::Albums {
                album: self.album_detail.as_ref().map(|detail| SessionAlbumKey {
                    artist: detail.album.artist.clone(),
                    title: detail.album.title.clone(),
                }),
            },
            Destination::Artists => match &self.artist_route {
                ArtistRoute::Index => SessionRoute::Artists {
                    artist: None,
                    album: None,
                },
                ArtistRoute::Detail { artist } => SessionRoute::Artists {
                    artist: Some(artist.clone()),
                    album: None,
                },
                ArtistRoute::Album { artist, album } => SessionRoute::Artists {
                    artist: Some(artist.clone()),
                    album: Some(SessionAlbumKey {
                        artist: artist.clone(),
                        title: album.clone(),
                    }),
                },
            },
            Destination::Tracks => SessionRoute::Tracks,
            Destination::Playlists => SessionRoute::Playlists {
                playlist_id: self.selected_playlist_id,
            },
            Destination::Storage => SessionRoute::Storage,
            Destination::Devices => SessionRoute::Devices,
        }
    }

    fn apply_restored_route(&mut self, route: RestoredRoute) {
        self.album_detail = None;
        self.artist_detail = None;
        self.artist_route = ArtistRoute::Index;
        match route {
            RestoredRoute::Albums(detail) => {
                self.destination = Destination::Albums;
                self.album_detail = detail.map(|(album, tracks)| AlbumDetail { album, tracks });
            }
            RestoredRoute::Artists { detail, album } => {
                self.destination = Destination::Artists;
                self.artist_detail = detail.map(|detail| *detail);
                if let Some((album, tracks)) = album {
                    self.artist_route = ArtistRoute::Album {
                        artist: album.artist.clone(),
                        album: album.title.clone(),
                    };
                    self.album_detail = Some(AlbumDetail { album, tracks });
                } else if let Some(detail) = &self.artist_detail {
                    self.artist_route = ArtistRoute::Detail {
                        artist: detail.artist.name.clone(),
                    };
                }
            }
            RestoredRoute::Tracks => self.destination = Destination::Tracks,
            RestoredRoute::Playlists(detail) => {
                self.destination = Destination::Playlists;
                if let Some((summary, entries)) = detail {
                    self.selected_playlist_id = Some(summary.playlist.id);
                    self.playlist_detail = Some(PlaylistDetail { summary, entries });
                }
            }
            RestoredRoute::Storage => self.destination = Destination::Storage,
            RestoredRoute::Devices => self.destination = Destination::Devices,
        }
    }
}

fn resolve_session_route(
    store: &ops::Store,
    route: &SessionRoute,
    album_sort: AlbumSortOrder,
) -> Result<RestoredRoute, LibraryError> {
    match route {
        SessionRoute::Albums { album: None } => Ok(RestoredRoute::Albums(None)),
        SessionRoute::Albums { album: Some(album) } => {
            let Some(found) = ops::catalog::album_by_key(store, &album.artist, &album.title)?
            else {
                return Ok(RestoredRoute::Albums(None));
            };
            let tracks = ops::catalog::album_tracks(store, &found.artist, &found.title)?;
            Ok(RestoredRoute::Albums(Some((found, tracks))))
        }
        SessionRoute::Artists {
            artist: None,
            album: None,
        } => Ok(RestoredRoute::Artists {
            detail: None,
            album: None,
        }),
        SessionRoute::Artists { artist, album } => {
            let Some(artist) = artist else {
                return Ok(RestoredRoute::Albums(None));
            };
            let Some(found) = ops::catalog::artist_index(store)?
                .into_iter()
                .find(|candidate| candidate.name == *artist)
            else {
                return Ok(RestoredRoute::Albums(None));
            };
            let detail = ops::catalog::artist_detail(store, found, album_sort)?;
            let album = match album {
                Some(key) => {
                    let Some(found) = detail
                        .albums
                        .iter()
                        .find(|candidate| {
                            candidate.artist == key.artist && candidate.title == key.title
                        })
                        .cloned()
                    else {
                        return Ok(RestoredRoute::Albums(None));
                    };
                    let tracks = ops::catalog::album_tracks(store, &found.artist, &found.title)?;
                    Some((found, tracks))
                }
                None => None,
            };
            Ok(RestoredRoute::Artists {
                detail: Some(Box::new(detail)),
                album,
            })
        }
        SessionRoute::Tracks => Ok(RestoredRoute::Tracks),
        SessionRoute::Playlists { playlist_id: None } => Ok(RestoredRoute::Playlists(None)),
        SessionRoute::Playlists {
            playlist_id: Some(playlist_id),
        } => {
            let Some(summary) = ops::playlists::list(store)?
                .into_iter()
                .find(|summary| summary.playlist.id == *playlist_id)
            else {
                return Ok(RestoredRoute::Albums(None));
            };
            let tracks = ops::playlists::tracks(store, *playlist_id)?;
            Ok(RestoredRoute::Playlists(Some((summary, tracks))))
        }
        SessionRoute::Storage => Ok(RestoredRoute::Storage),
        SessionRoute::Devices => Ok(RestoredRoute::Devices),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app_store::AppStore,
        backend::{AppSettings, RepeatMode, SessionState, testing as backend_testing},
    };
    use gpui::{AppContext, TestAppContext};

    struct LaunchHarness {
        _directory: tempfile::TempDir,
        settings_path: PathBuf,
        original_settings: Vec<u8>,
        app_store: Entity<AppStore>,
        view: Entity<LibraryView>,
    }

    fn saved_session() -> SessionState {
        SessionState {
            queue_track_ids: vec![1, 2, 3],
            queue_original_positions: vec![0, 1, 2],
            current_index: Some(1),
            position_ms: 42_000,
            shuffle_enabled: true,
            repeat_mode: RepeatMode::All,
            ..SessionState::default()
        }
    }

    fn launch_harness(cx: &mut TestAppContext, store: Option<ops::Store>) -> LaunchHarness {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        let settings = AppSettings {
            session: Some(saved_session()),
            ..AppSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let original_settings = fs::read(&settings_path).unwrap();
        let app_store = cx.new(|cx| AppStore::for_test(settings_path.clone(), settings, cx));
        let view = cx.new(|cx| {
            let mut view = LibraryView::for_test(
                app_store.clone(),
                directory.path().join("library.sqlite"),
                directory.path().join("covers"),
                cx,
            );
            view.store = store;
            view
        });
        LaunchHarness {
            _directory: directory,
            settings_path,
            original_settings,
            app_store,
            view,
        }
    }

    fn assert_blob_unchanged_after_shutdown(harness: &LaunchHarness, cx: &mut TestAppContext) {
        assert_eq!(
            fs::read(&harness.settings_path).unwrap(),
            harness.original_settings
        );
        cx.update_entity(&harness.app_store, |store, _| store.shutdown());
        assert_eq!(
            fs::read(&harness.settings_path).unwrap(),
            harness.original_settings
        );
    }

    fn populated_store(directory: &Path) -> ops::Store {
        let mut store = ops::Store::open_in_memory().unwrap();
        let root = ops::storage::add(&mut store, directory, "Test").unwrap();
        for (expected_id, title) in [(1, "One"), (2, "Two"), (3, "Three")] {
            assert_eq!(
                backend_testing::insert_track(
                    &mut store,
                    &root,
                    &format!("{expected_id}.flac"),
                    title,
                    expected_id,
                ),
                expected_id
            );
        }
        store
    }

    #[test]
    fn missing_album_and_playlist_routes_fall_back_to_albums() {
        let store = ops::Store::open_in_memory().unwrap();
        for route in [
            SessionRoute::Albums {
                album: Some(SessionAlbumKey {
                    artist: "Gone".to_string(),
                    title: "Gone".to_string(),
                }),
            },
            SessionRoute::Playlists {
                playlist_id: Some(404),
            },
        ] {
            assert!(matches!(
                resolve_session_route(&store, &route, AlbumSortOrder::Title).unwrap(),
                RestoredRoute::Albums(None)
            ));
        }
    }

    #[gpui::test]
    fn boot_failure_and_quit_preserve_the_blob_and_retry_restores_it(cx: &mut TestAppContext) {
        let failed = launch_harness(cx, None);
        cx.update_entity(&failed.view, |view, cx| {
            view.worker_tx
                .send(WorkerEvent::BootFinished(Err("open failed".to_string())))
                .unwrap();
            view.drain_worker_events(cx);
            assert!(matches!(view.boot, LibraryBoot::Failed { .. }));
        });
        assert_blob_unchanged_after_shutdown(&failed, cx);

        let retry = launch_harness(cx, None);
        let store = populated_store(retry._directory.path());
        cx.update_entity(&retry.view, |view, cx| {
            view.worker_tx
                .send(WorkerEvent::BootFinished(Err("open failed".to_string())))
                .unwrap();
            view.drain_worker_events(cx);
            view.worker_tx
                .send(WorkerEvent::BootFinished(Ok(store)))
                .unwrap();
            view.drain_worker_events(cx);
        });
        let restored = cx.read_entity(&retry.app_store, |store, _| store.playback_snapshot());
        assert_eq!(restored.queue.track_ids(), [1, 2, 3]);
        assert_eq!(restored.queue.current_index(), Some(1));
        assert_eq!(restored.position_ms, 42_000);
    }

    #[gpui::test]
    fn unavailable_store_path_preserves_the_blob(cx: &mut TestAppContext) {
        let harness = launch_harness(cx, None);
        cx.update_entity(&harness.view, |view, cx| view.restore_launch_state(cx));
        cx.read_entity(&harness.view, |view, _| {
            assert!(!view.launch_state_restored)
        });
        assert_blob_unchanged_after_shutdown(&harness, cx);
    }

    #[gpui::test]
    fn track_lookup_failure_preserves_the_blob(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let mut store = populated_store(directory.path());
        backend_testing::break_tracks(&mut store);
        let harness = launch_harness(cx, Some(store));
        cx.update_entity(&harness.view, |view, cx| view.restore_launch_state(cx));
        cx.read_entity(&harness.view, |view, _| {
            assert!(view.launch_state_restored);
            assert!(view.error.is_some());
        });
        assert_blob_unchanged_after_shutdown(&harness, cx);
    }
}
