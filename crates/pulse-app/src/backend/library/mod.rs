pub(crate) mod metadata;
mod path;
mod store;
mod walk;

use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub use store::{BackfillProgress, LibraryStore};
use store::{
    CompletedScan, artist_name_key_for_track, clear_track_cover, delete_track, refresh_artist_keys,
    set_track_cover, update_track_path, upsert_track,
};
use thiserror::Error;
use walk::walk_music_files_until;

pub type StorageRootId = i64;
pub type TrackId = i64;
pub type PlaylistId = i64;

pub const UNKNOWN_ALBUM: &str = "Unknown Album";
pub const UNKNOWN_ARTIST: &str = "Unknown Artist";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageRoot {
    pub id: StorageRootId,
    pub path: PathBuf,
    pub display_name: String,
    pub added_at_ms: i64,
    pub last_scan_at_ms: Option<i64>,
    pub is_reachable: bool,
    pub is_case_sensitive: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Track {
    pub id: TrackId,
    pub storage_root_id: StorageRootId,
    pub path: PathBuf,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration_ms: Option<i64>,
    pub sample_rate_hz: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    pub file_size_bytes: u64,
    pub modified_at_ns: i64,
    pub cover_art_path: Option<PathBuf>,
    pub cover_art_mime_type: Option<String>,
    pub added_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrackQueryFilter {
    All,
    HiRes,
    AddedSince(i64),
    Genre(String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TrackPage {
    pub tracks: Vec<Track>,
    pub total_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AlbumQueryFilter {
    All,
    HiRes,
    AddedSince(i64),
    Genre(String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AlbumPage {
    pub albums: Vec<Album>,
    pub total_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub year: Option<u32>,
    pub track_count: u64,
    pub total_duration_ms: u64,
    pub max_sample_rate_hz: Option<u32>,
    pub max_bit_depth: Option<u8>,
    pub cover_art_path: Option<PathBuf>,
    pub latest_added_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Artist {
    pub id: i64,
    pub name: String,
    pub name_key: String,
    pub album_count: u64,
    pub track_count: u64,
    pub total_duration_ms: u64,
    pub earliest_added_ms: i64,
    pub earliest_added_year: Option<u32>,
    pub cover_art_path: Option<PathBuf>,
    pub display_name: Option<String>,
    pub hidden: Option<bool>,
    pub mbid: Option<String>,
    pub photo_path: Option<PathBuf>,
    pub photo_source: Option<String>,
    pub enriched_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaylistSummary {
    pub playlist: Playlist,
    pub track_count: u64,
    pub total_duration_ms: u64,
    pub cover_art_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaylistTrack {
    pub position: usize,
    pub track: Track,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LibrarySearchResults {
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
    pub playlists: Vec<PlaylistSummary>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AlbumSortOrder {
    #[default]
    Title,
    Artist,
    DateAdded,
    ReleaseYear,
    Duration,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TrackSortOrder {
    #[default]
    Title,
    Artist,
    Album,
    DateAdded,
    ReleaseYear,
    Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanOutcome {
    Completed,
    CompletedWithErrors,
    Offline,
    Failed,
}

impl ScanOutcome {
    fn as_db_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::CompletedWithErrors => "completed_with_errors",
            Self::Offline => "offline",
            Self::Failed => "failed",
        }
    }

    fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "completed" => Some(Self::Completed),
            "completed_with_errors" => Some(Self::CompletedWithErrors),
            "offline" => Some(Self::Offline),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanFileError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanReport {
    pub scan_id: i64,
    pub storage_root_id: StorageRootId,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub discovered: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unsupported: usize,
    pub skipped: usize,
    pub removals_suppressed: bool,
    pub errors: Vec<ScanFileError>,
    pub outcome: ScanOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanProgressAction {
    Added,
    Updated,
    Unsupported,
    Skipped,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScanProgress {
    Discovering {
        discovered_files: usize,
        current_path: PathBuf,
    },
    Processing {
        processed_files: usize,
        total_files: usize,
        current_path: PathBuf,
        action: ScanProgressAction,
    },
    Finished {
        outcome: ScanOutcome,
        added: usize,
        updated: usize,
        removed: usize,
        unsupported: usize,
        skipped: usize,
        removals_suppressed: bool,
        error_count: usize,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LibrarySummary {
    pub album_count: u64,
    pub track_count: u64,
    pub file_size_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanHistoryEntry {
    pub id: i64,
    pub storage_root_id: StorageRootId,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub added_count: u64,
    pub updated_count: u64,
    pub removed_count: u64,
    pub unsupported_count: u64,
    pub error_count: u64,
    pub removals_suppressed: bool,
    pub outcome: Option<ScanOutcome>,
    pub error_message: Option<String>,
}

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("SQLite error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("I/O error at {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("path is not valid Unicode: {}", .0.display())]
    NonUnicodePath(PathBuf),
    #[error("storage root is not a directory: {}", .0.display())]
    NotDirectory(PathBuf),
    #[error("unsupported library schema version {0}")]
    UnsupportedSchemaVersion(i64),
    #[error("schema migration found rows violating foreign keys in table {0}")]
    MigrationIntegrity(String),
    #[error("storage root {0} was not found")]
    StorageRootNotFound(StorageRootId),
    #[error("playlist {0} was not found")]
    PlaylistNotFound(PlaylistId),
    #[error("playlist {playlist_id} has no entry at position {position}")]
    PlaylistEntryNotFound {
        playlist_id: PlaylistId,
        position: usize,
    },
    #[error("file modified time is too large to store: {}", .0.display())]
    FileTimestampOutOfRange(PathBuf),
    #[error("{0} is too large to store")]
    IntegerOutOfRange(&'static str),
}

pub fn scan_storage_root<F>(
    store: &mut LibraryStore,
    storage_root_id: StorageRootId,
    cover_cache_directory: impl AsRef<Path>,
    on_progress: F,
) -> Result<ScanReport, LibraryError>
where
    F: FnMut(ScanProgress),
{
    scan_storage_root_until(
        store,
        storage_root_id,
        cover_cache_directory,
        on_progress,
        || false,
    )
    .map(|report| report.expect("non-cancellable scan cannot be cancelled"))
}

pub(crate) fn scan_storage_root_cancellable<F, C>(
    store: &mut LibraryStore,
    storage_root_id: StorageRootId,
    cover_cache_directory: impl AsRef<Path>,
    on_progress: F,
    is_cancelled: C,
) -> Result<Option<ScanReport>, LibraryError>
where
    F: FnMut(ScanProgress),
    C: FnMut() -> bool,
{
    scan_storage_root_until(
        store,
        storage_root_id,
        cover_cache_directory,
        on_progress,
        is_cancelled,
    )
}

fn scan_storage_root_until<F, C>(
    store: &mut LibraryStore,
    storage_root_id: StorageRootId,
    cover_cache_directory: impl AsRef<Path>,
    mut on_progress: F,
    mut is_cancelled: C,
) -> Result<Option<ScanReport>, LibraryError>
where
    F: FnMut(ScanProgress),
    C: FnMut() -> bool,
{
    let root = store
        .storage_root(storage_root_id)?
        .ok_or(LibraryError::StorageRootNotFound(storage_root_id))?;
    let started_at_ms = system_time_ms(SystemTime::now())?;
    let scan_id = store.begin_scan(storage_root_id, started_at_ms)?;

    on_progress(ScanProgress::Discovering {
        discovered_files: 0,
        current_path: root.path.clone(),
    });

    let walk = match walk_music_files_until(
        &root.path,
        root.is_case_sensitive,
        |discovered_files, current_path| {
            on_progress(ScanProgress::Discovering {
                discovered_files,
                current_path: current_path.to_path_buf(),
            });
        },
        &mut is_cancelled,
    ) {
        Ok(Some(walk)) => walk,
        Ok(None) => {
            store.cancel_scan(scan_id)?;
            return Ok(None);
        }
        Err(error) => {
            return finish_offline_scan(
                store,
                scan_id,
                &root,
                started_at_ms,
                error,
                &mut on_progress,
            )
            .map(Some);
        }
    };

    if let Err(error) = store.mark_root_reachable(storage_root_id) {
        return finish_fatal_scan(store, scan_id, &root, error, &mut on_progress).map(Some);
    }

    match apply_reachable_scan(
        store,
        (scan_id, started_at_ms),
        &root,
        walk,
        cover_cache_directory.as_ref(),
        &mut on_progress,
        &mut is_cancelled,
    ) {
        Ok(report) => Ok(report),
        Err(error) => finish_fatal_scan(store, scan_id, &root, error, &mut on_progress).map(Some),
    }
}

fn apply_reachable_scan<F, C>(
    store: &mut LibraryStore,
    scan: (i64, i64),
    root: &StorageRoot,
    walk: walk::WalkResult,
    cover_cache_directory: &Path,
    on_progress: &mut F,
    is_cancelled: &mut C,
) -> Result<Option<ScanReport>, LibraryError>
where
    F: FnMut(ScanProgress),
    C: FnMut() -> bool,
{
    let (scan_id, started_at_ms) = scan;
    let existing = store.existing_tracks(root.id)?;
    let seen = walk
        .files
        .iter()
        .map(|file| file.path_key.clone())
        .collect::<HashSet<_>>();
    let removals_suppressed = !walk.errors.is_empty();
    let mut errors = walk
        .errors
        .into_iter()
        .map(|error| ScanFileError {
            path: error.path,
            message: error.message,
        })
        .collect::<Vec<_>>();
    let total_files = walk.files.len();
    let mut added = 0;
    let mut updated = 0;
    let mut removed = 0;
    let mut unsupported = 0;
    let mut skipped = 0;

    for (index, file) in walk.files.into_iter().enumerate() {
        if is_cancelled() {
            store.cancel_scan(scan_id)?;
            return Ok(None);
        }
        let current = existing.get(&file.path_key);
        let now_ms = system_time_ms(SystemTime::now())?;
        let mut action = ScanProgressAction::Skipped;

        if let Some(current) = current.filter(|track| track.modified_at_ns == file.modified_at_ns) {
            let mut changed = false;
            let mut artwork_failed = false;
            if current.path_text != file.path_text {
                let transaction = store.connection.transaction()?;
                update_track_path(&transaction, current.id, &file.path_text, now_ms)?;
                transaction.commit()?;
                changed = true;
            }
            if current.cover_art_path.is_none() {
                match discover_folder_artwork(&file.path) {
                    Ok(Some(artwork)) => {
                        let transaction = store.connection.transaction()?;
                        match cache_artwork(
                            &transaction,
                            cover_cache_directory,
                            current.id,
                            artwork,
                        ) {
                            Ok(_) => {
                                refresh_track_artist_keys(
                                    &transaction,
                                    None,
                                    Some(current.id),
                                    now_ms,
                                )?;
                                transaction.commit()?;
                                changed = true;
                            }
                            Err(ArtworkError::Cache(message)) => {
                                transaction.commit()?;
                                errors.push(ScanFileError {
                                    path: file.path.clone(),
                                    message,
                                });
                                artwork_failed = true;
                            }
                            Err(ArtworkError::Database(error)) => {
                                drop(transaction);
                                return Err(error);
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(ArtworkError::Cache(message)) => {
                        errors.push(ScanFileError {
                            path: file.path.clone(),
                            message,
                        });
                        artwork_failed = true;
                    }
                    Err(ArtworkError::Database(error)) => return Err(error),
                }
            }
            if changed {
                updated += 1;
                action = ScanProgressAction::Updated;
            } else if !artwork_failed {
                skipped += 1;
            }
            if artwork_failed {
                action = ScanProgressAction::Failed;
            }
        } else {
            match metadata::extract_metadata(&file.path) {
                Ok(metadata) => {
                    let previous_cover = current
                        .and_then(|track| track.cover_art_path.as_ref())
                        .cloned();
                    let transaction = store.connection.transaction()?;
                    let previous_artist_key = match current {
                        Some(current) => artist_name_key_for_track(&transaction, current.id)?,
                        None => None,
                    };
                    let track_id = upsert_track(&transaction, root.id, &file, &metadata, now_ms)?;
                    let artwork = match metadata.artwork {
                        Some(artwork) => Ok(Some(artwork)),
                        None => discover_folder_artwork(&file.path),
                    };
                    let artwork_result = artwork.and_then(|artwork| match artwork {
                        Some(artwork) => {
                            cache_artwork(&transaction, cover_cache_directory, track_id, artwork)
                                .map(Some)
                        }
                        None => clear_track_cover(&transaction, track_id)
                            .map(|()| None)
                            .map_err(ArtworkError::Database),
                    });
                    match artwork_result {
                        Ok(new_cover) => {
                            refresh_track_artist_keys(
                                &transaction,
                                previous_artist_key.as_deref(),
                                Some(track_id),
                                now_ms,
                            )?;
                            transaction.commit()?;
                            if current.is_some() {
                                updated += 1;
                                action = ScanProgressAction::Updated;
                            } else {
                                added += 1;
                                action = ScanProgressAction::Added;
                            }
                            let stale_cover = previous_cover
                                .filter(|previous| new_cover.as_ref() != Some(previous));
                            if let Some(path) = stale_cover
                                && let Err(error) = remove_cached_file(&path)
                            {
                                errors.push(ScanFileError {
                                    path: file.path.clone(),
                                    message: format!("failed to remove cached cover art: {error}"),
                                });
                                action = ScanProgressAction::Failed;
                            }
                        }
                        Err(ArtworkError::Cache(message)) => {
                            refresh_track_artist_keys(
                                &transaction,
                                previous_artist_key.as_deref(),
                                Some(track_id),
                                now_ms,
                            )?;
                            transaction.commit()?;
                            if current.is_some() {
                                updated += 1;
                            } else {
                                added += 1;
                            }
                            errors.push(ScanFileError {
                                path: file.path.clone(),
                                message,
                            });
                            action = ScanProgressAction::Failed;
                        }
                        Err(ArtworkError::Database(error)) => {
                            drop(transaction);
                            return Err(error);
                        }
                    }
                }
                Err(metadata::MetadataError::Unsupported(_)) => {
                    unsupported += 1;
                    action = ScanProgressAction::Unsupported;
                    if let Some(current) = current {
                        let transaction = store.connection.transaction()?;
                        let artist_key = artist_name_key_for_track(&transaction, current.id)?;
                        delete_track(&transaction, current.id)?;
                        if let Some(artist_key) = artist_key {
                            refresh_artist_keys(&transaction, &[artist_key], now_ms)?;
                        }
                        transaction.commit()?;
                        removed += 1;
                        if let Some(path) = &current.cover_art_path
                            && let Err(error) = remove_cached_file(path)
                        {
                            errors.push(ScanFileError {
                                path: file.path.clone(),
                                message: format!("failed to remove cached cover art: {error}"),
                            });
                            action = ScanProgressAction::Failed;
                        }
                    }
                }
                Err(error) => {
                    errors.push(ScanFileError {
                        path: file.path.clone(),
                        message: error.to_string(),
                    });
                    action = ScanProgressAction::Failed;
                }
            }
        }

        on_progress(ScanProgress::Processing {
            processed_files: index + 1,
            total_files,
            current_path: file.path,
            action,
        });
    }

    if is_cancelled() {
        store.cancel_scan(scan_id)?;
        return Ok(None);
    }

    if !removals_suppressed {
        let missing = existing
            .iter()
            .filter(|(path_key, _)| !seen.contains(*path_key))
            .map(|(_, track)| track)
            .collect::<Vec<_>>();
        let refreshed_at_ms = system_time_ms(SystemTime::now())?;
        let transaction = store.connection.transaction()?;
        let mut artist_keys = Vec::new();
        for track in &missing {
            if let Some(artist_key) = artist_name_key_for_track(&transaction, track.id)? {
                artist_keys.push(artist_key);
            }
            delete_track(&transaction, track.id)?;
        }
        refresh_artist_keys(&transaction, &artist_keys, refreshed_at_ms)?;
        transaction.commit()?;
        removed += missing.len();

        for track in missing {
            if let Some(path) = &track.cover_art_path
                && let Err(error) = remove_cached_file(path)
            {
                errors.push(ScanFileError {
                    path: track.path.clone(),
                    message: format!("failed to remove cached cover art: {error}"),
                });
            }
        }
    }

    let finished_at_ms = system_time_ms(SystemTime::now())?;
    let outcome = if errors.is_empty() {
        ScanOutcome::Completed
    } else {
        ScanOutcome::CompletedWithErrors
    };
    store.finish_completed_scan(
        scan_id,
        root.id,
        &CompletedScan {
            finished_at_ms,
            added,
            updated,
            removed,
            unsupported,
            errors: errors.len(),
            removals_suppressed,
            outcome,
        },
    )?;

    let report = ScanReport {
        scan_id,
        storage_root_id: root.id,
        started_at_ms,
        finished_at_ms,
        discovered: total_files,
        added,
        updated,
        removed,
        unsupported,
        skipped,
        removals_suppressed,
        errors,
        outcome,
    };
    on_progress(finished_progress(&report));
    Ok(Some(report))
}

fn refresh_track_artist_keys(
    transaction: &rusqlite::Transaction<'_>,
    previous_name_key: Option<&str>,
    track_id: Option<TrackId>,
    refreshed_at_ms: i64,
) -> Result<(), LibraryError> {
    let mut name_keys = previous_name_key
        .map(str::to_string)
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(track_id) = track_id
        && let Some(name_key) = artist_name_key_for_track(transaction, track_id)?
    {
        name_keys.push(name_key);
    }
    refresh_artist_keys(transaction, &name_keys, refreshed_at_ms)
}

/// Cover cache paths are content-unique (`{id}-{fingerprint}.cover`): the
/// bytes behind a given path never change, so path-keyed image caches (GPUI's)
/// can never show stale art, including after a removed root recycles track
/// ids. Returns the cached path so callers can delete a superseded one.
fn cache_artwork(
    transaction: &rusqlite::Transaction<'_>,
    cover_cache_directory: &Path,
    track_id: TrackId,
    artwork: metadata::EmbeddedArtwork,
) -> Result<PathBuf, ArtworkError> {
    fs::create_dir_all(cover_cache_directory).map_err(|error| {
        ArtworkError::Cache(format!(
            "failed to create cover cache {}: {error}",
            cover_cache_directory.display()
        ))
    })?;
    let file_name = format!(
        "{track_id}-{:016x}.cover",
        artwork_fingerprint(&artwork.data)
    );
    let path = cover_cache_directory.join(&file_name);
    let path_text = path.to_str().ok_or_else(|| {
        ArtworkError::Cache(format!(
            "cover cache path is not valid Unicode: {}",
            path.display()
        ))
    })?;
    let temporary_path = cover_cache_directory.join(format!(".{file_name}.tmp"));
    fs::write(&temporary_path, artwork.data).map_err(|error| {
        ArtworkError::Cache(format!(
            "failed to stage cover art at {}: {error}",
            temporary_path.display()
        ))
    })?;
    if let Err(error) = fs::rename(&temporary_path, &path) {
        let _ = remove_cached_file(&temporary_path);
        return Err(ArtworkError::Cache(format!(
            "failed to replace cover art at {}: {error}",
            path.display()
        )));
    }
    set_track_cover(
        transaction,
        track_id,
        path_text,
        artwork.mime_type.as_deref(),
    )
    .map_err(ArtworkError::Database)?;
    Ok(path)
}

/// FNV-1a, deterministic across runs so an unchanged cover keeps its path.
fn artwork_fingerprint(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn discover_folder_artwork(path: &Path) -> Result<Option<metadata::EmbeddedArtwork>, ArtworkError> {
    metadata::folder_artwork(path).map_err(|error| {
        ArtworkError::Cache(format!(
            "failed to read folder art beside {}: {error}",
            path.display()
        ))
    })
}

#[derive(Debug)]
enum ArtworkError {
    Cache(String),
    Database(LibraryError),
}

fn remove_cached_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn finish_offline_scan<F>(
    store: &mut LibraryStore,
    scan_id: i64,
    root: &StorageRoot,
    started_at_ms: i64,
    error: io::Error,
    on_progress: &mut F,
) -> Result<ScanReport, LibraryError>
where
    F: FnMut(ScanProgress),
{
    let finished_at_ms = system_time_ms(SystemTime::now())?;
    let message = error.to_string();
    store.finish_offline_scan(scan_id, root.id, finished_at_ms, &message)?;
    let report = ScanReport {
        scan_id,
        storage_root_id: root.id,
        started_at_ms,
        finished_at_ms,
        discovered: 0,
        added: 0,
        updated: 0,
        removed: 0,
        unsupported: 0,
        skipped: 0,
        removals_suppressed: false,
        errors: vec![ScanFileError {
            path: root.path.clone(),
            message,
        }],
        outcome: ScanOutcome::Offline,
    };
    on_progress(finished_progress(&report));
    Ok(report)
}

fn finish_fatal_scan<F>(
    store: &mut LibraryStore,
    scan_id: i64,
    root: &StorageRoot,
    error: LibraryError,
    on_progress: &mut F,
) -> Result<ScanReport, LibraryError>
where
    F: FnMut(ScanProgress),
{
    let finished_at_ms = system_time_ms(SystemTime::now())?;
    store.finish_failed_scan(scan_id, root.id, finished_at_ms, &error.to_string())?;
    on_progress(ScanProgress::Finished {
        outcome: ScanOutcome::Failed,
        added: 0,
        updated: 0,
        removed: 0,
        unsupported: 0,
        skipped: 0,
        removals_suppressed: false,
        error_count: 1,
    });
    Err(error)
}

fn finished_progress(report: &ScanReport) -> ScanProgress {
    ScanProgress::Finished {
        outcome: report.outcome,
        added: report.added,
        updated: report.updated,
        removed: report.removed,
        unsupported: report.unsupported,
        skipped: report.skipped,
        removals_suppressed: report.removals_suppressed,
        error_count: report.errors.len(),
    }
}

fn system_time_ms(time: SystemTime) -> Result<i64, LibraryError> {
    system_time_units(time, 1_000)
}

fn system_time_ns(time: SystemTime) -> Result<i64, LibraryError> {
    system_time_units(time, 1_000_000_000)
}

/// Attempt to delete each track's audio file from disk. A missing file
/// counts as deleted only while its storage root is reachable — an offline
/// or unmounted root yields the same NotFound for every child, and treating
/// that as success would drop rows for audio that still exists on the
/// disconnected volume. Returns the ids whose files are verifiably gone
/// (safe to remove from the library) and a message per file that could not
/// be deleted; those tracks keep their rows so the library never claims a
/// deletion that did not happen.
pub fn delete_track_files(
    tracks: &[Track],
    root_reachable: impl Fn(StorageRootId) -> bool,
) -> (Vec<TrackId>, Vec<String>) {
    let mut deleted = Vec::new();
    let mut failures = Vec::new();
    for track in tracks {
        match fs::remove_file(&track.path) {
            Ok(()) => deleted.push(track.id),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if root_reachable(track.storage_root_id) {
                    deleted.push(track.id);
                } else {
                    failures.push(format!("{}: storage root is offline", track.path.display()));
                }
            }
            Err(error) => failures.push(format!("{}: {error}", track.path.display())),
        }
    }
    (deleted, failures)
}

/// What `delete_album_tracks` accomplished. File deletion and the database
/// update cannot be atomic across the filesystem boundary, so the outcome
/// reports each side separately: files already unlinked stay unlinked even
/// when the row cleanup fails, and the caller must say so.
pub struct DeleteAlbumOutcome {
    pub deleted_ids: Vec<TrackId>,
    pub deleted_files: usize,
    pub total_files: usize,
    pub failures: Vec<String>,
    pub db_error: Option<String>,
}

/// Delete an album: unlink its audio files, remove the rows and playlist
/// entries for verifiably-gone files, then clean up their cover-cache
/// entries. The database write is preflighted before the first unlink so a
/// locked or read-only library fails the whole operation while everything
/// still exists. Runs on a worker thread — file I/O against music storage
/// never runs on the UI thread.
pub fn delete_album_tracks(
    store: &mut LibraryStore,
    artist: &str,
    title: &str,
) -> Result<DeleteAlbumOutcome, LibraryError> {
    let tracks = store.tracks_for_album(artist, title)?;
    let roots = store
        .storage_roots()?
        .into_iter()
        .map(|root| (root.id, root.path))
        .collect::<std::collections::HashMap<_, _>>();
    store.preflight_write()?;
    let (deleted_ids, mut failures) = delete_track_files(&tracks, |root_id| {
        roots.get(&root_id).is_some_and(|path| path.exists())
    });
    let db_error = if deleted_ids.is_empty() {
        None
    } else {
        store
            .delete_tracks(&deleted_ids)
            .err()
            .map(|error| error.to_string())
    };
    if db_error.is_none() {
        for track in &tracks {
            if deleted_ids.contains(&track.id)
                && let Some(cover) = &track.cover_art_path
                && let Err(error) = fs::remove_file(cover)
                && error.kind() != io::ErrorKind::NotFound
            {
                failures.push(format!("cover {}: {error}", cover.display()));
            }
        }
    }
    Ok(DeleteAlbumOutcome {
        deleted_files: deleted_ids.len(),
        total_files: tracks.len(),
        deleted_ids,
        failures,
        db_error,
    })
}

fn system_time_units(time: SystemTime, units_per_second: u128) -> Result<i64, LibraryError> {
    let units = |duration: std::time::Duration| {
        u128::from(duration.as_secs())
            .saturating_mul(units_per_second)
            .saturating_add(
                u128::from(duration.subsec_nanos()).saturating_mul(units_per_second)
                    / 1_000_000_000,
            )
    };
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            i64::try_from(units(duration)).map_err(|_| LibraryError::IntegerOutOfRange("timestamp"))
        }
        Err(error) => i64::try_from(units(error.duration()))
            .map(|value| -value)
            .map_err(|_| LibraryError::IntegerOutOfRange("timestamp")),
    }
}

#[cfg(test)]
mod tests;
