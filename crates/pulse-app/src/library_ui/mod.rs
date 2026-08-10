mod albums;
mod context_menu;
mod playlists;
mod storage;
mod tracks;
pub(crate) mod view_model;

use std::{
    fs, io,
    panic::{AssertUnwindSafe, catch_unwind},
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
        Album, AlbumSortOrder, BackfillProgress, DeleteAlbumOutcome, LibraryError,
        LibrarySearchResults, LibraryStore, LibrarySummary, PlaylistId, PlaylistSummary,
        PlaylistTrack, ScanHistoryEntry, ScanOutcome, ScanProgress, StorageRoot, StorageRootId,
        Track, TrackId, TrackSortOrder, delete_album_tracks, scan_storage_root_cancellable,
    },
    playback_row::PlaybackRow,
    preferences,
    shell::Destination,
    theme,
};

use view_model::{FilterChip, Pagination};

const SCAN_POLL_INTERVAL: Duration = Duration::from_millis(50);
const LIST_PAGE_SIZE: usize = 20;
/// Distance from the album grid's bottom edge that triggers the next page.
const ALBUM_PREFETCH_PX: f32 = 600.;

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
    Completed {
        outcome: ScanOutcome,
        removals_suppressed: bool,
    },
    Cancelled,
}

/// Artists matching the popover's search text, case-insensitively; an empty
/// search matches everything.
fn filter_artists(artists: &[(String, u64)], search: &str) -> Vec<(String, u64)> {
    let needle = search.trim().to_lowercase();
    artists
        .iter()
        .filter(|(artist, _)| needle.is_empty() || artist.to_lowercase().contains(&needle))
        .cloned()
        .collect()
}

/// The user-facing report for a finished album delete. Files and rows can
/// diverge (cross-filesystem atomicity is impossible) and the post-delete
/// reload can fail on the same broken database — every part that happened
/// must survive into the one message the user sees.
fn delete_album_notice(
    outcome: &DeleteAlbumOutcome,
    reload_error: Option<String>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(db_error) = &outcome.db_error {
        parts.push(format!(
            "Deleted {} of {} audio files, but updating the library failed: {db_error}. Run \
             Delete Album again to finish the cleanup.",
            outcome.deleted_files, outcome.total_files
        ));
        if !outcome.failures.is_empty() {
            parts.push(format!("Could not delete: {}", outcome.failures.join("; ")));
        }
    } else if !outcome.failures.is_empty() {
        parts.push(format!(
            "Deleted {} of {} audio files. Could not delete: {}",
            outcome.deleted_files,
            outcome.total_files,
            outcome.failures.join("; ")
        ));
    }
    if let Some(reload) = reload_error {
        if parts.is_empty() {
            return Some(reload);
        }
        parts.push(format!("Reloading the library also failed: {reload}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// Runtime FILE MISSING marks may be cleared only after a scan that verified
/// presence for every retained row: the walk finished (not cancelled, not
/// offline/failed) and the missing-row removal pass actually ran.
fn scan_verified_presence(completion: &ScanCompletion) -> bool {
    matches!(
        completion,
        ScanCompletion::Completed {
            outcome: ScanOutcome::Completed | ScanOutcome::CompletedWithErrors,
            removals_suppressed: false,
        }
    )
}

enum WorkerEvent {
    BootProgress(BackfillProgress),
    BootFinished(Result<LibraryStore, String>),
    ScanProgress {
        root_id: StorageRootId,
        progress: ScanProgress,
    },
    ScanFinished {
        root_id: StorageRootId,
        store: LibraryStore,
        result: Result<ScanCompletion, String>,
    },
    DeleteAlbumFinished {
        store: LibraryStore,
        result: Result<DeleteAlbumOutcome, String>,
    },
    /// The delete worker panicked and the store moved into it is gone; the
    /// UI must reopen the library to recover.
    DeleteAlbumPanicked,
    /// The scan worker panicked and the store moved into it is gone; the UI
    /// must reopen the library to recover.
    ScanPanicked {
        root_id: StorageRootId,
    },
}

/// App-open state: the store opens (and any schema backfill runs) off the
/// main thread so a slow first launch shows progress instead of blocking.
enum LibraryBoot {
    Opening { backfill: Option<BackfillProgress> },
    Ready,
    Failed { message: String },
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
    DeleteAlbum {
        album: Album,
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
    boot: LibraryBoot,
    database_path: PathBuf,
    cover_cache_directory: PathBuf,
    albums: Vec<Album>,
    tracks: Vec<Track>,
    genres: Vec<String>,
    artists: Vec<(String, u64)>,
    artist_popover_open: bool,
    artist_hint_press_closed_popover: bool,
    artist_search: String,
    /// Latched when an album page query fails so a persistent error cannot
    /// become a notify/requery loop; cleared by reset or a successful reload.
    album_load_stalled: bool,
    catalog_summary: LibrarySummary,
    roots: Vec<StorageRootView>,
    selected_root_id: Option<StorageRootId>,
    album_detail: Option<AlbumDetail>,
    album_menu_open: bool,
    album_menu_press_closed: bool,
    album_delete_in_flight: bool,
    playlists: Vec<PlaylistSummary>,
    selected_playlist_id: Option<PlaylistId>,
    playlist_detail: Option<PlaylistDetail>,
    selected_album_track_id: Option<TrackId>,
    selected_track_id: Option<TrackId>,
    selected_playlist_position: Option<usize>,
    album_sort: AlbumSortOrder,
    track_sort: TrackSortOrder,
    album_filter: FilterChip,
    track_filter: FilterChip,
    artist_filter: Option<String>,
    track_pagination: Pagination,
    album_total: usize,
    albums_scroll: ScrollHandle,
    album_detail_scroll: ScrollHandle,
    tracks_scroll: ScrollHandle,
    playlists_scroll: ScrollHandle,
    playlist_detail_scroll: ScrollHandle,
    scan: Option<ActiveScan>,
    worker_tx: Sender<WorkerEvent>,
    worker_rx: Receiver<WorkerEvent>,
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
        // Missing-file marks and now-playing state live on the row and change
        // asynchronously; without this the lists would render them stale.
        cx.observe(&row, |_, _, cx| cx.notify()).detach();
        let (worker_tx, worker_rx) = mpsc::channel();
        let mut view = Self {
            destination: Destination::Albums,
            row,
            store: None,
            boot: LibraryBoot::Opening { backfill: None },
            database_path,
            cover_cache_directory,
            albums: Vec::new(),
            tracks: Vec::new(),
            genres: Vec::new(),
            artists: Vec::new(),
            artist_popover_open: false,
            artist_hint_press_closed_popover: false,
            artist_search: String::new(),
            album_load_stalled: false,
            catalog_summary: LibrarySummary::default(),
            roots: Vec::new(),
            selected_root_id: None,
            album_detail: None,
            album_menu_open: false,
            album_menu_press_closed: false,
            album_delete_in_flight: false,
            playlists: Vec::new(),
            selected_playlist_id: None,
            playlist_detail: None,
            selected_album_track_id: None,
            selected_track_id: None,
            selected_playlist_position: None,
            album_sort: AlbumSortOrder::DateAdded,
            track_sort: TrackSortOrder::DateAdded,
            album_filter: FilterChip::All,
            track_filter: FilterChip::All,
            artist_filter: None,
            track_pagination: Pagination::new(LIST_PAGE_SIZE),
            album_total: 0,
            albums_scroll: ScrollHandle::new(),
            album_detail_scroll: ScrollHandle::new(),
            tracks_scroll: ScrollHandle::new(),
            playlists_scroll: ScrollHandle::new(),
            playlist_detail_scroll: ScrollHandle::new(),
            scan: None,
            worker_tx,
            worker_rx,
            modal: None,
            rename_draft: None,
            track_menu: None,
            playlist_menu: None,
            input_focus: cx.focus_handle(),
            error: None,
        };
        view.begin_open_store();

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SCAN_POLL_INTERVAL).await;
                if this
                    .update(cx, |this, cx| this.drain_worker_events(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        view
    }

    fn begin_open_store(&mut self) {
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

    fn is_library_loading(&self) -> bool {
        matches!(self.boot, LibraryBoot::Opening { .. })
    }

    fn store_busy_message(&self) -> String {
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
        self.artist_popover_open = false;
        cx.notify();
    }

    pub(crate) fn storage_root_count(&self) -> usize {
        self.roots.len()
    }

    fn reload(&mut self) -> Result<(), LibraryError> {
        let Some(store) = self.store.as_ref() else {
            return Ok(());
        };
        // Refresh the already-loaded portion of the grid so infinite-scroll
        // position survives reloads; at least one page always loads.
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
            self.track_pagination.page_size(),
            self.track_pagination.offset(),
        )?;
        let genres = store.genres()?;
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
        self.track_pagination
            .set_total_items(track_page.total_count);
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

    fn cycle_album_sort(&mut self, cx: &mut Context<Self>) {
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

    fn reset_albums(&mut self) {
        self.albums.clear();
        self.album_total = 0;
        self.album_load_stalled = false;
        self.albums_scroll = ScrollHandle::new();
    }

    /// The album grid loads more when scrolled within a prefetch margin of
    /// the bottom (or when the loaded set does not fill the viewport yet).
    fn should_load_more_albums(&self) -> bool {
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
    fn load_more_albums(&mut self) -> bool {
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

    /// Closes the popover before delegating, notifying unconditionally —
    /// `set_artist_filter` early-returns without a repaint when the value is
    /// unchanged, which must not leave a dismissed popover on screen.
    pub(super) fn choose_artist_filter(&mut self, artist: Option<String>, cx: &mut Context<Self>) {
        self.artist_popover_open = false;
        cx.notify();
        self.set_artist_filter(artist, cx);
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
        self.album_menu_open = false;
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
            self.error = Some(self.store_busy_message());
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
            self.error = Some(self.store_busy_message());
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

    fn request_delete_album(&mut self, cx: &mut Context<Self>) {
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
    fn confirm_delete_album(&mut self, cx: &mut Context<Self>) {
        if self.album_delete_in_flight {
            return;
        }
        let Some(Modal::DeleteAlbum { album }) = self.modal.as_ref() else {
            return;
        };
        let album = album.clone();
        let Some(mut store) = self.store.take() else {
            self.error = Some(self.store_busy_message());
            cx.notify();
            return;
        };
        self.album_delete_in_flight = true;
        let sender = self.worker_tx.clone();
        thread::Builder::new()
            .name("pulse-album-delete".to_string())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    let result = delete_album_tracks(&mut store, &album.artist, &album.title)
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

    fn add_track_to_playlist(
        &mut self,
        playlist_id: PlaylistId,
        track_id: TrackId,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.store.as_mut() else {
            self.error = Some(self.store_busy_message());
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
            self.error = Some(self.store_busy_message());
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
            self.error = Some(self.store_busy_message());
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
            self.error = Some(self.store_busy_message());
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
            self.error = Some(self.store_busy_message());
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
                self.row.update(cx, |row, _| row.clear_missing_marks());
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
            self.error = Some(self.store_busy_message());
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
        // Defense in depth: a worker (album delete) may own the store even
        // with no scan active.
        let Some(mut store) = self.store.take() else {
            self.error = Some(self.store_busy_message());
            cx.notify();
            return;
        };
        let sender = self.worker_tx.clone();
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
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    let result = scan_storage_root_cancellable(
                        &mut store,
                        root_id,
                        cover_cache_directory,
                        move |progress| {
                            let _ = progress_sender
                                .send(WorkerEvent::ScanProgress { root_id, progress });
                        },
                        || worker_cancel.load(Ordering::Acquire),
                    )
                    .map(|report| match report {
                        Some(report) => ScanCompletion::Completed {
                            outcome: report.outcome,
                            removals_suppressed: report.removals_suppressed,
                        },
                        None => ScanCompletion::Cancelled,
                    })
                    .map_err(|error| error.to_string());
                    (store, result)
                }));
                let _ = match outcome {
                    Ok((store, result)) => sender.send(WorkerEvent::ScanFinished {
                        root_id,
                        store,
                        result,
                    }),
                    // The store was consumed by the unwind; the UI reopens it.
                    Err(_) => sender.send(WorkerEvent::ScanPanicked { root_id }),
                };
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

    fn drain_worker_events(&mut self, cx: &mut Context<Self>) {
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
                            self.row.update(cx, |row, _| row.clear_missing_marks());
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
                                self.row.update(cx, |row, _| {
                                    row.remove_missing_marks(&outcome.deleted_ids);
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
                if self.artist_popover_open {
                    if let Some((artist, _)) = self.filtered_artists().into_iter().next() {
                        self.choose_artist_filter(Some(artist), cx);
                    }
                    return;
                }
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
                self.artist_popover_open = false;
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
        if self.artist_popover_open {
            return Some(&mut self.artist_search);
        }
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

    fn render_library_failed(&self, message: String, cx: &mut Context<Self>) -> AnyElement {
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
            .child(div().mt(px(6.)).child(
                crate::components::secondary_button("library-open-retry", "Retry").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.begin_open_store();
                        cx.notify();
                    }),
                ),
            ))
            .into_any_element()
    }

    fn filtered_artists(&self) -> Vec<(String, u64)> {
        filter_artists(&self.artists, &self.artist_search)
    }

    fn is_track_missing(&self, track_id: TrackId, cx: &Context<Self>) -> bool {
        self.row.read(cx).is_track_missing(track_id)
    }
}

impl Render for LibraryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content: AnyElement = match &self.boot {
            LibraryBoot::Failed { message } => self.render_library_failed(message.clone(), cx),
            LibraryBoot::Opening {
                backfill: Some(progress),
            } => render_library_opening(*progress),
            _ => match self.destination {
                Destination::Albums => self.render_albums(window, cx),
                Destination::Tracks => self.render_tracks(window, cx),
                Destination::Playlists => self.render_playlists(cx),
                Destination::Storage => self.render_storage(window, cx),
                Destination::Devices => div().into_any_element(),
            },
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

fn render_library_opening(progress: BackfillProgress) -> AnyElement {
    let fraction = if progress.total == 0 {
        0.0
    } else {
        (progress.processed as f32 / progress.total as f32).clamp(0.0, 1.0)
    };
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
                .child("Updating your library"),
        )
        .child(
            div()
                .font_family(theme::FONT_SANS)
                .text_size(px(13.))
                .text_color(theme::text_secondary())
                .child("Rebuilding track details — this runs once after an update."),
        )
        .child(
            div()
                .relative()
                .w(px(320.))
                .h(px(4.))
                .mt(px(6.))
                .rounded(px(2.))
                .bg(theme::bg_inset())
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(gpui::relative(fraction))
                        .rounded(px(2.))
                        .bg(theme::accent()),
                ),
        )
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .text_size(px(11.))
                .text_color(theme::text_muted())
                .child(format!(
                    "{} of {} files",
                    progress.processed, progress.total
                )),
        )
        .into_any_element()
}

/// Shared paged-list footer: range readout on the left, previous/next and
/// compact page numbers on the right. Tracks and Albums use the same control.
fn render_pagination_footer(
    pagination: &view_model::Pagination,
    noun: &'static str,
    id_prefix: &'static str,
    set_page: fn(&mut LibraryView, usize, &mut Context<LibraryView>),
    cx: &mut Context<LibraryView>,
) -> impl IntoElement {
    use view_model::PaginationItem;

    let (start, end) = pagination
        .range()
        .expect("pagination is only rendered for non-empty results");
    let current_page = pagination.current_page();
    let can_previous = pagination.can_previous();
    let can_next = pagination.can_next();
    let mut controls = div().flex().items_center().gap(px(4.));
    controls = controls.child(
        div()
            .id(format!("{id_prefix}-previous"))
            .flex()
            .items_center()
            .justify_center()
            .size(px(28.))
            .rounded(px(theme::RADIUS_SM))
            .border_1()
            .border_color(theme::border())
            .opacity(if can_previous { 1.0 } else { 0.4 })
            .when(can_previous, |button| {
                button
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        set_page(this, current_page - 1, cx);
                    }))
            })
            .child(
                gpui::svg()
                    .path("icons/chevron-left.svg")
                    .size(px(13.))
                    .text_color(theme::text_muted()),
            ),
    );
    for (index, item) in pagination.items().into_iter().enumerate() {
        controls = controls.child(match item {
            PaginationItem::Page(page) => {
                let active = page == current_page;
                div()
                    .id(format!("{id_prefix}-{}", page + 1))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .rounded(px(theme::RADIUS_SM))
                    .when(active, |button| {
                        button
                            .border_1()
                            .border_color(theme::accent())
                            .bg(theme::accent_soft())
                    })
                    .when(!active, |button| {
                        button
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| set_page(this, page, cx)))
                    })
                    .font_family(theme::FONT_MONO)
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_size(px(10.))
                    .text_color(if active {
                        theme::accent()
                    } else {
                        theme::text_secondary()
                    })
                    .child((page + 1).to_string())
                    .into_any_element()
            }
            PaginationItem::Ellipsis => div()
                .id(format!("{id_prefix}-ellipsis-{index}"))
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.))
                .font_family(theme::FONT_MONO)
                .font_weight(gpui::FontWeight::BOLD)
                .text_size(px(10.))
                .text_color(theme::text_secondary())
                .child("…")
                .into_any_element(),
        });
    }
    controls = controls.child(
        div()
            .id(format!("{id_prefix}-next"))
            .flex()
            .items_center()
            .justify_center()
            .size(px(28.))
            .rounded(px(theme::RADIUS_SM))
            .border_1()
            .border_color(theme::border())
            .opacity(if can_next { 1.0 } else { 0.4 })
            .when(can_next, |button| {
                button
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        set_page(this, current_page + 1, cx);
                    }))
            })
            .child(
                gpui::svg()
                    .path("icons/chevron-right.svg")
                    .size(px(13.))
                    .text_color(theme::text_secondary()),
            ),
    );

    div()
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .h(px(59.))
        .flex_none()
        .px(px(14.))
        .border_t_1()
        .border_color(theme::border())
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .font_weight(gpui::FontWeight::BOLD)
                .text_size(px(9.))
                .text_color(theme::text_muted())
                .child(format!(
                    "{start}–{end} OF {} {noun}",
                    pagination.total_items()
                )),
        )
        .child(controls)
}

/// The standard single-line text input: chrome, caret, cursor, and key
/// routing defined once so a styling change lands in one place. The shell
/// search field is the one exception — it carries real IME plumbing.
fn render_text_input(
    id: &'static str,
    value: String,
    input_focus: &FocusHandle,
    cx: &mut Context<LibraryView>,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .cursor_text()
        .h(px(36.))
        .w_full()
        .px(px(10.))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::accent())
        .bg(theme::bg_inset())
        .track_focus(input_focus)
        .on_key_down(cx.listener(|this, event, _, cx| {
            this.handle_text_input(event, cx);
        }))
        .font_family(theme::FONT_SANS)
        .text_size(px(12.))
        .text_color(theme::text_primary())
        .child(value)
        .child(crate::components::input_caret())
}

/// List-level loading state: shown in place of a list's empty state while the
/// store is still opening, so an untouched library never reads as empty.
fn list_loading_placeholder(label: &'static str) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .w_full()
        .font_family(theme::FONT_SANS)
        .text_size(px(13.))
        .text_color(theme::text_muted())
        .child(label)
        .into_any_element()
}

/// Runtime mark for a library row whose file was gone at play time. Warning
/// styling, not danger: with an offline storage root the absence is expected.
fn missing_file_badge() -> impl IntoElement {
    div()
        .flex_none()
        .font_family(theme::FONT_MONO)
        .font_weight(gpui::FontWeight::BOLD)
        .text_size(px(9.))
        .text_color(theme::warning())
        .child("FILE MISSING")
}

fn current_time_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(
        deleted_files: usize,
        total_files: usize,
        failures: Vec<String>,
        db_error: Option<String>,
    ) -> DeleteAlbumOutcome {
        DeleteAlbumOutcome {
            deleted_ids: Vec::new(),
            deleted_files,
            total_files,
            failures,
            db_error,
        }
    }

    #[test]
    fn delete_album_notice_preserves_partial_state_when_the_reload_also_fails() {
        let notice = delete_album_notice(
            &outcome(3, 5, Vec::new(), Some("no such table".into())),
            Some("no such table: playlist_tracks".into()),
        )
        .unwrap();
        assert!(notice.contains("Deleted 3 of 5 audio files"));
        assert!(notice.contains("Run Delete Album again"));
        assert!(notice.contains("Reloading the library also failed"));
    }

    #[test]
    fn delete_album_notice_keeps_file_failures_alongside_a_db_failure() {
        let notice = delete_album_notice(
            &outcome(
                1,
                3,
                vec!["b.wav: permission denied".into()],
                Some("database is locked".into()),
            ),
            None,
        )
        .unwrap();
        assert!(notice.contains("Deleted 1 of 3 audio files"));
        assert!(notice.contains("database is locked"));
        assert!(notice.contains("b.wav: permission denied"));
    }

    #[test]
    fn delete_album_notice_reports_file_failures_and_clean_runs() {
        let notice =
            delete_album_notice(&outcome(1, 2, vec!["a.wav: offline".into()], None), None).unwrap();
        assert!(notice.contains("Deleted 1 of 2 audio files"));
        assert!(notice.contains("a.wav: offline"));

        assert!(delete_album_notice(&outcome(2, 2, Vec::new(), None), None).is_none());
        assert_eq!(
            delete_album_notice(
                &outcome(2, 2, Vec::new(), None),
                Some("reload broke".into())
            ),
            Some("reload broke".into())
        );
    }

    #[test]
    fn missing_marks_clear_only_after_presence_verifying_scans() {
        let completed = |outcome, removals_suppressed| ScanCompletion::Completed {
            outcome,
            removals_suppressed,
        };
        assert!(scan_verified_presence(&completed(
            ScanOutcome::Completed,
            false
        )));
        assert!(scan_verified_presence(&completed(
            ScanOutcome::CompletedWithErrors,
            false
        )));
        assert!(!scan_verified_presence(&completed(
            ScanOutcome::Completed,
            true
        )));
        assert!(!scan_verified_presence(&completed(
            ScanOutcome::Offline,
            false
        )));
        assert!(!scan_verified_presence(&completed(
            ScanOutcome::Failed,
            false
        )));
        assert!(!scan_verified_presence(&ScanCompletion::Cancelled));
    }

    #[test]
    fn artist_search_filters_case_insensitively_and_empty_matches_all() {
        let artists = vec![
            ("Daft Punk".to_string(), 22_u64),
            ("宇多田ヒカル".to_string(), 14),
            ("Kamasi Washington".to_string(), 17),
        ];
        let names = |search: &str| {
            filter_artists(&artists, search)
                .into_iter()
                .map(|(artist, _)| artist)
                .collect::<Vec<_>>()
        };
        assert_eq!(names("").len(), 3);
        assert_eq!(names("daft"), ["Daft Punk"]);
        assert_eq!(names("  WASH  "), ["Kamasi Washington"]);
        assert_eq!(names("ヒカル"), ["宇多田ヒカル"]);
        assert!(names("zzz").is_empty());
        assert!(filter_artists(&[], "").is_empty());
    }
}
