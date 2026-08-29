use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::backend::{
    model::{
        LibraryError, ScanFileError, ScanOutcome, ScanProgress, ScanProgressAction, ScanReport,
        StorageRoot, StorageRootId, TrackId,
    },
    repo::{
        LibraryStore, LibraryTransaction,
        artists::{
            name_key_for_track as artist_name_key_for_track, refresh_keys as refresh_artist_keys,
        },
        scan_history::{self, CompletedScan},
        storage_roots,
        tracks::{
            self, clear_track_cover, delete_track, set_track_cover, update_track_path, upsert_track,
        },
    },
    scan::{
        metadata, system_time_ms,
        walk::{self, walk_music_files_until},
    },
};

#[allow(dead_code, reason = "shared non-cancellable entry point for pulse mcp")]
pub fn storage_root<F>(
    store: &mut LibraryStore,
    storage_root_id: StorageRootId,
    cover_cache_directory: impl AsRef<Path>,
    on_progress: F,
) -> Result<ScanReport, LibraryError>
where
    F: FnMut(ScanProgress),
{
    storage_root_until(
        store,
        storage_root_id,
        cover_cache_directory,
        on_progress,
        || false,
    )
    .map(|report| report.expect("non-cancellable scan cannot be cancelled"))
}

pub(crate) fn storage_root_cancellable<F, C>(
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
    storage_root_until(
        store,
        storage_root_id,
        cover_cache_directory,
        on_progress,
        is_cancelled,
    )
}

fn storage_root_until<F, C>(
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
    let root = super::storage::get(store, storage_root_id)?
        .ok_or(LibraryError::StorageRootNotFound(storage_root_id))?;
    let started_at_ms = system_time_ms(SystemTime::now())?;
    let scan_id = scan_history::begin(store, storage_root_id, started_at_ms)?;

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
            scan_history::cancel_and_refresh(store, scan_id)?;
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

    if let Err(error) = storage_roots::mark_reachable(store, storage_root_id) {
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
    let existing = tracks::existing(store, root.id)?;
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
            scan_history::cancel_and_refresh(store, scan_id)?;
            return Ok(None);
        }
        let current = existing.get(&file.path_key);
        let now_ms = system_time_ms(SystemTime::now())?;
        let mut action = ScanProgressAction::Skipped;

        if let Some(current) = current.filter(|track| track.modified_at_ns == file.modified_at_ns) {
            let mut changed = false;
            let mut artwork_failed = false;
            if current.path_text != file.path_text {
                let transaction = store.transaction()?;
                update_track_path(&transaction, current.id, &file.path_text, now_ms)?;
                transaction.commit()?;
                changed = true;
            }
            if current.cover_art_path.is_none() {
                match discover_folder_artwork(&file.path) {
                    Ok(Some(artwork)) => {
                        let transaction = store.transaction()?;
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
                    let transaction = store.transaction()?;
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
                        let transaction = store.transaction()?;
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
        scan_history::cancel_and_refresh(store, scan_id)?;
        return Ok(None);
    }

    if !removals_suppressed {
        let missing = existing
            .iter()
            .filter(|(path_key, _)| !seen.contains(*path_key))
            .map(|(_, track)| track)
            .collect::<Vec<_>>();
        let refreshed_at_ms = system_time_ms(SystemTime::now())?;
        let transaction = store.transaction()?;
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
    scan_history::finish_completed_scan_and_refresh(
        store,
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
    transaction: &LibraryTransaction<'_>,
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
    transaction: &LibraryTransaction<'_>,
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
    scan_history::finish_offline_and_refresh(store, scan_id, root.id, finished_at_ms, &message)?;
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
    scan_history::finish_failed_and_refresh(
        store,
        scan_id,
        root.id,
        finished_at_ms,
        &error.to_string(),
    )?;
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

#[cfg(test)]
mod tests;
