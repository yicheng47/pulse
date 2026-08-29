use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
};

use rusqlite::{Connection, Transaction, params, params_from_iter, types::Value};

use super::super::{
    LibraryError, LibrarySummary, StorageRootId, Track, TrackId, TrackPage, TrackQueryFilter,
    TrackSortOrder, UNKNOWN_ALBUM, UNKNOWN_ARTIST, metadata::AudioMetadata, walk::DiscoveredFile,
};
use super::usize_to_i64;

/// Column list matching `track_from_row_at`'s positional mapping — every
/// track SELECT is built from this one definition so the statements and the
/// mapper cannot drift apart silently.
pub const TRACK_COLUMNS: &str = "id, storage_root_id, path, title, artist, album, album_artist,
     year, genre, track_number, disc_number, duration_ms, sample_rate_hz,
     bit_depth, channels, file_size_bytes, modified_at_ns, cover_art_path,
     cover_art_mime_type, added_at_ms, updated_at_ms";

/// `alias.col, ...` — the same list qualified for one side of a JOIN.
pub fn qualified_track_columns(alias: &str) -> String {
    TRACK_COLUMNS
        .split(',')
        .map(|column| format!("{alias}.{}", column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug)]
pub struct ExistingTrack {
    pub id: TrackId,
    pub path: PathBuf,
    pub path_text: String,
    pub modified_at_ns: i64,
    pub cover_art_path: Option<PathBuf>,
}

pub fn for_root(
    conn: &Connection,
    storage_root_id: StorageRootId,
) -> Result<Vec<Track>, LibraryError> {
    let sql = format!(
        "SELECT {TRACK_COLUMNS}
         FROM tracks
         WHERE storage_root_id = ?1
         ORDER BY path_key"
    );
    let mut statement = conn.prepare(&sql)?;
    let tracks = statement
        .query_map([storage_root_id], track_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tracks)
}

pub fn all(conn: &Connection, sort_order: TrackSortOrder) -> Result<Vec<Track>, LibraryError> {
    let sql = format!(
        "SELECT {TRACK_COLUMNS}
         FROM tracks
         ORDER BY {}",
        track_order_by(sort_order)
    );
    let mut statement = conn.prepare(&sql)?;
    let tracks = statement
        .query_map([], track_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tracks)
}

pub fn page(
    conn: &Connection,
    sort_order: TrackSortOrder,
    filter: &TrackQueryFilter,
    artist_filter: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<TrackPage, LibraryError> {
    assert!(limit > 0, "track page size must be positive");

    let (where_clause, mut parameters) = track_filter_clause(filter, artist_filter);

    let count_sql = format!("SELECT COUNT(*) FROM tracks{where_clause}");
    let total_count = conn.query_row(&count_sql, params_from_iter(parameters.iter()), |row| {
        row.get::<_, i64>(0)
    })?;
    let total_count =
        usize::try_from(total_count).map_err(|_| LibraryError::IntegerOutOfRange("track count"))?;
    // Appending callers need an empty tail; snapping back would duplicate the last page.
    let offset = offset.min(total_count);

    let sql = format!(
        "SELECT {TRACK_COLUMNS}
         FROM tracks{where_clause}
         ORDER BY {}
         LIMIT ? OFFSET ?",
        track_order_by(sort_order)
    );
    parameters.push(Value::Integer(usize_to_i64(limit, "track page limit")?));
    parameters.push(Value::Integer(usize_to_i64(offset, "track page offset")?));
    let mut statement = conn.prepare(&sql)?;
    let tracks = statement
        .query_map(params_from_iter(parameters.iter()), track_from_row)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(TrackPage {
        tracks,
        total_count,
    })
}

pub fn matching(
    conn: &Connection,
    sort_order: TrackSortOrder,
    filter: &TrackQueryFilter,
    artist_filter: Option<&str>,
) -> Result<Vec<Track>, LibraryError> {
    let (where_clause, parameters) = track_filter_clause(filter, artist_filter);
    let sql = format!(
        "SELECT {TRACK_COLUMNS}
         FROM tracks{where_clause}
         ORDER BY {}",
        track_order_by(sort_order)
    );
    let mut statement = conn.prepare(&sql)?;
    let tracks = statement
        .query_map(params_from_iter(parameters.iter()), track_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tracks)
}

pub fn for_album(conn: &Connection, artist: &str, title: &str) -> Result<Vec<Track>, LibraryError> {
    let sql = format!(
        "SELECT {TRACK_COLUMNS}
         FROM tracks
         WHERE COALESCE(NULLIF(trim(album_artist), ''),
                        NULLIF(trim(artist), ''), ?1) = ?3
           AND COALESCE(NULLIF(trim(album), ''), ?2) = ?4
         ORDER BY COALESCE(disc_number, 1),
                  track_number IS NULL, track_number, path_key"
    );
    let mut statement = conn.prepare(&sql)?;
    let tracks = statement
        .query_map(
            params![UNKNOWN_ARTIST, UNKNOWN_ALBUM, artist, title],
            track_from_row,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tracks)
}

pub fn artists(conn: &Connection) -> Result<Vec<(String, u64)>, LibraryError> {
    let mut statement = conn.prepare(
        "SELECT COALESCE(NULLIF(trim(artist), ''), ?1) AS artist_name, COUNT(*)
         FROM tracks
         GROUP BY artist_name COLLATE NOCASE
         ORDER BY artist_name COLLATE NOCASE, artist_name",
    )?;
    let artists = statement
        .query_map([UNKNOWN_ARTIST], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(artists)
}

pub fn genres(conn: &Connection) -> Result<Vec<String>, LibraryError> {
    Ok(genre_album_counts(conn)?
        .into_iter()
        .map(|(genre, _)| genre)
        .collect())
}

pub fn genre_album_counts(conn: &Connection) -> Result<Vec<(String, u64)>, LibraryError> {
    let mut statement = conn.prepare(
        "SELECT COALESCE(NULLIF(trim(album_artist), ''),
                        NULLIF(trim(artist), ''), ?1) AS album_owner,
                COALESCE(NULLIF(trim(album), ''), ?2) AS album_title,
                trim(genre) AS genre_tag
         FROM tracks
         WHERE genre IS NOT NULL AND trim(genre) <> ''
         GROUP BY album_owner, album_title, genre_tag COLLATE NOCASE
         ORDER BY MIN(id)",
    )?;
    let stored: Vec<(String, String, String)> = statement
        .query_map(params![UNKNOWN_ARTIST, UNKNOWN_ALBUM], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut genres = BTreeMap::<String, (String, HashSet<(String, String)>)>::new();
    for (album_artist, album_title, value) in stored {
        for member in genre_tag_members(&value) {
            genres
                .entry(member.to_ascii_lowercase())
                .or_insert_with(|| (member.to_string(), HashSet::new()))
                .1
                .insert((album_artist.clone(), album_title.clone()));
        }
    }
    Ok(genres
        .into_values()
        .map(|(genre, albums)| (genre, albums.len() as u64))
        .collect())
}

pub fn root_summary(
    conn: &Connection,
    storage_root_id: StorageRootId,
) -> Result<LibrarySummary, LibraryError> {
    let summary = conn.query_row(
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

pub fn catalog_summary(conn: &Connection) -> Result<LibrarySummary, LibraryError> {
    let summary = conn.query_row(
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

pub fn existing(
    conn: &Connection,
    storage_root_id: StorageRootId,
) -> Result<HashMap<String, ExistingTrack>, LibraryError> {
    let mut statement = conn.prepare(
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

pub fn cover_paths_for_root(
    conn: &Connection,
    storage_root_id: StorageRootId,
) -> Result<Vec<PathBuf>, LibraryError> {
    let mut statement = conn.prepare(
        "SELECT cover_art_path
         FROM tracks
         WHERE storage_root_id = ?1 AND cover_art_path IS NOT NULL",
    )?;
    let paths = statement
        .query_map([storage_root_id], |row| {
            row.get::<_, String>(0).map(PathBuf::from)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(paths)
}

pub fn delete_for_root(
    conn: &Connection,
    storage_root_id: StorageRootId,
) -> Result<(), LibraryError> {
    conn.execute(
        "DELETE FROM tracks WHERE storage_root_id = ?1",
        [storage_root_id],
    )?;
    Ok(())
}

pub fn upsert_track(
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

pub fn update_track_path(
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

pub fn set_track_cover(
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

pub fn clear_track_cover(
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

pub fn delete_track(transaction: &Transaction<'_>, track_id: TrackId) -> Result<(), LibraryError> {
    super::playlists::delete_entries_for_track(transaction, track_id)?;
    transaction.execute("DELETE FROM tracks WHERE id = ?1", [track_id])?;
    Ok(())
}

pub fn track_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    track_from_row_at(row, 0)
}

pub fn track_from_row_at(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<Track> {
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

/// Splits a stored genre tag into its trimmed, non-empty comma members. The
/// single definition of membership: the genre picker enumerates values with
/// it and the `genre_has_member` SQL function matches rows with it, so a row can
/// never name a member the queries fail to find.
pub fn genre_tag_members(tag: &str) -> impl Iterator<Item = &str> {
    tag.split(',')
        .map(str::trim)
        .filter(|member| !member.is_empty())
}

/// True when the bound parameter is a member of the row's comma-separated
/// genre tag ("Musiques du monde, J-pop, Japon" matches "J-pop"). Exact
/// ASCII-case-insensitive equality per member — `%`/`_` are literal.
const GENRE_MEMBER_CLAUSE: &str = "genre_has_member(genre, ?)";

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
            clauses.push(GENRE_MEMBER_CLAUSE);
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

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use tempfile::tempdir;

    use crate::backend::library::{
        AlbumQueryFilter, AlbumSortOrder, LibraryStore, LibrarySummary, TrackQueryFilter,
        TrackSortOrder,
        store::testing::{insert_track, test_file, test_metadata},
    };

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
    fn delete_tracks_removes_rows_and_playlist_entries_for_only_the_given_tracks() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        let doomed_first = insert_track(
            &mut store,
            &root,
            &test_file(&root, "doomed-1.wav", 1, 10),
            &test_metadata("One", "Artist", Some("Doomed"), None),
        );
        let doomed_second = insert_track(
            &mut store,
            &root,
            &test_file(&root, "doomed-2.wav", 2, 10),
            &test_metadata("Two", "Artist", Some("Doomed"), None),
        );
        let survivor = insert_track(
            &mut store,
            &root,
            &test_file(&root, "survivor.wav", 3, 10),
            &test_metadata("Three", "Artist", Some("Kept"), None),
        );
        let playlist = store.create_playlist("Mixed").unwrap();
        store
            .append_playlist_tracks(playlist.id, &[doomed_first, survivor, doomed_second])
            .unwrap();

        store.delete_tracks(&[doomed_first, doomed_second]).unwrap();

        assert!(
            store
                .tracks_for_album("Artist", "Doomed")
                .unwrap()
                .is_empty()
        );
        let kept = store.tracks_for_album("Artist", "Kept").unwrap();
        assert_eq!(
            kept.iter().map(|track| track.id).collect::<Vec<_>>(),
            vec![survivor]
        );
        let entries = store.playlist_tracks(playlist.id).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.track.id)
                .collect::<Vec<_>>(),
            vec![survivor]
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
            ("Charlie", "Artist A", "Jazz, Modal", 16, 96_000, 300),
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

        let mut paged_titles = Vec::new();
        for offset in [0, 2, 4] {
            let page = store
                .track_page(
                    TrackSortOrder::Title,
                    &TrackQueryFilter::All,
                    None,
                    2,
                    offset,
                )
                .unwrap();
            assert_eq!(page.total_count, 6);
            paged_titles.extend(page.tracks.into_iter().map(|track| track.title.unwrap()));
        }
        assert_eq!(
            paged_titles,
            ["Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot"]
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
        assert_eq!(jazz.total_count, 3, "comma-list member still matches");
        let modal = store
            .track_page(
                TrackSortOrder::Title,
                &TrackQueryFilter::Genre("modal".to_string()),
                None,
                50,
                0,
            )
            .unwrap();
        assert_eq!(modal.total_count, 1, "secondary member of the list matches");
        assert_eq!(
            store.genres().unwrap(),
            ["Jazz", "Modal", "Rock"],
            "comma lists split into one chip per member"
        );
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
    fn track_pages_keep_a_total_order_for_tied_sort_keys() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        for (index, title) in ["Tie C", "Tie A", "Tie B"].into_iter().enumerate() {
            let id = insert_track(
                &mut store,
                &root,
                &test_file(&root, &format!("{title}.wav"), index as i64, 10),
                &test_metadata(title, "Artist", Some("Album"), None),
            );
            store
                .connection
                .execute(
                    "UPDATE tracks SET added_at_ms = 100 WHERE id = ?1",
                    params![id],
                )
                .unwrap();
        }

        let mut seen = Vec::new();
        for offset in [0, 1, 2] {
            let page = store
                .track_page(
                    TrackSortOrder::DateAdded,
                    &TrackQueryFilter::All,
                    None,
                    1,
                    offset,
                )
                .unwrap();
            assert_eq!(page.total_count, 3);
            assert_eq!(page.tracks.len(), 1);
            seen.push(page.tracks[0].title.clone().unwrap());
        }
        seen.sort();
        assert_eq!(
            seen,
            ["Tie A", "Tie B", "Tie C"],
            "paging tied tracks must neither duplicate nor drop a row"
        );
    }

    #[test]
    fn track_pages_are_empty_at_and_beyond_the_end() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        for (index, title) in ["Alpha", "Bravo"].into_iter().enumerate() {
            insert_track(
                &mut store,
                &root,
                &test_file(&root, &format!("{title}.wav"), index as i64, 10),
                &test_metadata(title, "Artist", Some("Album"), None),
            );
        }

        for offset in [2, 100] {
            let page = store
                .track_page(
                    TrackSortOrder::Title,
                    &TrackQueryFilter::All,
                    None,
                    2,
                    offset,
                )
                .unwrap();
            assert_eq!(page.total_count, 2);
            assert!(page.tracks.is_empty());
        }
    }

    #[test]
    fn genre_album_counts_split_members_and_count_each_album_once() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        for (index, (title, album, genre)) in [
            ("One A", "Album One", "Jazz, Modal"),
            ("One B", "Album One", "jazz"),
            ("Two", "Album Two", "JAZZ"),
            ("Three", "Album Three", "Rock"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut metadata = test_metadata(title, "Artist", Some(album), None);
            metadata.genre = Some(genre.to_string());
            insert_track(
                &mut store,
                &root,
                &test_file(&root, &format!("{title}.wav"), index as i64, 10),
                &metadata,
            );
        }

        assert_eq!(
            store.genre_album_counts().unwrap(),
            [
                ("Jazz".to_string(), 2),
                ("Modal".to_string(), 1),
                ("Rock".to_string(), 1),
            ]
        );
    }

    #[test]
    fn genre_membership_is_exact_for_chips_tracks_and_albums() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        for (index, (title, album, genre)) in [
            ("Alpha", "Album One", "Jazz,  Modal"),
            ("Bravo", "Album Two", "Rock%"),
            ("Charlie", "Album Three", "Rockabilly"),
            ("Delta", "Album Four", "R_B"),
            ("Echo", "Album Five", "RnB"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut metadata = test_metadata(title, "Artist", Some(album), None);
            metadata.genre = Some(genre.to_string());
            insert_track(
                &mut store,
                &root,
                &test_file(&root, &format!("{title}.wav"), index as i64, 10),
                &metadata,
            );
        }

        assert_eq!(
            store.genres().unwrap(),
            ["Jazz", "Modal", "R_B", "RnB", "Rock%", "Rockabilly"],
            "chips trim every member, including repeated delimiter whitespace"
        );

        let tracks_for = |genre: &str| {
            store
                .track_page(
                    TrackSortOrder::Title,
                    &TrackQueryFilter::Genre(genre.to_string()),
                    None,
                    50,
                    0,
                )
                .unwrap()
                .tracks
                .into_iter()
                .map(|track| track.title.unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            tracks_for("Modal"),
            ["Alpha"],
            "a member after repeated delimiter whitespace still matches"
        );
        assert_eq!(
            tracks_for("Rock%"),
            ["Bravo"],
            "percent is literal, not a wildcard"
        );
        assert_eq!(
            tracks_for("R_B"),
            ["Delta"],
            "underscore is literal, not a wildcard"
        );

        let albums_for = |genre: &str| {
            store
                .album_page(
                    AlbumSortOrder::Title,
                    &AlbumQueryFilter::Genre(genre.to_string()),
                    None,
                    50,
                    0,
                )
                .unwrap()
                .albums
                .into_iter()
                .map(|album| album.title)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            albums_for("Modal"),
            ["Album One"],
            "the album filter matches a secondary comma-list member"
        );
        assert_eq!(albums_for("Rock%"), ["Album Two"]);
        assert_eq!(albums_for("R_B"), ["Album Four"]);
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
}
