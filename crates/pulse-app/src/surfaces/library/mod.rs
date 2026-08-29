mod albums;
mod albums_logic;
mod artist_detail;
mod artists;
mod artists_logic;
mod catalog;
mod context_menu;
mod genre_filter;
mod lifecycle;
mod lifecycle_logic;
pub(crate) mod logic;
mod playback_actions;
mod playlist_actions;
mod playlists;
mod playlists_logic;
mod storage;
mod storage_actions;
mod storage_logic;
mod storage_modals;
mod tracks;
mod tracks_logic;

use std::{
    collections::HashSet,
    fs, io,
    ops::Range,
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
    AnyElement, Bounds, Context, ElementInputHandler, Entity, EntityInputHandler, FocusHandle,
    IntoElement, KeyDownEvent, PathPromptOptions, Pixels, Point, Render, ScrollHandle,
    Subscription, UTF16Selection, Window, canvas, div, prelude::*, px,
};
use pulse_engine::PlaybackState;

use crate::{
    app_store::{AppStore, StoreRevisions, global_app_store},
    backend::{
        Album, AlbumSortOrder, Artist, BackfillProgress, DeleteAlbumOutcome, LibraryError,
        LibrarySearchResults, LibraryStore, LibrarySummary, PlaybackAction, PlaylistId,
        PlaylistSummary, PlaylistTrack, ScanHistoryEntry, ScanOutcome, ScanProgress, StorageRoot,
        StorageRootId, Track, TrackId, TrackSortOrder, cover_cache_directory, delete_album_tracks,
        library_database_path, scan_storage_root_cancellable,
    },
    surfaces::Destination,
    text_input::{self, TextInput},
    theme,
    ui::{Button, Scrollbar},
};

use artists_logic::ArtistRoute;
use lifecycle_logic::{
    delete_album_notice, filter_artists, filter_genres, scan_verified_presence, selected_genre,
};
use logic::{self as view_model, FilterChip};

const SCAN_POLL_INTERVAL: Duration = Duration::from_millis(50);
const LIST_PAGE_SIZE: usize = 20;
/// Distance from the album grid's bottom edge that triggers the next page.
const ALBUM_PREFETCH_PX: f32 = 600.;
/// Distance from the track list's bottom edge that triggers the next page.
const TRACK_PREFETCH_PX: f32 = 600.;

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
/// The user-facing report for a finished album delete. Files and rows can
/// diverge (cross-filesystem atomicity is impossible) and the post-delete
/// reload can fail on the same broken database — every part that happened
/// must survive into the one message the user sees.
/// Runtime FILE MISSING marks may be cleared only after a scan that verified
/// presence for every retained row: the walk finished (not cancelled, not
/// offline/failed) and the missing-row removal pass actually ran.
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

struct ArtistDetail {
    artist: Artist,
    albums: Vec<Album>,
    tracks: Vec<Track>,
}

struct PlaylistDetail {
    summary: PlaylistSummary,
    entries: Vec<PlaylistTrack>,
}

struct AddStorageDraft {
    path: Option<PathBuf>,
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
}

pub(crate) struct LibraryView {
    destination: Destination,
    app_store: Entity<AppStore>,
    store_revisions: StoreRevisions,
    playback_source_path: Option<PathBuf>,
    playback_state: PlaybackState,
    missing_track_ids: Arc<HashSet<TrackId>>,
    store: Option<LibraryStore>,
    boot: LibraryBoot,
    database_path: PathBuf,
    cover_cache_directory: PathBuf,
    albums: Vec<Album>,
    artist_index: Vec<Artist>,
    artist_detail: Option<ArtistDetail>,
    artist_route: ArtistRoute,
    tracks: Vec<Track>,
    genres: Vec<(String, u64)>,
    genre_popover_open: bool,
    genre_hint_press_closed_popover: bool,
    artists: Vec<(String, u64)>,
    artist_popover_open: bool,
    artist_hint_press_closed_popover: bool,
    /// Latched when an album page query fails so a persistent error cannot
    /// become a notify/requery loop; cleared by reset or a successful reload.
    album_load_stalled: bool,
    track_load_stalled: bool,
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
    track_total: usize,
    album_total: usize,
    albums_scroll: ScrollHandle,
    artists_scroll: ScrollHandle,
    artist_scrollbar: Entity<Scrollbar>,
    artist_detail_scroll: ScrollHandle,
    album_detail_scroll: ScrollHandle,
    tracks_scroll: ScrollHandle,
    track_scrollbar: Entity<Scrollbar>,
    playlists_scroll: ScrollHandle,
    playlist_detail_scroll: ScrollHandle,
    scan: Option<ActiveScan>,
    worker_tx: Sender<WorkerEvent>,
    worker_rx: Receiver<WorkerEvent>,
    modal: Option<Modal>,
    rename_draft: Option<RenameDraft>,
    track_menu: Option<TrackMenu>,
    playlist_menu: Option<PlaylistMenu>,
    text_input: TextInput,
    input_focus: FocusHandle,
    error: Option<String>,
    _store_subscription: Subscription,
}

impl LibraryView {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let database_path =
            library_database_path().expect("failed to resolve library database path");
        let cover_cache_directory =
            cover_cache_directory().expect("failed to resolve library cover cache path");
        let app_store = global_app_store(cx);
        let store_revisions = app_store.read(cx).revisions;
        let playback = app_store.read(cx).playback_snapshot();
        let (worker_tx, worker_rx) = mpsc::channel();
        let tracks_scroll = ScrollHandle::new();
        let track_scrollbar = cx.new(|_| {
            Scrollbar::new("tracks-scrollbar", tracks_scroll.clone())
                .thumb_id("tracks-scrollbar-thumb")
        });
        let artists_scroll = ScrollHandle::new();
        let artist_scrollbar = cx.new(|_| {
            Scrollbar::new("artists-scrollbar", artists_scroll.clone())
                .thumb_id("artists-scrollbar-thumb")
        });
        let mut view = Self {
            destination: Destination::Albums,
            app_store: app_store.clone(),
            store_revisions,
            playback_source_path: playback.source_path,
            playback_state: playback.playback_state,
            missing_track_ids: playback.missing_track_ids,
            store: None,
            boot: LibraryBoot::Opening { backfill: None },
            database_path,
            cover_cache_directory,
            albums: Vec::new(),
            artist_index: Vec::new(),
            artist_detail: None,
            artist_route: ArtistRoute::default(),
            tracks: Vec::new(),
            genres: Vec::new(),
            genre_popover_open: false,
            genre_hint_press_closed_popover: false,
            artists: Vec::new(),
            artist_popover_open: false,
            artist_hint_press_closed_popover: false,
            album_load_stalled: false,
            track_load_stalled: false,
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
            track_total: 0,
            album_total: 0,
            albums_scroll: ScrollHandle::new(),
            artists_scroll,
            artist_scrollbar,
            artist_detail_scroll: ScrollHandle::new(),
            album_detail_scroll: ScrollHandle::new(),
            tracks_scroll,
            track_scrollbar,
            playlists_scroll: ScrollHandle::new(),
            playlist_detail_scroll: ScrollHandle::new(),
            scan: None,
            worker_tx,
            worker_rx,
            modal: None,
            rename_draft: None,
            track_menu: None,
            playlist_menu: None,
            text_input: TextInput::default(),
            input_focus: cx.focus_handle(),
            error: None,
            _store_subscription: cx.observe(&app_store, |this, _, cx| {
                let revisions = this.app_store.read(cx).revisions;
                let reactions = revisions.reactions_since(this.store_revisions);
                this.store_revisions = revisions;
                if reactions.playback {
                    let playback = this.app_store.read(cx).playback_snapshot();
                    this.playback_source_path = playback.source_path;
                    this.playback_state = playback.playback_state;
                    this.missing_track_ids = playback.missing_track_ids;
                    cx.notify();
                }
            }),
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
}

impl EntityInputHandler for LibraryView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        Some(self.text_input.text_for_range(range, adjusted_range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(self.text_input.selected_text_range())
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.text_input.marked_text_range()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.text_input.unmark_text();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.text_input.replace_text_in_range(range, text);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.text_input
            .replace_and_mark_text_in_range(range, new_text, new_selected_range);
        cx.notify();
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
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.text_input.character_index_utf16())
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
                Destination::Artists => self.render_artists(window, cx),
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

/// The standard single-line text input: chrome, native input registration,
/// caret, selection, and key routing defined once for every library field.
fn render_text_input(
    id: &'static str,
    input: &TextInput,
    input_focus: &FocusHandle,
    cx: &mut Context<LibraryView>,
) -> impl IntoElement {
    let input_entity = cx.entity();
    div()
        .id(id)
        .relative()
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
        .child(text_input::render_text(input, true))
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
