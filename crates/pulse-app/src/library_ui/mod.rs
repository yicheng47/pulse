mod albums;
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
    AnyElement, Context, Entity, FocusHandle, IntoElement, KeyDownEvent, PathPromptOptions, Render,
    ScrollHandle, Window, div, prelude::*, px,
};

use crate::{
    library::{
        Album, AlbumSortOrder, LibraryError, LibraryStore, LibrarySummary, ScanHistoryEntry,
        ScanProgress, StorageRoot, StorageRootId, Track, TrackId, TrackSortOrder,
        scan_storage_root_cancellable,
    },
    playback_row::PlaybackRow,
    preferences,
    shell::Destination,
    theme,
};

use view_model::FilterChip;

const SCAN_POLL_INTERVAL: Duration = Duration::from_millis(50);

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

struct AddStorageDraft {
    path: PathBuf,
    display_name: String,
    scan_now: bool,
}

enum Modal {
    AddStorage(AddStorageDraft),
    RemoveStorage {
        root_id: StorageRootId,
        display_name: String,
    },
}

struct RenameDraft {
    root_id: StorageRootId,
    display_name: String,
}

pub(crate) struct LibraryView {
    destination: Destination,
    row: Entity<PlaybackRow>,
    store: Option<LibraryStore>,
    cover_cache_directory: PathBuf,
    albums: Vec<Album>,
    tracks: Vec<Track>,
    genres: Vec<String>,
    catalog_summary: LibrarySummary,
    roots: Vec<StorageRootView>,
    selected_root_id: Option<StorageRootId>,
    album_detail: Option<AlbumDetail>,
    selected_album_track_id: Option<TrackId>,
    selected_track_id: Option<TrackId>,
    album_sort: AlbumSortOrder,
    albums_as_grid: bool,
    track_sort: TrackSortOrder,
    album_filter: FilterChip,
    track_filter: FilterChip,
    artist_filter: Option<String>,
    albums_scroll: ScrollHandle,
    album_detail_scroll: ScrollHandle,
    tracks_scroll: ScrollHandle,
    scan: Option<ActiveScan>,
    scan_tx: Sender<ScanWorkerEvent>,
    scan_rx: Receiver<ScanWorkerEvent>,
    modal: Option<Modal>,
    rename_draft: Option<RenameDraft>,
    input_focus: FocusHandle,
    error: Option<String>,
}

impl LibraryView {
    pub(crate) fn new(row: Entity<PlaybackRow>, cx: &mut Context<Self>) -> Self {
        let database_path =
            preferences::library_database_path().expect("failed to resolve library database path");
        let cover_cache_directory = preferences::cover_cache_directory()
            .expect("failed to resolve library cover cache path");
        let store = LibraryStore::open(database_path).expect("failed to open the music library");
        let (scan_tx, scan_rx) = mpsc::channel();
        let mut view = Self {
            destination: Destination::Albums,
            row,
            store: Some(store),
            cover_cache_directory,
            albums: Vec::new(),
            tracks: Vec::new(),
            genres: Vec::new(),
            catalog_summary: LibrarySummary::default(),
            roots: Vec::new(),
            selected_root_id: None,
            album_detail: None,
            selected_album_track_id: None,
            selected_track_id: None,
            album_sort: AlbumSortOrder::DateAdded,
            albums_as_grid: true,
            track_sort: TrackSortOrder::DateAdded,
            album_filter: FilterChip::All,
            track_filter: FilterChip::All,
            artist_filter: None,
            albums_scroll: ScrollHandle::new(),
            album_detail_scroll: ScrollHandle::new(),
            tracks_scroll: ScrollHandle::new(),
            scan: None,
            scan_tx,
            scan_rx,
            modal: None,
            rename_draft: None,
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
        let tracks = store.all_tracks(self.track_sort)?;
        let genres = store.genres()?;
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
        self.tracks = tracks;
        self.genres = genres;
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
        self.reload_or_show_error();
        cx.notify();
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

    fn play_path(
        &mut self,
        path: PathBuf,
        cover_art_path: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.row.update(cx, |row, cx| {
            row.play_library_path(path, cover_art_path, cx)
        });
    }

    fn select_path(
        &mut self,
        path: PathBuf,
        cover_art_path: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.row.update(cx, |row, cx| {
            row.select_library_path(path, cover_art_path, cx)
        });
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
                            this.modal = Some(Modal::AddStorage(AddStorageDraft {
                                path,
                                display_name,
                                scan_now: true,
                            }));
                            this.error = None;
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
        let store = self.store.as_mut().expect("store availability checked");
        match store.add_storage_root(&draft.path, draft.display_name.trim()) {
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
                if self.rename_draft.is_some() {
                    self.commit_rename_storage(cx);
                } else if matches!(self.modal, Some(Modal::AddStorage(_))) {
                    self.confirm_add_storage(cx);
                }
                return;
            }
            "escape" => {
                if self.rename_draft.take().is_none() {
                    self.modal = None;
                }
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
        if let Some(Modal::AddStorage(draft)) = &mut self.modal {
            return Some(&mut draft.display_name);
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
