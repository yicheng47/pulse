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

#[test]
fn delete_track_files_tolerates_missing_files_and_keeps_undeletable_tracks() {
    use super::store::testing::{insert_track, test_file, test_metadata};

    let temp = tempdir().unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = store.add_storage_root(temp.path(), "Music").unwrap();
    let present = test_file(&root, "present.wav", 1, 10);
    fs::write(&present.path, b"audio").unwrap();
    let vanished = test_file(&root, "vanished.wav", 2, 10);
    let blocked = test_file(&root, "blocked.wav", 3, 10);
    fs::create_dir(&blocked.path).unwrap();
    let metadata = test_metadata("Track", "Artist", Some("Album"), None);
    let present_id = insert_track(&mut store, &root, &present, &metadata);
    let vanished_id = insert_track(&mut store, &root, &vanished, &metadata);
    insert_track(&mut store, &root, &blocked, &metadata);

    let tracks = store.tracks_for_root(root.id).unwrap();
    let (mut deleted, failures) = delete_track_files(&tracks, |_| true);
    deleted.sort_unstable();

    let mut expected = vec![present_id, vanished_id];
    expected.sort_unstable();
    assert_eq!(deleted, expected);
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("blocked.wav"));
    assert!(!present.path.exists());
    assert!(blocked.path.exists());
}

#[test]
fn delete_album_tracks_keeps_rows_when_the_root_went_offline() {
    use super::store::testing::{insert_track, test_file, test_metadata};

    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    fs::create_dir(&music).unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = store.add_storage_root(&music, "Music").unwrap();
    let metadata = test_metadata("Track", "Artist", Some("Album"), None);
    insert_track(
        &mut store,
        &root,
        &test_file(&root, "one.wav", 1, 10),
        &metadata,
    );
    insert_track(
        &mut store,
        &root,
        &test_file(&root, "two.wav", 2, 10),
        &metadata,
    );
    fs::rename(&music, temp.path().join("unmounted")).unwrap();

    let outcome = delete_album_tracks(&mut store, "Artist", "Album").unwrap();

    assert_eq!(outcome.deleted_files, 0);
    assert_eq!(outcome.total_files, 2);
    assert_eq!(outcome.failures.len(), 2);
    assert!(outcome.failures.iter().all(|f| f.contains("offline")));
    assert!(outcome.db_error.is_none());
    assert_eq!(store.tracks_for_album("Artist", "Album").unwrap().len(), 2);
}

#[test]
fn delete_album_tracks_reports_db_failure_after_files_are_gone() {
    use super::store::testing::{break_playlist_entries, insert_track, test_file, test_metadata};

    let temp = tempdir().unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = store.add_storage_root(temp.path(), "Music").unwrap();
    let file = test_file(&root, "doomed.wav", 1, 10);
    fs::write(&file.path, b"audio").unwrap();
    insert_track(
        &mut store,
        &root,
        &file,
        &test_metadata("Track", "Artist", Some("Album"), None),
    );
    break_playlist_entries(&mut store);

    let outcome = delete_album_tracks(&mut store, "Artist", "Album").unwrap();

    assert_eq!(outcome.deleted_files, 1);
    assert!(outcome.db_error.is_some());
    assert!(!file.path.exists());
    assert_eq!(store.tracks_for_album("Artist", "Album").unwrap().len(), 1);
}

#[test]
fn delete_album_tracks_removes_files_rows_and_covers() {
    use super::store::testing::{insert_track, set_cover, test_file, test_metadata};

    let temp = tempdir().unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = store.add_storage_root(temp.path(), "Music").unwrap();
    let file = test_file(&root, "track.wav", 1, 10);
    fs::write(&file.path, b"audio").unwrap();
    let track_id = insert_track(
        &mut store,
        &root,
        &file,
        &test_metadata("Track", "Artist", Some("Album"), None),
    );
    let cover = temp.path().join("cover.png");
    fs::write(&cover, b"art").unwrap();
    set_cover(&mut store, track_id, &cover);

    let outcome = delete_album_tracks(&mut store, "Artist", "Album").unwrap();

    assert_eq!(outcome.deleted_files, 1);
    assert!(outcome.failures.is_empty());
    assert!(outcome.db_error.is_none());
    assert!(!file.path.exists());
    assert!(!cover.exists());
    assert!(
        store
            .tracks_for_album("Artist", "Album")
            .unwrap()
            .is_empty()
    );
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
        .set_times(fs::FileTimes::new().set_modified(previous_modified + Duration::from_secs(2)))
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
fn scan_adds_updates_and_prunes_materialized_artists() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    let cache = temp.path().join("covers");
    fs::create_dir(&music).unwrap();
    let first_path = music.join("01-first.wav");
    metadata::write_test_wav(&first_path, "First", "Artist", "First Album").unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = store.add_storage_root(&music, "Music").unwrap();

    scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let artists = store.artist_index().unwrap();
    assert_eq!(artists.len(), 1);
    assert_eq!(artists[0].name_key, "Artist");
    assert_eq!((artists[0].album_count, artists[0].track_count), (1, 1));

    let second_path = music.join("02-second.wav");
    metadata::write_test_wav(&second_path, "Second", "Artist", "Second Album").unwrap();
    scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let artists = store.artist_index().unwrap();
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
    scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let artists = store.artist_index().unwrap();
    assert_eq!(artists.len(), 2);
    assert!(artists.iter().any(|artist| {
        artist.name_key == "Artist" && (artist.album_count, artist.track_count) == (1, 1)
    }));
    assert!(artists.iter().any(|artist| {
        artist.name_key == "Other Artist" && (artist.album_count, artist.track_count) == (1, 1)
    }));

    fs::remove_file(&second_path).unwrap();
    scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let artists = store.artist_index().unwrap();
    assert_eq!(artists.len(), 1);
    assert_eq!(artists[0].name_key, "Artist");
    assert_eq!((artists[0].album_count, artists[0].track_count), (1, 1));
}

#[test]
fn cancellation_keeps_partial_commits_without_recording_a_failed_scan() {
    let temp = tempdir().unwrap();
    let music = temp.path().join("music");
    fs::create_dir(&music).unwrap();
    metadata::write_test_wav(&music.join("first.wav"), "First", "Artist", "Album").unwrap();
    metadata::write_test_wav(&music.join("second.wav"), "Second", "Artist", "Album").unwrap();
    let mut store = LibraryStore::open_in_memory().unwrap();
    let root = store.add_storage_root(&music, "Music").unwrap();
    let cancelled = Cell::new(false);

    let report = scan_storage_root_cancellable(
        &mut store,
        root.id,
        temp.path().join("covers"),
        |progress| {
            if matches!(progress, ScanProgress::Processing { .. }) {
                cancelled.set(true);
            }
        },
        || cancelled.get(),
    )
    .unwrap();

    assert!(report.is_none());
    assert_eq!(store.tracks_for_root(root.id).unwrap().len(), 1);
    let artists = store.artist_index().unwrap();
    assert_eq!(artists.len(), 1);
    assert_eq!(artists[0].name_key, "Artist");
    assert_eq!((artists[0].album_count, artists[0].track_count), (1, 1));
    assert!(store.recent_scans(root.id, 1).unwrap().is_empty());
}

#[test]
fn failed_artist_refresh_rolls_back_the_current_scan_track() {
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
    let root = store.add_storage_root(&music, "Music").unwrap();
    store
        .connection
        .execute_batch(
            "CREATE TRIGGER fail_second_artist
             BEFORE INSERT ON artists
             WHEN NEW.name_key = 'Second Artist'
             BEGIN SELECT RAISE(ABORT, 'artist refresh failed'); END;",
        )
        .unwrap();

    let error = scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap_err();

    assert!(error.to_string().contains("artist refresh failed"));
    let tracks = store.tracks_for_root(root.id).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].artist.as_deref(), Some("First Artist"));
    let artists = store.artist_index().unwrap();
    assert_eq!(artists.len(), 1);
    assert_eq!(artists[0].name_key, "First Artist");
    assert_eq!((artists[0].album_count, artists[0].track_count), (1, 1));
    assert_eq!(
        store.recent_scans(root.id, 1).unwrap()[0].outcome,
        Some(ScanOutcome::Failed)
    );
}

#[test]
#[ignore = "agent-runnable Stage 12 library proof; run with --ignored --nocapture"]
fn stage_12_library_ui_proof() {
    use crate::queue::QueueState;
    use crate::surfaces::library::logic::root_row_state;

    let temp = tempdir().unwrap();
    let database_path = temp.path().join("library.sqlite");
    let music = temp.path().join("music");
    let offline_music = temp.path().join("offline-music");
    fs::create_dir(&music).unwrap();
    fs::create_dir(&offline_music).unwrap();
    metadata::write_test_wav(
        &music.join("02-second.wav"),
        "Second",
        "Proof Artist",
        "Proof Album",
    )
    .unwrap();
    metadata::write_test_wav(
        &music.join("01-first.wav"),
        "First",
        "Proof Artist",
        "Proof Album",
    )
    .unwrap();
    metadata::write_test_wav(
        &offline_music.join("offline.wav"),
        "Offline",
        "Other Artist",
        "Other Album",
    )
    .unwrap();

    let mut store = LibraryStore::open(&database_path).unwrap();
    let music_root = store.add_storage_root(&music, "Music").unwrap();
    let offline_root = store
        .add_storage_root(&offline_music, "Offline Music")
        .unwrap();
    scan_storage_root(
        &mut store,
        music_root.id,
        temp.path().join("covers"),
        |_| {},
    )
    .unwrap();
    scan_storage_root(
        &mut store,
        offline_root.id,
        temp.path().join("covers"),
        |_| {},
    )
    .unwrap();
    fs::rename(&offline_music, temp.path().join("disconnected-music")).unwrap();
    scan_storage_root(
        &mut store,
        offline_root.id,
        temp.path().join("covers"),
        |_| {},
    )
    .unwrap();
    drop(store);

    let mut store = LibraryStore::open(&database_path).unwrap();
    let albums = store.albums(AlbumSortOrder::Title).unwrap();
    println!("album count: {}", albums.len());
    let proof_album = albums
        .iter()
        .find(|album| album.title == "Proof Album")
        .unwrap();
    let ordered_tracks = store
        .tracks_for_album(&proof_album.artist, &proof_album.title)
        .unwrap();
    println!(
        "ordered tracks for {} / {}:",
        proof_album.artist, proof_album.title
    );
    for track in &ordered_tracks {
        println!(
            "  disc={:?} track={:?} title={} path={}",
            track.disc_number,
            track.track_number,
            track.title.as_deref().unwrap_or("Untitled"),
            track.path.display()
        );
    }
    for root in store.storage_roots().unwrap() {
        let latest = store.recent_scans(root.id, 1).unwrap().into_iter().next();
        println!(
            "root {}: {:?}",
            root.display_name,
            root_row_state(&root, latest.as_ref(), false)
        );
    }

    let playlist = store.create_playlist("Proof Playlist").unwrap();
    let track_ids = ordered_tracks
        .iter()
        .map(|track| track.id)
        .collect::<Vec<_>>();
    store
        .append_playlist_tracks(playlist.id, &track_ids)
        .unwrap();
    store.move_playlist_entry(playlist.id, 0, 1).unwrap();
    let playlist_tracks = store
        .playlist_tracks(playlist.id)
        .unwrap()
        .into_iter()
        .map(|entry| entry.track)
        .collect::<Vec<_>>();
    println!("playlist after first-to-last reorder:");
    for track in &playlist_tracks {
        println!("  {}", track.title.as_deref().unwrap_or("Untitled"));
    }
    println!("queue order when playing from the middle row:");
    let mut queue = QueueState::from_tracks(&playlist_tracks, 1);
    if let Some(track) = queue.current() {
        println!("  {}", track.title);
    }
    while let Some(track) = queue.advance() {
        println!("  {}", track.title);
    }

    assert_eq!(albums.len(), 2);
    assert_eq!(ordered_tracks.len(), 2);
    assert_eq!(playlist_tracks.len(), 2);
    assert_eq!(queue.remaining_count(), 0);
    assert!(
        !store
            .storage_root(offline_root.id)
            .unwrap()
            .unwrap()
            .is_reachable
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
    metadata::write_test_wav_with_format(&music.join("float.wav"), "Float", "Artist", "Album", 3)
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
    let root = store.add_storage_root(&music, "Music").unwrap();

    scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();

    let track = &store.tracks_for_root(root.id).unwrap()[0];
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
    let root = store.add_storage_root(&music, "Music").unwrap();

    scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();

    assert!(
        store.tracks_for_root(root.id).unwrap()[0]
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
    let root = store.add_storage_root(&music, "Music").unwrap();
    scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    assert!(
        store.tracks_for_root(root.id).unwrap()[0]
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

    let report = scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();

    assert_eq!((report.updated, report.skipped), (1, 0));
    let track = &store.tracks_for_root(root.id).unwrap()[0];
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
    let root = store.add_storage_root(&music, "Music").unwrap();
    scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let first_track = store.tracks_for_root(root.id).unwrap().remove(0);
    let first_cover_path = first_track.cover_art_path.unwrap();

    fs::write(&art_path, test_art(2)).unwrap();
    let previous_modified = fs::metadata(&track_path).unwrap().modified().unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&track_path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(previous_modified + Duration::from_secs(2)))
        .unwrap();
    let report = scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let track = &store.tracks_for_root(root.id).unwrap()[0];

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
    let root = store.add_storage_root(&music, "Music").unwrap();
    scan_storage_root(&mut store, root.id, &cache, |_| {}).unwrap();
    let track_id = store.tracks_for_root(root.id).unwrap()[0].id;

    let cache_bytes = |store: &mut LibraryStore, data: Vec<u8>, mime: &str| {
        let transaction = store.connection.transaction().unwrap();
        let path = cache_artwork(
            &transaction,
            &cache,
            track_id,
            metadata::EmbeddedArtwork {
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
    let track = &store.tracks_for_root(root.id).unwrap()[0];
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
        println!("duration comparison skipped; set PULSE_REAL_LIBRARY_COMPARE_DURATIONS to opt in");
        return;
    }

    let mut compared = 0;
    let mut divergent = 0;
    for track in store.tracks_for_root(root.id).unwrap() {
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
