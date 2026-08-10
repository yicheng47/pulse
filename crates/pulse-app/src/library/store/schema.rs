use rusqlite::{Connection, OptionalExtension, params};

use super::super::{LibraryError, metadata};

pub const SCHEMA_VERSION: i64 = 4;

/// Fresh-database schema. Foreign keys are plain `REFERENCES` — related-data
/// deletion is application-owned (`remove_storage_root`, `delete_track`, and
/// `delete_playlist` delete children explicitly, in dependency order); the
/// historical `ON DELETE CASCADE` safety nets left in the v3→v4 rebuild.
const SCHEMA: &str = r#"
BEGIN IMMEDIATE;

CREATE TABLE storage_roots (
    id                  INTEGER PRIMARY KEY,
    path                TEXT NOT NULL,
    path_key            TEXT NOT NULL UNIQUE,
    display_name        TEXT NOT NULL,
    added_at_ms         INTEGER NOT NULL,
    last_scan_at_ms     INTEGER,
    is_reachable        INTEGER NOT NULL CHECK (is_reachable IN (0, 1)),
    is_case_sensitive   INTEGER NOT NULL CHECK (is_case_sensitive IN (0, 1))
);

CREATE TABLE tracks (
    id                      INTEGER PRIMARY KEY,
    storage_root_id         INTEGER NOT NULL REFERENCES storage_roots(id),
    path                    TEXT NOT NULL,
    path_key                TEXT NOT NULL,
    title                   TEXT,
    artist                  TEXT,
    album                   TEXT,
    album_artist            TEXT,
    year                    INTEGER,
    genre                   TEXT,
    track_number            INTEGER,
    disc_number             INTEGER,
    duration_ms             INTEGER,
    sample_rate_hz          INTEGER,
    bit_depth               INTEGER,
    channels                INTEGER,
    file_size_bytes         INTEGER NOT NULL,
    modified_at_ns          INTEGER NOT NULL,
    cover_art_path          TEXT,
    cover_art_mime_type     TEXT,
    added_at_ms             INTEGER NOT NULL,
    updated_at_ms           INTEGER NOT NULL,
    UNIQUE (storage_root_id, path_key)
);

CREATE INDEX tracks_storage_root_id_idx ON tracks(storage_root_id);
CREATE INDEX tracks_album_idx ON tracks(album, album_artist);

CREATE TABLE scan_history (
    id                  INTEGER PRIMARY KEY,
    storage_root_id     INTEGER NOT NULL REFERENCES storage_roots(id),
    scan_session_id     TEXT NOT NULL,
    started_at_ms       INTEGER NOT NULL,
    finished_at_ms      INTEGER,
    added_count         INTEGER NOT NULL DEFAULT 0,
    updated_count       INTEGER NOT NULL DEFAULT 0,
    removed_count       INTEGER NOT NULL DEFAULT 0,
    unsupported_count   INTEGER NOT NULL DEFAULT 0,
    error_count         INTEGER NOT NULL DEFAULT 0,
    removals_suppressed INTEGER NOT NULL DEFAULT 0 CHECK (removals_suppressed IN (0, 1)),
    outcome             TEXT CHECK (
        outcome IS NULL OR outcome IN ('completed', 'completed_with_errors', 'offline', 'failed')
    ),
    error_message       TEXT
);

CREATE INDEX scan_history_root_started_idx
    ON scan_history(storage_root_id, started_at_ms DESC);

CREATE TABLE playlists (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL,
    created_at_ms   INTEGER NOT NULL,
    updated_at_ms   INTEGER NOT NULL
);

CREATE TABLE playlist_tracks (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id),
    track_id    INTEGER NOT NULL REFERENCES tracks(id),
    position    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, position)
);

CREATE INDEX playlist_tracks_track_id_idx ON playlist_tracks(track_id);

-- FTS5 is deliberately deferred. MVP search uses capped LIKE queries whose
-- result types can be preserved when an FTS table is added later.

PRAGMA user_version = 4;
COMMIT;
"#;

/// Progress of the v1→v2 year/genre backfill, which re-reads every track file
/// and can block for a while on a large network library.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackfillProgress {
    pub processed: usize,
    pub total: usize,
}

pub fn migrate_to_current(
    connection: &mut Connection,
    on_backfill_progress: impl FnMut(BackfillProgress),
) -> Result<(), LibraryError> {
    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    match version {
        0 => connection.execute_batch(SCHEMA)?,
        1 => {
            migrate_v1_to_v2(connection, on_backfill_progress)?;
            migrate_v2_to_v3(connection)?;
            migrate_v3_to_v4(connection)?;
        }
        2 => {
            migrate_v2_to_v3(connection)?;
            migrate_v3_to_v4(connection)?;
        }
        3 => migrate_v3_to_v4(connection)?,
        SCHEMA_VERSION => {}
        version => return Err(LibraryError::UnsupportedSchemaVersion(version)),
    }
    Ok(())
}

fn migrate_v1_to_v2(
    connection: &mut Connection,
    mut on_progress: impl FnMut(BackfillProgress),
) -> Result<(), LibraryError> {
    let existing_tracks = {
        let mut statement = connection.prepare("SELECT id, path FROM tracks ORDER BY id")?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    std::path::PathBuf::from(row.get::<_, String>(1)?),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    let total = existing_tracks.len();
    let backfill = existing_tracks
        .iter()
        .enumerate()
        .filter_map(|(index, (id, path))| {
            on_progress(BackfillProgress {
                processed: index,
                total,
            });
            metadata::extract_metadata(path)
                .ok()
                .map(|metadata| (*id, metadata.year, metadata.genre))
        })
        .collect::<Vec<_>>();
    on_progress(BackfillProgress {
        processed: total,
        total,
    });
    let backfilled_ids = backfill
        .iter()
        .map(|(id, _, _)| *id)
        .collect::<std::collections::HashSet<_>>();

    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "ALTER TABLE tracks ADD COLUMN year INTEGER;
         ALTER TABLE tracks ADD COLUMN genre TEXT;",
    )?;
    for (id, year, genre) in backfill {
        transaction.execute(
            "UPDATE tracks SET year = ?2, genre = ?3 WHERE id = ?1",
            params![id, year, genre],
        )?;
    }
    for (id, _) in existing_tracks {
        if !backfilled_ids.contains(&id) {
            transaction.execute("UPDATE tracks SET modified_at_ns = -1 WHERE id = ?1", [id])?;
        }
    }
    transaction.execute_batch("PRAGMA user_version = 2;")?;
    transaction.commit()?;
    Ok(())
}

fn migrate_v2_to_v3(connection: &mut Connection) -> Result<(), LibraryError> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS playlists (
             id              INTEGER PRIMARY KEY,
             name            TEXT NOT NULL,
             created_at_ms   INTEGER NOT NULL,
             updated_at_ms   INTEGER NOT NULL
         );

         CREATE TABLE IF NOT EXISTS playlist_tracks (
             playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
             track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
             position    INTEGER NOT NULL,
             PRIMARY KEY (playlist_id, position)
         );

         CREATE INDEX IF NOT EXISTS playlist_tracks_track_id_idx
             ON playlist_tracks(track_id);

         PRAGMA user_version = 3;",
    )?;
    transaction.commit()?;
    Ok(())
}

/// Converts the dormant `ON DELETE CASCADE` clauses to plain `REFERENCES`.
/// SQLite cannot edit a foreign-key clause in place, so this is the
/// documented table-rebuild procedure: create the new shape under a temporary
/// name, copy rows (preserving ids — cover-cache paths and playlist entries
/// reference track ids), drop the old table, rename, recreate indexes.
fn migrate_v3_to_v4(connection: &mut Connection) -> Result<(), LibraryError> {
    // `PRAGMA foreign_keys` is a no-op while a transaction is open, so
    // enforcement is toggled off out here and restored on both exit paths (a
    // failed rebuild rolls back when the transaction drops).
    connection.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let rebuilt = rebuild_without_cascades(connection);
    let restored = connection.execute_batch("PRAGMA foreign_keys = ON;");
    rebuilt?;
    restored?;
    Ok(())
}

fn rebuild_without_cascades(connection: &mut Connection) -> Result<(), LibraryError> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        r#"
        CREATE TABLE tracks_v4 (
            id                      INTEGER PRIMARY KEY,
            storage_root_id         INTEGER NOT NULL REFERENCES storage_roots(id),
            path                    TEXT NOT NULL,
            path_key                TEXT NOT NULL,
            title                   TEXT,
            artist                  TEXT,
            album                   TEXT,
            album_artist            TEXT,
            year                    INTEGER,
            genre                   TEXT,
            track_number            INTEGER,
            disc_number             INTEGER,
            duration_ms             INTEGER,
            sample_rate_hz          INTEGER,
            bit_depth               INTEGER,
            channels                INTEGER,
            file_size_bytes         INTEGER NOT NULL,
            modified_at_ns          INTEGER NOT NULL,
            cover_art_path          TEXT,
            cover_art_mime_type     TEXT,
            added_at_ms             INTEGER NOT NULL,
            updated_at_ms           INTEGER NOT NULL,
            UNIQUE (storage_root_id, path_key)
        );
        INSERT INTO tracks_v4 (
            id, storage_root_id, path, path_key, title, artist, album, album_artist,
            year, genre, track_number, disc_number, duration_ms, sample_rate_hz,
            bit_depth, channels, file_size_bytes, modified_at_ns, cover_art_path,
            cover_art_mime_type, added_at_ms, updated_at_ms)
        SELECT
            id, storage_root_id, path, path_key, title, artist, album, album_artist,
            year, genre, track_number, disc_number, duration_ms, sample_rate_hz,
            bit_depth, channels, file_size_bytes, modified_at_ns, cover_art_path,
            cover_art_mime_type, added_at_ms, updated_at_ms
        FROM tracks;
        DROP TABLE tracks;
        ALTER TABLE tracks_v4 RENAME TO tracks;
        CREATE INDEX tracks_storage_root_id_idx ON tracks(storage_root_id);
        CREATE INDEX tracks_album_idx ON tracks(album, album_artist);

        CREATE TABLE scan_history_v4 (
            id                  INTEGER PRIMARY KEY,
            storage_root_id     INTEGER NOT NULL REFERENCES storage_roots(id),
            scan_session_id     TEXT NOT NULL,
            started_at_ms       INTEGER NOT NULL,
            finished_at_ms      INTEGER,
            added_count         INTEGER NOT NULL DEFAULT 0,
            updated_count       INTEGER NOT NULL DEFAULT 0,
            removed_count       INTEGER NOT NULL DEFAULT 0,
            unsupported_count   INTEGER NOT NULL DEFAULT 0,
            error_count         INTEGER NOT NULL DEFAULT 0,
            removals_suppressed INTEGER NOT NULL DEFAULT 0 CHECK (removals_suppressed IN (0, 1)),
            outcome             TEXT CHECK (
                outcome IS NULL
                OR outcome IN ('completed', 'completed_with_errors', 'offline', 'failed')
            ),
            error_message       TEXT
        );
        INSERT INTO scan_history_v4 (
            id, storage_root_id, scan_session_id, started_at_ms, finished_at_ms,
            added_count, updated_count, removed_count, unsupported_count, error_count,
            removals_suppressed, outcome, error_message)
        SELECT
            id, storage_root_id, scan_session_id, started_at_ms, finished_at_ms,
            added_count, updated_count, removed_count, unsupported_count, error_count,
            removals_suppressed, outcome, error_message
        FROM scan_history;
        DROP TABLE scan_history;
        ALTER TABLE scan_history_v4 RENAME TO scan_history;
        CREATE INDEX scan_history_root_started_idx
            ON scan_history(storage_root_id, started_at_ms DESC);

        CREATE TABLE playlist_tracks_v4 (
            playlist_id INTEGER NOT NULL REFERENCES playlists(id),
            track_id    INTEGER NOT NULL REFERENCES tracks(id),
            position    INTEGER NOT NULL,
            PRIMARY KEY (playlist_id, position)
        );
        INSERT INTO playlist_tracks_v4 (playlist_id, track_id, position)
        SELECT playlist_id, track_id, position FROM playlist_tracks;
        DROP TABLE playlist_tracks;
        ALTER TABLE playlist_tracks_v4 RENAME TO playlist_tracks;
        CREATE INDEX playlist_tracks_track_id_idx ON playlist_tracks(track_id);
        "#,
    )?;
    // The rebuild must never commit rows the restored enforcement would
    // reject; fail closed and leave the database on v3 rather than drop rows.
    let violation: Option<String> = transaction
        .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
        .optional()?;
    if let Some(table) = violation {
        return Err(LibraryError::MigrationIntegrity(table));
    }
    transaction.execute_batch("PRAGMA user_version = 4;")?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::{BackfillProgress, SCHEMA_VERSION};
    use crate::library::{LibraryError, LibraryStore, ScanOutcome, metadata};

    /// The schema exactly as v3 shipped it — the shape Jason's live library
    /// database has on disk — frozen here so migration tests exercise the
    /// real starting point, cascades included.
    const V3_SCHEMA: &str = r#"
BEGIN IMMEDIATE;

CREATE TABLE storage_roots (
    id                  INTEGER PRIMARY KEY,
    path                TEXT NOT NULL,
    path_key            TEXT NOT NULL UNIQUE,
    display_name        TEXT NOT NULL,
    added_at_ms         INTEGER NOT NULL,
    last_scan_at_ms     INTEGER,
    is_reachable        INTEGER NOT NULL CHECK (is_reachable IN (0, 1)),
    is_case_sensitive   INTEGER NOT NULL CHECK (is_case_sensitive IN (0, 1))
);

CREATE TABLE tracks (
    id                      INTEGER PRIMARY KEY,
    storage_root_id         INTEGER NOT NULL REFERENCES storage_roots(id) ON DELETE CASCADE,
    path                    TEXT NOT NULL,
    path_key                TEXT NOT NULL,
    title                   TEXT,
    artist                  TEXT,
    album                   TEXT,
    album_artist            TEXT,
    year                    INTEGER,
    genre                   TEXT,
    track_number            INTEGER,
    disc_number             INTEGER,
    duration_ms             INTEGER,
    sample_rate_hz          INTEGER,
    bit_depth               INTEGER,
    channels                INTEGER,
    file_size_bytes         INTEGER NOT NULL,
    modified_at_ns          INTEGER NOT NULL,
    cover_art_path          TEXT,
    cover_art_mime_type     TEXT,
    added_at_ms             INTEGER NOT NULL,
    updated_at_ms           INTEGER NOT NULL,
    UNIQUE (storage_root_id, path_key)
);

CREATE INDEX tracks_storage_root_id_idx ON tracks(storage_root_id);
CREATE INDEX tracks_album_idx ON tracks(album, album_artist);

CREATE TABLE scan_history (
    id                  INTEGER PRIMARY KEY,
    storage_root_id     INTEGER NOT NULL REFERENCES storage_roots(id) ON DELETE CASCADE,
    scan_session_id     TEXT NOT NULL,
    started_at_ms       INTEGER NOT NULL,
    finished_at_ms      INTEGER,
    added_count         INTEGER NOT NULL DEFAULT 0,
    updated_count       INTEGER NOT NULL DEFAULT 0,
    removed_count       INTEGER NOT NULL DEFAULT 0,
    unsupported_count   INTEGER NOT NULL DEFAULT 0,
    error_count         INTEGER NOT NULL DEFAULT 0,
    removals_suppressed INTEGER NOT NULL DEFAULT 0 CHECK (removals_suppressed IN (0, 1)),
    outcome             TEXT CHECK (
        outcome IS NULL OR outcome IN ('completed', 'completed_with_errors', 'offline', 'failed')
    ),
    error_message       TEXT
);

CREATE INDEX scan_history_root_started_idx
    ON scan_history(storage_root_id, started_at_ms DESC);

CREATE TABLE playlists (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL,
    created_at_ms   INTEGER NOT NULL,
    updated_at_ms   INTEGER NOT NULL
);

CREATE TABLE playlist_tracks (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, position)
);

CREATE INDEX playlist_tracks_track_id_idx ON playlist_tracks(track_id);

PRAGMA user_version = 3;
COMMIT;
"#;

    const PLAYLIST_SCHEMA: &str = r#"CREATE TABLE playlists (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL,
    created_at_ms   INTEGER NOT NULL,
    updated_at_ms   INTEGER NOT NULL
);

CREATE TABLE playlist_tracks (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, position)
);

CREATE INDEX playlist_tracks_track_id_idx ON playlist_tracks(track_id);

"#;

    fn pre_v3_schema(version: i64) -> String {
        let schema = V3_SCHEMA.replace(PLAYLIST_SCHEMA, "");
        assert_ne!(schema, V3_SCHEMA);
        schema.replace(
            "PRAGMA user_version = 3;",
            &format!("PRAGMA user_version = {version};"),
        )
    }

    fn user_version(connection: &Connection) -> i64 {
        connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    fn assert_no_cascades(connection: &Connection) {
        for table in ["tracks", "scan_history", "playlist_tracks"] {
            let sql: String = connection
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(sql.contains("REFERENCES"), "{table} keeps its FK clauses");
            assert!(
                !sql.to_uppercase().contains("CASCADE"),
                "{table} still declares a cascade: {sql}"
            );
        }
    }

    #[test]
    fn fresh_databases_initialize_at_v4_with_plain_references() {
        let store = LibraryStore::open_in_memory().unwrap();
        assert_eq!(user_version(&store.connection), SCHEMA_VERSION);
        assert_no_cascades(&store.connection);
    }

    #[test]
    fn rejects_databases_newer_than_the_supported_schema() {
        let temp = tempdir().unwrap();
        let database_path = temp.path().join("library.sqlite");
        let connection = Connection::open(&database_path).unwrap();
        connection
            .execute_batch("PRAGMA user_version = 99;")
            .unwrap();
        drop(connection);

        assert!(matches!(
            LibraryStore::open(&database_path),
            Err(LibraryError::UnsupportedSchemaVersion(99))
        ));
    }

    #[test]
    fn migrates_v1_and_backfills_year_and_genre_from_existing_files() {
        let temp = tempdir().unwrap();
        let track_path = temp.path().join("track.wav");
        metadata::write_test_wav(&track_path, "Track", "Artist", "Album").unwrap();
        let database_path = temp.path().join("library.sqlite");
        let connection = Connection::open(&database_path).unwrap();
        let v1_schema = pre_v3_schema(1)
            .replace("    year                    INTEGER,\n", "")
            .replace("    genre                   TEXT,\n", "");
        connection.execute_batch(&v1_schema).unwrap();
        connection
            .execute(
                "INSERT INTO storage_roots (
                     id, path, path_key, display_name, added_at_ms, is_reachable,
                     is_case_sensitive
                 ) VALUES (1, ?1, ?1, 'Music', 1, 1, 1)",
                [temp.path().to_str().unwrap()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO tracks (
                     storage_root_id, path, path_key, title, artist, album,
                     file_size_bytes, modified_at_ns, added_at_ms, updated_at_ms
                 ) VALUES (1, ?1, ?1, 'Track', 'Artist', 'Album', 1, 1, 1, 1)",
                [track_path.to_str().unwrap()],
            )
            .unwrap();
        let missing_path = temp.path().join("missing.wav");
        connection
            .execute(
                "INSERT INTO tracks (
                     storage_root_id, path, path_key, title, artist, album,
                     file_size_bytes, modified_at_ns, added_at_ms, updated_at_ms
                 ) VALUES (1, ?1, ?1, 'Missing', 'Artist', 'Album', 1, 1, 1, 1)",
                [missing_path.to_str().unwrap()],
            )
            .unwrap();
        drop(connection);

        let mut progress = Vec::new();
        let store = LibraryStore::open_with_progress(&database_path, |backfill| {
            progress.push(backfill);
        })
        .unwrap();
        assert_eq!(
            progress.first(),
            Some(&BackfillProgress {
                processed: 0,
                total: 2
            })
        );
        assert_eq!(
            progress.last(),
            Some(&BackfillProgress {
                processed: 2,
                total: 2
            })
        );
        let tracks = store.tracks_for_root(1).unwrap();
        let track = tracks
            .iter()
            .find(|track| track.path == track_path)
            .unwrap();
        let missing = tracks
            .iter()
            .find(|track| track.path == missing_path)
            .unwrap();

        assert_eq!(track.year, Some(2024));
        assert_eq!(track.genre.as_deref(), Some("Electronic"));
        assert_eq!(missing.year, None);
        assert_eq!(missing.genre, None);
        assert_eq!(missing.modified_at_ns, -1);
        assert_eq!(user_version(&store.connection), 4);
        assert_no_cascades(&store.connection);
    }

    #[test]
    fn migrates_v2_to_v4_and_creates_playlist_tables() {
        let temp = tempdir().unwrap();
        let database_path = temp.path().join("library.sqlite");
        let connection = Connection::open(&database_path).unwrap();
        connection.execute_batch(&pre_v3_schema(2)).unwrap();
        drop(connection);

        let mut store = LibraryStore::open(&database_path).unwrap();
        let playlist = store.create_playlist("Migrated").unwrap();

        assert_eq!(store.playlist(playlist.id).unwrap(), Some(playlist));
        assert_eq!(user_version(&store.connection), 4);
        assert_no_cascades(&store.connection);
    }

    #[test]
    fn migrates_v3_to_v4_rebuilding_tables_and_preserving_row_ids() {
        let temp = tempdir().unwrap();
        let database_path = temp.path().join("library.sqlite");
        let connection = Connection::open(&database_path).unwrap();
        connection.execute_batch(V3_SCHEMA).unwrap();
        connection
            .execute_batch(
                "INSERT INTO storage_roots
                     (id, path, path_key, display_name, added_at_ms, is_reachable,
                      is_case_sensitive)
                 VALUES (1, '/music', '/music', 'Music', 1, 1, 1);
                 INSERT INTO tracks
                     (id, storage_root_id, path, path_key, title, file_size_bytes,
                      modified_at_ns, cover_art_path, added_at_ms, updated_at_ms)
                 VALUES (10, 1, '/music/a.flac', '/music/a.flac', 'A', 1, 1, NULL, 1, 1),
                        (42, 1, '/music/b.flac', '/music/b.flac', 'B', 1, 1,
                         '/covers/42-abc.cover', 1, 1);
                 INSERT INTO scan_history
                     (id, storage_root_id, scan_session_id, started_at_ms,
                      finished_at_ms, outcome)
                 VALUES (7, 1, 'past-session', 100, 200, 'completed');
                 INSERT INTO playlists (id, name, created_at_ms, updated_at_ms)
                 VALUES (3, 'Keeper', 1, 1);
                 INSERT INTO playlist_tracks (playlist_id, track_id, position)
                 VALUES (3, 42, 0), (3, 10, 1);",
            )
            .unwrap();
        drop(connection);

        let mut store = LibraryStore::open(&database_path).unwrap();

        assert_eq!(user_version(&store.connection), 4);
        let foreign_keys: i64 = store
            .connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1, "enforcement is restored after the rebuild");
        assert_no_cascades(&store.connection);
        let index_count: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name IN
                     ('tracks_storage_root_id_idx', 'tracks_album_idx',
                      'scan_history_root_started_idx', 'playlist_tracks_track_id_idx')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 4, "indexes are recreated after the rebuild");

        let tracks = store.tracks_for_root(1).unwrap();
        assert_eq!(
            tracks.iter().map(|track| track.id).collect::<Vec<_>>(),
            [10, 42],
            "row ids survive the rebuild (cover paths and playlists reference them)"
        );
        assert_eq!(
            tracks[1].cover_art_path.as_deref(),
            Some(std::path::Path::new("/covers/42-abc.cover"))
        );
        assert_eq!(
            store
                .playlist_tracks(3)
                .unwrap()
                .iter()
                .map(|entry| (entry.position, entry.track.id))
                .collect::<Vec<_>>(),
            [(0, 42), (1, 10)]
        );
        let history = store.recent_scans(1, 5).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, 7);
        assert_eq!(history[0].outcome, Some(ScanOutcome::Completed));

        // The cascades are behaviorally gone: with enforcement on, a bare
        // parent delete is rejected instead of silently deleting children.
        assert!(
            store
                .connection
                .execute("DELETE FROM playlists WHERE id = 3", [])
                .is_err()
        );
        assert!(
            store
                .connection
                .execute("DELETE FROM storage_roots WHERE id = 1", [])
                .is_err()
        );
        assert_eq!(store.playlist_tracks(3).unwrap().len(), 2);

        // Application-owned deletion still clears children first.
        store.delete_playlist(3).unwrap();
        assert_eq!(
            store.remove_storage_root(1).unwrap(),
            vec![std::path::PathBuf::from("/covers/42-abc.cover")]
        );
    }

    #[test]
    fn v3_to_v4_fails_closed_when_rows_violate_foreign_keys() {
        let temp = tempdir().unwrap();
        let database_path = temp.path().join("library.sqlite");
        let connection = Connection::open(&database_path).unwrap();
        connection.execute_batch(V3_SCHEMA).unwrap();
        // An orphan row only FK-off writes could have produced: neither
        // playlist 9 nor track 9 exists.
        connection
            .execute_batch("PRAGMA foreign_keys = OFF;")
            .unwrap();
        connection
            .execute(
                "INSERT INTO playlist_tracks (playlist_id, track_id, position)
                 VALUES (9, 9, 0)",
                [],
            )
            .unwrap();
        drop(connection);

        let error = LibraryStore::open(&database_path).err().unwrap();
        assert!(
            matches!(error, LibraryError::MigrationIntegrity(_)),
            "unexpected error: {error}"
        );

        let connection = Connection::open(&database_path).unwrap();
        assert_eq!(
            user_version(&connection),
            3,
            "a failed rebuild rolls back to v3"
        );
        let rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM playlist_tracks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1, "no rows are dropped by a failed rebuild");
    }
}
