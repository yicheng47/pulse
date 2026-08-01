mod albums;
mod context_menu;
mod playlists;
mod storage;
mod tracks;
pub(crate) mod view_model;

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui::{
    AnyElement, Context, Entity, FocusHandle, IntoElement, KeyDownEvent, PathPromptOptions, Pixels,
    Point, Render, ScrollHandle, Window, div, prelude::*, px,
};

use crate::{
    library::{
        Album, AlbumSortOrder, LibraryError, LibrarySearchResults, LibraryStore, LibrarySummary,
        PlaylistId, PlaylistSummary, PlaylistTrack, ScanHistoryEntry, ScanProgress, StorageRoot,
        StorageRootId, Track, TrackId, TrackSortOrder, scan_storage_root_cancellable,
    },
    playback_row::PlaybackRow,
    preferences,
    shell::Destination,
    theme,
};

use view_model::{FilterChip, Pagination};

const SCAN_POLL_INTERVAL: Duration = Duration::from_millis(50);
const TRACK_PAGE_SIZE: usize = 50;

#[derive(Clone)]
struct StorageRootView {
    root: StorageRoot,
    summary: LibrarySummary,
    latest_scan: Option<ScanHistoryEntry>,
}

struct ActiveScan {
    root_id: StorageRootId,
    progress: Option<ScanProgress>,
    cancel: Arc<AtomicBool>,
}

enum ScanCompletion {
    Completed,
    Cancelled,
}

enum ScanWorkerEvent {
    Progress {
        root_id: StorageRootId,
        progress: ScanProgress,
    },
    Finished {
        root_id: StorageRootId,
        store: LibraryStore,
        result: Result<ScanCompletion, String>,
    },
}

struct AlbumDetail {
    album: Album,
    tracks: Vec<Track>,
}

struct PlaylistDetail {
    summary: PlaylistSummary,
    entries: Vec<PlaylistTrack>,
}

struct AddStorageDraft {
    path: Option<PathBuf>,
    display_name: String,
    scan_now: bool,
}

enum Modal {
    AddStorage(AddStorageDraft),
    RemoveStorage {
        root_id: StorageRootId,
        display_name: String,
    },
    PlaylistName {
        mode: PlaylistNameMode,
        name: String,
    },
    DeletePlaylist {
        playlist_id: PlaylistId,
        name: String,
        entry_count: u64,
    },
}

#[derive(Clone, Copy)]
enum PlaylistNameMode {
    Create { add_track_id: Option<TrackId> },
    Rename { playlist_id: PlaylistId },
}

#[derive(Clone, Copy)]
enum TrackSurface {
    Album,
    Tracks,
    Playlist {
        playlist_id: PlaylistId,
        position: usize,
    },
}

#[derive(Clone)]
struct TrackMenu {
    track_id: TrackId,
    surface: TrackSurface,
    anchor: Point<Pixels>,
    flyout_open: bool,
}

#[derive(Clone)]
struct PlaylistMenu {
    playlist_id: PlaylistId,
    anchor: Point<Pixels>,
}

struct RenameDraft {
    root_id: StorageRootId,
    display_name: String,
}

pub(crate) struct LibraryView {
    destination: Destination,
    row: Entity<PlaybackRow>,
    store: Option<LibraryStore>,
    database_path: PathBuf,
    cover_cache_directory: PathBuf,
    albums: Vec<Album>,
    tracks: Vec<Track>,
    genres: Vec<String>,
    catalog_summary: LibrarySummary,
    roots: Vec<StorageRootView>,
    selected_root_id: Option<StorageRootId>,
    album_detail: Option<AlbumDetail>,
    playlists: Vec<PlaylistSummary>,
    selected_playlist_id: Option<PlaylistId>,
    playlist_detail: Option<PlaylistDetail>,
    selected_album_track_id: Option<TrackId>,
    selected_track_id: Option<TrackId>,
    selected_playlist_position: Option<usize>,
    album_sort: AlbumSortOrder,
    albums_as_grid: bool,
    track_sort: TrackSortOrder,
    album_filter: FilterChip,
    track_filter: FilterChip,
    artist_filter: Option<String>,
    track_pagination: Pagination,
    albums_scroll: ScrollHandle,
    album_detail_scroll: ScrollHandle,
    tracks_scroll: ScrollHandle,
    playlists_scroll: ScrollHandle,
    playlist_detail_scroll: ScrollHandle,
    scan: Option<ActiveScan>,
    scan_tx: Sender<ScanWorkerEvent>,
    scan_rx: Receiver<ScanWorkerEvent>,
    modal: Option<Modal>,
    rename_draft: Option<RenameDraft>,
    track_menu: Option<TrackMenu>,
    playlist_menu: Option<PlaylistMenu>,
    input_focus: FocusHandle,
    error: Option<String>,
}

impl LibraryView {
    pub(crate) fn new(row: Entity<PlaybackRow>, cx: &mut Context<Self>) -> Self {
        let database_path =
            preferences::library_database_path().expect("failed to resolve library database path");
        let cover_cache_directory = preferences::cover_cache_directory()
            .expect("failed to resolve library cover cache path");
        let store = LibraryStore::open(&database_path).expect("failed to open the music library");
        let (scan_tx, scan_rx) = mpsc::channel();
        let mut view = Self {
            destination: Destination::Albums,
            row,
            store: Some(store),
            database_path,
            cover_cache_directory,
            albums: Vec::new(),
            tracks: Vec::new(),
            genres: Vec::new(),
            catalog_summary: LibrarySummary::default(),
            roots: Vec::new(),
            selected_root_id: None,
            album_detail: None,
            playlists: Vec::new(),
            selected_playlist_id: None,
            playlist_detail: None,
            selected_album_track_id: None,
            selected_track_id: None,
            selected_playlist_position: None,
            album_sort: AlbumSortOrder::DateAdded,
            albums_as_grid: true,
            track_sort: TrackSortOrder::DateAdded,
            album_filter: FilterChip::All,
            track_filter: FilterChip::All,
            artist_filter: None,
            track_pagination: Pagination::new(TRACK_PAGE_SIZE),
            albums_scroll: ScrollHandle::new(),
            album_detail_scroll: ScrollHandle::new(),
            tracks_scroll: ScrollHandle::new(),
            playlists_scroll: ScrollHandle::new(),
            playlist_detail_scroll: ScrollHandle::new(),
            scan: None,
            scan_tx,
            scan_rx,
            modal: None,
            rename_draft: None,
            track_menu: None,
            playlist_menu: None,
            input_focus: cx.focus_handle(),
            error: None,
        };
        view.reload().expect("failed to load the music library");

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SCAN_POLL_INTERVAL).await;
                if this
                    .update(cx, |this, cx| this.drain_scan_events(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        view
    }

    pub(crate) fn set_destination(&mut self, destination: Destination, cx: &mut Context<Self>) {
        if destination != self.destination && destination == Destination::Albums {
            self.album_detail = None;
            self.selected_album_track_id = None;
        }
        self.destination = destination;
        self.track_menu = None;
        self.playlist_menu = None;
        cx.notify();
    }

    pub(crate) fn storage_root_count(&self) -> usize {
        self.roots.len()
    }

    fn reload(&mut self) -> Result<(), LibraryError> {
        let Some(store) = self.store.as_ref() else {
            return Ok(());
        };
        let albums = store.albums(self.album_sort)?;
        let track_page = store.track_page(
            self.track_sort,
            &self.track_filter.track_query_filter(current_time_ms()),
            self.artist_filter.as_deref(),
            self.track_pagination.page_size(),
            self.track_pagination.offset(),
        )?;
        let genres = store.genres()?;
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
        self.albums = albums;
        self.track_pagination
            .set_total_items(track_page.total_count);
        self.tracks = track_page.tracks;
        self.genres = genres;
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

    fn cycle_album_sort(&mut self, cx: &mut Context<Self>) {
        self.album_sort = match self.album_sort {
            AlbumSortOrder::Title => AlbumSortOrder::Artist,
            AlbumSortOrder::Artist => AlbumSortOrder::DateAdded,
            AlbumSortOrder::DateAdded => AlbumSortOrder::ReleaseYear,
            AlbumSortOrder::ReleaseYear => AlbumSortOrder::Duration,
            AlbumSortOrder::Duration => AlbumSortOrder::Title,
        };
        self.reload_or_show_error();
        cx.notify();
    }

    fn cycle_track_sort(&mut self, cx: &mut Context<Self>) {
        self.track_sort = match self.track_sort {
            TrackSortOrder::Title => TrackSortOrder::Artist,
            TrackSortOrder::Artist => TrackSortOrder::Album,
            TrackSortOrder::Album => TrackSortOrder::DateAdded,
            TrackSortOrder::DateAdded => TrackSortOrder::ReleaseYear,
            TrackSortOrder::ReleaseYear => TrackSortOrder::Duration,
            TrackSortOrder::Duration => TrackSortOrder::Title,
        };
        self.reset_track_page();
        self.reload_or_show_error();
        cx.notify();
    }

    fn set_track_filter(&mut self, filter: FilterChip, cx: &mut Context<Self>) {
        if self.track_filter == filter {
            return;
        }
        self.track_filter = filter;
        self.reset_track_page();
        self.reload_or_show_error();
        cx.notify();
    }

    fn set_artist_filter(&mut self, artist: Option<String>, cx: &mut Context<Self>) {
        if self.artist_filter == artist {
            return;
        }
        self.artist_filter = artist;
        self.reset_track_page();
        self.reload_or_show_error();
        cx.notify();
    }

    fn set_track_page(&mut self, page: usize, cx: &mut Context<Self>) {
        if !self.track_pagination.set_page(page) {
            return;
        }
        self.tracks_scroll = ScrollHandle::new();
        self.reload_or_show_error();
        cx.notify();
    }

    fn reset_track_page(&mut self) {
        self.track_pagination.reset();
        self.tracks_scroll = ScrollHandle::new();
    }

    fn open_album(&mut self, album: Album, cx: &mut Context<Self>) {
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
        cx.notify();
    }

    fn play_tracks(&mut self, tracks: Vec<Track>, index: usize, cx: &mut Context<Self>) {
        self.row
            .update(cx, |row, cx| row.play_library_tracks(&tracks, index, cx));
    }

    fn select_tracks(&mut self, tracks: Vec<Track>, index: usize, cx: &mut Context<Self>) {
        self.row
            .update(cx, |row, cx| row.select_library_tracks(&tracks, index, cx));
    }

    fn activate_album_track(&mut self, index: usize, play: bool, cx: &mut Context<Self>) {
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

    fn matching_tracks(&self) -> Result<Vec<Track>, LibraryError> {
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

    fn activate_visible_track(&mut self, track_id: TrackId, play: bool, cx: &mut Context<Self>) {
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

    fn activate_playlist_entry(&mut self, position: usize, play: bool, cx: &mut Context<Self>) {
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

    fn play_album(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &self.album_detail else {
            return;
        };
        if !detail.tracks.is_empty() {
            self.play_tracks(detail.tracks.clone(), 0, cx);
        }
    }

    fn play_playlist(&mut self, cx: &mut Context<Self>) {
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

    fn select_playlist(&mut self, playlist_id: PlaylistId, cx: &mut Context<Self>) {
        if !self
            .playlists
            .iter()
            .any(|playlist| playlist.playlist.id == playlist_id)
        {
            return;
        }
        self.selected_playlist_id = Some(playlist_id);
        self.selected_playlist_position = None;
        self.reload_or_show_error();
        cx.notify();
    }

    fn begin_create_playlist(
        &mut self,
        add_track_id: Option<TrackId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.track_menu = None;
        self.playlist_menu = None;
        self.modal = Some(Modal::PlaylistName {
            mode: PlaylistNameMode::Create { add_track_id },
            name: String::new(),
        });
        window.focus(&self.input_focus, cx);
        cx.notify();
    }

    fn begin_rename_playlist(
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
        self.track_menu = None;
        self.playlist_menu = None;
        self.modal = Some(Modal::PlaylistName {
            mode: PlaylistNameMode::Rename { playlist_id },
            name: playlist.playlist.name.clone(),
        });
        window.focus(&self.input_focus, cx);
        cx.notify();
    }

    fn confirm_playlist_name(&mut self, cx: &mut Context<Self>) {
        let Some(Modal::PlaylistName { mode, name }) = self.modal.take() else {
            return;
        };
        let name = name.trim();
        if name.is_empty() {
            cx.notify();
            return;
        }
        let Some(store) = self.store.as_mut() else {
            self.error = Some("Wait for the active library scan to finish.".to_string());
            self.modal = Some(Modal::PlaylistName {
                mode,
                name: name.to_string(),
            });
            cx.notify();
            return;
        };
        let result = match mode {
            PlaylistNameMode::Create { add_track_id } => {
                store.create_playlist(name).and_then(|playlist| {
                    if let Some(track_id) = add_track_id {
                        store.append_playlist_tracks(playlist.id, &[track_id])?;
                    }
                    Ok(playlist.id)
                })
            }
            PlaylistNameMode::Rename { playlist_id } => store
                .rename_playlist(playlist_id, name)
                .map(|playlist| playlist.id),
        };
        match result {
            Ok(playlist_id) => {
                self.selected_playlist_id = Some(playlist_id);
                self.selected_playlist_position = None;
                self.reload_or_show_error();
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.modal = Some(Modal::PlaylistName {
                    mode,
                    name: name.to_string(),
                });
            }
        }
        cx.notify();
    }

    fn request_delete_playlist(&mut self, playlist_id: PlaylistId, cx: &mut Context<Self>) {
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

    fn confirm_delete_playlist(&mut self, cx: &mut Context<Self>) {
        let Some(Modal::DeletePlaylist {
            playlist_id,
            name,
            entry_count,
        }) = self.modal.take()
        else {
            return;
        };
        let Some(store) = self.store.as_mut() else {
            self.error = Some("Wait for the active library scan to finish.".to_string());
            self.modal = Some(Modal::DeletePlaylist {
                playlist_id,
                name,
                entry_count,
            });
            cx.notify();
            return;
        };
        match store.delete_playlist(playlist_id) {
            Ok(()) => {
                if self.selected_playlist_id == Some(playlist_id) {
                    self.selected_playlist_id = None;
                    self.selected_playlist_position = None;
                }
                self.reload_or_show_error();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn add_track_to_playlist(
        &mut self,
        playlist_id: PlaylistId,
        track_id: TrackId,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.store.as_mut() else {
            self.error = Some("Wait for the active library scan to finish.".to_string());
            cx.notify();
            return;
        };
        if let Err(error) = store.append_playlist_tracks(playlist_id, &[track_id]) {
            self.error = Some(error.to_string());
        } else {
            self.reload_or_show_error();
        }
        self.track_menu = None;
        cx.notify();
    }

    fn remove_playlist_entry(
        &mut self,
        playlist_id: PlaylistId,
        position: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.store.as_mut() else {
            self.error = Some("Wait for the active library scan to finish.".to_string());
            cx.notify();
            return;
        };
        if let Err(error) = store.remove_playlist_entry(playlist_id, position) {
            self.error = Some(error.to_string());
        } else {
            self.selected_playlist_position = None;
            self.reload_or_show_error();
        }
        self.track_menu = None;
        cx.notify();
    }

    fn move_playlist_entry(
        &mut self,
        playlist_id: PlaylistId,
        from_position: usize,
        to_position: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.store.as_mut() else {
            self.error = Some("Wait for the active library scan to finish.".to_string());
            cx.notify();
            return;
        };
        if let Err(error) = store.move_playlist_entry(playlist_id, from_position, to_position) {
            self.error = Some(error.to_string());
        } else {
            self.selected_playlist_position = Some(to_position);
            self.reload_or_show_error();
        }
        self.track_menu = None;
        cx.notify();
    }

    fn begin_add_storage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.scan.is_some() {
            return;
        }
        self.modal = Some(Modal::AddStorage(AddStorageDraft {
            path: None,
            display_name: String::new(),
            scan_now: true,
        }));
        window.focus(&self.input_focus, cx);
        cx.notify();
    }

    fn choose_storage_folder(&mut self, cx: &mut Context<Self>) {
        if self.scan.is_some() {
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            let response = receiver.await;
            let _ = this.update(cx, |this, cx| {
                match response {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            let display_name = path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.display().to_string());
                            if let Some(Modal::AddStorage(draft)) = &mut this.modal {
                                draft.path = Some(path);
                                if draft.display_name.trim().is_empty() {
                                    draft.display_name = display_name;
                                }
                                this.error = None;
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => this.error = Some(error.to_string()),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_add_storage(&mut self, cx: &mut Context<Self>) {
        if self.store.is_none() {
            self.error = Some("Wait for the active library scan to finish.".to_string());
            cx.notify();
            return;
        }
        let Some(Modal::AddStorage(draft)) = self.modal.take() else {
            return;
        };
        let Some(path) = draft.path.as_ref() else {
            self.modal = Some(Modal::AddStorage(draft));
            cx.notify();
            return;
        };
        if draft.display_name.trim().is_empty() {
            self.modal = Some(Modal::AddStorage(draft));
            cx.notify();
            return;
        }
        let store = self.store.as_mut().expect("store availability checked");
        match store.add_storage_root(path, draft.display_name.trim()) {
            Ok(root) => {
                self.selected_root_id = Some(root.id);
                self.reload_or_show_error();
                if draft.scan_now {
                    self.start_scan(root.id, cx);
                }
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.modal = Some(Modal::AddStorage(draft));
            }
        }
        cx.notify();
    }

    fn request_remove_storage(&mut self, root_id: StorageRootId, cx: &mut Context<Self>) {
        if self.scan.is_some() {
            return;
        }
        let Some(root) = self.roots.iter().find(|root| root.root.id == root_id) else {
            return;
        };
        self.modal = Some(Modal::RemoveStorage {
            root_id,
            display_name: root.root.display_name.clone(),
        });
        cx.notify();
    }

    fn confirm_remove_storage(&mut self, cx: &mut Context<Self>) {
        if self.store.is_none() {
            self.error = Some("Wait for the active library scan to finish.".to_string());
            cx.notify();
            return;
        }
        let Some(Modal::RemoveStorage { root_id, .. }) = self.modal.take() else {
            return;
        };
        let store = self.store.as_mut().expect("store availability checked");
        match store.remove_storage_root(root_id) {
            Ok(cover_paths) => {
                for path in cover_paths {
                    if let Err(error) = fs::remove_file(&path)
                        && error.kind() != io::ErrorKind::NotFound
                    {
                        self.error = Some(format!(
                            "Removed the storage root, but could not delete {}: {error}",
                            path.display()
                        ));
                    }
                }
                self.reload_or_show_error();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn begin_rename_storage(&mut self, root_id: StorageRootId, cx: &mut Context<Self>) {
        if self.scan.is_some() {
            return;
        }
        let Some(root) = self.roots.iter().find(|root| root.root.id == root_id) else {
            return;
        };
        self.rename_draft = Some(RenameDraft {
            root_id,
            display_name: root.root.display_name.clone(),
        });
        cx.notify();
    }

    fn commit_rename_storage(&mut self, cx: &mut Context<Self>) {
        if self.store.is_none() {
            self.error = Some("Wait for the active library scan to finish.".to_string());
            cx.notify();
            return;
        }
        let Some(draft) = self.rename_draft.take() else {
            return;
        };
        let store = self.store.as_mut().expect("store availability checked");
        if let Err(error) = store.rename_storage_root(draft.root_id, draft.display_name.trim()) {
            self.error = Some(error.to_string());
            self.rename_draft = Some(draft);
        } else {
            self.reload_or_show_error();
        }
        cx.notify();
    }

    fn start_scan(&mut self, root_id: StorageRootId, cx: &mut Context<Self>) {
        if self.scan.is_some() {
            return;
        }
        let mut store = self.store.take().expect("store is available before scan");
        let sender = self.scan_tx.clone();
        let cover_cache_directory = self.cover_cache_directory.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        self.scan = Some(ActiveScan {
            root_id,
            progress: None,
            cancel,
        });
        thread::Builder::new()
            .name("pulse-library-scan".to_string())
            .spawn(move || {
                let progress_sender = sender.clone();
                let result = scan_storage_root_cancellable(
                    &mut store,
                    root_id,
                    cover_cache_directory,
                    move |progress| {
                        let _ =
                            progress_sender.send(ScanWorkerEvent::Progress { root_id, progress });
                    },
                    || worker_cancel.load(Ordering::Acquire),
                )
                .map(|report| {
                    if report.is_some() {
                        ScanCompletion::Completed
                    } else {
                        ScanCompletion::Cancelled
                    }
                })
                .map_err(|error| error.to_string());
                let _ = sender.send(ScanWorkerEvent::Finished {
                    root_id,
                    store,
                    result,
                });
            })
            .expect("failed to spawn library scan worker");
        cx.notify();
    }

    fn cancel_scan(&mut self, cx: &mut Context<Self>) {
        if let Some(scan) = &self.scan {
            scan.cancel.store(true, Ordering::Release);
        }
        cx.notify();
    }

    fn drain_scan_events(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        loop {
            match self.scan_rx.try_recv() {
                Ok(ScanWorkerEvent::Progress { root_id, progress }) => {
                    if let Some(scan) = &mut self.scan
                        && scan.root_id == root_id
                    {
                        scan.progress = Some(progress);
                        changed = true;
                    }
                }
                Ok(ScanWorkerEvent::Finished {
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
                    if let Err(error) = result {
                        self.error = Some(error);
                    }
                    self.reload_or_show_error();
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.error = Some("Library scan worker disconnected.".to_string());
                    changed = true;
                    break;
                }
            }
        }
        if changed {
            cx.notify();
        }
    }

    fn reload_or_show_error(&mut self) {
        if let Err(error) = self.reload() {
            self.error = Some(error.to_string());
        }
    }

    fn select_root(&mut self, root_id: StorageRootId, cx: &mut Context<Self>) {
        self.selected_root_id = Some(root_id);
        self.rename_draft = None;
        cx.notify();
    }

    fn selected_root(&self) -> Option<&StorageRootView> {
        let selected = self.selected_root_id?;
        self.roots.iter().find(|root| root.root.id == selected)
    }

    fn handle_text_input(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "backspace" => {
                if let Some(value) = self.input_value_mut() {
                    value.pop();
                }
            }
            "enter" => {
                if matches!(self.modal, Some(Modal::PlaylistName { .. })) {
                    self.confirm_playlist_name(cx);
                } else if self.rename_draft.is_some() {
                    self.commit_rename_storage(cx);
                } else if matches!(self.modal, Some(Modal::AddStorage(_))) {
                    self.confirm_add_storage(cx);
                }
                return;
            }
            "escape" => {
                self.rename_draft = None;
                self.modal = None;
            }
            _ if !event.keystroke.modifiers.platform && !event.keystroke.modifiers.control => {
                if let Some(text) = &event.keystroke.key_char
                    && let Some(value) = self.input_value_mut()
                {
                    value.push_str(text);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn input_value_mut(&mut self) -> Option<&mut String> {
        match &mut self.modal {
            Some(Modal::AddStorage(draft)) => return Some(&mut draft.display_name),
            Some(Modal::PlaylistName { name, .. }) => return Some(name),
            _ => {}
        }
        self.rename_draft
            .as_mut()
            .map(|draft| &mut draft.display_name)
    }

    fn is_now_playing(&self, path: &Path, cx: &Context<Self>) -> bool {
        self.row.read(cx).is_now_playing(path)
    }
}

impl Render for LibraryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content: AnyElement = match self.destination {
            Destination::Albums => self.render_albums(window, cx),
            Destination::Tracks => self.render_tracks(window, cx),
            Destination::Playlists => self.render_playlists(cx),
            Destination::Storage => self.render_storage(window, cx),
            Destination::Devices => div().into_any_element(),
        };

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(content)
            .when(self.track_menu.is_some(), |view| {
                view.child(self.render_track_context_menu(cx))
            })
            .when(self.playlist_menu.is_some(), |view| {
                view.child(self.render_playlist_context_menu(cx))
            })
            .when_some(self.error.clone(), |view, error| {
                view.child(
                    div()
                        .absolute()
                        .right_4()
                        .bottom_4()
                        .max_w(px(520.))
                        .px(px(12.))
                        .py(px(9.))
                        .rounded(px(theme::RADIUS_MD))
                        .border_1()
                        .border_color(theme::danger())
                        .bg(theme::danger_soft())
                        .font_family(theme::FONT_SANS)
                        .text_size(px(12.))
                        .text_color(theme::danger())
                        .child(error),
                )
            })
            .when(self.modal.is_some(), |view| {
                view.child(self.render_modal(window, cx))
            })
    }
}

fn current_time_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}
