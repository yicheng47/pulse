use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::{
    LibraryError, LibrarySummary, ScanHistoryEntry, ScanOutcome, StorageRoot, StorageRootId, Track,
    TrackId, metadata::AudioMetadata, path::normalize_storage_root, system_time_ms,
    walk::DiscoveredFile,
};

const SCHEMA_VERSION: i64 = 1;

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
    storage_root_id         INTEGER NOT NULL REFERENCES storage_roots(id) ON DELETE CASCADE,
    path                    TEXT NOT NULL,
    path_key                TEXT NOT NULL,
    title                   TEXT,
    artist                  TEXT,
    album                   TEXT,
    album_artist            TEXT,
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

-- FTS5 is deliberately deferred from schema v1. Search will add an FTS table and backfill it
-- from tracks when that stage starts; the base catalog does not need an index it cannot query.

PRAGMA user_version = 1;
COMMIT;
"#;

pub struct LibraryStore {
    pub(super) connection: Connection,
    scan_session_id: String,
}

#[derive(Debug)]
pub(super) struct ExistingTrack {
    pub id: TrackId,
    pub path: PathBuf,
    pub path_text: String,
    pub modified_at_ns: i64,
    pub cover_art_path: Option<PathBuf>,
}

impl LibraryStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LibraryError> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|source| LibraryError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        Self::from_connection(
            Connection::open(path)?,
            process_scan_session_id().to_owned(),
        )
    }

    pub fn open_in_memory() -> Result<Self, LibraryError> {
        Self::from_connection(
            Connection::open_in_memory()?,
            process_scan_session_id().to_owned(),
        )
    }

    fn from_connection(
        connection: Connection,
        scan_session_id: String,
    ) -> Result<Self, LibraryError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;",
        )?;

        let version =
            connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
        match version {
            0 => connection.execute_batch(SCHEMA)?,
            SCHEMA_VERSION => {}
            version => return Err(LibraryError::UnsupportedSchemaVersion(version)),
        }

        let recovered_at_ms = system_time_ms(SystemTime::now())?;
        connection.execute(
            "UPDATE scan_history
             SET finished_at_ms = ?1,
                 error_count = CASE WHEN error_count = 0 THEN 1 ELSE error_count END,
                 outcome = 'failed',
                 error_message = 'Scan interrupted before completion'
             WHERE finished_at_ms IS NULL
               AND outcome IS NULL
               AND scan_session_id <> ?2",
            params![recovered_at_ms, scan_session_id],
        )?;

        Ok(Self {
            connection,
            scan_session_id,
        })
    }

    pub fn add_storage_root(
        &mut self,
        path: impl AsRef<Path>,
        display_name: impl AsRef<str>,
    ) -> Result<StorageRoot, LibraryError> {
        let normalized = normalize_storage_root(path.as_ref())?;
        let added_at_ms = system_time_ms(std::time::SystemTime::now())?;
        let id = self.connection.query_row(
            "INSERT INTO storage_roots (
                 path, path_key, display_name, added_at_ms, is_reachable, is_case_sensitive
             ) VALUES (?1, ?2, ?3, ?4, 1, ?5)
             RETURNING id",
            params![
                normalized.path_text,
                normalized.path_key,
                display_name.as_ref(),
                added_at_ms,
                normalized.is_case_sensitive,
            ],
            |row| row.get(0),
        )?;

        self.storage_root(id)?
            .ok_or(LibraryError::StorageRootNotFound(id))
    }

    pub fn storage_root(
        &self,
        storage_root_id: StorageRootId,
    ) -> Result<Option<StorageRoot>, LibraryError> {
        self.connection
            .query_row(
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

    pub fn storage_roots(&self) -> Result<Vec<StorageRoot>, LibraryError> {
        let mut statement = self.connection.prepare(
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

    pub fn remove_storage_root(
        &mut self,
        storage_root_id: StorageRootId,
    ) -> Result<Vec<PathBuf>, LibraryError> {
        let transaction = self.connection.transaction()?;
        let cover_art_paths = {
            let mut statement = transaction.prepare(
                "SELECT cover_art_path
                 FROM tracks
                 WHERE storage_root_id = ?1 AND cover_art_path IS NOT NULL",
            )?;
            statement
                .query_map([storage_root_id], |row| {
                    row.get::<_, String>(0).map(PathBuf::from)
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let deleted =
            transaction.execute("DELETE FROM storage_roots WHERE id = ?1", [storage_root_id])?;
        if deleted == 0 {
            return Err(LibraryError::StorageRootNotFound(storage_root_id));
        }
        transaction.commit()?;
        Ok(cover_art_paths)
    }

    pub fn tracks_for_root(
        &self,
        storage_root_id: StorageRootId,
    ) -> Result<Vec<Track>, LibraryError> {
        let mut statement = self.connection.prepare(
            "SELECT id, storage_root_id, path, title, artist, album, album_artist,
                    track_number, disc_number, duration_ms, sample_rate_hz, bit_depth,
                    channels, file_size_bytes, modified_at_ns, cover_art_path,
                    cover_art_mime_type, added_at_ms, updated_at_ms
             FROM tracks
             WHERE storage_root_id = ?1
             ORDER BY path_key",
        )?;
        let tracks = statement
            .query_map([storage_root_id], track_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tracks)
    }

    pub fn root_summary(
        &self,
        storage_root_id: StorageRootId,
    ) -> Result<LibrarySummary, LibraryError> {
        let summary = self.connection.query_row(
            "SELECT
                 (
                     SELECT COUNT(*)
                     FROM (
                         SELECT album, COALESCE(album_artist, artist, '') AS album_owner
                         FROM tracks
                         WHERE storage_root_id = ?1
                           AND album IS NOT NULL
                           AND trim(album) <> ''
                         GROUP BY album, album_owner
                     )
                 ),
                 COUNT(*),
                 COALESCE(SUM(file_size_bytes), 0)
             FROM tracks
             WHERE storage_root_id = ?1",
            [storage_root_id],
            summary_from_row,
        )?;
        Ok(summary)
    }

    pub fn catalog_summary(&self) -> Result<LibrarySummary, LibraryError> {
        let summary = self.connection.query_row(
            "SELECT
                 (
                     SELECT COUNT(*)
                     FROM (
                         SELECT album, COALESCE(album_artist, artist, '') AS album_owner
                         FROM tracks
                         WHERE album IS NOT NULL AND trim(album) <> ''
                         GROUP BY album, album_owner
                     )
                 ),
                 COUNT(*),
                 COALESCE(SUM(file_size_bytes), 0)
             FROM tracks",
            [],
            summary_from_row,
        )?;
        Ok(summary)
    }

    pub fn recent_scans(
        &self,
        storage_root_id: StorageRootId,
        limit: usize,
    ) -> Result<Vec<ScanHistoryEntry>, LibraryError> {
        let limit = i64::try_from(limit)
            .map_err(|_| LibraryError::IntegerOutOfRange("scan history limit"))?;
        let mut statement = self.connection.prepare(
            "SELECT id, storage_root_id, started_at_ms, finished_at_ms,
                    added_count, updated_count, removed_count, unsupported_count,
                    error_count, removals_suppressed, outcome, error_message
             FROM scan_history
             WHERE storage_root_id = ?1
             ORDER BY started_at_ms DESC, id DESC
             LIMIT ?2",
        )?;
        let scans = statement
            .query_map(params![storage_root_id, limit], scan_history_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(scans)
    }

    pub(super) fn begin_scan(
        &self,
        storage_root_id: StorageRootId,
        started_at_ms: i64,
    ) -> Result<i64, LibraryError> {
        self.connection
            .query_row(
                "INSERT INTO scan_history (storage_root_id, scan_session_id, started_at_ms)
                 VALUES (?1, ?2, ?3)
                 RETURNING id",
                params![storage_root_id, self.scan_session_id, started_at_ms],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub(super) fn mark_root_reachable(
        &self,
        storage_root_id: StorageRootId,
    ) -> Result<(), LibraryError> {
        self.connection.execute(
            "UPDATE storage_roots SET is_reachable = 1 WHERE id = ?1",
            [storage_root_id],
        )?;
        Ok(())
    }

    pub(super) fn finish_offline_scan(
        &mut self,
        scan_id: i64,
        storage_root_id: StorageRootId,
        finished_at_ms: i64,
        error_message: &str,
    ) -> Result<(), LibraryError> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "UPDATE storage_roots
             SET is_reachable = 0, last_scan_at_ms = ?2
             WHERE id = ?1",
            params![storage_root_id, finished_at_ms],
        )?;
        transaction.execute(
            "UPDATE scan_history
             SET finished_at_ms = ?2, error_count = 1, outcome = 'offline',
                 error_message = ?3
             WHERE id = ?1",
            params![scan_id, finished_at_ms, error_message],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(super) fn finish_failed_scan(
        &mut self,
        scan_id: i64,
        storage_root_id: StorageRootId,
        finished_at_ms: i64,
        error_message: &str,
    ) -> Result<(), LibraryError> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "UPDATE storage_roots
             SET last_scan_at_ms = ?2
             WHERE id = ?1",
            params![storage_root_id, finished_at_ms],
        )?;
        transaction.execute(
            "UPDATE scan_history
             SET finished_at_ms = ?2, error_count = 1, outcome = 'failed',
                 error_message = ?3
             WHERE id = ?1",
            params![scan_id, finished_at_ms, error_message],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(super) fn existing_tracks(
        &self,
        storage_root_id: StorageRootId,
    ) -> Result<HashMap<String, ExistingTrack>, LibraryError> {
        let mut statement = self.connection.prepare(
            "SELECT id, path, path_key, modified_at_ns, cover_art_path
             FROM tracks
             WHERE storage_root_id = ?1",
        )?;
        let rows = statement.query_map([storage_root_id], |row| {
            let path_text = row.get::<_, String>(1)?;
            Ok((
                row.get::<_, String>(2)?,
                ExistingTrack {
                    id: row.get(0)?,
                    path: PathBuf::from(&path_text),
                    path_text,
                    modified_at_ns: row.get(3)?,
                    cover_art_path: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
                },
            ))
        })?;
        Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
    }
}

pub(super) fn upsert_track(
    transaction: &Transaction<'_>,
    storage_root_id: StorageRootId,
    file: &DiscoveredFile,
    metadata: &AudioMetadata,
    now_ms: i64,
) -> Result<TrackId, LibraryError> {
    let file_size_bytes = i64::try_from(file.file_size_bytes)
        .map_err(|_| LibraryError::IntegerOutOfRange("file size"))?;
    let track_number = metadata.track_number.map(i64::from);
    let disc_number = metadata.disc_number.map(i64::from);
    let sample_rate_hz = metadata.sample_rate_hz.map(i64::from);
    let bit_depth = metadata.bit_depth.map(i64::from);
    let channels = metadata.channels.map(i64::from);

    let id = transaction.query_row(
        "INSERT INTO tracks (
             storage_root_id, path, path_key, title, artist, album, album_artist,
             track_number, disc_number, duration_ms, sample_rate_hz, bit_depth,
             channels, file_size_bytes, modified_at_ns, added_at_ms, updated_at_ms
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16
         )
         ON CONFLICT(storage_root_id, path_key) DO UPDATE SET
             path = excluded.path,
             title = excluded.title,
             artist = excluded.artist,
             album = excluded.album,
             album_artist = excluded.album_artist,
             track_number = excluded.track_number,
             disc_number = excluded.disc_number,
             duration_ms = excluded.duration_ms,
             sample_rate_hz = excluded.sample_rate_hz,
             bit_depth = excluded.bit_depth,
             channels = excluded.channels,
             file_size_bytes = excluded.file_size_bytes,
             modified_at_ns = excluded.modified_at_ns,
             updated_at_ms = excluded.updated_at_ms
         RETURNING id",
        params![
            storage_root_id,
            file.path_text,
            file.path_key,
            metadata.title,
            metadata.artist,
            metadata.album,
            metadata.album_artist,
            track_number,
            disc_number,
            metadata.duration_ms,
            sample_rate_hz,
            bit_depth,
            channels,
            file_size_bytes,
            file.modified_at_ns,
            now_ms,
        ],
        |row| row.get(0),
    )?;
    Ok(id)
}

pub(super) fn update_track_path(
    transaction: &Transaction<'_>,
    track_id: TrackId,
    path: &str,
    updated_at_ms: i64,
) -> Result<(), LibraryError> {
    transaction.execute(
        "UPDATE tracks SET path = ?2, updated_at_ms = ?3 WHERE id = ?1",
        params![track_id, path, updated_at_ms],
    )?;
    Ok(())
}

pub(super) fn set_track_cover(
    transaction: &Transaction<'_>,
    track_id: TrackId,
    path: &str,
    mime_type: Option<&str>,
) -> Result<(), LibraryError> {
    transaction.execute(
        "UPDATE tracks
         SET cover_art_path = ?2, cover_art_mime_type = ?3
         WHERE id = ?1",
        params![track_id, path, mime_type],
    )?;
    Ok(())
}

pub(super) fn clear_track_cover(
    transaction: &Transaction<'_>,
    track_id: TrackId,
) -> Result<(), LibraryError> {
    transaction.execute(
        "UPDATE tracks
         SET cover_art_path = NULL, cover_art_mime_type = NULL
         WHERE id = ?1",
        [track_id],
    )?;
    Ok(())
}

pub(super) fn delete_track(
    transaction: &Transaction<'_>,
    track_id: TrackId,
) -> Result<(), LibraryError> {
    transaction.execute("DELETE FROM tracks WHERE id = ?1", [track_id])?;
    Ok(())
}

pub(super) struct CompletedScan {
    pub finished_at_ms: i64,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unsupported: usize,
    pub errors: usize,
    pub removals_suppressed: bool,
    pub outcome: ScanOutcome,
}

pub(super) fn finish_completed_scan(
    transaction: &Transaction<'_>,
    scan_id: i64,
    storage_root_id: StorageRootId,
    completed: &CompletedScan,
) -> Result<(), LibraryError> {
    transaction.execute(
        "UPDATE storage_roots
         SET is_reachable = 1, last_scan_at_ms = ?2
         WHERE id = ?1",
        params![storage_root_id, completed.finished_at_ms],
    )?;
    transaction.execute(
        "UPDATE scan_history
         SET finished_at_ms = ?2, added_count = ?3, updated_count = ?4,
             removed_count = ?5, unsupported_count = ?6, error_count = ?7,
             removals_suppressed = ?8, outcome = ?9
         WHERE id = ?1",
        params![
            scan_id,
            completed.finished_at_ms,
            usize_to_i64(completed.added, "added count")?,
            usize_to_i64(completed.updated, "updated count")?,
            usize_to_i64(completed.removed, "removed count")?,
            usize_to_i64(completed.unsupported, "unsupported count")?,
            usize_to_i64(completed.errors, "error count")?,
            completed.removals_suppressed,
            completed.outcome.as_db_str(),
        ],
    )?;
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

fn track_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    let file_size_bytes = row.get::<_, i64>(13)?;
    Ok(Track {
        id: row.get(0)?,
        storage_root_id: row.get(1)?,
        path: PathBuf::from(row.get::<_, String>(2)?),
        title: row.get(3)?,
        artist: row.get(4)?,
        album: row.get(5)?,
        album_artist: row.get(6)?,
        track_number: row.get::<_, Option<i64>>(7)?.map(|value| value as u32),
        disc_number: row.get::<_, Option<i64>>(8)?.map(|value| value as u32),
        duration_ms: row.get(9)?,
        sample_rate_hz: row.get::<_, Option<i64>>(10)?.map(|value| value as u32),
        bit_depth: row.get::<_, Option<i64>>(11)?.map(|value| value as u8),
        channels: row.get::<_, Option<i64>>(12)?.map(|value| value as u8),
        file_size_bytes: file_size_bytes as u64,
        modified_at_ns: row.get(14)?,
        cover_art_path: row.get::<_, Option<String>>(15)?.map(PathBuf::from),
        cover_art_mime_type: row.get(16)?,
        added_at_ms: row.get(17)?,
        updated_at_ms: row.get(18)?,
    })
}

fn summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LibrarySummary> {
    Ok(LibrarySummary {
        album_count: row.get::<_, i64>(0)? as u64,
        track_count: row.get::<_, i64>(1)? as u64,
        file_size_bytes: row.get::<_, i64>(2)? as u64,
    })
}

fn scan_history_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScanHistoryEntry> {
    let outcome = row
        .get::<_, Option<String>>(10)?
        .map(|outcome| {
            ScanOutcome::from_db_str(&outcome).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    10,
                    rusqlite::types::Type::Text,
                    format!("unknown scan outcome {outcome}").into(),
                )
            })
        })
        .transpose()?;
    Ok(ScanHistoryEntry {
        id: row.get(0)?,
        storage_root_id: row.get(1)?,
        started_at_ms: row.get(2)?,
        finished_at_ms: row.get(3)?,
        added_count: row.get::<_, i64>(4)? as u64,
        updated_count: row.get::<_, i64>(5)? as u64,
        removed_count: row.get::<_, i64>(6)? as u64,
        unsupported_count: row.get::<_, i64>(7)? as u64,
        error_count: row.get::<_, i64>(8)? as u64,
        removals_suppressed: row.get(9)?,
        outcome,
        error_message: row.get(11)?,
    })
}

fn process_scan_session_id() -> &'static str {
    static SESSION_ID: OnceLock<String> = OnceLock::new();
    SESSION_ID.get_or_init(|| {
        let started_at_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        format!("{}-{started_at_ns}", std::process::id())
    })
}

fn usize_to_i64(value: usize, name: &'static str) -> Result<i64, LibraryError> {
    i64::try_from(value).map_err(|_| LibraryError::IntegerOutOfRange(name))
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::library::{metadata::EmbeddedArtwork, path::path_identity};

    fn test_file(root: &StorageRoot, name: &str, modified_at_ns: i64, size: u64) -> DiscoveredFile {
        let path = root.path.join(name);
        let (path_text, path_key) = path_identity(&path, root.is_case_sensitive).unwrap();
        DiscoveredFile {
            path,
            path_text,
            path_key,
            modified_at_ns,
            file_size_bytes: size,
        }
    }

    fn test_metadata(
        title: &str,
        artist: &str,
        album: Option<&str>,
        album_artist: Option<&str>,
    ) -> AudioMetadata {
        AudioMetadata {
            title: Some(title.to_string()),
            artist: Some(artist.to_string()),
            album: album.map(str::to_string),
            album_artist: album_artist.map(str::to_string),
            track_number: Some(1),
            disc_number: Some(1),
            duration_ms: Some(1_000),
            sample_rate_hz: Some(44_100),
            bit_depth: Some(16),
            channels: Some(2),
            artwork: None,
        }
    }

    fn insert_track(
        store: &mut LibraryStore,
        root: &StorageRoot,
        file: &DiscoveredFile,
        metadata: &AudioMetadata,
    ) -> TrackId {
        let transaction = store.connection.transaction().unwrap();
        let id = upsert_track(&transaction, root.id, file, metadata, 100).unwrap();
        transaction.commit().unwrap();
        id
    }

    #[test]
    fn inserts_and_incrementally_updates_a_track_in_place() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        let mut file = test_file(&root, "track.wav", 10, 100);

        let first_id = insert_track(
            &mut store,
            &root,
            &file,
            &test_metadata("First", "Artist", Some("Album"), Some("Album Artist")),
        );
        file.modified_at_ns = 20;
        file.file_size_bytes = 200;
        let second_id = insert_track(
            &mut store,
            &root,
            &file,
            &test_metadata("Updated", "Artist", Some("Album"), Some("Album Artist")),
        );

        let tracks = store.tracks_for_root(root.id).unwrap();
        assert_eq!(first_id, second_id);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title.as_deref(), Some("Updated"));
        assert_eq!(tracks[0].modified_at_ns, 20);
        assert_eq!(tracks[0].file_size_bytes, 200);
        assert_eq!(
            store.existing_tracks(root.id).unwrap()[&file.path_key].modified_at_ns,
            20
        );
    }

    #[test]
    fn derives_root_and_catalog_counts_without_denormalized_counters() {
        let first_temp = tempdir().unwrap();
        let second_temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let first_root = store.add_storage_root(first_temp.path(), "First").unwrap();
        let second_root = store
            .add_storage_root(second_temp.path(), "Second")
            .unwrap();

        for (index, (album, album_artist)) in [
            (Some("Shared"), Some("Artist A")),
            (Some("Shared"), Some("Artist A")),
            (Some("Shared"), Some("Artist B")),
            (None, None),
        ]
        .into_iter()
        .enumerate()
        {
            insert_track(
                &mut store,
                &first_root,
                &test_file(&first_root, &format!("{index}.wav"), index as i64, 10),
                &test_metadata("Track", "Artist", album, album_artist),
            );
        }
        insert_track(
            &mut store,
            &second_root,
            &test_file(&second_root, "other.wav", 1, 25),
            &test_metadata("Other", "Artist C", Some("Other Album"), None),
        );

        assert_eq!(
            store.root_summary(first_root.id).unwrap(),
            LibrarySummary {
                album_count: 2,
                track_count: 4,
                file_size_bytes: 40,
            }
        );
        assert_eq!(
            store.catalog_summary().unwrap(),
            LibrarySummary {
                album_count: 3,
                track_count: 5,
                file_size_bytes: 65,
            }
        );
    }

    #[test]
    fn marks_an_offline_root_without_removing_its_tracks() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        insert_track(
            &mut store,
            &root,
            &test_file(&root, "track.wav", 1, 10),
            &test_metadata("Track", "Artist", Some("Album"), None),
        );
        let scan_id = store.begin_scan(root.id, 100).unwrap();

        store
            .finish_offline_scan(scan_id, root.id, 200, "not mounted")
            .unwrap();

        assert_eq!(store.tracks_for_root(root.id).unwrap().len(), 1);
        let stored_root = store.storage_root(root.id).unwrap().unwrap();
        assert!(!stored_root.is_reachable);
        assert_eq!(stored_root.last_scan_at_ms, Some(200));
        let history = store.recent_scans(root.id, 1).unwrap();
        assert_eq!(history[0].outcome, Some(ScanOutcome::Offline));
        assert_eq!(history[0].error_count, 1);
    }

    #[test]
    fn removing_a_root_cascades_rows_and_returns_cover_paths() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
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

        assert_eq!(
            store.remove_storage_root(root.id).unwrap(),
            vec![cover_path]
        );
        assert!(store.storage_root(root.id).unwrap().is_none());
        assert!(store.tracks_for_root(root.id).unwrap().is_empty());
    }

    #[test]
    fn reopens_persistent_data_and_recovers_an_interrupted_other_session() {
        let temp = tempdir().unwrap();
        let music = temp.path().join("music");
        fs::create_dir(&music).unwrap();
        let database_path = temp.path().join("library.sqlite");
        let mut first = LibraryStore::from_connection(
            Connection::open(&database_path).unwrap(),
            "first-session".to_string(),
        )
        .unwrap();
        let root = first.add_storage_root(&music, "Music").unwrap();
        let root_id = root.id;
        let scan_id = first.begin_scan(root.id, 100).unwrap();
        drop(first);

        let same_session = LibraryStore::from_connection(
            Connection::open(&database_path).unwrap(),
            "first-session".to_string(),
        )
        .unwrap();
        assert_eq!(
            same_session.recent_scans(root_id, 1).unwrap()[0].outcome,
            None
        );
        drop(same_session);

        let reopened = LibraryStore::from_connection(
            Connection::open(&database_path).unwrap(),
            "second-session".to_string(),
        )
        .unwrap();
        assert_eq!(reopened.storage_roots().unwrap(), vec![root]);
        let history = reopened.recent_scans(root_id, 1).unwrap();
        assert_eq!(history[0].id, scan_id);
        assert_eq!(history[0].outcome, Some(ScanOutcome::Failed));
        assert!(history[0].finished_at_ms.is_some());
        assert_eq!(
            history[0].error_message.as_deref(),
            Some("Scan interrupted before completion")
        );
    }
}
