use std::{collections::HashMap, fs, io};

use crate::backend::{
    model::{DeleteAlbumOutcome, LibraryError, StorageRootId, Track, TrackId},
    repo::{LibraryStore, storage_roots, tracks as track_repo},
};

/// Attempt to delete each track's audio file from disk. A missing file
/// counts as deleted only while its storage root is reachable — an offline
/// or unmounted root yields the same NotFound for every child, and treating
/// that as success would drop rows for audio that still exists on the
/// disconnected volume. Returns the ids whose files are verifiably gone
/// (safe to remove from the library) and a message per file that could not
/// be deleted; those tracks keep their rows so the library never claims a
/// deletion that did not happen.
pub fn track_files(
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

/// Delete an album: unlink its audio files, remove the rows and playlist
/// entries for verifiably-gone files, then clean up their cover-cache
/// entries. The database write is preflighted before the first unlink so a
/// locked or read-only library fails the whole operation while everything
/// still exists. Runs on a worker thread — file I/O against music storage
/// never runs on the UI thread.
pub fn album_tracks(
    store: &mut LibraryStore,
    artist: &str,
    title: &str,
) -> Result<DeleteAlbumOutcome, LibraryError> {
    let tracks = track_repo::for_album(store, artist, title)?;
    let roots = storage_roots::list(store)?
        .into_iter()
        .map(|root| (root.id, root.path))
        .collect::<HashMap<_, _>>();
    track_repo::preflight_write(store)?;
    let (deleted_ids, mut failures) = track_files(&tracks, |root_id| {
        roots.get(&root_id).is_some_and(|path| path.exists())
    });
    let db_error = if deleted_ids.is_empty() {
        None
    } else {
        track_repo::delete_tracks(store, &deleted_ids)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{
        LibraryStore,
        repo::testing::{
            break_playlist_entries, insert_track, set_cover, test_file, test_metadata,
        },
    };
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn delete_track_files_tolerates_missing_files_and_keeps_undeletable_tracks() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
        let present = test_file(&root, "present.wav", 1, 10);
        fs::write(&present.path, b"audio").unwrap();
        let vanished = test_file(&root, "vanished.wav", 2, 10);
        let blocked = test_file(&root, "blocked.wav", 3, 10);
        fs::create_dir(&blocked.path).unwrap();
        let metadata = test_metadata("Track", "Artist", Some("Album"), None);
        let present_id = insert_track(&mut store, &root, &present, &metadata);
        let vanished_id = insert_track(&mut store, &root, &vanished, &metadata);
        insert_track(&mut store, &root, &blocked, &metadata);

        let tracks = crate::backend::repo::tracks::for_root(&store, root.id).unwrap();
        let (mut deleted, failures) = track_files(&tracks, |_| true);
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
        let temp = tempdir().unwrap();
        let music = temp.path().join("music");
        fs::create_dir(&music).unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = crate::backend::repo::storage_roots::add(&mut store, &music, "Music").unwrap();
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

        let outcome = album_tracks(&mut store, "Artist", "Album").unwrap();

        assert_eq!(outcome.deleted_files, 0);
        assert_eq!(outcome.total_files, 2);
        assert_eq!(outcome.failures.len(), 2);
        assert!(outcome.failures.iter().all(|f| f.contains("offline")));
        assert!(outcome.db_error.is_none());
        assert_eq!(
            crate::backend::repo::tracks::for_album(&store, "Artist", "Album")
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn delete_album_tracks_reports_db_failure_after_files_are_gone() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
        let file = test_file(&root, "doomed.wav", 1, 10);
        fs::write(&file.path, b"audio").unwrap();
        insert_track(
            &mut store,
            &root,
            &file,
            &test_metadata("Track", "Artist", Some("Album"), None),
        );
        break_playlist_entries(&mut store);

        let outcome = album_tracks(&mut store, "Artist", "Album").unwrap();

        assert_eq!(outcome.deleted_files, 1);
        assert!(outcome.db_error.is_some());
        assert!(!file.path.exists());
        assert_eq!(
            crate::backend::repo::tracks::for_album(&store, "Artist", "Album")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn delete_album_tracks_removes_files_rows_and_covers() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
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

        let outcome = album_tracks(&mut store, "Artist", "Album").unwrap();

        assert_eq!(outcome.deleted_files, 1);
        assert!(outcome.failures.is_empty());
        assert!(outcome.db_error.is_none());
        assert!(!file.path.exists());
        assert!(!cover.exists());
        assert!(
            crate::backend::repo::tracks::for_album(&store, "Artist", "Album")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn delete_album_treats_a_missing_file_on_a_reachable_root_as_deleted() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
        let track_id = insert_track(
            &mut store,
            &root,
            &test_file(&root, "missing.wav", 1, 10),
            &test_metadata("Missing", "Artist", Some("Album"), None),
        );

        let outcome = album_tracks(&mut store, "Artist", "Album").unwrap();

        assert_eq!(outcome.deleted_ids, [track_id]);
        assert_eq!(outcome.deleted_files, 1);
        assert_eq!(outcome.total_files, 1);
        assert!(outcome.failures.is_empty());
        assert!(outcome.db_error.is_none());
        assert!(
            crate::backend::repo::tracks::for_album(&store, "Artist", "Album")
                .unwrap()
                .is_empty()
        );
    }
}
