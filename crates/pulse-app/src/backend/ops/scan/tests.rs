use std::{
    cell::Cell,
    env, fs,
    time::{Duration, Instant},
};

use tempfile::tempdir;

use super::*;

const TEST_PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D', b'R',
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, b'I', b'E',
    b'N', b'D', 0xae, 0x42, 0x60, 0x82,
];

fn test_art(marker: u8) -> Vec<u8> {
    let mut art = TEST_PNG.to_vec();
    art.push(marker);
    art
}

#[derive(Default)]
struct CountingExtractor {
    metadata_calls: std::sync::atomic::AtomicUsize,
    folder_artwork_calls: std::sync::atomic::AtomicUsize,
}

impl ScanExtractor for CountingExtractor {
    fn extract_metadata(
        &self,
        path: &Path,
    ) -> Result<metadata::AudioMetadata, metadata::MetadataError> {
        self.metadata_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        metadata::extract_metadata(path)
    }

    fn folder_artwork(&self, path: &Path) -> io::Result<Option<metadata::EmbeddedArtwork>> {
        self.folder_artwork_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        metadata::folder_artwork(path)
    }
}

fn test_scan_options(worker_count: usize) -> ScanOptions {
    ScanOptions {
        worker_count,
        queue_capacity: 2,
        files_per_transaction: 2,
        transaction_max_age: Duration::MAX,
    }
}

#[test]
fn scan_is_incremental_removes_missing_tracks_and_preserves_offline_roots() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    let cache = temp.path().join("covers");
    fs::create_dir(&music).unwrap();
    let track_path = music.join("Café.wav");
    metadata::write_test_wav(&track_path, "First", "Artist", "Album").unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();

    let mut progress = Vec::new();
    let first = storage_root(&mut store, root.id, &cache, |event| progress.push(event)).unwrap();
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

    let second = storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
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
        .set_times(fs::FileTimes::new().set_modified(previous_modified + Duration::from_secs(2)))
        .unwrap();
    let third = storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    assert_eq!(
        (third.added, third.updated, third.removed, third.skipped),
        (0, 1, 0, 0)
    );
    assert_eq!(
        crate::backend::repo::tracks::for_root(&store, root.id).unwrap()[0]
            .title
            .as_deref(),
        Some("Updated")
    );

    fs::remove_file(&track_path).unwrap();
    let fourth = storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    assert_eq!((fourth.added, fourth.updated, fourth.removed), (0, 0, 1));
    assert_eq!(
        crate::backend::repo::tracks::root_summary(&store, root.id)
            .unwrap()
            .track_count,
        0
    );

    metadata::write_test_wav(&track_path, "Returned", "Artist", "Album").unwrap();
    storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let offline_path = temp.path().join("music-offline");
    fs::rename(&music, &offline_path).unwrap();
    let offline = storage_root(&mut store, root.id, &cache, |_| {}).unwrap();

    assert_eq!(offline.outcome, ScanOutcome::Offline);
    assert_eq!(
        crate::backend::repo::tracks::for_root(&store, root.id)
            .unwrap()
            .len(),
        1
    );
    assert!(
        !crate::backend::repo::storage_roots::get(&store, root.id)
            .unwrap()
            .unwrap()
            .is_reachable
    );
    assert_eq!(
        crate::backend::repo::scan_history::recent(&store, root.id, 1).unwrap()[0].outcome,
        Some(ScanOutcome::Offline)
    );
}

#[test]
fn scan_adds_updates_and_prunes_materialized_artists() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    let cache = temp.path().join("covers");
    fs::create_dir(&music).unwrap();
    let first_path = music.join("01-first.wav");
    metadata::write_test_wav(&first_path, "First", "Artist", "First Album").unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();

    storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let artists = crate::backend::repo::artists::index(&store).unwrap();
    assert_eq!(artists.len(), 1);
    assert_eq!(artists[0].name_key, "Artist");
    assert_eq!((artists[0].album_count, artists[0].track_count), (1, 1));

    let second_path = music.join("02-second.wav");
    metadata::write_test_wav(&second_path, "Second", "Artist", "Second Album").unwrap();
    storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let artists = crate::backend::repo::artists::index(&store).unwrap();
    assert_eq!(artists.len(), 1);
    assert_eq!((artists[0].album_count, artists[0].track_count), (2, 2));

    let previous_modified = fs::metadata(&second_path).unwrap().modified().unwrap();
    metadata::write_test_wav(&second_path, "Second", "Other Artist", "Other Album").unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&second_path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(previous_modified + Duration::from_secs(2)))
        .unwrap();
    storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let artists = crate::backend::repo::artists::index(&store).unwrap();
    assert_eq!(artists.len(), 2);
    assert!(artists.iter().any(|artist| {
        artist.name_key == "Artist" && (artist.album_count, artist.track_count) == (1, 1)
    }));
    assert!(artists.iter().any(|artist| {
        artist.name_key == "Other Artist" && (artist.album_count, artist.track_count) == (1, 1)
    }));

    fs::remove_file(&second_path).unwrap();
    storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let artists = crate::backend::repo::artists::index(&store).unwrap();
    assert_eq!(artists.len(), 1);
    assert_eq!(artists[0].name_key, "Artist");
    assert_eq!((artists[0].album_count, artists[0].track_count), (1, 1));
}

#[test]
fn scan_drops_placeholder_album_artist_and_uses_track_artist_fallback() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    let cache = temp.path().join("covers");
    fs::create_dir(&music).unwrap();
    metadata::write_test_wav_with_album_artist(
        &music.join("track.wav"),
        "Track",
        "王菲",
        "######",
        "######",
    )
    .unwrap();
    metadata::write_test_wav(
        &music.join("unknown.wav"),
        "Unknown Artist Track",
        "----",
        "######",
    )
    .unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();

    storage_root(&mut store, root.id, &cache, |_| {}).unwrap();

    let stored = crate::backend::repo::tracks::for_root(&store, root.id).unwrap();
    assert_eq!(stored.len(), 2);
    assert!(
        stored
            .iter()
            .all(|track| track.album.as_deref() == Some("######"))
    );
    let track = stored
        .iter()
        .find(|track| track.title.as_deref() == Some("Track"))
        .unwrap();
    assert!(track.album_artist.is_none());
    let unknown_artist_track = stored
        .iter()
        .find(|track| track.title.as_deref() == Some("Unknown Artist Track"))
        .unwrap();
    assert!(unknown_artist_track.artist.is_none());

    let effective = crate::backend::repo::tracks::for_album(&store, "王菲", "######").unwrap();
    assert_eq!(effective.len(), 1);
    assert_eq!(effective[0].title.as_deref(), Some("Track"));
    let unknown_effective =
        crate::backend::repo::tracks::for_album(&store, crate::backend::UNKNOWN_ARTIST, "######")
            .unwrap();
    assert_eq!(unknown_effective.len(), 1);
    assert_eq!(
        unknown_effective[0].title.as_deref(),
        Some("Unknown Artist Track")
    );
}

#[test]
fn sequential_and_parallel_extraction_produce_identical_scan_results() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    let first_album = music.join("first-album");
    let second_album = music.join("second-album");
    fs::create_dir_all(&first_album).unwrap();
    fs::create_dir(&second_album).unwrap();
    metadata::write_test_wav(
        &first_album.join("01-first.wav"),
        "First",
        "Artist",
        "First Album",
    )
    .unwrap();
    metadata::write_test_wav(
        &first_album.join("02-second.wav"),
        "Second",
        "Artist",
        "First Album",
    )
    .unwrap();
    fs::write(first_album.join("folder.png"), test_art(7)).unwrap();
    metadata::write_test_wav(
        &second_album.join("01-third.wav"),
        "Third",
        "Other Artist",
        "Second Album",
    )
    .unwrap();
    metadata::write_test_wav_with_format(
        &second_album.join("02-unsupported.wav"),
        "Unsupported",
        "Other Artist",
        "Second Album",
        3,
    )
    .unwrap();
    fs::write(second_album.join("03-invalid.wav"), b"not audio").unwrap();

    let mut sequential_store = LibraryStore::open_in_memory().unwrap();
    let sequential_root =
        crate::backend::repo::storage_roots::add(&mut sequential_store, &music, "Sequential")
            .unwrap();
    let mut parallel_store = LibraryStore::open_in_memory().unwrap();
    let parallel_root =
        crate::backend::repo::storage_roots::add(&mut parallel_store, &music, "Parallel").unwrap();
    let extractor = FilesystemExtractor;

    let sequential = storage_root_until_with(
        &mut sequential_store,
        sequential_root.id,
        temp.path().join("sequential-covers"),
        |_| {},
        || false,
        &extractor,
        test_scan_options(1),
    )
    .unwrap()
    .unwrap();
    let parallel = storage_root_until_with(
        &mut parallel_store,
        parallel_root.id,
        temp.path().join("parallel-covers"),
        |_| {},
        || false,
        &extractor,
        test_scan_options(4),
    )
    .unwrap()
    .unwrap();

    let report_semantics = |report: &ScanReport| {
        (
            report.discovered,
            report.added,
            report.updated,
            report.removed,
            report.unsupported,
            report.skipped,
            report.removals_suppressed,
            report.errors.clone(),
            report.outcome,
        )
    };
    assert_eq!(report_semantics(&parallel), report_semantics(&sequential));

    let library_semantics = |store: &LibraryStore, root_id| {
        crate::backend::repo::tracks::for_root(store, root_id)
            .unwrap()
            .into_iter()
            .map(|track| {
                let cover = track
                    .cover_art_path
                    .as_ref()
                    .map(fs::read)
                    .transpose()
                    .unwrap();
                (
                    track.path,
                    track.title,
                    track.artist,
                    track.album,
                    track.sample_rate_hz,
                    track.bit_depth,
                    track.channels,
                    track.cover_art_mime_type,
                    cover,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        library_semantics(&parallel_store, parallel_root.id),
        library_semantics(&sequential_store, sequential_root.id)
    );
}

#[test]
fn folder_artwork_is_listed_once_per_directory_per_scan() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    fs::create_dir(&music).unwrap();
    for index in 0..8 {
        metadata::write_test_wav(
            &music.join(format!("{index:02}.wav")),
            &format!("Track {index}"),
            "Artist",
            "Album",
        )
        .unwrap();
    }
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();
    let extractor = CountingExtractor::default();
    let options = ScanOptions {
        files_per_transaction: 200,
        ..test_scan_options(4)
    };

    let first = storage_root_until_with(
        &mut store,
        root.id,
        temp.path().join("covers"),
        |_| {},
        || false,
        &extractor,
        options,
    )
    .unwrap()
    .unwrap();
    assert_eq!(first.added, 8);
    assert_eq!(
        extractor
            .folder_artwork_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        extractor
            .metadata_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        8
    );

    extractor
        .metadata_calls
        .store(0, std::sync::atomic::Ordering::Relaxed);
    let second = storage_root_until_with(
        &mut store,
        root.id,
        temp.path().join("covers"),
        |_| {},
        || false,
        &extractor,
        options,
    )
    .unwrap()
    .unwrap();
    assert_eq!(second.skipped, 8);
    assert_eq!(
        extractor
            .folder_artwork_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        2
    );
    assert_eq!(
        extractor
            .metadata_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        0,
        "mtime-matched artless tracks must not reread audio metadata"
    );
}

#[test]
fn folder_artwork_memo_releases_a_directory_after_its_last_file() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    fs::create_dir(&music).unwrap();
    metadata::write_test_wav(&music.join("first.wav"), "First", "Artist", "Album").unwrap();
    metadata::write_test_wav(&music.join("second.wav"), "Second", "Artist", "Album").unwrap();
    let walk = walk::walk_music_files(&music, true, |_, _| {}).unwrap();
    let extractor = CountingExtractor::default();
    let memo = FolderArtworkMemo::new(&extractor, &walk.files);

    assert_eq!(memo.entry_count(), 1);
    assert!(memo.get(&walk.files[0].path).unwrap().is_none());
    memo.release(&walk.files[0].path);
    assert_eq!(memo.entry_count(), 1);
    memo.release(&walk.files[1].path);
    assert_eq!(memo.entry_count(), 0);
    assert_eq!(
        extractor
            .folder_artwork_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[test]
fn cancellation_after_batch_progress_keeps_the_committed_batch_without_history() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    fs::create_dir(&music).unwrap();
    metadata::write_test_wav(&music.join("first.wav"), "First", "Artist", "Album").unwrap();
    metadata::write_test_wav(&music.join("second.wav"), "Second", "Artist", "Album").unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();
    let cancelled = Cell::new(false);

    let report = storage_root_until_with(
        &mut store,
        root.id,
        temp.path().join("covers"),
        |progress| {
            if matches!(progress, ScanProgress::Processing { .. }) {
                cancelled.set(true);
            }
        },
        || cancelled.get(),
        &FilesystemExtractor,
        test_scan_options(2),
    )
    .unwrap();

    assert!(report.is_none());
    assert_eq!(
        crate::backend::repo::tracks::for_root(&store, root.id)
            .unwrap()
            .len(),
        2
    );
    let artists = crate::backend::repo::artists::index(&store).unwrap();
    assert_eq!(artists.len(), 1);
    assert_eq!(artists[0].name_key, "Artist");
    assert_eq!((artists[0].album_count, artists[0].track_count), (1, 2));
    assert!(
        crate::backend::repo::scan_history::recent(&store, root.id, 1)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cancellation_commits_finished_files_in_the_open_chunk() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    fs::create_dir(&music).unwrap();
    for index in 0..5 {
        metadata::write_test_wav(
            &music.join(format!("{index:02}.wav")),
            &format!("Track {index}"),
            "Artist",
            "Album",
        )
        .unwrap();
    }
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();
    let walk = walk::walk_music_files(&music, root.is_case_sensitive, |_, _| {}).unwrap();
    let started_at_ms = system_time_ms(SystemTime::now()).unwrap();
    let scan_id =
        crate::backend::repo::scan_history::begin(&mut store, root.id, started_at_ms).unwrap();
    let cancellation_checks = Cell::new(0);
    let mut progress = Vec::new();

    let report = apply_reachable_scan_with(
        &mut store,
        (scan_id, started_at_ms),
        &root,
        walk,
        &temp.path().join("covers"),
        &mut |event| progress.push(event),
        &mut || {
            let check = cancellation_checks.get();
            cancellation_checks.set(check + 1);
            check == 3
        },
        &FilesystemExtractor,
        test_scan_options(2),
    )
    .unwrap();

    assert!(report.is_none());
    assert_eq!(
        crate::backend::repo::tracks::for_root(&store, root.id)
            .unwrap()
            .len(),
        3,
        "the two-file completed chunk and the finished file in the open chunk commit"
    );
    assert_eq!(
        progress
            .iter()
            .filter(|event| matches!(event, ScanProgress::Processing { .. }))
            .count(),
        3
    );
    assert!(
        crate::backend::repo::scan_history::recent(&store, root.id, 1)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cancellation_bounds_extractor_work_in_flight() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    fs::create_dir(&music).unwrap();
    for index in 0..24 {
        metadata::write_test_wav(
            &music.join(format!("{index:02}.wav")),
            &format!("Track {index}"),
            "Artist",
            "Album",
        )
        .unwrap();
    }
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();
    let walk = walk::walk_music_files(&music, root.is_case_sensitive, |_, _| {}).unwrap();
    let started_at_ms = system_time_ms(SystemTime::now()).unwrap();
    let scan_id =
        crate::backend::repo::scan_history::begin(&mut store, root.id, started_at_ms).unwrap();
    let extractor = CountingExtractor::default();
    let options = test_scan_options(2);

    let report = apply_reachable_scan_with(
        &mut store,
        (scan_id, started_at_ms),
        &root,
        walk,
        &temp.path().join("covers"),
        &mut |_| {},
        &mut || true,
        &extractor,
        options,
    )
    .unwrap();

    let started = extractor
        .metadata_calls
        .load(std::sync::atomic::Ordering::Relaxed);
    let in_flight_bound = options.worker_count + options.queue_capacity * 2 + 1;
    assert!(report.is_none());
    assert!(started > 0);
    assert!(
        started <= in_flight_bound,
        "started {started} extractions with an in-flight bound of {in_flight_bound}"
    );
    assert!(
        crate::backend::repo::tracks::for_root(&store, root.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn failed_artist_refresh_rolls_back_the_open_writer_chunk() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    let cache = temp.path().join("covers");
    fs::create_dir(&music).unwrap();
    metadata::write_test_wav(
        &music.join("01-first.wav"),
        "First",
        "First Artist",
        "First Album",
    )
    .unwrap();
    metadata::write_test_wav(
        &music.join("02-second.wav"),
        "Second",
        "Second Artist",
        "Second Album",
    )
    .unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();
    crate::backend::repo::testing::fail_second_artist_refresh(&mut store);

    let error = storage_root(&mut store, root.id, &cache, |_| {}).unwrap_err();

    assert!(error.to_string().contains("artist refresh failed"));
    let tracks = crate::backend::repo::tracks::for_root(&store, root.id).unwrap();
    assert!(tracks.is_empty());
    let artists = crate::backend::repo::artists::index(&store).unwrap();
    assert!(artists.is_empty());
    assert_eq!(
        crate::backend::repo::scan_history::recent(&store, root.id, 1).unwrap()[0].outcome,
        Some(ScanOutcome::Failed)
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
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();
    let mut removed = false;

    let report = storage_root(&mut store, root.id, temp.path().join("covers"), |event| {
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
    assert_eq!(
        crate::backend::repo::tracks::for_root(&store, root.id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn unsupported_pcm_container_codec_is_counted_without_failing_the_scan() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    fs::create_dir(&music).unwrap();
    metadata::write_test_wav(&music.join("pcm.wav"), "PCM", "Artist", "Album").unwrap();
    metadata::write_test_wav_with_format(&music.join("float.wav"), "Float", "Artist", "Album", 3)
        .unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();
    let mut actions = Vec::new();

    let report = storage_root(
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
    assert_eq!(
        crate::backend::repo::tracks::for_root(&store, root.id)
            .unwrap()
            .len(),
        1
    );
    let history = crate::backend::repo::scan_history::recent(&store, root.id, 1).unwrap();
    assert_eq!(history[0].unsupported_count, 1);
    assert_eq!(history[0].error_count, 0);
}

#[test]
fn scan_stores_true_dsd_rate_and_skips_dst() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    fs::create_dir(&music).unwrap();
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pulse-engine/tests/fixtures/dsd-interleave.dff"
        ),
        music.join("Tagless DSD.dff"),
    )
    .unwrap();
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pulse-engine/tests/fixtures/dst-refusal.dff"
        ),
        music.join("Compressed.dff"),
    )
    .unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();

    let report = storage_root(&mut store, root.id, temp.path().join("covers"), |_| {}).unwrap();
    let tracks = crate::backend::repo::tracks::for_root(&store, root.id).unwrap();

    assert_eq!((report.added, report.unsupported), (1, 1));
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title.as_deref(), Some("Tagless DSD"));
    assert_eq!(tracks[0].sample_rate_hz, Some(2_822_400));
    assert_eq!(tracks[0].bit_depth, Some(1));
}

#[test]
fn walk_errors_surface_that_missing_track_removals_were_suppressed() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    fs::create_dir(&music).unwrap();
    let track_path = music.join("track.wav");
    metadata::write_test_wav(&track_path, "Track", "Artist", "Album").unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();
    storage_root(&mut store, root.id, temp.path().join("covers"), |_| {}).unwrap();
    fs::remove_file(track_path).unwrap();
    let started_at_ms = system_time_ms(SystemTime::now()).unwrap();
    let scan_id =
        crate::backend::repo::scan_history::begin(&mut store, root.id, started_at_ms).unwrap();

    let report = apply_reachable_scan(
        &mut store,
        (scan_id, started_at_ms),
        &root,
        walk::WalkResult {
            files: Vec::new(),
            errors: vec![walk::WalkError {
                path: music.join("unreadable"),
                message: "permission denied".to_string(),
            }],
        },
        &temp.path().join("covers"),
        &mut |_| {},
        &mut || false,
    )
    .unwrap()
    .unwrap();

    assert!(report.removals_suppressed);
    assert_eq!(report.outcome, ScanOutcome::CompletedWithErrors);
    assert_eq!(report.removed, 0);
    assert_eq!(
        crate::backend::repo::tracks::for_root(&store, root.id)
            .unwrap()
            .len(),
        1
    );
    assert!(
        crate::backend::repo::scan_history::recent(&store, root.id, 1).unwrap()[0]
            .removals_suppressed
    );
}

#[test]
fn progress_callbacks_can_read_committed_rows_from_a_second_connection() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    let database = temp.path().join("library.sqlite");
    fs::create_dir(&music).unwrap();
    metadata::write_test_wav(&music.join("track.wav"), "Track", "Artist", "Album").unwrap();
    let mut store = LibraryStore::open(&database).unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();
    let mut visible_tracks = 0;

    storage_root(
        &mut store,
        root.id,
        temp.path().join("covers"),
        |progress| {
            if matches!(progress, ScanProgress::Processing { .. }) {
                visible_tracks = crate::backend::repo::tracks::root_summary(
                    &LibraryStore::open(&database).unwrap(),
                    root.id,
                )
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
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();

    let report = storage_root(&mut store, root.id, &unusable_cache_path, |_| {}).unwrap();

    assert_eq!(report.outcome, ScanOutcome::CompletedWithErrors);
    assert_eq!(report.added, 1);
    assert_eq!(report.errors.len(), 1);
    let tracks = crate::backend::repo::tracks::for_root(&store, root.id).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title.as_deref(), Some("Track"));
    assert!(tracks[0].cover_art_path.is_none());
}

#[test]
fn folder_art_prefers_conventional_names_and_extensions() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    let cache = temp.path().join("covers");
    fs::create_dir(&music).unwrap();
    metadata::write_test_wav(&music.join("track.wav"), "Track", "Artist", "Album").unwrap();
    fs::write(music.join("front.png"), test_art(4)).unwrap();
    fs::write(music.join("Folder.JPEG"), test_art(3)).unwrap();
    fs::write(music.join("cover.png"), test_art(2)).unwrap();
    fs::write(music.join("COVER.JPG"), test_art(1)).unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();

    storage_root(&mut store, root.id, &cache, |_| {}).unwrap();

    let track = &crate::backend::repo::tracks::for_root(&store, root.id).unwrap()[0];
    assert_eq!(track.cover_art_mime_type.as_deref(), Some("image/jpeg"));
    assert_eq!(
        fs::read(track.cover_art_path.as_ref().unwrap()).unwrap(),
        test_art(1)
    );
}

#[test]
fn folder_art_ignores_booklet_scans() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    let cache = temp.path().join("covers");
    fs::create_dir(&music).unwrap();
    metadata::write_test_wav(&music.join("track.wav"), "Track", "Artist", "Album").unwrap();
    fs::write(music.join("P001.jpg"), TEST_PNG).unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();

    storage_root(&mut store, root.id, &cache, |_| {}).unwrap();

    assert!(
        crate::backend::repo::tracks::for_root(&store, root.id).unwrap()[0]
            .cover_art_path
            .is_none()
    );
    assert!(!cache.exists());
}

#[test]
fn incremental_scan_fills_missing_cover_without_rereading_audio() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    let cache = temp.path().join("covers");
    fs::create_dir(&music).unwrap();
    let track_path = music.join("track.wav");
    metadata::write_test_wav(&track_path, "Track", "Artist", "Album").unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();
    storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    assert!(
        crate::backend::repo::tracks::for_root(&store, root.id).unwrap()[0]
            .cover_art_path
            .is_none()
    );
    let modified = fs::metadata(&track_path).unwrap().modified().unwrap();
    fs::write(&track_path, b"invalid audio that must remain mtime-skipped").unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&track_path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    fs::write(music.join("Folder.PNG"), test_art(1)).unwrap();

    let report = storage_root(&mut store, root.id, &cache, |_| {}).unwrap();

    assert_eq!((report.updated, report.skipped), (1, 0));
    let track = &crate::backend::repo::tracks::for_root(&store, root.id).unwrap()[0];
    assert_eq!(track.cover_art_mime_type.as_deref(), Some("image/png"));
    assert_eq!(
        fs::read(track.cover_art_path.as_ref().unwrap()).unwrap(),
        test_art(1)
    );
}

#[test]
fn folder_art_rescan_moves_the_cover_to_a_new_content_unique_path() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    let cache = temp.path().join("covers");
    fs::create_dir(&music).unwrap();
    let track_path = music.join("track.wav");
    let art_path = music.join("folder.png");
    metadata::write_test_wav(&track_path, "Track", "Artist", "Album").unwrap();
    fs::write(&art_path, test_art(1)).unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();
    storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let first_track = crate::backend::repo::tracks::for_root(&store, root.id)
        .unwrap()
        .remove(0);
    let first_cover_path = first_track.cover_art_path.unwrap();

    fs::write(&art_path, test_art(2)).unwrap();
    let previous_modified = fs::metadata(&track_path).unwrap().modified().unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&track_path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(previous_modified + Duration::from_secs(2)))
        .unwrap();
    let report = storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let track = &crate::backend::repo::tracks::for_root(&store, root.id).unwrap()[0];

    assert_eq!(report.updated, 1);
    assert_eq!(track.id, first_track.id);
    let second_cover_path = track.cover_art_path.as_ref().unwrap();
    assert_ne!(
        second_cover_path, &first_cover_path,
        "changed art must live at a new path so path-keyed image caches refresh"
    );
    assert_eq!(fs::read(second_cover_path).unwrap(), test_art(2));
    assert!(
        !first_cover_path.exists(),
        "the superseded cover file is removed"
    );
    assert_eq!(fs::read_dir(cache).unwrap().count(), 1);
}

#[test]
fn cover_cache_paths_are_content_unique_and_deterministic() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    let cache = temp.path().join("covers");
    fs::create_dir(&music).unwrap();
    metadata::write_test_wav(&music.join("track.wav"), "Track", "Artist", "Album").unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();
    storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let track_id = crate::backend::repo::tracks::for_root(&store, root.id).unwrap()[0].id;

    let cache_bytes = |store: &mut LibraryStore, data: Vec<u8>, mime: &str| {
        let transaction = store.transaction().unwrap();
        let path = cache_artwork(
            &transaction,
            &cache,
            track_id,
            &metadata::EmbeddedArtwork {
                data,
                mime_type: Some(mime.to_string()),
            },
        )
        .unwrap();
        transaction.commit().unwrap();
        path
    };

    let first = cache_bytes(&mut store, vec![1, 2, 3], "image/png");
    let same_again = cache_bytes(&mut store, vec![1, 2, 3], "image/png");
    let changed = cache_bytes(&mut store, vec![9, 8], "image/jpeg");

    assert_eq!(first, same_again, "identical bytes keep a stable path");
    assert_ne!(first, changed, "different bytes get a different path");
    assert_eq!(fs::read(&changed).unwrap(), [9, 8]);
    let track = &crate::backend::repo::tracks::for_root(&store, root.id).unwrap()[0];
    assert_eq!(track.cover_art_path.as_ref(), Some(&changed));
    assert_eq!(track.cover_art_mime_type.as_deref(), Some("image/jpeg"));
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
    let root = crate::backend::repo::storage_roots::add(
        &mut store,
        std::path::Path::new(&root_path),
        "Real library",
    )
    .unwrap();
    let cache = temp.path().join("covers");

    let first_started = Instant::now();
    let first = storage_root(&mut store, root.id, &cache, print_scan_milestones).unwrap();
    let first_elapsed = first_started.elapsed();
    let second_started = Instant::now();
    let second = storage_root(&mut store, root.id, &cache, print_scan_milestones).unwrap();
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
        println!("duration comparison skipped; set PULSE_REAL_LIBRARY_COMPARE_DURATIONS to opt in");
        return;
    }

    let mut compared = 0;
    let mut divergent = 0;
    for track in crate::backend::repo::tracks::for_root(&store, root.id).unwrap() {
        let Ok(stream) = pulse_engine::decode::open(&track.path) else {
            continue;
        };
        let (Some(frames), Some(library_duration_ms)) = (stream.frames, track.duration_ms) else {
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
