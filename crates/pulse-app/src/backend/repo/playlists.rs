use std::{path::PathBuf, time::SystemTime};

use rusqlite::{OptionalExtension, Transaction, params};

use super::super::{
    LibraryError, Playlist, PlaylistId, PlaylistSummary, PlaylistTrack, StorageRootId, TrackId,
    scan::system_time_ms,
};
use super::{
    LibraryStore, LibraryTransaction, qualified_select_list, select_list, tracks, usize_to_i64,
};

const COLUMNS: &[&str] = &["id", "name", "created_at_ms", "updated_at_ms"];
const PLAYLIST_TRACK_COLUMNS: &[&str] = &["playlist_id", "track_id", "position"];

struct PlaylistRow {
    id: PlaylistId,
    name: String,
    created_at_ms: i64,
    updated_at_ms: i64,
}

struct PlaylistTrackRow {
    _playlist_id: PlaylistId,
    track_id: TrackId,
    position: i64,
}

struct PlaylistSummaryRow {
    playlist: Playlist,
    track_count: i64,
    total_duration_ms: i64,
    cover_art_path: Option<String>,
}

impl From<PlaylistRow> for Playlist {
    fn from(row: PlaylistRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            created_at_ms: row.created_at_ms,
            updated_at_ms: row.updated_at_ms,
        }
    }
}

/// Shared head of the playlist-summary aggregation; `list` and search append
/// their own WHERE/ORDER clauses so both read the same counts and cover.
pub fn playlist_summary_select() -> String {
    let columns = qualified_select_list("p", COLUMNS);
    format!(
        "SELECT {columns}, COUNT(pt.track_id), COALESCE(SUM(t.duration_ms), 0),
            (
                SELECT t2.cover_art_path
                FROM playlist_tracks pt2
                JOIN tracks t2 ON t2.id = pt2.track_id
                WHERE pt2.playlist_id = p.id AND t2.cover_art_path IS NOT NULL
                ORDER BY pt2.position
                LIMIT 1
            )
     FROM playlists p
     LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
     LEFT JOIN tracks t ON t.id = pt.track_id"
    )
}

pub fn create(store: &mut LibraryStore, name: &str) -> Result<Playlist, LibraryError> {
    let conn = &store.connection;
    let now_ms = system_time_ms(SystemTime::now())?;
    let id = conn.query_row(
        "INSERT INTO playlists (name, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?2)
         RETURNING id",
        params![name, now_ms],
        |row| row.get(0),
    )?;
    get(store, id)?.ok_or(LibraryError::PlaylistNotFound(id))
}

pub fn get(
    store: &LibraryStore,
    playlist_id: PlaylistId,
) -> Result<Option<Playlist>, LibraryError> {
    let conn = &store.connection;
    let columns = select_list(COLUMNS);
    let sql = format!(
        "SELECT {columns}
         FROM playlists
         WHERE id = ?1"
    );
    conn.query_row(&sql, [playlist_id], playlist_from_row)
        .optional()
        .map_err(Into::into)
}

pub fn rename(
    store: &mut LibraryStore,
    playlist_id: PlaylistId,
    name: &str,
) -> Result<Playlist, LibraryError> {
    let conn = &store.connection;
    let now_ms = system_time_ms(SystemTime::now())?;
    let updated = conn.execute(
        "UPDATE playlists SET name = ?2, updated_at_ms = ?3 WHERE id = ?1",
        params![playlist_id, name, now_ms],
    )?;
    if updated == 0 {
        return Err(LibraryError::PlaylistNotFound(playlist_id));
    }
    get(store, playlist_id)?.ok_or(LibraryError::PlaylistNotFound(playlist_id))
}

pub fn delete(
    transaction: &LibraryTransaction<'_>,
    playlist_id: PlaylistId,
) -> Result<(), LibraryError> {
    let transaction = &transaction.inner;
    transaction.execute(
        "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
        [playlist_id],
    )?;
    let deleted = transaction.execute("DELETE FROM playlists WHERE id = ?1", [playlist_id])?;
    if deleted == 0 {
        return Err(LibraryError::PlaylistNotFound(playlist_id));
    }
    Ok(())
}

pub fn delete_transactional(
    store: &mut LibraryStore,
    playlist_id: PlaylistId,
) -> Result<(), LibraryError> {
    let transaction = store.transaction()?;
    delete(&transaction, playlist_id)?;
    transaction.commit()?;
    Ok(())
}

pub fn append_tracks(
    transaction: &LibraryTransaction<'_>,
    playlist_id: PlaylistId,
    track_ids: &[TrackId],
) -> Result<(), LibraryError> {
    let transaction = &transaction.inner;
    require_playlist(transaction, playlist_id)?;
    if track_ids.is_empty() {
        return Ok(());
    }
    let next_position = transaction.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1
         FROM playlist_tracks
         WHERE playlist_id = ?1",
        [playlist_id],
        |row| row.get::<_, i64>(0),
    )?;
    for (offset, track_id) in track_ids.iter().enumerate() {
        let offset = usize_to_i64(offset, "playlist position")?;
        transaction.execute(
            "INSERT INTO playlist_tracks (playlist_id, track_id, position)
             VALUES (?1, ?2, ?3)",
            params![playlist_id, track_id, next_position + offset],
        )?;
    }
    touch_playlist(transaction, playlist_id)?;
    Ok(())
}

pub fn append_tracks_transactional(
    store: &mut LibraryStore,
    playlist_id: PlaylistId,
    track_ids: &[TrackId],
) -> Result<(), LibraryError> {
    let transaction = store.transaction()?;
    append_tracks(&transaction, playlist_id, track_ids)?;
    transaction.commit()?;
    Ok(())
}

pub fn remove_entry(
    transaction: &LibraryTransaction<'_>,
    playlist_id: PlaylistId,
    position: usize,
) -> Result<(), LibraryError> {
    let transaction = &transaction.inner;
    require_playlist(transaction, playlist_id)?;
    let mut entries = playlist_entry_ids(transaction, playlist_id)?;
    let Some(index) = entries
        .iter()
        .position(|(stored_position, _)| *stored_position == position)
    else {
        return Err(LibraryError::PlaylistEntryNotFound {
            playlist_id,
            position,
        });
    };
    entries.remove(index);
    replace_playlist_entries(transaction, playlist_id, &entries)?;
    touch_playlist(transaction, playlist_id)?;
    Ok(())
}

pub fn remove_entry_transactional(
    store: &mut LibraryStore,
    playlist_id: PlaylistId,
    position: usize,
) -> Result<(), LibraryError> {
    let transaction = store.transaction()?;
    remove_entry(&transaction, playlist_id, position)?;
    transaction.commit()?;
    Ok(())
}

/// Moves the entry at a stored position to a zero-based index in the ordered entries.
pub fn move_entry(
    transaction: &LibraryTransaction<'_>,
    playlist_id: PlaylistId,
    from_position: usize,
    to_position: usize,
) -> Result<(), LibraryError> {
    let transaction = &transaction.inner;
    require_playlist(transaction, playlist_id)?;
    let mut entries = playlist_entry_ids(transaction, playlist_id)?;
    let Some(from_index) = entries
        .iter()
        .position(|(stored_position, _)| *stored_position == from_position)
    else {
        return Err(LibraryError::PlaylistEntryNotFound {
            playlist_id,
            position: from_position,
        });
    };
    if to_position >= entries.len() {
        return Err(LibraryError::PlaylistEntryNotFound {
            playlist_id,
            position: to_position,
        });
    }
    if from_index != to_position {
        let entry = entries.remove(from_index);
        entries.insert(to_position, entry);
        replace_playlist_entries(transaction, playlist_id, &entries)?;
        touch_playlist(transaction, playlist_id)?;
    }
    Ok(())
}

pub fn move_entry_transactional(
    store: &mut LibraryStore,
    playlist_id: PlaylistId,
    from_position: usize,
    to_position: usize,
) -> Result<(), LibraryError> {
    let transaction = store.transaction()?;
    move_entry(&transaction, playlist_id, from_position, to_position)?;
    transaction.commit()?;
    Ok(())
}

pub fn list(store: &LibraryStore) -> Result<Vec<PlaylistSummary>, LibraryError> {
    let conn = &store.connection;
    let summary_select = playlist_summary_select();
    let sql = format!(
        "{summary_select}
         GROUP BY p.id
         ORDER BY p.updated_at_ms DESC, p.id DESC"
    );
    let mut statement = conn.prepare(&sql)?;
    let playlists = statement
        .query_map([], playlist_summary_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(playlists)
}

pub fn tracks(
    store: &LibraryStore,
    playlist_id: PlaylistId,
) -> Result<Vec<PlaylistTrack>, LibraryError> {
    if get(store, playlist_id)?.is_none() {
        return Err(LibraryError::PlaylistNotFound(playlist_id));
    }
    let conn = &store.connection;
    let playlist_track_columns = qualified_select_list("pt", PLAYLIST_TRACK_COLUMNS);
    let sql = format!(
        "SELECT {playlist_track_columns}, {}
         FROM playlist_tracks pt
         JOIN tracks t ON t.id = pt.track_id
         WHERE pt.playlist_id = ?1
         ORDER BY pt.position",
        tracks::qualified_track_columns("t")
    );
    let mut statement = conn.prepare(&sql)?;
    let entries = statement
        .query_map([playlist_id], |row| {
            let playlist_track = PlaylistTrackRow {
                _playlist_id: row.get(0)?,
                track_id: row.get(1)?,
                position: row.get(2)?,
            };
            Ok(PlaylistTrack {
                position: usize::try_from(playlist_track.position).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })?,
                track: tracks::track_from_row_at(row, 3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(entries)
}

pub fn delete_entries_for_track(
    transaction: &LibraryTransaction<'_>,
    track_id: TrackId,
) -> Result<(), LibraryError> {
    let transaction = &transaction.inner;
    transaction.execute(
        "DELETE FROM playlist_tracks WHERE track_id = ?1",
        [track_id],
    )?;
    Ok(())
}

pub fn delete_entries_for_root(
    transaction: &LibraryTransaction<'_>,
    storage_root_id: StorageRootId,
) -> Result<(), LibraryError> {
    let transaction = &transaction.inner;
    transaction.execute(
        "DELETE FROM playlist_tracks
         WHERE track_id IN (SELECT id FROM tracks WHERE storage_root_id = ?1)",
        [storage_root_id],
    )?;
    Ok(())
}

fn require_playlist(
    transaction: &Transaction<'_>,
    playlist_id: PlaylistId,
) -> Result<(), LibraryError> {
    let exists = transaction
        .query_row(
            "SELECT 1 FROM playlists WHERE id = ?1",
            [playlist_id],
            |_| Ok(()),
        )
        .optional()?;
    if exists.is_none() {
        return Err(LibraryError::PlaylistNotFound(playlist_id));
    }
    Ok(())
}

fn playlist_entry_ids(
    transaction: &Transaction<'_>,
    playlist_id: PlaylistId,
) -> Result<Vec<(usize, TrackId)>, LibraryError> {
    let columns = select_list(PLAYLIST_TRACK_COLUMNS);
    let sql = format!(
        "SELECT {columns}
         FROM playlist_tracks
         WHERE playlist_id = ?1
         ORDER BY position"
    );
    let mut statement = transaction.prepare(&sql)?;
    let rows = statement.query_map([playlist_id], |row| {
        let playlist_track = PlaylistTrackRow {
            _playlist_id: row.get(0)?,
            track_id: row.get(1)?,
            position: row.get(2)?,
        };
        let position = usize::try_from(playlist_track.position).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?;
        Ok((position, playlist_track.track_id))
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn replace_playlist_entries(
    transaction: &Transaction<'_>,
    playlist_id: PlaylistId,
    entries: &[(usize, TrackId)],
) -> Result<(), LibraryError> {
    transaction.execute(
        "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
        [playlist_id],
    )?;
    for (position, (_, track_id)) in entries.iter().enumerate() {
        transaction.execute(
            "INSERT INTO playlist_tracks (playlist_id, track_id, position)
             VALUES (?1, ?2, ?3)",
            params![
                playlist_id,
                track_id,
                usize_to_i64(position, "playlist position")?
            ],
        )?;
    }
    Ok(())
}

fn touch_playlist(
    transaction: &Transaction<'_>,
    playlist_id: PlaylistId,
) -> Result<(), LibraryError> {
    let now_ms = system_time_ms(SystemTime::now())?;
    transaction.execute(
        "UPDATE playlists SET updated_at_ms = ?2 WHERE id = ?1",
        params![playlist_id, now_ms],
    )?;
    Ok(())
}

fn playlist_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Playlist> {
    Ok(PlaylistRow {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at_ms: row.get(2)?,
        updated_at_ms: row.get(3)?,
    }
    .into())
}

pub fn playlist_summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlaylistSummary> {
    let row = PlaylistSummaryRow {
        playlist: playlist_from_row(row)?,
        track_count: row.get(4)?,
        total_duration_ms: row.get(5)?,
        cover_art_path: row.get(6)?,
    };
    Ok(PlaylistSummary {
        playlist: row.playlist,
        track_count: row.track_count as u64,
        total_duration_ms: row.total_duration_ms as u64,
        cover_art_path: row.cover_art_path.map(PathBuf::from),
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::backend::{
        LibraryError, LibraryStore,
        repo::{
            testing::{insert_track, test_file, test_metadata},
            tracks::{delete_track, set_track_cover},
        },
    };

    #[test]
    fn playlist_crud_allows_duplicate_names() {
        let mut store = LibraryStore::open_in_memory().unwrap();

        let first = crate::backend::repo::playlists::create(&mut store, "Night Drive").unwrap();
        let second = crate::backend::repo::playlists::create(&mut store, "Night Drive").unwrap();
        let renamed =
            crate::backend::repo::playlists::rename(&mut store, first.id, "Night Drive").unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(renamed.name, "Night Drive");
        assert_eq!(
            crate::backend::repo::playlists::list(&store).unwrap().len(),
            2
        );

        crate::backend::repo::playlists::delete_transactional(&mut store, first.id).unwrap();
        assert!(
            crate::backend::repo::playlists::get(&store, first.id)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            crate::backend::repo::playlists::list(&store).unwrap().len(),
            1
        );
        assert!(matches!(
            crate::backend::repo::playlists::delete_transactional(&mut store, first.id),
            Err(LibraryError::PlaylistNotFound(id)) if id == first.id
        ));
    }

    #[test]
    fn playlist_entries_allow_duplicates_and_remove_with_contiguous_positions() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
        let first = insert_track(
            &mut store,
            &root,
            &test_file(&root, "first.wav", 1, 10),
            &test_metadata("First", "Artist", Some("Album"), None),
        );
        let second = insert_track(
            &mut store,
            &root,
            &test_file(&root, "second.wav", 2, 10),
            &test_metadata("Second", "Artist", Some("Album"), None),
        );
        let playlist = crate::backend::repo::playlists::create(&mut store, "Duplicates").unwrap();

        crate::backend::repo::playlists::append_tracks_transactional(
            &mut store,
            playlist.id,
            &[first, first, second],
        )
        .unwrap();
        let entries = crate::backend::repo::playlists::tracks(&store, playlist.id).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.position, entry.track.id))
                .collect::<Vec<_>>(),
            [(0, first), (1, first), (2, second)]
        );

        crate::backend::repo::playlists::remove_entry_transactional(&mut store, playlist.id, 1)
            .unwrap();
        let entries = crate::backend::repo::playlists::tracks(&store, playlist.id).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.position, entry.track.id))
                .collect::<Vec<_>>(),
            [(0, first), (1, second)]
        );
    }

    #[test]
    fn playlist_moves_first_last_and_middle_with_contiguous_positions() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
        let mut ids = Vec::new();
        for (index, title) in ["A", "B", "C", "D"].into_iter().enumerate() {
            ids.push(insert_track(
                &mut store,
                &root,
                &test_file(&root, &format!("{title}.wav"), index as i64, 10),
                &test_metadata(title, "Artist", Some("Album"), None),
            ));
        }
        let playlist = crate::backend::repo::playlists::create(&mut store, "Order").unwrap();
        crate::backend::repo::playlists::append_tracks_transactional(&mut store, playlist.id, &ids)
            .unwrap();
        let titles = |store: &LibraryStore| {
            crate::backend::repo::playlists::tracks(store, playlist.id)
                .unwrap()
                .into_iter()
                .map(|entry| (entry.position, entry.track.title.unwrap()))
                .collect::<Vec<_>>()
        };

        crate::backend::repo::playlists::move_entry_transactional(&mut store, playlist.id, 0, 3)
            .unwrap();
        assert_eq!(
            titles(&store),
            [
                (0, "B".into()),
                (1, "C".into()),
                (2, "D".into()),
                (3, "A".into())
            ]
        );

        crate::backend::repo::playlists::move_entry_transactional(&mut store, playlist.id, 3, 0)
            .unwrap();
        assert_eq!(
            titles(&store),
            [
                (0, "A".into()),
                (1, "B".into()),
                (2, "C".into()),
                (3, "D".into())
            ]
        );

        crate::backend::repo::playlists::move_entry_transactional(&mut store, playlist.id, 1, 2)
            .unwrap();
        assert_eq!(
            titles(&store),
            [
                (0, "A".into()),
                (1, "C".into()),
                (2, "B".into()),
                (3, "D".into())
            ]
        );
    }

    #[test]
    fn playlist_entries_are_deleted_with_their_tracks_and_playlists() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        // With FK enforcement off, nothing in the schema can mask a missing
        // application-owned child delete (standing no-cascade rule).
        store
            .connection
            .execute_batch("PRAGMA foreign_keys = OFF")
            .unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
        let track = insert_track(
            &mut store,
            &root,
            &test_file(&root, "track.wav", 1, 10),
            &test_metadata("Track", "Artist", Some("Album"), None),
        );
        let first = crate::backend::repo::playlists::create(&mut store, "First").unwrap();
        let second = crate::backend::repo::playlists::create(&mut store, "Second").unwrap();
        crate::backend::repo::playlists::append_tracks_transactional(
            &mut store,
            first.id,
            &[track],
        )
        .unwrap();
        crate::backend::repo::playlists::append_tracks_transactional(
            &mut store,
            second.id,
            &[track],
        )
        .unwrap();

        let transaction = store.transaction().unwrap();
        delete_track(&transaction, track).unwrap();
        transaction.commit().unwrap();
        assert!(
            crate::backend::repo::playlists::tracks(&store, first.id)
                .unwrap()
                .is_empty()
        );
        assert!(
            crate::backend::repo::playlists::tracks(&store, second.id)
                .unwrap()
                .is_empty()
        );

        let replacement = insert_track(
            &mut store,
            &root,
            &test_file(&root, "replacement.wav", 2, 10),
            &test_metadata("Replacement", "Artist", Some("Album"), None),
        );
        crate::backend::repo::playlists::append_tracks_transactional(
            &mut store,
            first.id,
            &[replacement],
        )
        .unwrap();
        crate::backend::repo::playlists::delete_transactional(&mut store, first.id).unwrap();
        let remaining: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM playlist_tracks WHERE playlist_id = ?1",
                [first.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn playlist_listing_aggregates_duration_count_and_first_available_cover() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
        let mut ids = Vec::new();
        for (index, duration) in [1_000, 2_000, 3_000].into_iter().enumerate() {
            let mut metadata =
                test_metadata(&format!("Track {index}"), "Artist", Some("Album"), None);
            metadata.duration_ms = Some(duration);
            ids.push(insert_track(
                &mut store,
                &root,
                &test_file(&root, &format!("{index}.wav"), index as i64, 10),
                &metadata,
            ));
        }
        let first_cover = temp.path().join("first.cover");
        let later_cover = temp.path().join("later.cover");
        let transaction = store.transaction().unwrap();
        set_track_cover(
            &transaction,
            ids[1],
            first_cover.to_str().unwrap(),
            Some("image/jpeg"),
        )
        .unwrap();
        set_track_cover(
            &transaction,
            ids[2],
            later_cover.to_str().unwrap(),
            Some("image/jpeg"),
        )
        .unwrap();
        transaction.commit().unwrap();
        let playlist = crate::backend::repo::playlists::create(&mut store, "Aggregate").unwrap();
        crate::backend::repo::playlists::append_tracks_transactional(&mut store, playlist.id, &ids)
            .unwrap();
        let empty = crate::backend::repo::playlists::create(&mut store, "Empty").unwrap();

        let playlists = crate::backend::repo::playlists::list(&store).unwrap();
        let aggregate = playlists
            .iter()
            .find(|summary| summary.playlist.id == playlist.id)
            .unwrap();
        assert_eq!(aggregate.track_count, 3);
        assert_eq!(aggregate.total_duration_ms, 6_000);
        assert_eq!(aggregate.cover_art_path.as_ref(), Some(&first_cover));
        let empty = playlists
            .iter()
            .find(|summary| summary.playlist.id == empty.id)
            .unwrap();
        assert_eq!(empty.track_count, 0);
        assert_eq!(empty.total_duration_ms, 0);
        assert!(empty.cover_art_path.is_none());
    }
}
