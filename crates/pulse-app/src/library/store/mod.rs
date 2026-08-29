// Repo-style persistence layer: one submodule per entity family (roots,
// tracks, albums, playlists, scans, search) owning that family's SQL and row
// mappers, over the shared connection lifecycle here.
//
// Conventions:
//   - Module functions take `&Connection` (or a `&Transaction` via `Deref`),
//     never the store — `LibraryStore` methods own connection acquisition and
//     transaction boundaries, so multi-entity operations compose entity
//     functions inside one transaction (see `remove_storage_root`).
//   - `LibraryStore` keeps the public API the rest of the app calls; the
//     split is an internal reorganization, not a redesign.
//   - Functions return `Result<_, LibraryError>` — the same contract the
//     monolithic store had — so caller-side error handling is untouched.

mod albums;
mod artists;
mod playlists;
mod roots;
mod scans;
mod schema;
mod search;
mod tracks;

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, Transaction, config::DbConfig, functions::FunctionFlags};

use super::{
    Album, AlbumPage, AlbumQueryFilter, AlbumSortOrder, Artist, LibraryError, LibrarySearchResults,
    LibrarySummary, Playlist, PlaylistId, PlaylistSummary, PlaylistTrack, ScanHistoryEntry,
    StorageRoot, StorageRootId, Track, TrackId, TrackPage, TrackQueryFilter, TrackSortOrder,
    system_time_ms,
};

// SQLite expression indexes cannot match a bound fallback parameter, so the
// shared identity expression keeps the shipped unknown-artist value literal.
const EFFECTIVE_ALBUM_ARTIST_SQL: &str =
    "COALESCE(NULLIF(trim(album_artist), ''), NULLIF(trim(artist), ''), 'Unknown Artist')";

pub(super) use scans::CompletedScan;
pub use schema::BackfillProgress;
pub(super) use tracks::{
    ExistingTrack, clear_track_cover, delete_track, set_track_cover, update_track_path,
    upsert_track,
};

pub(super) fn artist_name_key_for_track(
    conn: &Connection,
    track_id: TrackId,
) -> Result<Option<String>, LibraryError> {
    artists::name_key_for_track(conn, track_id)
}

pub(super) fn refresh_artist_keys(
    transaction: &Transaction<'_>,
    name_keys: &[String],
    refreshed_at_ms: i64,
) -> Result<(), LibraryError> {
    artists::refresh_keys(transaction, name_keys, refreshed_at_ms)
}

pub struct LibraryStore {
    pub(super) connection: Connection,
    scan_session_id: String,
}

impl LibraryStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LibraryError> {
        Self::open_with_progress(path, |_| {})
    }

    pub fn open_with_progress(
        path: impl AsRef<Path>,
        on_backfill_progress: impl FnMut(BackfillProgress),
    ) -> Result<Self, LibraryError> {
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
            on_backfill_progress,
        )
    }

    pub fn open_in_memory() -> Result<Self, LibraryError> {
        Self::from_connection(
            Connection::open_in_memory()?,
            process_scan_session_id().to_owned(),
            |_| {},
        )
    }

    fn from_connection(
        mut connection: Connection,
        scan_session_id: String,
        on_backfill_progress: impl FnMut(BackfillProgress),
    ) -> Result<Self, LibraryError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, false)?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;",
        )?;
        // Exact-membership genre predicate for SQL queries, sharing
        // `genre_tag_members` with the picker enumeration in `genres()` so both
        // sides split and trim identically and LIKE wildcards never apply.
        connection.create_scalar_function(
            "genre_has_member",
            2,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            |ctx| {
                let Some(tag) = ctx.get::<Option<String>>(0)? else {
                    return Ok(false);
                };
                let member = ctx.get::<String>(1)?;
                Ok(tracks::genre_tag_members(&tag)
                    .any(|candidate| candidate.eq_ignore_ascii_case(&member)))
            },
        )?;

        schema::migrate_to_current(&mut connection, on_backfill_progress)?;

        let recovered_at_ms = system_time_ms(SystemTime::now())?;
        scans::recover_interrupted(&connection, recovered_at_ms, &scan_session_id)?;

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
        roots::add(&self.connection, path.as_ref(), display_name.as_ref())
    }

    pub fn storage_root(
        &self,
        storage_root_id: StorageRootId,
    ) -> Result<Option<StorageRoot>, LibraryError> {
        roots::get(&self.connection, storage_root_id)
    }

    pub fn storage_roots(&self) -> Result<Vec<StorageRoot>, LibraryError> {
        roots::list(&self.connection)
    }

    pub fn rename_storage_root(
        &mut self,
        storage_root_id: StorageRootId,
        display_name: impl AsRef<str>,
    ) -> Result<StorageRoot, LibraryError> {
        roots::rename(&self.connection, storage_root_id, display_name.as_ref())
    }

    pub fn remove_storage_root(
        &mut self,
        storage_root_id: StorageRootId,
    ) -> Result<Vec<PathBuf>, LibraryError> {
        let refreshed_at_ms = system_time_ms(SystemTime::now())?;
        let transaction = self.connection.transaction()?;
        let cover_art_paths = tracks::cover_paths_for_root(&transaction, storage_root_id)?;
        // Children are deleted explicitly, in dependency order — related-data
        // removal is application-owned (standing preference, 2026-08-07), and
        // since schema v4 there are no cascades behind these deletes at all.
        playlists::delete_entries_for_root(&transaction, storage_root_id)?;
        scans::delete_for_root(&transaction, storage_root_id)?;
        tracks::delete_for_root(&transaction, storage_root_id)?;
        let deleted = roots::delete(&transaction, storage_root_id)?;
        if deleted == 0 {
            return Err(LibraryError::StorageRootNotFound(storage_root_id));
        }
        artists::refresh(&transaction, refreshed_at_ms)?;
        transaction.commit()?;
        Ok(cover_art_paths)
    }

    pub fn tracks_for_root(
        &self,
        storage_root_id: StorageRootId,
    ) -> Result<Vec<Track>, LibraryError> {
        tracks::for_root(&self.connection, storage_root_id)
    }

    pub fn albums(&self, sort_order: AlbumSortOrder) -> Result<Vec<Album>, LibraryError> {
        albums::list(&self.connection, sort_order)
    }

    /// One page of the grouped album catalog, filtered in SQL so the grid's
    /// infinite scroll never has to load the whole library.
    pub fn album_page(
        &self,
        sort_order: AlbumSortOrder,
        filter: &AlbumQueryFilter,
        artist_filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<AlbumPage, LibraryError> {
        albums::page(
            &self.connection,
            sort_order,
            filter,
            artist_filter,
            limit,
            offset,
        )
    }

    pub fn artist_index(&self) -> Result<Vec<Artist>, LibraryError> {
        artists::index(&self.connection)
    }

    pub fn all_tracks(&self, sort_order: TrackSortOrder) -> Result<Vec<Track>, LibraryError> {
        tracks::all(&self.connection, sort_order)
    }

    pub fn track_page(
        &self,
        sort_order: TrackSortOrder,
        filter: &TrackQueryFilter,
        artist_filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<TrackPage, LibraryError> {
        tracks::page(
            &self.connection,
            sort_order,
            filter,
            artist_filter,
            limit,
            offset,
        )
    }

    pub fn matching_tracks(
        &self,
        sort_order: TrackSortOrder,
        filter: &TrackQueryFilter,
        artist_filter: Option<&str>,
    ) -> Result<Vec<Track>, LibraryError> {
        tracks::matching(&self.connection, sort_order, filter, artist_filter)
    }

    pub fn tracks_for_album(&self, artist: &str, title: &str) -> Result<Vec<Track>, LibraryError> {
        tracks::for_album(&self.connection, artist, title)
    }

    /// Acquire and immediately release a write lock, proving the database
    /// accepts writes before a caller starts irreversible filesystem work.
    pub fn preflight_write(&mut self) -> Result<(), LibraryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        transaction.rollback()?;
        Ok(())
    }

    /// Delete a set of tracks and their playlist entries in one transaction.
    /// Callers own the files: audio and cover-cache deletion happens outside
    /// the store, mirroring `remove_storage_root`.
    pub fn delete_tracks(&mut self, track_ids: &[TrackId]) -> Result<(), LibraryError> {
        let refreshed_at_ms = system_time_ms(SystemTime::now())?;
        let transaction = self.connection.transaction()?;
        for &track_id in track_ids {
            tracks::delete_track(&transaction, track_id)?;
        }
        artists::refresh(&transaction, refreshed_at_ms)?;
        transaction.commit()?;
        Ok(())
    }

    /// Distinct normalized artists with track counts, for the artist-filter
    /// popover. Normalization matches the artist-filter query clause.
    pub fn artists(&self) -> Result<Vec<(String, u64)>, LibraryError> {
        tracks::artists(&self.connection)
    }

    /// Distinct individual genres. Stores like Qobuz write one comma-separated
    /// list into the tag ("Musiques du monde, J-pop, Japon"), so the stored
    /// strings are split into members here rather than surfaced as one value.
    pub fn genres(&self) -> Result<Vec<String>, LibraryError> {
        tracks::genres(&self.connection)
    }

    /// Distinct individual genres with unique album counts, for the shared
    /// Albums and Tracks genre-filter popover.
    pub fn genre_album_counts(&self) -> Result<Vec<(String, u64)>, LibraryError> {
        tracks::genre_album_counts(&self.connection)
    }

    pub fn root_summary(
        &self,
        storage_root_id: StorageRootId,
    ) -> Result<LibrarySummary, LibraryError> {
        tracks::root_summary(&self.connection, storage_root_id)
    }

    pub fn catalog_summary(&self) -> Result<LibrarySummary, LibraryError> {
        tracks::catalog_summary(&self.connection)
    }

    pub fn recent_scans(
        &self,
        storage_root_id: StorageRootId,
        limit: usize,
    ) -> Result<Vec<ScanHistoryEntry>, LibraryError> {
        scans::recent(&self.connection, storage_root_id, limit)
    }

    pub fn create_playlist(&mut self, name: &str) -> Result<Playlist, LibraryError> {
        playlists::create(&self.connection, name)
    }

    pub fn playlist(&self, playlist_id: PlaylistId) -> Result<Option<Playlist>, LibraryError> {
        playlists::get(&self.connection, playlist_id)
    }

    pub fn rename_playlist(
        &mut self,
        playlist_id: PlaylistId,
        name: &str,
    ) -> Result<Playlist, LibraryError> {
        playlists::rename(&self.connection, playlist_id, name)
    }

    pub fn delete_playlist(&mut self, playlist_id: PlaylistId) -> Result<(), LibraryError> {
        let transaction = self.connection.transaction()?;
        playlists::delete(&transaction, playlist_id)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn append_playlist_tracks(
        &mut self,
        playlist_id: PlaylistId,
        track_ids: &[TrackId],
    ) -> Result<(), LibraryError> {
        let transaction = self.connection.transaction()?;
        playlists::append_tracks(&transaction, playlist_id, track_ids)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn remove_playlist_entry(
        &mut self,
        playlist_id: PlaylistId,
        position: usize,
    ) -> Result<(), LibraryError> {
        let transaction = self.connection.transaction()?;
        playlists::remove_entry(&transaction, playlist_id, position)?;
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
        playlists::move_entry(&transaction, playlist_id, from_position, to_position)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn playlists(&self) -> Result<Vec<PlaylistSummary>, LibraryError> {
        playlists::list(&self.connection)
    }

    pub fn playlist_tracks(
        &self,
        playlist_id: PlaylistId,
    ) -> Result<Vec<PlaylistTrack>, LibraryError> {
        playlists::tracks(&self.connection, playlist_id)
    }

    pub fn search(&self, query: &str) -> Result<LibrarySearchResults, LibraryError> {
        search::search(&self.connection, query)
    }

    pub(super) fn begin_scan(
        &self,
        storage_root_id: StorageRootId,
        started_at_ms: i64,
    ) -> Result<i64, LibraryError> {
        scans::begin(
            &self.connection,
            &self.scan_session_id,
            storage_root_id,
            started_at_ms,
        )
    }

    pub(super) fn cancel_scan(&mut self, scan_id: i64) -> Result<(), LibraryError> {
        let refreshed_at_ms = system_time_ms(SystemTime::now())?;
        let transaction = self.connection.transaction()?;
        scans::cancel(&transaction, scan_id)?;
        artists::refresh(&transaction, refreshed_at_ms)?;
        transaction.commit()?;
        Ok(())
    }

    pub(super) fn mark_root_reachable(
        &self,
        storage_root_id: StorageRootId,
    ) -> Result<(), LibraryError> {
        roots::mark_reachable(&self.connection, storage_root_id)
    }

    pub(super) fn finish_offline_scan(
        &mut self,
        scan_id: i64,
        storage_root_id: StorageRootId,
        finished_at_ms: i64,
        error_message: &str,
    ) -> Result<(), LibraryError> {
        let transaction = self.connection.transaction()?;
        scans::finish_offline(
            &transaction,
            scan_id,
            storage_root_id,
            finished_at_ms,
            error_message,
        )?;
        artists::refresh(&transaction, finished_at_ms)?;
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
        scans::finish_failed(
            &transaction,
            scan_id,
            storage_root_id,
            finished_at_ms,
            error_message,
        )?;
        artists::refresh(&transaction, finished_at_ms)?;
        transaction.commit()?;
        Ok(())
    }

    pub(super) fn finish_completed_scan(
        &mut self,
        scan_id: i64,
        storage_root_id: StorageRootId,
        completed: &CompletedScan,
    ) -> Result<(), LibraryError> {
        let transaction = self.connection.transaction()?;
        scans::finish_completed_scan(&transaction, scan_id, storage_root_id, completed)?;
        artists::refresh(&transaction, completed.finished_at_ms)?;
        transaction.commit()?;
        Ok(())
    }

    pub(super) fn existing_tracks(
        &self,
        storage_root_id: StorageRootId,
    ) -> Result<HashMap<String, ExistingTrack>, LibraryError> {
        tracks::existing(&self.connection, storage_root_id)
    }
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
pub(crate) mod testing {
    use super::super::{
        StorageRoot, TrackId, metadata::AudioMetadata, path::path_identity, walk::DiscoveredFile,
    };
    use super::{LibraryStore, tracks::upsert_track};

    pub fn set_cover(store: &mut LibraryStore, track_id: TrackId, path: &std::path::Path) {
        let transaction = store.connection.transaction().unwrap();
        super::tracks::set_track_cover(
            &transaction,
            track_id,
            &path.display().to_string(),
            Some("image/png"),
        )
        .unwrap();
        let name_key = super::artists::name_key_for_track(&transaction, track_id)
            .unwrap()
            .unwrap();
        super::artists::refresh_keys(&transaction, &[name_key], 100).unwrap();
        transaction.commit().unwrap();
    }

    /// Drop the playlist_tracks table so the next `delete_tracks` fails after
    /// `preflight_write` has already succeeded — the post-unlink database
    /// failure path.
    pub fn break_playlist_entries(store: &mut LibraryStore) {
        store
            .connection
            .execute("DROP TABLE playlist_tracks", [])
            .unwrap();
    }

    pub fn test_file(
        root: &StorageRoot,
        name: &str,
        modified_at_ns: i64,
        size: u64,
    ) -> DiscoveredFile {
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

    pub fn test_metadata(
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

    pub fn insert_track(
        store: &mut LibraryStore,
        root: &StorageRoot,
        file: &DiscoveredFile,
        metadata: &AudioMetadata,
    ) -> TrackId {
        let transaction = store.connection.transaction().unwrap();
        let id = upsert_track(&transaction, root.id, file, metadata, 100).unwrap();
        let name_key = super::artists::name_key_for_track(&transaction, id)
            .unwrap()
            .unwrap();
        super::artists::refresh_keys(&transaction, &[name_key], 100).unwrap();
        transaction.commit().unwrap();
        id
    }
}
