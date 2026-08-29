// Persistence layer: one submodule per table family, with derived album and
// search query modules over tracks, over the shared connection lifecycle here.

pub(super) mod albums;
pub(super) mod artists;
pub(super) mod playlists;
pub(super) mod scan_history;
mod schema;
pub(super) mod search;
pub(super) mod storage_roots;
pub(super) mod tracks;

use std::{
    fs,
    path::Path,
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, Transaction, config::DbConfig, functions::FunctionFlags};

use super::{LibraryError, scan::system_time_ms};

impl From<rusqlite::Error> for LibraryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error.to_string())
    }
}

// SQLite expression indexes cannot match a bound fallback parameter, so the
// shared identity expression keeps the shipped unknown-artist value literal.
const EFFECTIVE_ALBUM_ARTIST_SQL: &str =
    "COALESCE(NULLIF(trim(album_artist), ''), NULLIF(trim(artist), ''), 'Unknown Artist')";

const ALBUM_TITLE_SQL: &str = "COALESCE(NULLIF(trim(album), ''), {fallback})";

fn album_title_sql(fallback: &str) -> String {
    ALBUM_TITLE_SQL.replace("{fallback}", fallback)
}

fn select_list(columns: &[&str]) -> String {
    columns.join(", ")
}

fn qualified_select_list(alias: &str, columns: &[&str]) -> String {
    columns
        .iter()
        .map(|column| format!("{alias}.{column}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub use schema::BackfillProgress;

pub(super) struct LibraryTransaction<'connection> {
    inner: Transaction<'connection>,
}

impl LibraryTransaction<'_> {
    pub(super) fn commit(self) -> Result<(), LibraryError> {
        self.inner.commit()?;
        Ok(())
    }
}

pub struct LibraryStore {
    connection: Connection,
    scan_session_id: String,
}

impl LibraryStore {
    pub(super) fn transaction(&mut self) -> Result<LibraryTransaction<'_>, LibraryError> {
        Ok(LibraryTransaction {
            inner: self.connection.transaction()?,
        })
    }

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

        let mut store = Self {
            connection,
            scan_session_id,
        };
        let recovered_at_ms = system_time_ms(SystemTime::now())?;
        scan_history::recover_interrupted(&mut store, recovered_at_ms)?;
        Ok(store)
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
        StorageRoot, TrackId,
        scan::{metadata::AudioMetadata, path::path_identity, walk::DiscoveredFile},
    };
    use super::{LibraryStore, tracks::upsert_track};

    pub fn set_cover(store: &mut LibraryStore, track_id: TrackId, path: &std::path::Path) {
        let transaction = store.transaction().unwrap();
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

    pub fn fail_second_artist_refresh(store: &mut LibraryStore) {
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_second_artist
                 BEFORE INSERT ON artists
                 WHEN NEW.name_key = 'Second Artist'
                 BEGIN SELECT RAISE(ABORT, 'artist refresh failed'); END;",
            )
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
        let transaction = store.transaction().unwrap();
        let id = upsert_track(&transaction, root.id, file, metadata, 100).unwrap();
        let name_key = super::artists::name_key_for_track(&transaction, id)
            .unwrap()
            .unwrap();
        super::artists::refresh_keys(&transaction, &[name_key], 100).unwrap();
        transaction.commit().unwrap();
        id
    }
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn database_error_display_keeps_the_existing_user_message() {
        let source = Connection::open_in_memory()
            .unwrap()
            .execute("SELECT missing_column", [])
            .unwrap_err();
        let expected = format!("SQLite error: {source}");

        assert_eq!(LibraryError::from(source).to_string(), expected);
    }
}
