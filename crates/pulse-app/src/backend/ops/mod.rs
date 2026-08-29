use std::path::Path;

use super::{BackfillProgress, LibraryError, LibraryStore};

pub mod catalog;
pub mod delete;
pub mod playlists;
pub mod scan;
pub mod storage;

pub type Store = LibraryStore;

pub fn open(path: impl AsRef<Path>) -> Result<Store, LibraryError> {
    LibraryStore::open(path)
}

pub fn open_with_progress(
    path: impl AsRef<Path>,
    on_backfill_progress: impl FnMut(BackfillProgress),
) -> Result<Store, LibraryError> {
    LibraryStore::open_with_progress(path, on_backfill_progress)
}
