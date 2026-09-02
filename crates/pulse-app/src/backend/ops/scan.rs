use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, TrySendError},
    },
    thread,
    time::{Duration, Instant, SystemTime},
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

const FILES_PER_TRANSACTION: usize = 200;
const TRANSACTION_MAX_AGE: Duration = Duration::from_millis(250);
const EXTRACTION_QUEUE_CAPACITY: usize = 4;

#[derive(Clone, Copy)]
struct ScanOptions {
    worker_count: usize,
    queue_capacity: usize,
    files_per_transaction: usize,
    transaction_max_age: Duration,
}

impl ScanOptions {
    fn production() -> Self {
        Self {
            worker_count: thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1),
            queue_capacity: EXTRACTION_QUEUE_CAPACITY,
            files_per_transaction: FILES_PER_TRANSACTION,
            transaction_max_age: TRANSACTION_MAX_AGE,
        }
    }
}

trait ScanExtractor: Send + Sync {
    fn extract_metadata(
        &self,
        path: &Path,
    ) -> Result<metadata::AudioMetadata, metadata::MetadataError>;

    fn folder_artwork(&self, path: &Path) -> io::Result<Option<metadata::EmbeddedArtwork>>;
}

struct FilesystemExtractor;

impl ScanExtractor for FilesystemExtractor {
    fn extract_metadata(
        &self,
        path: &Path,
    ) -> Result<metadata::AudioMetadata, metadata::MetadataError> {
        metadata::extract_metadata(path)
    }

    fn folder_artwork(&self, path: &Path) -> io::Result<Option<metadata::EmbeddedArtwork>> {
        metadata::folder_artwork(path)
    }
}

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
    on_progress: F,
    is_cancelled: C,
) -> Result<Option<ScanReport>, LibraryError>
where
    F: FnMut(ScanProgress),
    C: FnMut() -> bool,
{
    storage_root_until_with(
        store,
        storage_root_id,
        cover_cache_directory,
        on_progress,
        is_cancelled,
        &FilesystemExtractor,
        ScanOptions::production(),
    )
}

fn storage_root_until_with<F, C>(
    store: &mut LibraryStore,
    storage_root_id: StorageRootId,
    cover_cache_directory: impl AsRef<Path>,
    mut on_progress: F,
    mut is_cancelled: C,
    extractor: &dyn ScanExtractor,
    options: ScanOptions,
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

    match apply_reachable_scan_with(
        store,
        (scan_id, started_at_ms),
        &root,
        walk,
        cover_cache_directory.as_ref(),
        &mut on_progress,
        &mut is_cancelled,
        extractor,
        options,
    ) {
        Ok(report) => Ok(report),
        Err(error) => finish_fatal_scan(store, scan_id, &root, error, &mut on_progress).map(Some),
    }
}

#[cfg(test)]
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
    apply_reachable_scan_with(
        store,
        scan,
        root,
        walk,
        cover_cache_directory,
        on_progress,
        is_cancelled,
        &FilesystemExtractor,
        ScanOptions::production(),
    )
}

#[allow(clippy::too_many_arguments)]
fn apply_reachable_scan_with<F, C>(
    store: &mut LibraryStore,
    scan: (i64, i64),
    root: &StorageRoot,
    walk: walk::WalkResult,
    cover_cache_directory: &Path,
    on_progress: &mut F,
    is_cancelled: &mut C,
    extractor: &dyn ScanExtractor,
    options: ScanOptions,
) -> Result<Option<ScanReport>, LibraryError>
where
    F: FnMut(ScanProgress),
    C: FnMut() -> bool,
{
    let (scan_id, started_at_ms) = scan;
    let existing = tracks::existing(store, root.id)?;
    let walk::WalkResult {
        files,
        errors: walk_errors,
    } = walk;
    let seen = files
        .iter()
        .map(|file| file.path_key.clone())
        .collect::<HashSet<_>>();
    let removals_suppressed = !walk_errors.is_empty();
    let walk_errors = walk_errors
        .into_iter()
        .map(|error| ScanFileError {
            path: error.path,
            message: error.message,
        })
        .collect::<Vec<_>>();
    let total_files = files.len();
    let folder_artwork = FolderArtworkMemo::new(extractor, &files);
    let tasks = files
        .into_iter()
        .enumerate()
        .map(|(walk_index, file)| {
            let kind = match existing
                .get(&file.path_key)
                .filter(|track| track.modified_at_ns == file.modified_at_ns)
            {
                Some(current) => ExtractionTaskKind::Unchanged {
                    discover_folder_artwork: current.cover_art_path.is_none(),
                },
                None => ExtractionTaskKind::Metadata,
            };
            ExtractionTask {
                walk_index,
                file,
                kind,
            }
        })
        .collect::<VecDeque<_>>();
    let mut scan = ScanAccumulator::default();

    if !tasks.is_empty()
        && matches!(
            extract_and_apply(
                store,
                root.id,
                tasks,
                total_files,
                &existing,
                cover_cache_directory,
                extractor,
                &folder_artwork,
                options,
                &mut scan,
                on_progress,
                is_cancelled,
            )?,
            ExtractionPipelineOutcome::Cancelled
        )
    {
        scan_history::cancel_and_refresh(store, scan_id)?;
        return Ok(None);
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
        scan.removed += missing.len();

        for track in missing {
            if let Some(path) = &track.cover_art_path
                && let Err(error) = remove_cached_file(path)
            {
                scan.trailing_errors.push(ScanFileError {
                    path: track.path.clone(),
                    message: format!("failed to remove cached cover art: {error}"),
                });
            }
        }
    }

    let finished_at_ms = system_time_ms(SystemTime::now())?;
    scan.file_errors.sort_by_key(|(walk_index, _)| *walk_index);
    let mut errors = walk_errors;
    errors.extend(scan.file_errors.into_iter().map(|(_, error)| error));
    errors.extend(scan.trailing_errors);
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
            added: scan.added,
            updated: scan.updated,
            removed: scan.removed,
            unsupported: scan.unsupported,
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
        added: scan.added,
        updated: scan.updated,
        removed: scan.removed,
        unsupported: scan.unsupported,
        skipped: scan.skipped,
        removals_suppressed,
        errors,
        outcome,
    };
    on_progress(finished_progress(&report));
    Ok(Some(report))
}

type FolderArtworkResult = Result<Option<Arc<metadata::EmbeddedArtwork>>, String>;

struct FolderArtworkMemo<'a> {
    extractor: &'a dyn ScanExtractor,
    entries: Mutex<HashMap<PathBuf, FolderArtworkMemoEntry>>,
}

struct FolderArtworkMemoEntry {
    remaining_files: usize,
    result: Arc<OnceLock<FolderArtworkResult>>,
}

impl<'a> FolderArtworkMemo<'a> {
    fn new(extractor: &'a dyn ScanExtractor, files: &[walk::DiscoveredFile]) -> Self {
        let mut entries = HashMap::new();
        for file in files {
            entries
                .entry(parent_directory(&file.path))
                .and_modify(|entry: &mut FolderArtworkMemoEntry| entry.remaining_files += 1)
                .or_insert_with(|| FolderArtworkMemoEntry {
                    remaining_files: 1,
                    result: Arc::new(OnceLock::new()),
                });
        }
        Self {
            extractor,
            entries: Mutex::new(entries),
        }
    }

    fn get(&self, path: &Path) -> FolderArtworkResult {
        let entry = {
            let entries = self.entries.lock().expect("folder artwork memo poisoned");
            Arc::clone(
                &entries
                    .get(&parent_directory(path))
                    .expect("scan file directory is registered in the artwork memo")
                    .result,
            )
        };
        entry
            .get_or_init(|| {
                self.extractor
                    .folder_artwork(path)
                    .map(|artwork| artwork.map(Arc::new))
                    .map_err(|error| error.to_string())
            })
            .clone()
    }

    fn release(&self, path: &Path) {
        let directory = parent_directory(path);
        let mut entries = self.entries.lock().expect("folder artwork memo poisoned");
        let remove = {
            let entry = entries
                .get_mut(&directory)
                .expect("scan file directory is registered in the artwork memo");
            entry.remaining_files -= 1;
            entry.remaining_files == 0
        };
        if remove {
            entries.remove(&directory);
        }
    }

    #[cfg(test)]
    fn entry_count(&self) -> usize {
        self.entries
            .lock()
            .expect("folder artwork memo poisoned")
            .len()
    }
}

fn parent_directory(path: &Path) -> PathBuf {
    path.parent().unwrap_or(Path::new("")).to_path_buf()
}

struct ExtractionTask {
    walk_index: usize,
    file: walk::DiscoveredFile,
    kind: ExtractionTaskKind,
}

enum ExtractionTaskKind {
    Unchanged { discover_folder_artwork: bool },
    Metadata,
}

struct ExtractedFile {
    walk_index: usize,
    file: walk::DiscoveredFile,
    result: ExtractionResult,
}

enum ExtractionResult {
    Unchanged(Option<FolderArtworkResult>),
    Metadata(Box<ExtractedMetadata>),
    MetadataError(metadata::MetadataError),
}

struct ExtractedMetadata {
    metadata: metadata::AudioMetadata,
    folder_artwork: Option<FolderArtworkResult>,
}

#[derive(Default)]
struct ScanAccumulator {
    added: usize,
    updated: usize,
    removed: usize,
    unsupported: usize,
    skipped: usize,
    processed_files: usize,
    file_errors: Vec<(usize, ScanFileError)>,
    trailing_errors: Vec<ScanFileError>,
}

struct PendingFile {
    walk_index: usize,
    path: PathBuf,
    action: ScanProgressAction,
    added: usize,
    updated: usize,
    removed: usize,
    unsupported: usize,
    skipped: usize,
    errors: Vec<ScanFileError>,
    cached_files_to_remove: Vec<PathBuf>,
}

impl PendingFile {
    fn new(walk_index: usize, path: PathBuf) -> Self {
        Self {
            walk_index,
            path,
            action: ScanProgressAction::Skipped,
            added: 0,
            updated: 0,
            removed: 0,
            unsupported: 0,
            skipped: 0,
            errors: Vec::new(),
            cached_files_to_remove: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ExtractionPipelineOutcome {
    Completed,
    Cancelled,
}

#[allow(clippy::too_many_arguments)]
fn extract_and_apply<F, C>(
    store: &mut LibraryStore,
    storage_root_id: StorageRootId,
    mut tasks: VecDeque<ExtractionTask>,
    total_files: usize,
    existing: &HashMap<String, tracks::ExistingTrack>,
    cover_cache_directory: &Path,
    extractor: &dyn ScanExtractor,
    folder_artwork: &FolderArtworkMemo<'_>,
    options: ScanOptions,
    scan: &mut ScanAccumulator,
    on_progress: &mut F,
    is_cancelled: &mut C,
) -> Result<ExtractionPipelineOutcome, LibraryError>
where
    F: FnMut(ScanProgress),
    C: FnMut() -> bool,
{
    let worker_count = options.worker_count.min(total_files);

    thread::scope(|scope| {
        let (task_sender, task_receiver) = mpsc::sync_channel(options.queue_capacity);
        let task_receiver = Arc::new(Mutex::new(task_receiver));
        let (result_sender, result_receiver) = mpsc::sync_channel(options.queue_capacity);
        let stop = Arc::new(AtomicBool::new(false));

        for _ in 0..worker_count {
            let task_receiver = Arc::clone(&task_receiver);
            let result_sender = result_sender.clone();
            let stop = Arc::clone(&stop);
            scope.spawn(move || {
                loop {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let task = match task_receiver
                        .lock()
                        .expect("extractor task queue poisoned")
                        .recv()
                    {
                        Ok(task) => task,
                        Err(_) => break,
                    };
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let Some(result) = extract_file(task, extractor, folder_artwork, &stop) else {
                        break;
                    };
                    if result_sender.send(result).is_err() {
                        break;
                    }
                }
            });
        }
        drop(result_sender);

        let mut task_sender = Some(task_sender);
        let writer_result = (|| {
            let mut pending = Vec::with_capacity(options.files_per_transaction);
            let mut applied_files = 0;

            while applied_files < total_files {
                let transaction = store.transaction()?;
                let batch_started = Instant::now();
                let mut cancelled = false;

                loop {
                    while let Some(task) = tasks.pop_front() {
                        match task_sender
                            .as_ref()
                            .expect("task sender exists while tasks remain")
                            .try_send(task)
                        {
                            Ok(()) => {}
                            Err(TrySendError::Full(task)) => {
                                tasks.push_front(task);
                                break;
                            }
                            Err(TrySendError::Disconnected(_)) => {
                                panic!("extractor workers disconnected before scan completed")
                            }
                        }
                    }
                    if tasks.is_empty() {
                        task_sender.take();
                    }

                    let extracted = result_receiver
                        .recv()
                        .expect("extractor workers disconnected before returning every file");
                    if is_cancelled() {
                        stop.store(true, Ordering::Release);
                        task_sender.take();
                        cancelled = true;
                        break;
                    }

                    let current = existing.get(&extracted.file.path_key);
                    let now_ms = system_time_ms(SystemTime::now())?;
                    let outcome = apply_extracted_file(
                        &transaction,
                        storage_root_id,
                        current,
                        extracted,
                        cover_cache_directory,
                        now_ms,
                    )?;
                    folder_artwork.release(&outcome.path);
                    pending.push(outcome);
                    applied_files += 1;

                    let chunk_edge = pending.len() >= options.files_per_transaction
                        || batch_started.elapsed() >= options.transaction_max_age
                        || applied_files == total_files;
                    if chunk_edge {
                        break;
                    }
                }

                transaction.commit()?;
                flush_pending(&mut pending, scan, total_files, on_progress);
                if cancelled {
                    return Ok(ExtractionPipelineOutcome::Cancelled);
                }
            }

            Ok(ExtractionPipelineOutcome::Completed)
        })();

        stop.store(true, Ordering::Release);
        drop(task_sender);
        drop(result_receiver);
        writer_result
    })
}

fn extract_file(
    task: ExtractionTask,
    extractor: &dyn ScanExtractor,
    folder_artwork: &FolderArtworkMemo<'_>,
    stop: &AtomicBool,
) -> Option<ExtractedFile> {
    let ExtractionTask {
        walk_index,
        file,
        kind,
    } = task;
    let result = match kind {
        ExtractionTaskKind::Unchanged {
            discover_folder_artwork,
        } => {
            let artwork = discover_folder_artwork.then(|| folder_artwork.get(&file.path));
            ExtractionResult::Unchanged(artwork)
        }
        ExtractionTaskKind::Metadata => match extractor.extract_metadata(&file.path) {
            Ok(metadata) => {
                let artwork = if metadata.artwork.is_none() {
                    if stop.load(Ordering::Acquire) {
                        return None;
                    }
                    Some(folder_artwork.get(&file.path))
                } else {
                    None
                };
                ExtractionResult::Metadata(Box::new(ExtractedMetadata {
                    metadata,
                    folder_artwork: artwork,
                }))
            }
            Err(error) => ExtractionResult::MetadataError(error),
        },
    };
    (!stop.load(Ordering::Acquire)).then_some(ExtractedFile {
        walk_index,
        file,
        result,
    })
}

fn apply_extracted_file(
    transaction: &LibraryTransaction<'_>,
    storage_root_id: StorageRootId,
    current: Option<&tracks::ExistingTrack>,
    extracted: ExtractedFile,
    cover_cache_directory: &Path,
    now_ms: i64,
) -> Result<PendingFile, LibraryError> {
    let ExtractedFile {
        walk_index,
        file,
        result,
    } = extracted;
    let mut pending = PendingFile::new(walk_index, file.path.clone());

    match result {
        ExtractionResult::Unchanged(folder_artwork) => {
            let current = current.expect("unchanged extraction has an existing track");
            let mut changed = false;
            let mut artwork_failed = false;
            if current.path_text != file.path_text {
                update_track_path(transaction, current.id, &file.path_text, now_ms)?;
                changed = true;
            }
            if let Some(folder_artwork) = folder_artwork {
                match folder_artwork {
                    Ok(Some(artwork)) => {
                        match cache_artwork(
                            transaction,
                            cover_cache_directory,
                            current.id,
                            artwork.as_ref(),
                        ) {
                            Ok(_) => {
                                refresh_track_artist_keys(
                                    transaction,
                                    None,
                                    Some(current.id),
                                    now_ms,
                                )?;
                                changed = true;
                            }
                            Err(ArtworkError::Cache(message)) => {
                                pending.errors.push(ScanFileError {
                                    path: file.path.clone(),
                                    message,
                                });
                                artwork_failed = true;
                            }
                            Err(ArtworkError::Database(error)) => return Err(error),
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        pending.errors.push(ScanFileError {
                            path: file.path.clone(),
                            message: folder_artwork_error(&file.path, &error),
                        });
                        artwork_failed = true;
                    }
                }
            }
            if changed {
                pending.updated = 1;
                pending.action = ScanProgressAction::Updated;
            } else if !artwork_failed {
                pending.skipped = 1;
            }
            if artwork_failed {
                pending.action = ScanProgressAction::Failed;
            }
        }
        ExtractionResult::Metadata(extracted_metadata) => {
            let previous_cover = current
                .and_then(|track| track.cover_art_path.as_ref())
                .cloned();
            let previous_artist_key = match current {
                Some(current) => artist_name_key_for_track(transaction, current.id)?,
                None => None,
            };
            let track_id = upsert_track(
                transaction,
                storage_root_id,
                &file,
                &extracted_metadata.metadata,
                now_ms,
            )?;
            let artwork_result = if let Some(artwork) = extracted_metadata.metadata.artwork.as_ref()
            {
                cache_artwork(transaction, cover_cache_directory, track_id, artwork).map(Some)
            } else {
                match extracted_metadata
                    .folder_artwork
                    // A cancelling worker exits before sending metadata that still needs this probe.
                    .expect("metadata without embedded art probes folder art")
                {
                    Ok(Some(artwork)) => cache_artwork(
                        transaction,
                        cover_cache_directory,
                        track_id,
                        artwork.as_ref(),
                    )
                    .map(Some),
                    Ok(None) => clear_track_cover(transaction, track_id)
                        .map(|()| None)
                        .map_err(ArtworkError::Database),
                    Err(error) => Err(ArtworkError::Cache(folder_artwork_error(
                        &file.path, &error,
                    ))),
                }
            };
            match artwork_result {
                Ok(new_cover) => {
                    refresh_track_artist_keys(
                        transaction,
                        previous_artist_key.as_deref(),
                        Some(track_id),
                        now_ms,
                    )?;
                    if current.is_some() {
                        pending.updated = 1;
                        pending.action = ScanProgressAction::Updated;
                    } else {
                        pending.added = 1;
                        pending.action = ScanProgressAction::Added;
                    }
                    if let Some(path) =
                        previous_cover.filter(|previous| new_cover.as_ref() != Some(previous))
                    {
                        pending.cached_files_to_remove.push(path);
                    }
                }
                Err(ArtworkError::Cache(message)) => {
                    refresh_track_artist_keys(
                        transaction,
                        previous_artist_key.as_deref(),
                        Some(track_id),
                        now_ms,
                    )?;
                    if current.is_some() {
                        pending.updated = 1;
                    } else {
                        pending.added = 1;
                    }
                    pending.errors.push(ScanFileError {
                        path: file.path.clone(),
                        message,
                    });
                    pending.action = ScanProgressAction::Failed;
                }
                Err(ArtworkError::Database(error)) => return Err(error),
            }
        }
        ExtractionResult::MetadataError(metadata::MetadataError::Unsupported(_)) => {
            pending.unsupported = 1;
            pending.action = ScanProgressAction::Unsupported;
            if let Some(current) = current {
                let artist_key = artist_name_key_for_track(transaction, current.id)?;
                delete_track(transaction, current.id)?;
                if let Some(artist_key) = artist_key {
                    refresh_artist_keys(transaction, &[artist_key], now_ms)?;
                }
                pending.removed = 1;
                if let Some(path) = &current.cover_art_path {
                    pending.cached_files_to_remove.push(path.clone());
                }
            }
        }
        ExtractionResult::MetadataError(error) => {
            pending.errors.push(ScanFileError {
                path: file.path.clone(),
                message: error.to_string(),
            });
            pending.action = ScanProgressAction::Failed;
        }
    }

    Ok(pending)
}

fn flush_pending<F>(
    pending: &mut Vec<PendingFile>,
    scan: &mut ScanAccumulator,
    total_files: usize,
    on_progress: &mut F,
) where
    F: FnMut(ScanProgress),
{
    for mut file in pending.drain(..) {
        for path in &file.cached_files_to_remove {
            if let Err(error) = remove_cached_file(path) {
                file.errors.push(ScanFileError {
                    path: file.path.clone(),
                    message: format!("failed to remove cached cover art: {error}"),
                });
                file.action = ScanProgressAction::Failed;
            }
        }
        scan.added += file.added;
        scan.updated += file.updated;
        scan.removed += file.removed;
        scan.unsupported += file.unsupported;
        scan.skipped += file.skipped;
        scan.file_errors.extend(
            file.errors
                .into_iter()
                .map(|error| (file.walk_index, error)),
        );
        scan.processed_files += 1;
        on_progress(ScanProgress::Processing {
            processed_files: scan.processed_files,
            total_files,
            current_path: file.path,
            action: file.action,
        });
    }
}

fn folder_artwork_error(path: &Path, error: &str) -> String {
    format!(
        "failed to read folder art beside {}: {error}",
        path.display()
    )
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
    artwork: &metadata::EmbeddedArtwork,
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
    fs::write(&temporary_path, &artwork.data).map_err(|error| {
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
