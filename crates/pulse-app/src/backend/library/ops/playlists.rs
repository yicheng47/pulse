use super::super::{
    LibraryError, LibraryStore, Playlist, PlaylistId, PlaylistSummary, PlaylistTrack, TrackId,
};

pub fn create(
    store: &mut LibraryStore,
    name: &str,
    track_ids: &[TrackId],
) -> Result<Playlist, LibraryError> {
    let playlist = store.create_playlist(name)?;
    if !track_ids.is_empty() {
        store.append_playlist_tracks(playlist.id, track_ids)?;
    }
    Ok(playlist)
}

pub fn rename(
    store: &mut LibraryStore,
    playlist_id: PlaylistId,
    name: &str,
) -> Result<Playlist, LibraryError> {
    store.rename_playlist(playlist_id, name)
}

pub fn delete(store: &mut LibraryStore, playlist_id: PlaylistId) -> Result<(), LibraryError> {
    store.delete_playlist(playlist_id)
}

pub fn append_tracks(
    store: &mut LibraryStore,
    playlist_id: PlaylistId,
    track_ids: &[TrackId],
) -> Result<(), LibraryError> {
    store.append_playlist_tracks(playlist_id, track_ids)
}

pub fn remove_entry(
    store: &mut LibraryStore,
    playlist_id: PlaylistId,
    position: usize,
) -> Result<(), LibraryError> {
    store.remove_playlist_entry(playlist_id, position)
}

pub fn move_entry(
    store: &mut LibraryStore,
    playlist_id: PlaylistId,
    from_position: usize,
    to_position: usize,
) -> Result<(), LibraryError> {
    store.move_playlist_entry(playlist_id, from_position, to_position)
}

pub fn list(store: &LibraryStore) -> Result<Vec<PlaylistSummary>, LibraryError> {
    store.playlists()
}

pub fn tracks(
    store: &LibraryStore,
    playlist_id: PlaylistId,
) -> Result<Vec<PlaylistTrack>, LibraryError> {
    store.playlist_tracks(playlist_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::library::{
        LibraryStore,
        repo::testing::{insert_track, test_file, test_metadata},
    };
    use tempfile::tempdir;

    #[test]
    fn playlist_use_cases_create_append_reorder_remove_rename_and_delete() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        let metadata = test_metadata("Track", "Artist", Some("Album"), None);
        let first = insert_track(
            &mut store,
            &root,
            &test_file(&root, "first.wav", 1, 10),
            &metadata,
        );
        let second = insert_track(
            &mut store,
            &root,
            &test_file(&root, "second.wav", 2, 10),
            &metadata,
        );

        let playlist = create(&mut store, "Queue", &[first]).unwrap();
        append_tracks(&mut store, playlist.id, &[second]).unwrap();
        assert_eq!(
            tracks(&store, playlist.id)
                .unwrap()
                .iter()
                .map(|entry| entry.track.id)
                .collect::<Vec<_>>(),
            [first, second]
        );

        move_entry(&mut store, playlist.id, 0, 1).unwrap();
        assert_eq!(
            tracks(&store, playlist.id)
                .unwrap()
                .iter()
                .map(|entry| entry.track.id)
                .collect::<Vec<_>>(),
            [second, first]
        );
        remove_entry(&mut store, playlist.id, 0).unwrap();
        assert_eq!(tracks(&store, playlist.id).unwrap()[0].track.id, first);

        let renamed = rename(&mut store, playlist.id, "Renamed").unwrap();
        assert_eq!(renamed.name, "Renamed");
        assert_eq!(list(&store).unwrap()[0].track_count, 1);

        delete(&mut store, playlist.id).unwrap();
        assert!(list(&store).unwrap().is_empty());
    }
}
