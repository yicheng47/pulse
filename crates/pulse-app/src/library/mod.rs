mod metadata;
mod path;
mod store;
mod walk;

use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub use store::LibraryStore;
use store::{
    CompletedScan, clear_track_cover, delete_track, finish_completed_scan, set_track_cover,
    update_track_path, upsert_track,
};
use thiserror::Error;
use walk::walk_music_files;

pub type StorageRootId = i64;
pub type TrackId = i64;

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
    #[error("storage root {0} was not found")]
    StorageRootNotFound(StorageRootId),
    #[error("file modified time is too large to store: {}", .0.display())]
    FileTimestampOutOfRange(PathBuf),
    #[error("{0} is too large to store")]
    IntegerOutOfRange(&'static str),
}

pub fn scan_storage_root<F>(
    store: &mut LibraryStore,
    storage_root_id: StorageRootId,
    cover_cache_directory: impl AsRef<Path>,
    mut on_progress: F,
) -> Result<ScanReport, LibraryError>
where
    F: FnMut(ScanProgress),
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

    let walk = match walk_music_files(
        &root.path,
        root.is_case_sensitive,
        |discovered_files, current_path| {
            on_progress(ScanProgress::Discovering {
                discovered_files,
                current_path: current_path.to_path_buf(),
            });
        },
    ) {
        Ok(walk) => walk,
        Err(error) => {
            return finish_offline_scan(
                store,
                scan_id,
                &root,
                started_at_ms,
                error,
                &mut on_progress,
            );
        }
    };

    if let Err(error) = store.mark_root_reachable(storage_root_id) {
        return finish_fatal_scan(store, scan_id, &root, error, &mut on_progress);
    }

    match apply_reachable_scan(
        store,
        scan_id,
        &root,
        started_at_ms,
        walk,
        cover_cache_directory.as_ref(),
        &mut on_progress,
    ) {
        Ok(report) => Ok(report),
        Err(error) => finish_fatal_scan(store, scan_id, &root, error, &mut on_progress),
    }
}

fn apply_reachable_scan<F>(
    store: &mut LibraryStore,
    scan_id: i64,
    root: &StorageRoot,
    started_at_ms: i64,
    walk: walk::WalkResult,
    cover_cache_directory: &Path,
    on_progress: &mut F,
) -> Result<ScanReport, LibraryError>
where
    F: FnMut(ScanProgress),
{
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
        let current = existing.get(&file.path_key);
        let now_ms = system_time_ms(SystemTime::now())?;
        let mut action = ScanProgressAction::Skipped;

        if let Some(current) = current.filter(|track| track.modified_at_ns == file.modified_at_ns) {
            if current.path_text == file.path_text {
                skipped += 1;
            } else {
                let transaction = store.connection.transaction()?;
                update_track_path(&transaction, current.id, &file.path_text, now_ms)?;
                transaction.commit()?;
                updated += 1;
                action = ScanProgressAction::Updated;
            }
        } else {
            match metadata::extract_metadata(&file.path) {
                Ok(metadata) => {
                    let previous_cover = current
                        .and_then(|track| track.cover_art_path.as_ref())
                        .cloned();
                    let has_artwork = metadata.artwork.is_some();
                    let transaction = store.connection.transaction()?;
                    let track_id = upsert_track(&transaction, root.id, &file, &metadata, now_ms)?;
                    let artwork_result = match metadata.artwork {
                        Some(artwork) => {
                            cache_artwork(&transaction, cover_cache_directory, track_id, artwork)
                        }
                        None => clear_track_cover(&transaction, track_id)
                            .map_err(ArtworkError::Database),
                    };
                    match artwork_result {
                        Ok(()) => {
                            transaction.commit()?;
                            if current.is_some() {
                                updated += 1;
                                action = ScanProgressAction::Updated;
                            } else {
                                added += 1;
                                action = ScanProgressAction::Added;
                            }
                            if !has_artwork
                                && let Some(path) = previous_cover
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
                        delete_track(&transaction, current.id)?;
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

    if !removals_suppressed {
        let missing = existing
            .iter()
            .filter(|(path_key, _)| !seen.contains(*path_key))
            .map(|(_, track)| track)
            .collect::<Vec<_>>();
        let transaction = store.connection.transaction()?;
        for track in &missing {
            delete_track(&transaction, track.id)?;
        }
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
    let transaction = store.connection.transaction()?;
    finish_completed_scan(
        &transaction,
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
    transaction.commit()?;

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
    Ok(report)
}

fn cache_artwork(
    transaction: &rusqlite::Transaction<'_>,
    cover_cache_directory: &Path,
    track_id: TrackId,
    artwork: metadata::EmbeddedArtwork,
) -> Result<(), ArtworkError> {
    fs::create_dir_all(cover_cache_directory).map_err(|error| {
        ArtworkError::Cache(format!(
            "failed to create cover cache {}: {error}",
            cover_cache_directory.display()
        ))
    })?;
    let path = cover_cache_directory.join(format!("{track_id}.cover"));
    let path_text = path.to_str().ok_or_else(|| {
        ArtworkError::Cache(format!(
            "cover cache path is not valid Unicode: {}",
            path.display()
        ))
    })?;
    let temporary_path = cover_cache_directory.join(format!(".{track_id}.cover.tmp"));
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
    .map_err(ArtworkError::Database)
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
mod tests {
    use std::{
        env, fs,
        time::{Duration, Instant},
    };

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn scan_is_incremental_removes_missing_tracks_and_preserves_offline_roots() {
        let temp = tempdir().unwrap();
        let music = temp.path().join("music");
        let cache = temp.path().join("covers");
        fs::create_dir(&music).unwrap();
        let track_path = music.join("Café.wav");
        metadata::write_test_wav(&track_path, "First", "Artist", "Album").unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(&music, "Music").unwrap();

        let mut progress = Vec::new();
        let first =
            scan_storage_root(&mut store, root.id, &cache, |event| progress.push(event)).unwrap();
        assert_eq!(first.outcome, ScanOutcome::Completed);
        assert_eq!(
            (first.added, first.updated, first.removed, first.skipped),
            (1, 0, 0, 0)
        );
        assert!(matches!(
            progress.last(),
            Some(ScanProgress::Finished {
                outcome: ScanOutcome::Completed,
                ..
            })
        ));

        let second = scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
        assert_eq!(
            (second.added, second.updated, second.removed, second.skipped),
            (0, 0, 0, 1)
        );

        let previous_modified = fs::metadata(&track_path).unwrap().modified().unwrap();
        metadata::write_test_wav(&track_path, "Updated", "Artist", "Album").unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&track_path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(previous_modified + Duration::from_secs(2)),
            )
            .unwrap();
        let third = scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
        assert_eq!(
            (third.added, third.updated, third.removed, third.skipped),
            (0, 1, 0, 0)
        );
        assert_eq!(
            store.tracks_for_root(root.id).unwrap()[0].title.as_deref(),
            Some("Updated")
        );

        fs::remove_file(&track_path).unwrap();
        let fourth = scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
        assert_eq!((fourth.added, fourth.updated, fourth.removed), (0, 0, 1));
        assert_eq!(store.root_summary(root.id).unwrap().track_count, 0);

        metadata::write_test_wav(&track_path, "Returned", "Artist", "Album").unwrap();
        scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
        let offline_path = temp.path().join("music-offline");
        fs::rename(&music, &offline_path).unwrap();
        let offline = scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();

        assert_eq!(offline.outcome, ScanOutcome::Offline);
        assert_eq!(store.tracks_for_root(root.id).unwrap().len(), 1);
        assert!(!store.storage_root(root.id).unwrap().unwrap().is_reachable);
        assert_eq!(
            store.recent_scans(root.id, 1).unwrap()[0].outcome,
            Some(ScanOutcome::Offline)
        );
    }

    #[test]
    fn a_file_vanishing_after_the_walk_is_a_per_file_error() {
        let temp = tempdir().unwrap();
        let music = temp.path().join("music");
        fs::create_dir(&music).unwrap();
        let vanished = music.join("vanished.wav");
        let valid = music.join("valid.wav");
        metadata::write_test_wav(&vanished, "Vanished", "Artist", "Album").unwrap();
        metadata::write_test_wav(&valid, "Valid", "Artist", "Album").unwrap();
        let vanished = fs::canonicalize(vanished).unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(&music, "Music").unwrap();
        let mut removed = false;

        let report = scan_storage_root(&mut store, root.id, temp.path().join("covers"), |event| {
            if let ScanProgress::Discovering { current_path, .. } = event
                && current_path == vanished
                && !removed
            {
                fs::remove_file(&vanished).unwrap();
                removed = true;
            }
        })
        .unwrap();

        assert!(removed);
        assert_eq!(report.outcome, ScanOutcome::CompletedWithErrors);
        assert_eq!(report.discovered, 2);
        assert_eq!(report.added, 1);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].path, vanished);
        assert_eq!(store.tracks_for_root(root.id).unwrap().len(), 1);
    }

    #[test]
    fn unsupported_pcm_container_codec_is_counted_without_failing_the_scan() {
        let temp = tempdir().unwrap();
        let music = temp.path().join("music");
        fs::create_dir(&music).unwrap();
        metadata::write_test_wav(&music.join("pcm.wav"), "PCM", "Artist", "Album").unwrap();
        metadata::write_test_wav_with_format(
            &music.join("float.wav"),
            "Float",
            "Artist",
            "Album",
            3,
        )
        .unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(&music, "Music").unwrap();
        let mut actions = Vec::new();

        let report = scan_storage_root(
            &mut store,
            root.id,
            temp.path().join("covers"),
            |progress| {
                if let ScanProgress::Processing { action, .. } = progress {
                    actions.push(action);
                }
            },
        )
        .unwrap();

        assert_eq!(report.outcome, ScanOutcome::Completed);
        assert_eq!(report.added, 1);
        assert_eq!(report.unsupported, 1);
        assert!(report.errors.is_empty());
        assert!(actions.contains(&ScanProgressAction::Unsupported));
        assert_eq!(store.tracks_for_root(root.id).unwrap().len(), 1);
        let history = store.recent_scans(root.id, 1).unwrap();
        assert_eq!(history[0].unsupported_count, 1);
        assert_eq!(history[0].error_count, 0);
    }

    #[test]
    fn walk_errors_surface_that_missing_track_removals_were_suppressed() {
        let temp = tempdir().unwrap();
        let music = temp.path().join("music");
        fs::create_dir(&music).unwrap();
        let track_path = music.join("track.wav");
        metadata::write_test_wav(&track_path, "Track", "Artist", "Album").unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(&music, "Music").unwrap();
        scan_storage_root(&mut store, root.id, temp.path().join("covers"), |_| {}).unwrap();
        fs::remove_file(track_path).unwrap();
        let started_at_ms = system_time_ms(SystemTime::now()).unwrap();
        let scan_id = store.begin_scan(root.id, started_at_ms).unwrap();

        let report = apply_reachable_scan(
            &mut store,
            scan_id,
            &root,
            started_at_ms,
            walk::WalkResult {
                files: Vec::new(),
                errors: vec![walk::WalkError {
                    path: music.join("unreadable"),
                    message: "permission denied".to_string(),
                }],
            },
            &temp.path().join("covers"),
            &mut |_| {},
        )
        .unwrap();

        assert!(report.removals_suppressed);
        assert_eq!(report.outcome, ScanOutcome::CompletedWithErrors);
        assert_eq!(report.removed, 0);
        assert_eq!(store.tracks_for_root(root.id).unwrap().len(), 1);
        assert!(store.recent_scans(root.id, 1).unwrap()[0].removals_suppressed);
    }

    #[test]
    fn progress_callbacks_can_read_committed_rows_from_a_second_connection() {
        let temp = tempdir().unwrap();
        let music = temp.path().join("music");
        let database = temp.path().join("library.sqlite");
        fs::create_dir(&music).unwrap();
        metadata::write_test_wav(&music.join("track.wav"), "Track", "Artist", "Album").unwrap();
        let mut store = LibraryStore::open(&database).unwrap();
        let root = store.add_storage_root(&music, "Music").unwrap();
        let mut visible_tracks = 0;

        scan_storage_root(
            &mut store,
            root.id,
            temp.path().join("covers"),
            |progress| {
                if matches!(progress, ScanProgress::Processing { .. }) {
                    visible_tracks = LibraryStore::open(&database)
                        .unwrap()
                        .root_summary(root.id)
                        .unwrap()
                        .track_count;
                }
            },
        )
        .unwrap();

        assert_eq!(visible_tracks, 1);
    }

    #[test]
    fn cover_cache_failure_keeps_the_scanned_track() {
        let temp = tempdir().unwrap();
        let music = temp.path().join("music");
        fs::create_dir(&music).unwrap();
        metadata::write_test_wav_with_artwork(&music.join("track.wav"), "Track", "Artist", "Album")
            .unwrap();
        let unusable_cache_path = temp.path().join("not-a-directory");
        fs::write(&unusable_cache_path, "occupied").unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(&music, "Music").unwrap();

        let report = scan_storage_root(&mut store, root.id, &unusable_cache_path, |_| {}).unwrap();

        assert_eq!(report.outcome, ScanOutcome::CompletedWithErrors);
        assert_eq!(report.added, 1);
        assert_eq!(report.errors.len(), 1);
        let tracks = store.tracks_for_root(root.id).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title.as_deref(), Some("Track"));
        assert!(tracks[0].cover_art_path.is_none());
    }

    #[test]
    fn cover_cache_uses_one_stable_track_key_and_overwrites_it() {
        let temp = tempdir().unwrap();
        let music = temp.path().join("music");
        let cache = temp.path().join("covers");
        fs::create_dir(&music).unwrap();
        metadata::write_test_wav(&music.join("track.wav"), "Track", "Artist", "Album").unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(&music, "Music").unwrap();
        scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
        let track_id = store.tracks_for_root(root.id).unwrap()[0].id;

        {
            let transaction = store.connection.transaction().unwrap();
            cache_artwork(
                &transaction,
                &cache,
                track_id,
                metadata::EmbeddedArtwork {
                    data: vec![1, 2, 3],
                    mime_type: Some("image/png".to_string()),
                },
            )
            .unwrap();
            transaction.commit().unwrap();
        }
        let first_path = store.tracks_for_root(root.id).unwrap()[0]
            .cover_art_path
            .clone()
            .unwrap();
        {
            let transaction = store.connection.transaction().unwrap();
            cache_artwork(
                &transaction,
                &cache,
                track_id,
                metadata::EmbeddedArtwork {
                    data: vec![9, 8],
                    mime_type: Some("image/jpeg".to_string()),
                },
            )
            .unwrap();
            transaction.commit().unwrap();
        }
        let track = &store.tracks_for_root(root.id).unwrap()[0];

        assert_eq!(track.cover_art_path.as_ref(), Some(&first_path));
        assert_eq!(track.cover_art_mime_type.as_deref(), Some("image/jpeg"));
        assert_eq!(fs::read(first_path).unwrap(), [9, 8]);
        assert_eq!(fs::read_dir(cache).unwrap().count(), 1);
    }

    #[test]
    #[ignore = "set PULSE_REAL_LIBRARY_ROOT to run the real-library timing diagnostic"]
    fn real_library_scan_timing() {
        let Ok(root_path) = env::var("PULSE_REAL_LIBRARY_ROOT") else {
            eprintln!("PULSE_REAL_LIBRARY_ROOT is not set; real-library scan skipped");
            return;
        };
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open(temp.path().join("library.sqlite")).unwrap();
        let root = store.add_storage_root(&root_path, "Real library").unwrap();
        let cache = temp.path().join("covers");

        let first_started = Instant::now();
        let first = scan_storage_root(&mut store, root.id, &cache, print_scan_milestones).unwrap();
        let first_elapsed = first_started.elapsed();
        let second_started = Instant::now();
        let second = scan_storage_root(&mut store, root.id, &cache, print_scan_milestones).unwrap();
        let second_elapsed = second_started.elapsed();
        let ratio = first_elapsed.as_secs_f64() / second_elapsed.as_secs_f64();

        println!(
            "real scan root={root_path} discovered={} added={} updated={} removed={} unsupported={} skipped={} removals_suppressed={} errors={} elapsed={first_elapsed:?}",
            first.discovered,
            first.added,
            first.updated,
            first.removed,
            first.unsupported,
            first.skipped,
            first.removals_suppressed,
            first.errors.len(),
        );
        println!(
            "unchanged rescan discovered={} added={} updated={} removed={} unsupported={} skipped={} removals_suppressed={} errors={} elapsed={second_elapsed:?} speedup={ratio:.2}x",
            second.discovered,
            second.added,
            second.updated,
            second.removed,
            second.unsupported,
            second.skipped,
            second.removals_suppressed,
            second.errors.len(),
        );

        if env::var_os("PULSE_REAL_LIBRARY_COMPARE_DURATIONS").is_none() {
            println!(
                "duration comparison skipped; set PULSE_REAL_LIBRARY_COMPARE_DURATIONS to opt in"
            );
            return;
        }

        let mut compared = 0;
        let mut divergent = 0;
        for track in store.tracks_for_root(root.id).unwrap() {
            let Ok(stream) = pulse_engine::decode::open(&track.path) else {
                continue;
            };
            let (Some(frames), Some(library_duration_ms)) = (stream.frames, track.duration_ms)
            else {
                continue;
            };
            let engine_duration_ms =
                frames.saturating_mul(1_000) / u64::from(stream.format.sample_rate);
            let difference = library_duration_ms.abs_diff(engine_duration_ms as i64);
            let noticeable = 1_000_u64.max(engine_duration_ms / 100);
            compared += 1;
            if difference > noticeable {
                divergent += 1;
                if divergent <= 10 {
                    println!(
                        "duration divergence path={} lofty={}ms symphonia={}ms difference={}ms",
                        track.path.display(),
                        library_duration_ms,
                        engine_duration_ms,
                        difference,
                    );
                }
            }
        }
        println!("duration comparison tracks={compared} noticeable_divergences={divergent}");
    }

    fn print_scan_milestones(progress: ScanProgress) {
        if let ScanProgress::Processing {
            processed_files,
            total_files,
            ..
        } = progress
            && (processed_files == total_files || processed_files % 1_000 == 0)
        {
            println!("processed {processed_files}/{total_files}");
        }
    }
}
