use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{
    Connection, OptionalExtension, Transaction, params, params_from_iter, types::Value,
};

use super::{
    Album, AlbumSortOrder, LibraryError, LibrarySearchResults, LibrarySummary, Playlist,
    PlaylistId, PlaylistSummary, PlaylistTrack, ScanHistoryEntry, ScanOutcome, StorageRoot,
    StorageRootId, Track, TrackId, TrackPage, TrackQueryFilter, TrackSortOrder, UNKNOWN_ALBUM,
    UNKNOWN_ARTIST,
    metadata::{self, AudioMetadata},
    path::normalize_storage_root,
    system_time_ms,
    walk::DiscoveredFile,
};

const SCHEMA_VERSION: i64 = 3;

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

-- FTS5 is deliberately deferred. MVP search uses capped LIKE queries whose
-- result types can be preserved when an FTS table is added later.

PRAGMA user_version = 3;
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
        mut connection: Connection,
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
            1 => {
                migrate_v1_to_v2(&mut connection)?;
                migrate_v2_to_v3(&mut connection)?;
            }
            2 => migrate_v2_to_v3(&mut connection)?,
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

    pub fn rename_storage_root(
        &mut self,
        storage_root_id: StorageRootId,
        display_name: impl AsRef<str>,
    ) -> Result<StorageRoot, LibraryError> {
        let updated = self.connection.execute(
            "UPDATE storage_roots SET display_name = ?2 WHERE id = ?1",
            params![storage_root_id, display_name.as_ref()],
        )?;
        if updated == 0 {
            return Err(LibraryError::StorageRootNotFound(storage_root_id));
        }
        self.storage_root(storage_root_id)?
            .ok_or(LibraryError::StorageRootNotFound(storage_root_id))
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
                    year, genre, track_number, disc_number, duration_ms, sample_rate_hz,
                    bit_depth, channels, file_size_bytes, modified_at_ns, cover_art_path,
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

    pub fn albums(&self, sort_order: AlbumSortOrder) -> Result<Vec<Album>, LibraryError> {
        let order_by = match sort_order {
            AlbumSortOrder::Title => "album_title COLLATE NOCASE, album_owner COLLATE NOCASE",
            AlbumSortOrder::Artist => "album_owner COLLATE NOCASE, album_title COLLATE NOCASE",
            AlbumSortOrder::DateAdded => "latest_added_at_ms DESC, album_title COLLATE NOCASE",
            AlbumSortOrder::ReleaseYear => {
                "album_year IS NULL, album_year DESC, album_title COLLATE NOCASE"
            }
            AlbumSortOrder::Duration => "total_duration_ms DESC, album_title COLLATE NOCASE",
        };
        let sql = format!(
            "WITH normalized AS (
                 SELECT id,
                        COALESCE(NULLIF(trim(album_artist), ''),
                                 NULLIF(trim(artist), ''), ?1) AS album_owner,
                        COALESCE(NULLIF(trim(album), ''), ?2) AS album_title,
                        year, duration_ms, sample_rate_hz, bit_depth, cover_art_path, added_at_ms
                 FROM tracks
             )
             SELECT album_title, album_owner, MIN(year) AS album_year,
                    COUNT(*) AS track_count,
                    COALESCE(SUM(duration_ms), 0) AS total_duration_ms,
                    MAX(sample_rate_hz) AS max_sample_rate_hz,
                    MAX(bit_depth) AS max_bit_depth,
                    substr(MIN(
                        CASE WHEN cover_art_path IS NOT NULL
                             THEN printf('%020lld%s', id, cover_art_path)
                        END
                    ), 21) AS cover_art_path,
                    MAX(added_at_ms) AS latest_added_at_ms
             FROM normalized
             GROUP BY album_owner, album_title
             ORDER BY {order_by}"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let mut albums = statement
            .query_map(params![UNKNOWN_ARTIST, UNKNOWN_ALBUM], album_from_row)?
            .collect::<Result<Vec<_>, _>>()?;

        let mut genres_by_album = HashMap::<(String, String), Vec<String>>::new();
        let mut genres_statement = self.connection.prepare(
            "SELECT
                 COALESCE(NULLIF(trim(album_artist), ''), NULLIF(trim(artist), ''), ?1),
                 COALESCE(NULLIF(trim(album), ''), ?2),
                 trim(genre)
             FROM tracks
             WHERE genre IS NOT NULL AND trim(genre) <> ''
             GROUP BY 1, 2, 3
             ORDER BY trim(genre) COLLATE NOCASE",
        )?;
        let genre_rows =
            genres_statement.query_map(params![UNKNOWN_ARTIST, UNKNOWN_ALBUM], |row| {
                Ok((
                    (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                    row.get::<_, String>(2)?,
                ))
            })?;
        for row in genre_rows {
            let (key, genre) = row?;
            genres_by_album.entry(key).or_default().push(genre);
        }
        for album in &mut albums {
            album.genres = genres_by_album
                .remove(&(album.artist.clone(), album.title.clone()))
                .unwrap_or_default();
        }

        Ok(albums)
    }

    pub fn all_tracks(&self, sort_order: TrackSortOrder) -> Result<Vec<Track>, LibraryError> {
        let order_by = track_order_by(sort_order);
        let sql = format!(
            "SELECT id, storage_root_id, path, title, artist, album, album_artist,
                    year, genre, track_number, disc_number, duration_ms, sample_rate_hz,
                    bit_depth, channels, file_size_bytes, modified_at_ns, cover_art_path,
                    cover_art_mime_type, added_at_ms, updated_at_ms
             FROM tracks
             ORDER BY {order_by}"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let tracks = statement
            .query_map([], track_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tracks)
    }

    pub fn track_page(
        &self,
        sort_order: TrackSortOrder,
        filter: &TrackQueryFilter,
        artist_filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<TrackPage, LibraryError> {
        assert!(limit > 0, "track page size must be positive");

        let (where_clause, mut parameters) = track_filter_clause(filter, artist_filter);

        let count_sql = format!("SELECT COUNT(*) FROM tracks{where_clause}");
        let total_count =
            self.connection
                .query_row(&count_sql, params_from_iter(parameters.iter()), |row| {
                    row.get::<_, i64>(0)
                })?;
        let total_count = usize::try_from(total_count)
            .map_err(|_| LibraryError::IntegerOutOfRange("track count"))?;
        let last_offset = total_count.saturating_sub(1) / limit * limit;
        let offset = offset.min(last_offset);

        let sql = format!(
            "SELECT id, storage_root_id, path, title, artist, album, album_artist,
                    year, genre, track_number, disc_number, duration_ms, sample_rate_hz,
                    bit_depth, channels, file_size_bytes, modified_at_ns, cover_art_path,
                    cover_art_mime_type, added_at_ms, updated_at_ms
             FROM tracks{where_clause}
             ORDER BY {}
             LIMIT ? OFFSET ?",
            track_order_by(sort_order)
        );
        parameters.push(Value::Integer(usize_to_i64(limit, "track page limit")?));
        parameters.push(Value::Integer(usize_to_i64(offset, "track page offset")?));
        let mut statement = self.connection.prepare(&sql)?;
        let tracks = statement
            .query_map(params_from_iter(parameters.iter()), track_from_row)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(TrackPage {
            tracks,
            total_count,
        })
    }

    pub fn matching_tracks(
        &self,
        sort_order: TrackSortOrder,
        filter: &TrackQueryFilter,
        artist_filter: Option<&str>,
    ) -> Result<Vec<Track>, LibraryError> {
        let (where_clause, parameters) = track_filter_clause(filter, artist_filter);
        let sql = format!(
            "SELECT id, storage_root_id, path, title, artist, album, album_artist,
                    year, genre, track_number, disc_number, duration_ms, sample_rate_hz,
                    bit_depth, channels, file_size_bytes, modified_at_ns, cover_art_path,
                    cover_art_mime_type, added_at_ms, updated_at_ms
             FROM tracks{where_clause}
             ORDER BY {}",
            track_order_by(sort_order)
        );
        let mut statement = self.connection.prepare(&sql)?;
        let tracks = statement
            .query_map(params_from_iter(parameters.iter()), track_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tracks)
    }

    pub fn tracks_for_album(&self, artist: &str, title: &str) -> Result<Vec<Track>, LibraryError> {
        let mut statement = self.connection.prepare(
            "SELECT id, storage_root_id, path, title, artist, album, album_artist,
                    year, genre, track_number, disc_number, duration_ms, sample_rate_hz,
                    bit_depth, channels, file_size_bytes, modified_at_ns, cover_art_path,
                    cover_art_mime_type, added_at_ms, updated_at_ms
             FROM tracks
             WHERE COALESCE(NULLIF(trim(album_artist), ''),
                            NULLIF(trim(artist), ''), ?1) = ?3
               AND COALESCE(NULLIF(trim(album), ''), ?2) = ?4
             ORDER BY COALESCE(disc_number, 1),
                      track_number IS NULL, track_number, path_key",
        )?;
        let tracks = statement
            .query_map(
                params![UNKNOWN_ARTIST, UNKNOWN_ALBUM, artist, title],
                track_from_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tracks)
    }

    pub fn genres(&self) -> Result<Vec<String>, LibraryError> {
        let mut statement = self.connection.prepare(
            "SELECT trim(genre)
             FROM tracks
             WHERE genre IS NOT NULL AND trim(genre) <> ''
             GROUP BY trim(genre) COLLATE NOCASE
             ORDER BY trim(genre) COLLATE NOCASE",
        )?;
        let genres = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(genres)
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
                         SELECT
                             COALESCE(NULLIF(trim(album), ''), ?2) AS album_title,
                             COALESCE(NULLIF(trim(album_artist), ''),
                                      NULLIF(trim(artist), ''), ?3) AS album_owner
                         FROM tracks
                         WHERE storage_root_id = ?1
                         GROUP BY album_title, album_owner
                     )
                 ),
                 COUNT(*),
                 COALESCE(SUM(file_size_bytes), 0)
             FROM tracks
             WHERE storage_root_id = ?1",
            params![storage_root_id, UNKNOWN_ALBUM, UNKNOWN_ARTIST],
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
                         SELECT
                             COALESCE(NULLIF(trim(album), ''), ?1) AS album_title,
                             COALESCE(NULLIF(trim(album_artist), ''),
                                      NULLIF(trim(artist), ''), ?2) AS album_owner
                         FROM tracks
                         GROUP BY album_title, album_owner
                     )
                 ),
                 COUNT(*),
                 COALESCE(SUM(file_size_bytes), 0)
             FROM tracks",
            params![UNKNOWN_ALBUM, UNKNOWN_ARTIST],
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

    pub fn create_playlist(&mut self, name: &str) -> Result<Playlist, LibraryError> {
        let now_ms = system_time_ms(SystemTime::now())?;
        let id = self.connection.query_row(
            "INSERT INTO playlists (name, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?2)
             RETURNING id",
            params![name, now_ms],
            |row| row.get(0),
        )?;
        self.playlist(id)?.ok_or(LibraryError::PlaylistNotFound(id))
    }

    pub fn playlist(&self, playlist_id: PlaylistId) -> Result<Option<Playlist>, LibraryError> {
        self.connection
            .query_row(
                "SELECT id, name, created_at_ms, updated_at_ms
                 FROM playlists
                 WHERE id = ?1",
                [playlist_id],
                playlist_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn rename_playlist(
        &mut self,
        playlist_id: PlaylistId,
        name: &str,
    ) -> Result<Playlist, LibraryError> {
        let now_ms = system_time_ms(SystemTime::now())?;
        let updated = self.connection.execute(
            "UPDATE playlists SET name = ?2, updated_at_ms = ?3 WHERE id = ?1",
            params![playlist_id, name, now_ms],
        )?;
        if updated == 0 {
            return Err(LibraryError::PlaylistNotFound(playlist_id));
        }
        self.playlist(playlist_id)?
            .ok_or(LibraryError::PlaylistNotFound(playlist_id))
    }

    pub fn delete_playlist(&mut self, playlist_id: PlaylistId) -> Result<(), LibraryError> {
        let deleted = self
            .connection
            .execute("DELETE FROM playlists WHERE id = ?1", [playlist_id])?;
        if deleted == 0 {
            return Err(LibraryError::PlaylistNotFound(playlist_id));
        }
        Ok(())
    }

    pub fn append_playlist_tracks(
        &mut self,
        playlist_id: PlaylistId,
        track_ids: &[TrackId],
    ) -> Result<(), LibraryError> {
        let transaction = self.connection.transaction()?;
        require_playlist(&transaction, playlist_id)?;
        if track_ids.is_empty() {
            transaction.commit()?;
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
        touch_playlist(&transaction, playlist_id)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn remove_playlist_entry(
        &mut self,
        playlist_id: PlaylistId,
        position: usize,
    ) -> Result<(), LibraryError> {
        let transaction = self.connection.transaction()?;
        require_playlist(&transaction, playlist_id)?;
        let mut entries = playlist_entry_ids(&transaction, playlist_id)?;
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
        replace_playlist_entries(&transaction, playlist_id, &entries)?;
        touch_playlist(&transaction, playlist_id)?;
        transaction.commit()?;
        Ok(())
    }

    /// Moves the entry at a stored position to a zero-based index in the ordered entries.
    pub fn move_playlist_entry(
        &mut self,
        playlist_id: PlaylistId,
        from_position: usize,
        to_position: usize,
    ) -> Result<(), LibraryError> {
        let transaction = self.connection.transaction()?;
        require_playlist(&transaction, playlist_id)?;
        let mut entries = playlist_entry_ids(&transaction, playlist_id)?;
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
            replace_playlist_entries(&transaction, playlist_id, &entries)?;
            touch_playlist(&transaction, playlist_id)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn playlists(&self) -> Result<Vec<PlaylistSummary>, LibraryError> {
        let mut statement = self.connection.prepare(
            "SELECT p.id, p.name, p.created_at_ms, p.updated_at_ms,
                    COUNT(pt.track_id), COALESCE(SUM(t.duration_ms), 0),
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
             LEFT JOIN tracks t ON t.id = pt.track_id
             GROUP BY p.id
             ORDER BY p.updated_at_ms DESC, p.id DESC",
        )?;
        let playlists = statement
            .query_map([], playlist_summary_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(playlists)
    }

    pub fn playlist_tracks(
        &self,
        playlist_id: PlaylistId,
    ) -> Result<Vec<PlaylistTrack>, LibraryError> {
        if self.playlist(playlist_id)?.is_none() {
            return Err(LibraryError::PlaylistNotFound(playlist_id));
        }
        let mut statement = self.connection.prepare(
            "SELECT pt.position,
                    t.id, t.storage_root_id, t.path, t.title, t.artist, t.album, t.album_artist,
                    t.year, t.genre, t.track_number, t.disc_number, t.duration_ms,
                    t.sample_rate_hz, t.bit_depth, t.channels, t.file_size_bytes,
                    t.modified_at_ns, t.cover_art_path, t.cover_art_mime_type,
                    t.added_at_ms, t.updated_at_ms
             FROM playlist_tracks pt
             JOIN tracks t ON t.id = pt.track_id
             WHERE pt.playlist_id = ?1
             ORDER BY pt.position",
        )?;
        let tracks = statement
            .query_map([playlist_id], |row| {
                let position = row.get::<_, i64>(0)?;
                Ok(PlaylistTrack {
                    position: usize::try_from(position).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Integer,
                            Box::new(error),
                        )
                    })?,
                    track: track_from_row_at(row, 1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tracks)
    }

    pub fn search(&self, query: &str) -> Result<LibrarySearchResults, LibraryError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(LibrarySearchResults::default());
        }
        let pattern = like_pattern(query);

        let mut albums_statement = self.connection.prepare(
            "WITH normalized AS (
                 SELECT id, title, artist, album, album_artist,
                        COALESCE(NULLIF(trim(album_artist), ''),
                                 NULLIF(trim(artist), ''), ?1) AS album_owner,
                        COALESCE(NULLIF(trim(album), ''), ?2) AS album_title,
                        year, duration_ms, sample_rate_hz, bit_depth, cover_art_path, added_at_ms
                 FROM tracks
             ), matching_albums AS (
                 SELECT DISTINCT album_owner, album_title
                 FROM normalized
                 WHERE COALESCE(title, '') LIKE ?3 ESCAPE '\\'
                    OR COALESCE(artist, '') LIKE ?3 ESCAPE '\\'
                    OR COALESCE(album, '') LIKE ?3 ESCAPE '\\'
                    OR COALESCE(album_artist, '') LIKE ?3 ESCAPE '\\'
             )
             SELECT n.album_title, n.album_owner, MIN(n.year), COUNT(*),
                    COALESCE(SUM(n.duration_ms), 0), MAX(n.sample_rate_hz), MAX(n.bit_depth),
                    substr(MIN(
                        CASE WHEN n.cover_art_path IS NOT NULL
                             THEN printf('%020lld%s', n.id, n.cover_art_path)
                        END
                    ), 21),
                    MAX(n.added_at_ms)
             FROM normalized n
             JOIN matching_albums m
               ON m.album_owner = n.album_owner AND m.album_title = n.album_title
             GROUP BY n.album_owner, n.album_title
             ORDER BY n.album_title COLLATE NOCASE, n.album_owner COLLATE NOCASE
             LIMIT 3",
        )?;
        let albums = albums_statement
            .query_map(
                params![UNKNOWN_ARTIST, UNKNOWN_ALBUM, pattern],
                album_from_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;

        let mut tracks_statement = self.connection.prepare(
            "SELECT id, storage_root_id, path, title, artist, album, album_artist,
                    year, genre, track_number, disc_number, duration_ms, sample_rate_hz,
                    bit_depth, channels, file_size_bytes, modified_at_ns, cover_art_path,
                    cover_art_mime_type, added_at_ms, updated_at_ms
             FROM tracks
             WHERE COALESCE(title, '') LIKE ?1 ESCAPE '\\'
                OR COALESCE(artist, '') LIKE ?1 ESCAPE '\\'
                OR COALESCE(album, '') LIKE ?1 ESCAPE '\\'
                OR COALESCE(album_artist, '') LIKE ?1 ESCAPE '\\'
             ORDER BY COALESCE(NULLIF(trim(title), ''), path) COLLATE NOCASE, path_key
             LIMIT 5",
        )?;
        let tracks = tracks_statement
            .query_map([&pattern], track_from_row)?
            .collect::<Result<Vec<_>, _>>()?;

        let mut playlists_statement = self.connection.prepare(
            "SELECT p.id, p.name, p.created_at_ms, p.updated_at_ms,
                    COUNT(pt.track_id), COALESCE(SUM(t.duration_ms), 0),
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
             LEFT JOIN tracks t ON t.id = pt.track_id
             WHERE p.name LIKE ?1 ESCAPE '\\'
             GROUP BY p.id
             ORDER BY p.name COLLATE NOCASE, p.id
             LIMIT 3",
        )?;
        let playlists = playlists_statement
            .query_map([&pattern], playlist_summary_from_row)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(LibrarySearchResults {
            albums,
            tracks,
            playlists,
        })
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

    pub(super) fn cancel_scan(&self, scan_id: i64) -> Result<(), LibraryError> {
        self.connection.execute(
            "DELETE FROM scan_history
             WHERE id = ?1 AND finished_at_ms IS NULL AND outcome IS NULL",
            [scan_id],
        )?;
        Ok(())
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

fn migrate_v1_to_v2(connection: &mut Connection) -> Result<(), LibraryError> {
    let existing_tracks = {
        let mut statement = connection.prepare("SELECT id, path FROM tracks ORDER BY id")?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    PathBuf::from(row.get::<_, String>(1)?),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    let backfill = existing_tracks
        .iter()
        .filter_map(|(id, path)| {
            metadata::extract_metadata(path)
                .ok()
                .map(|metadata| (*id, metadata.year, metadata.genre))
        })
        .collect::<Vec<_>>();
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
    let mut statement = transaction.prepare(
        "SELECT position, track_id
         FROM playlist_tracks
         WHERE playlist_id = ?1
         ORDER BY position",
    )?;
    let rows = statement.query_map([playlist_id], |row| {
        let position = row.get::<_, i64>(0)?;
        let position = usize::try_from(position).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?;
        Ok((position, row.get(1)?))
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

fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
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
             year, genre, track_number, disc_number, duration_ms, sample_rate_hz,
             bit_depth, channels, file_size_bytes, modified_at_ns, added_at_ms, updated_at_ms
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
             ?18, ?18
         )
         ON CONFLICT(storage_root_id, path_key) DO UPDATE SET
             path = excluded.path,
             title = excluded.title,
             artist = excluded.artist,
             album = excluded.album,
             album_artist = excluded.album_artist,
             year = excluded.year,
             genre = excluded.genre,
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
            metadata.year,
            metadata.genre,
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
    track_from_row_at(row, 0)
}

fn track_from_row_at(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<Track> {
    let file_size_bytes = row.get::<_, i64>(offset + 15)?;
    Ok(Track {
        id: row.get(offset)?,
        storage_root_id: row.get(offset + 1)?,
        path: PathBuf::from(row.get::<_, String>(offset + 2)?),
        title: row.get(offset + 3)?,
        artist: row.get(offset + 4)?,
        album: row.get(offset + 5)?,
        album_artist: row.get(offset + 6)?,
        year: row
            .get::<_, Option<i64>>(offset + 7)?
            .map(|value| value as u32),
        genre: row.get(offset + 8)?,
        track_number: row
            .get::<_, Option<i64>>(offset + 9)?
            .map(|value| value as u32),
        disc_number: row
            .get::<_, Option<i64>>(offset + 10)?
            .map(|value| value as u32),
        duration_ms: row.get(offset + 11)?,
        sample_rate_hz: row
            .get::<_, Option<i64>>(offset + 12)?
            .map(|value| value as u32),
        bit_depth: row
            .get::<_, Option<i64>>(offset + 13)?
            .map(|value| value as u8),
        channels: row
            .get::<_, Option<i64>>(offset + 14)?
            .map(|value| value as u8),
        file_size_bytes: file_size_bytes as u64,
        modified_at_ns: row.get(offset + 16)?,
        cover_art_path: row
            .get::<_, Option<String>>(offset + 17)?
            .map(PathBuf::from),
        cover_art_mime_type: row.get(offset + 18)?,
        added_at_ms: row.get(offset + 19)?,
        updated_at_ms: row.get(offset + 20)?,
    })
}

fn playlist_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Playlist> {
    Ok(Playlist {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at_ms: row.get(2)?,
        updated_at_ms: row.get(3)?,
    })
}

fn playlist_summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlaylistSummary> {
    Ok(PlaylistSummary {
        playlist: playlist_from_row(row)?,
        track_count: row.get::<_, i64>(4)? as u64,
        total_duration_ms: row.get::<_, i64>(5)? as u64,
        cover_art_path: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
    })
}

fn album_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Album> {
    Ok(Album {
        title: row.get(0)?,
        artist: row.get(1)?,
        year: row.get::<_, Option<i64>>(2)?.map(|value| value as u32),
        track_count: row.get::<_, i64>(3)? as u64,
        total_duration_ms: row.get::<_, i64>(4)? as u64,
        max_sample_rate_hz: row.get::<_, Option<i64>>(5)?.map(|value| value as u32),
        max_bit_depth: row.get::<_, Option<i64>>(6)?.map(|value| value as u8),
        cover_art_path: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
        latest_added_at_ms: row.get(8)?,
        genres: Vec::new(),
    })
}

fn track_order_by(sort_order: TrackSortOrder) -> &'static str {
    match sort_order {
        TrackSortOrder::Title => "COALESCE(NULLIF(trim(title), ''), path) COLLATE NOCASE, path_key",
        TrackSortOrder::Artist => {
            "COALESCE(NULLIF(trim(artist), ''), 'Unknown Artist') COLLATE NOCASE,
             COALESCE(NULLIF(trim(title), ''), path) COLLATE NOCASE, path_key"
        }
        TrackSortOrder::Album => {
            "COALESCE(NULLIF(trim(album), ''), 'Unknown Album') COLLATE NOCASE,
             COALESCE(disc_number, 1), COALESCE(track_number, 2147483647), path_key"
        }
        TrackSortOrder::DateAdded => "added_at_ms DESC, path_key",
        TrackSortOrder::ReleaseYear => "year IS NULL, year DESC, path_key",
        TrackSortOrder::Duration => "duration_ms IS NULL, duration_ms DESC, path_key",
    }
}

fn track_filter_clause(
    filter: &TrackQueryFilter,
    artist_filter: Option<&str>,
) -> (String, Vec<Value>) {
    let mut clauses = Vec::new();
    let mut parameters = Vec::new();
    match filter {
        TrackQueryFilter::All => {}
        TrackQueryFilter::HiRes => {
            clauses.push("(bit_depth > 16 OR sample_rate_hz > 48000)");
        }
        TrackQueryFilter::AddedSince(timestamp_ms) => {
            clauses.push("added_at_ms >= ?");
            parameters.push(Value::Integer(*timestamp_ms));
        }
        TrackQueryFilter::Genre(genre) => {
            clauses.push("trim(genre) = ? COLLATE NOCASE");
            parameters.push(Value::Text(genre.clone()));
        }
    }
    if let Some(artist) = artist_filter {
        clauses.push("COALESCE(NULLIF(trim(artist), ''), 'Unknown Artist') = ? COLLATE NOCASE");
        parameters.push(Value::Text(artist.to_string()));
    }
    let where_clause = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    (where_clause, parameters)
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
        let schema = SCHEMA.replace(PLAYLIST_SCHEMA, "");
        assert_ne!(schema, SCHEMA);
        schema.replace(
            "PRAGMA user_version = 3;",
            &format!("PRAGMA user_version = {version};"),
        )
    }

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
            year: Some(2024),
            genre: Some("Electronic".to_string()),
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
                album_count: 3,
                track_count: 4,
                file_size_bytes: 40,
            }
        );
        assert_eq!(
            store.catalog_summary().unwrap(),
            LibrarySummary {
                album_count: 4,
                track_count: 5,
                file_size_bytes: 65,
            }
        );
    }

    #[test]
    fn groups_albums_with_unknown_bucket_genres_and_cover_fallback() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        let first = insert_track(
            &mut store,
            &root,
            &test_file(&root, "first.wav", 1, 10),
            &test_metadata("First", "Artist", Some("Album"), None),
        );
        let second = insert_track(
            &mut store,
            &root,
            &test_file(&root, "second.wav", 2, 20),
            &test_metadata("Second", "Artist", Some("Album"), None),
        );
        let mut unknown = test_metadata("Loose", "", None, None);
        unknown.genre = None;
        insert_track(
            &mut store,
            &root,
            &test_file(&root, "loose.wav", 3, 30),
            &unknown,
        );
        let cover = temp.path().join("album.cover");
        let transaction = store.connection.transaction().unwrap();
        set_track_cover(
            &transaction,
            second,
            cover.to_str().unwrap(),
            Some("image/jpeg"),
        )
        .unwrap();
        transaction.commit().unwrap();

        let albums = store.albums(AlbumSortOrder::Title).unwrap();

        assert_eq!(albums.len(), 2);
        assert_eq!(albums[0].title, "Album");
        assert_eq!(albums[0].artist, "Artist");
        assert_eq!(albums[0].track_count, 2);
        assert_eq!(albums[0].cover_art_path.as_ref(), Some(&cover));
        assert_eq!(albums[0].genres, ["Electronic"]);
        assert_eq!(albums[1].title, UNKNOWN_ALBUM);
        assert_eq!(albums[1].artist, UNKNOWN_ARTIST);
        assert!(albums[1].cover_art_path.is_none());
        assert_ne!(first, second);
        assert_eq!(store.genres().unwrap(), ["Electronic"]);
    }

    #[test]
    fn albums_implements_every_mvp_sort_order() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        let mut beta = test_metadata("Track B", "Alpha", Some("Beta"), None);
        beta.year = Some(2020);
        beta.duration_ms = Some(3_000);
        let beta_id = insert_track(
            &mut store,
            &root,
            &test_file(&root, "beta.wav", 1, 10),
            &beta,
        );
        let mut alpha = test_metadata("Track A", "Zulu", Some("Alpha"), None);
        alpha.year = Some(2024);
        alpha.duration_ms = Some(1_000);
        let alpha_id = insert_track(
            &mut store,
            &root,
            &test_file(&root, "alpha.wav", 2, 10),
            &alpha,
        );
        store
            .connection
            .execute(
                "UPDATE tracks
                 SET added_at_ms = CASE id WHEN ?1 THEN 100 WHEN ?2 THEN 200 END",
                params![beta_id, alpha_id],
            )
            .unwrap();

        let titles = |sort_order| {
            store
                .albums(sort_order)
                .unwrap()
                .into_iter()
                .map(|album| album.title)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            titles(AlbumSortOrder::Title),
            ["Alpha".to_string(), "Beta".to_string()]
        );
        assert_eq!(
            titles(AlbumSortOrder::Artist),
            ["Beta".to_string(), "Alpha".to_string()]
        );
        assert_eq!(
            titles(AlbumSortOrder::DateAdded),
            ["Alpha".to_string(), "Beta".to_string()]
        );
        assert_eq!(
            titles(AlbumSortOrder::ReleaseYear),
            ["Alpha".to_string(), "Beta".to_string()]
        );
        assert_eq!(
            titles(AlbumSortOrder::Duration),
            ["Beta".to_string(), "Alpha".to_string()]
        );
    }

    #[test]
    fn all_tracks_implements_every_mvp_sort_order() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        let mut beta = test_metadata("Beta", "Alpha", Some("Zulu"), None);
        beta.year = Some(2020);
        beta.duration_ms = Some(3_000);
        let beta_id = insert_track(
            &mut store,
            &root,
            &test_file(&root, "beta.wav", 1, 10),
            &beta,
        );
        let mut alpha = test_metadata("Alpha", "Zulu", Some("Alpha"), None);
        alpha.year = Some(2024);
        alpha.duration_ms = Some(1_000);
        let alpha_id = insert_track(
            &mut store,
            &root,
            &test_file(&root, "alpha.wav", 2, 10),
            &alpha,
        );
        store
            .connection
            .execute(
                "UPDATE tracks
                 SET added_at_ms = CASE id WHEN ?1 THEN 100 WHEN ?2 THEN 200 END",
                params![beta_id, alpha_id],
            )
            .unwrap();

        let titles = |sort_order| {
            store
                .all_tracks(sort_order)
                .unwrap()
                .into_iter()
                .map(|track| track.title.unwrap())
                .collect::<Vec<_>>()
        };

        assert_eq!(
            titles(TrackSortOrder::Title),
            ["Alpha".to_string(), "Beta".to_string()]
        );
        assert_eq!(
            titles(TrackSortOrder::Artist),
            ["Beta".to_string(), "Alpha".to_string()]
        );
        assert_eq!(
            titles(TrackSortOrder::Album),
            ["Alpha".to_string(), "Beta".to_string()]
        );
        assert_eq!(
            titles(TrackSortOrder::DateAdded),
            ["Alpha".to_string(), "Beta".to_string()]
        );
        assert_eq!(
            titles(TrackSortOrder::ReleaseYear),
            ["Alpha".to_string(), "Beta".to_string()]
        );
        assert_eq!(
            titles(TrackSortOrder::Duration),
            ["Beta".to_string(), "Alpha".to_string()]
        );
    }

    #[test]
    fn track_pages_apply_backend_filters_counts_and_offsets() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        for (index, (title, artist, genre, bit_depth, sample_rate_hz, added_at_ms)) in [
            ("Alpha", "Artist A", "Jazz", 16, 44_100, 100),
            ("Bravo", "Artist B", "Rock", 24, 44_100, 200),
            ("Charlie", "Artist A", "Jazz", 16, 96_000, 300),
            ("Delta", "Artist B", "Jazz", 16, 44_100, 400),
            ("Echo", "Artist A", "Rock", 16, 44_100, 500),
            ("Foxtrot", "Artist B", "Rock", 16, 44_100, 600),
        ]
        .into_iter()
        .enumerate()
        {
            let mut metadata = test_metadata(title, artist, Some("Album"), None);
            metadata.genre = Some(genre.to_string());
            metadata.bit_depth = Some(bit_depth);
            metadata.sample_rate_hz = Some(sample_rate_hz);
            let id = insert_track(
                &mut store,
                &root,
                &test_file(&root, &format!("{title}.wav"), index as i64, 10),
                &metadata,
            );
            store
                .connection
                .execute(
                    "UPDATE tracks SET added_at_ms = ?2 WHERE id = ?1",
                    params![id, added_at_ms],
                )
                .unwrap();
        }

        let page = store
            .track_page(TrackSortOrder::Title, &TrackQueryFilter::All, None, 2, 2)
            .unwrap();
        assert_eq!(page.total_count, 6);
        let first_track_on_page = page.tracks[0].id;
        assert_eq!(
            page.tracks
                .into_iter()
                .map(|track| track.title.unwrap())
                .collect::<Vec<_>>(),
            ["Charlie", "Delta"]
        );

        let artist = store
            .track_page(
                TrackSortOrder::Title,
                &TrackQueryFilter::All,
                Some("artist a"),
                50,
                0,
            )
            .unwrap();
        assert_eq!(artist.total_count, 3);
        let jazz = store
            .track_page(
                TrackSortOrder::Title,
                &TrackQueryFilter::Genre("jazz".to_string()),
                None,
                50,
                0,
            )
            .unwrap();
        assert_eq!(jazz.total_count, 3);
        let hi_res = store
            .track_page(TrackSortOrder::Title, &TrackQueryFilter::HiRes, None, 50, 0)
            .unwrap();
        assert_eq!(hi_res.total_count, 2);
        let recent = store
            .track_page(
                TrackSortOrder::Title,
                &TrackQueryFilter::AddedSince(400),
                None,
                50,
                0,
            )
            .unwrap();
        assert_eq!(recent.total_count, 3);

        let clamped = store
            .track_page(TrackSortOrder::Title, &TrackQueryFilter::All, None, 2, 100)
            .unwrap();
        assert_eq!(
            clamped
                .tracks
                .into_iter()
                .map(|track| track.title.unwrap())
                .collect::<Vec<_>>(),
            ["Echo", "Foxtrot"]
        );

        let queue = store
            .matching_tracks(TrackSortOrder::Title, &TrackQueryFilter::All, None)
            .unwrap();
        assert_eq!(queue.len(), 6);
        assert_eq!(
            queue
                .iter()
                .position(|track| track.id == first_track_on_page),
            Some(2)
        );

        let matching = store
            .matching_tracks(
                TrackSortOrder::Title,
                &TrackQueryFilter::Genre("jazz".to_string()),
                None,
            )
            .unwrap();
        assert_eq!(matching.len(), 3);
        assert_eq!(
            matching
                .into_iter()
                .map(|track| track.title.unwrap())
                .collect::<Vec<_>>(),
            ["Alpha", "Charlie", "Delta"]
        );
    }

    #[test]
    fn orders_album_tracks_by_disc_then_track_number() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();

        for (name, disc, track) in [("Disc Two", 2, 1), ("Second", 1, 2), ("First", 1, 1)] {
            let mut metadata = test_metadata(name, "Artist", Some("Album"), None);
            metadata.disc_number = Some(disc);
            metadata.track_number = Some(track);
            insert_track(
                &mut store,
                &root,
                &test_file(&root, &format!("{name}.wav"), i64::from(track), 10),
                &metadata,
            );
        }

        let titles = store
            .tracks_for_album("Artist", "Album")
            .unwrap()
            .into_iter()
            .map(|track| track.title.unwrap())
            .collect::<Vec<_>>();
        assert_eq!(titles, ["First", "Second", "Disc Two"]);
    }

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
    fn playlist_crud_allows_duplicate_names() {
        let mut store = LibraryStore::open_in_memory().unwrap();

        let first = store.create_playlist("Night Drive").unwrap();
        let second = store.create_playlist("Night Drive").unwrap();
        let renamed = store.rename_playlist(first.id, "Night Drive").unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(renamed.name, "Night Drive");
        assert_eq!(store.playlists().unwrap().len(), 2);

        store.delete_playlist(first.id).unwrap();
        assert!(store.playlist(first.id).unwrap().is_none());
        assert_eq!(store.playlists().unwrap().len(), 1);
        assert!(matches!(
            store.delete_playlist(first.id),
            Err(LibraryError::PlaylistNotFound(id)) if id == first.id
        ));
    }

    #[test]
    fn playlist_entries_allow_duplicates_and_remove_with_contiguous_positions() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
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
        let playlist = store.create_playlist("Duplicates").unwrap();

        store
            .append_playlist_tracks(playlist.id, &[first, first, second])
            .unwrap();
        let entries = store.playlist_tracks(playlist.id).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.position, entry.track.id))
                .collect::<Vec<_>>(),
            [(0, first), (1, first), (2, second)]
        );

        store.remove_playlist_entry(playlist.id, 1).unwrap();
        let entries = store.playlist_tracks(playlist.id).unwrap();
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
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        let mut ids = Vec::new();
        for (index, title) in ["A", "B", "C", "D"].into_iter().enumerate() {
            ids.push(insert_track(
                &mut store,
                &root,
                &test_file(&root, &format!("{title}.wav"), index as i64, 10),
                &test_metadata(title, "Artist", Some("Album"), None),
            ));
        }
        let playlist = store.create_playlist("Order").unwrap();
        store.append_playlist_tracks(playlist.id, &ids).unwrap();
        let titles = |store: &LibraryStore| {
            store
                .playlist_tracks(playlist.id)
                .unwrap()
                .into_iter()
                .map(|entry| (entry.position, entry.track.title.unwrap()))
                .collect::<Vec<_>>()
        };

        store.move_playlist_entry(playlist.id, 0, 3).unwrap();
        assert_eq!(
            titles(&store),
            [
                (0, "B".into()),
                (1, "C".into()),
                (2, "D".into()),
                (3, "A".into())
            ]
        );

        store.move_playlist_entry(playlist.id, 3, 0).unwrap();
        assert_eq!(
            titles(&store),
            [
                (0, "A".into()),
                (1, "B".into()),
                (2, "C".into()),
                (3, "D".into())
            ]
        );

        store.move_playlist_entry(playlist.id, 1, 2).unwrap();
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
    fn playlist_entries_cascade_from_tracks_and_playlists() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        let track = insert_track(
            &mut store,
            &root,
            &test_file(&root, "track.wav", 1, 10),
            &test_metadata("Track", "Artist", Some("Album"), None),
        );
        let first = store.create_playlist("First").unwrap();
        let second = store.create_playlist("Second").unwrap();
        store.append_playlist_tracks(first.id, &[track]).unwrap();
        store.append_playlist_tracks(second.id, &[track]).unwrap();

        let transaction = store.connection.transaction().unwrap();
        delete_track(&transaction, track).unwrap();
        transaction.commit().unwrap();
        assert!(store.playlist_tracks(first.id).unwrap().is_empty());
        assert!(store.playlist_tracks(second.id).unwrap().is_empty());

        let replacement = insert_track(
            &mut store,
            &root,
            &test_file(&root, "replacement.wav", 2, 10),
            &test_metadata("Replacement", "Artist", Some("Album"), None),
        );
        store
            .append_playlist_tracks(first.id, &[replacement])
            .unwrap();
        store.delete_playlist(first.id).unwrap();
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
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
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
        let transaction = store.connection.transaction().unwrap();
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
        let playlist = store.create_playlist("Aggregate").unwrap();
        store.append_playlist_tracks(playlist.id, &ids).unwrap();
        let empty = store.create_playlist("Empty").unwrap();

        let playlists = store.playlists().unwrap();
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

    #[test]
    fn search_matches_case_insensitive_fields_cjk_substrings_and_album_artist() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        insert_track(
            &mut store,
            &root,
            &test_file(&root, "frank.wav", 1, 10),
            &test_metadata("Nights", "Frank Ocean", Some("Blonde"), None),
        );
        insert_track(
            &mut store,
            &root,
            &test_file(&root, "cjk.wav", 2, 10),
            &test_metadata("天空", "王菲", Some("菲靡靡之音"), None),
        );
        insert_track(
            &mut store,
            &root,
            &test_file(&root, "collective.wav", 3, 10),
            &test_metadata(
                "Guest Song",
                "Guest Singer",
                Some("Compilation"),
                Some("The Collective"),
            ),
        );
        store.create_playlist("Frank favorites").unwrap();

        let frank = store.search("FRANK").unwrap();
        assert_eq!(frank.tracks.len(), 1);
        assert_eq!(frank.albums.len(), 1);
        assert_eq!(frank.playlists.len(), 1);

        let cjk = store.search("菲").unwrap();
        assert_eq!(cjk.tracks[0].artist.as_deref(), Some("王菲"));
        assert_eq!(cjk.albums[0].title, "菲靡靡之音");

        let album_artist = store.search("collective").unwrap();
        assert_eq!(album_artist.tracks.len(), 1);
        assert_eq!(album_artist.albums[0].artist, "The Collective");

        assert_eq!(
            store.search("gibberish").unwrap(),
            LibrarySearchResults::default()
        );
        assert_eq!(
            store.search("   ").unwrap(),
            LibrarySearchResults::default()
        );
    }

    #[test]
    fn search_caps_each_result_group() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        for index in 0..7 {
            insert_track(
                &mut store,
                &root,
                &test_file(&root, &format!("cap-{index}.wav"), index, 10),
                &test_metadata(
                    &format!("Cap Track {index}"),
                    "Cap Artist",
                    Some(&format!("Cap Album {index}")),
                    None,
                ),
            );
        }
        for index in 0..4 {
            store
                .create_playlist(&format!("Cap Playlist {index}"))
                .unwrap();
        }

        let results = store.search("cap").unwrap();
        assert_eq!(results.albums.len(), 3);
        assert_eq!(results.tracks.len(), 5);
        assert_eq!(results.playlists.len(), 3);
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

        let store = LibraryStore::open(&database_path).unwrap();
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
        assert_eq!(
            store
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
    }

    #[test]
    fn migrates_v2_to_v3_and_creates_playlist_tables() {
        let temp = tempdir().unwrap();
        let database_path = temp.path().join("library.sqlite");
        let connection = Connection::open(&database_path).unwrap();
        connection.execute_batch(&pre_v3_schema(2)).unwrap();
        drop(connection);

        let mut store = LibraryStore::open(&database_path).unwrap();
        let playlist = store.create_playlist("Migrated").unwrap();

        assert_eq!(store.playlist(playlist.id).unwrap(), Some(playlist));
        assert_eq!(
            store
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            3
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
