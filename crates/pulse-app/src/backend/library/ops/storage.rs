use std::path::{Path, PathBuf};

use super::super::{
    LibraryError, LibraryStore, LibrarySummary, ScanHistoryEntry, StorageRoot, StorageRootId,
};

pub fn add(
    store: &mut LibraryStore,
    path: impl AsRef<Path>,
    display_name: impl AsRef<str>,
) -> Result<StorageRoot, LibraryError> {
    store.add_storage_root(path, display_name)
}

pub fn rename(
    store: &mut LibraryStore,
    storage_root_id: StorageRootId,
    display_name: impl AsRef<str>,
) -> Result<StorageRoot, LibraryError> {
    store.rename_storage_root(storage_root_id, display_name)
}

pub fn remove(
    store: &mut LibraryStore,
    storage_root_id: StorageRootId,
) -> Result<Vec<PathBuf>, LibraryError> {
    store.remove_storage_root(storage_root_id)
}

pub fn list(store: &LibraryStore) -> Result<Vec<StorageRoot>, LibraryError> {
    store.storage_roots()
}

pub fn get(
    store: &LibraryStore,
    storage_root_id: StorageRootId,
) -> Result<Option<StorageRoot>, LibraryError> {
    store.storage_root(storage_root_id)
}

pub fn summary(
    store: &LibraryStore,
    storage_root_id: StorageRootId,
) -> Result<LibrarySummary, LibraryError> {
    store.root_summary(storage_root_id)
}

pub fn recent_scans(
    store: &LibraryStore,
    storage_root_id: StorageRootId,
    limit: usize,
) -> Result<Vec<ScanHistoryEntry>, LibraryError> {
    store.recent_scans(storage_root_id, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::library::{
        LibraryStore,
        repo::testing::{insert_track, set_cover, test_file, test_metadata},
    };
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn storage_root_use_cases_add_rename_list_and_remove() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = add(&mut store, temp.path(), "Music").unwrap();

        assert_eq!(get(&store, root.id).unwrap(), Some(root.clone()));
        assert_eq!(list(&store).unwrap(), std::slice::from_ref(&root));
        assert_eq!(summary(&store, root.id).unwrap(), LibrarySummary::default());
        assert!(recent_scans(&store, root.id, 1).unwrap().is_empty());

        let renamed = rename(&mut store, root.id, "Library").unwrap();
        assert_eq!(renamed.display_name, "Library");

        let file = test_file(&renamed, "track.wav", 1, 10);
        let track_id = insert_track(
            &mut store,
            &renamed,
            &file,
            &test_metadata("Track", "Artist", Some("Album"), None),
        );
        let cover = temp.path().join("cover.png");
        fs::write(&cover, b"art").unwrap();
        set_cover(&mut store, track_id, &cover);

        assert_eq!(remove(&mut store, renamed.id).unwrap(), [cover]);
        assert!(list(&store).unwrap().is_empty());
    }
}
