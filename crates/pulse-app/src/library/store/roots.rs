use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::{Connection, OptionalExtension, params};

use super::super::{
    LibraryError, StorageRoot, StorageRootId, path::normalize_storage_root, system_time_ms,
};

pub fn add(
    conn: &Connection,
    path: &Path,
    display_name: &str,
) -> Result<StorageRoot, LibraryError> {
    let normalized = normalize_storage_root(path)?;
    let added_at_ms = system_time_ms(SystemTime::now())?;
    let id = conn.query_row(
        "INSERT INTO storage_roots (
             path, path_key, display_name, added_at_ms, is_reachable, is_case_sensitive
         ) VALUES (?1, ?2, ?3, ?4, 1, ?5)
         RETURNING id",
        params![
            normalized.path_text,
            normalized.path_key,
            display_name,
            added_at_ms,
            normalized.is_case_sensitive,
        ],
        |row| row.get(0),
    )?;

    get(conn, id)?.ok_or(LibraryError::StorageRootNotFound(id))
}

pub fn get(
    conn: &Connection,
    storage_root_id: StorageRootId,
) -> Result<Option<StorageRoot>, LibraryError> {
    conn.query_row(
        "SELECT id, path, display_name, added_at_ms, last_scan_at_ms,
                is_reachable, is_case_sensitive
         FROM storage_roots
         WHERE id = ?1",
        [storage_root_id],
        storage_root_from_row,
    )
    .optional()
    .map_err(Into::into)
}

pub fn list(conn: &Connection) -> Result<Vec<StorageRoot>, LibraryError> {
    let mut statement = conn.prepare(
        "SELECT id, path, display_name, added_at_ms, last_scan_at_ms,
                is_reachable, is_case_sensitive
         FROM storage_roots
         ORDER BY added_at_ms, id",
    )?;
    let roots = statement
        .query_map([], storage_root_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(roots)
}

pub fn rename(
    conn: &Connection,
    storage_root_id: StorageRootId,
    display_name: &str,
) -> Result<StorageRoot, LibraryError> {
    let updated = conn.execute(
        "UPDATE storage_roots SET display_name = ?2 WHERE id = ?1",
        params![storage_root_id, display_name],
    )?;
    if updated == 0 {
        return Err(LibraryError::StorageRootNotFound(storage_root_id));
    }
    get(conn, storage_root_id)?.ok_or(LibraryError::StorageRootNotFound(storage_root_id))
}

/// Deletes the root row alone; callers clear the root's children first.
/// Returns the number of rows deleted so callers can map zero to not-found.
pub fn delete(conn: &Connection, storage_root_id: StorageRootId) -> Result<usize, LibraryError> {
    Ok(conn.execute("DELETE FROM storage_roots WHERE id = ?1", [storage_root_id])?)
}

pub fn mark_reachable(
    conn: &Connection,
    storage_root_id: StorageRootId,
) -> Result<(), LibraryError> {
    conn.execute(
        "UPDATE storage_roots SET is_reachable = 1 WHERE id = ?1",
        [storage_root_id],
    )?;
    Ok(())
}

/// Stamps a finished scan onto the root: always the last-scan time, and the
/// reachability verdict when the scan produced one (a failed scan does not).
pub fn record_scan_outcome(
    conn: &Connection,
    storage_root_id: StorageRootId,
    last_scan_at_ms: i64,
    is_reachable: Option<bool>,
) -> Result<(), LibraryError> {
    match is_reachable {
        Some(is_reachable) => conn.execute(
            "UPDATE storage_roots
             SET is_reachable = ?3, last_scan_at_ms = ?2
             WHERE id = ?1",
            params![storage_root_id, last_scan_at_ms, is_reachable],
        )?,
        None => conn.execute(
            "UPDATE storage_roots
             SET last_scan_at_ms = ?2
             WHERE id = ?1",
            params![storage_root_id, last_scan_at_ms],
        )?,
    };
    Ok(())
}

fn storage_root_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StorageRoot> {
    Ok(StorageRoot {
        id: row.get(0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
        display_name: row.get(2)?,
        added_at_ms: row.get(3)?,
        last_scan_at_ms: row.get(4)?,
        is_reachable: row.get(5)?,
        is_case_sensitive: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::library::{
        LibraryStore,
        metadata::{AudioMetadata, EmbeddedArtwork},
        store::{
            testing::{insert_track, test_file, test_metadata},
            tracks::set_track_cover,
        },
    };

    #[test]
    fn renames_a_storage_root() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Before").unwrap();

        let renamed = store.rename_storage_root(root.id, "After").unwrap();

        assert_eq!(renamed.display_name, "After");
        assert_eq!(
            store.storage_root(root.id).unwrap().unwrap().display_name,
            "After"
        );
    }

    #[test]
    fn removing_a_root_deletes_related_rows_and_returns_cover_paths() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        // FK enforcement off: only the explicit application-owned deletes may
        // be responsible for clearing children.
        store
            .connection
            .execute_batch("PRAGMA foreign_keys = OFF")
            .unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        let file = test_file(&root, "track.wav", 1, 10);
        let id = insert_track(
            &mut store,
            &root,
            &file,
            &AudioMetadata {
                artwork: Some(EmbeddedArtwork {
                    data: vec![1, 2, 3],
                    mime_type: Some("image/png".to_string()),
                }),
                ..test_metadata("Track", "Artist", Some("Album"), None)
            },
        );
        let cover_path = temp.path().join("cover.cache");
        {
            let transaction = store.connection.transaction().unwrap();
            set_track_cover(
                &transaction,
                id,
                cover_path.to_str().unwrap(),
                Some("image/png"),
            )
            .unwrap();
            transaction.commit().unwrap();
        }

        let playlist = store.create_playlist("Keeper").unwrap();
        store.append_playlist_tracks(playlist.id, &[id]).unwrap();
        store.begin_scan(root.id, 100).unwrap();

        assert_eq!(
            store.remove_storage_root(root.id).unwrap(),
            vec![cover_path]
        );
        assert!(store.storage_root(root.id).unwrap().is_none());
        assert!(store.tracks_for_root(root.id).unwrap().is_empty());
        let count = |sql: &str| -> i64 {
            store
                .connection
                .query_row(sql, [root.id], |row| row.get(0))
                .unwrap()
        };
        assert_eq!(
            count("SELECT COUNT(*) FROM scan_history WHERE storage_root_id = ?1"),
            0,
            "scan history is deleted explicitly"
        );
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM playlist_tracks", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "playlist entries of the root's tracks are deleted explicitly"
        );
        assert_eq!(
            store.playlist(playlist.id).unwrap().unwrap().name,
            "Keeper",
            "the playlist itself survives"
        );
    }
}
