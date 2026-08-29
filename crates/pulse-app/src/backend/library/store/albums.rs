use std::path::PathBuf;

use rusqlite::{Connection, params, params_from_iter, types::Value};

use super::super::{
    Album, AlbumPage, AlbumQueryFilter, AlbumSortOrder, LibraryError, UNKNOWN_ALBUM,
};
use super::{EFFECTIVE_ALBUM_ARTIST_SQL, usize_to_i64};

pub fn list(conn: &Connection, sort_order: AlbumSortOrder) -> Result<Vec<Album>, LibraryError> {
    let order_by = album_order_by(sort_order);
    let sql = format!(
        "WITH normalized AS (
             SELECT id,
                    {EFFECTIVE_ALBUM_ARTIST_SQL} AS album_owner,
                    COALESCE(NULLIF(trim(album), ''), ?1) AS album_title,
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
    let mut statement = conn.prepare(&sql)?;
    let albums = statement
        .query_map(params![UNKNOWN_ALBUM], album_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(albums)
}

pub fn page(
    conn: &Connection,
    sort_order: AlbumSortOrder,
    filter: &AlbumQueryFilter,
    artist_filter: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<AlbumPage, LibraryError> {
    assert!(limit > 0, "album page size must be positive");
    let order_by = album_order_by(sort_order);
    let normalized_cte = format!(
        "WITH normalized AS (
             SELECT id,
                    {EFFECTIVE_ALBUM_ARTIST_SQL} AS album_owner,
                    COALESCE(NULLIF(trim(album), ''), ?1) AS album_title,
                    year, genre, duration_ms, sample_rate_hz, bit_depth,
                    cover_art_path, added_at_ms
             FROM tracks
         )"
    );
    let mut having_clauses = Vec::new();
    let mut filter_parameters = Vec::new();
    match filter {
        AlbumQueryFilter::All => {}
        AlbumQueryFilter::HiRes => {
            having_clauses.push("(MAX(bit_depth) > 16 OR MAX(sample_rate_hz) > 48000)");
        }
        AlbumQueryFilter::AddedSince(since) => {
            having_clauses.push("MAX(added_at_ms) >= ?");
            filter_parameters.push(Value::Integer(*since));
        }
        AlbumQueryFilter::Genre(genre) => {
            having_clauses.push("SUM(genre_has_member(genre, ?)) > 0");
            filter_parameters.push(Value::Text(genre.clone()));
        }
    }
    if let Some(artist) = artist_filter {
        having_clauses.push("album_owner = ?");
        filter_parameters.push(Value::Text(artist.to_string()));
    }
    let having = if having_clauses.is_empty() {
        String::new()
    } else {
        format!("HAVING {}", having_clauses.join(" AND "))
    };
    let mut parameters = vec![Value::Text(UNKNOWN_ALBUM.to_string())];
    parameters.extend(filter_parameters);

    let count_sql = format!(
        "{normalized_cte}
         SELECT COUNT(*) FROM (
             SELECT 1 FROM normalized GROUP BY album_owner, album_title {having}
         )"
    );
    let total_count = conn.query_row(&count_sql, params_from_iter(parameters.iter()), |row| {
        row.get::<_, i64>(0)
    })?;
    let total_count =
        usize::try_from(total_count).map_err(|_| LibraryError::IntegerOutOfRange("album count"))?;
    let offset = offset.min(total_count);

    let sql = format!(
        "{normalized_cte}
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
         GROUP BY album_owner, album_title {having}
         ORDER BY {order_by}
         LIMIT ? OFFSET ?"
    );
    parameters.push(Value::Integer(usize_to_i64(limit, "album page limit")?));
    parameters.push(Value::Integer(usize_to_i64(offset, "album page offset")?));
    let mut statement = conn.prepare(&sql)?;
    let albums = statement
        .query_map(params_from_iter(parameters.iter()), album_from_row)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(AlbumPage {
        albums,
        total_count,
    })
}

/// Every sort ends with the exact (album_title, album_owner) pair — the GROUP
/// BY key — so the order is total and LIMIT/OFFSET paging can never duplicate
/// or drop a tied group between queries.
pub(super) fn album_order_by(sort_order: AlbumSortOrder) -> String {
    let display = match sort_order {
        AlbumSortOrder::Title => "album_title COLLATE NOCASE, album_owner COLLATE NOCASE",
        AlbumSortOrder::Artist => "album_owner COLLATE NOCASE, album_title COLLATE NOCASE",
        AlbumSortOrder::DateAdded => "latest_added_at_ms DESC, album_title COLLATE NOCASE",
        AlbumSortOrder::ReleaseYear => {
            "album_year IS NULL, album_year DESC, album_title COLLATE NOCASE"
        }
        AlbumSortOrder::Duration => "total_duration_ms DESC, album_title COLLATE NOCASE",
    };
    format!("{display}, album_title, album_owner")
}

pub fn album_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Album> {
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
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use tempfile::tempdir;

    use crate::backend::library::{
        AlbumPage, AlbumQueryFilter, AlbumSortOrder, LibraryStore, UNKNOWN_ALBUM, UNKNOWN_ARTIST,
        store::{
            testing::{insert_track, test_file, test_metadata},
            tracks::{set_track_cover, upsert_track},
        },
    };

    #[test]
    fn album_pages_apply_filters_counts_and_offsets() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        let mut add = |name: &str,
                       album: &str,
                       artist: &str,
                       genre: &str,
                       bit_depth: u8,
                       sample_rate_hz: u32,
                       added_at_ms: i64| {
            let file = test_file(&root, name, 10, 100);
            let mut metadata = test_metadata(name, artist, Some(album), Some(artist));
            metadata.genre = Some(genre.to_string());
            metadata.bit_depth = Some(bit_depth);
            metadata.sample_rate_hz = Some(sample_rate_hz);
            let transaction = store.connection.transaction().unwrap();
            upsert_track(&transaction, root.id, &file, &metadata, added_at_ms).unwrap();
            transaction.commit().unwrap();
        };
        add("a.wav", "Alpha", "Artist A", "Jazz", 24, 96_000, 1_000);
        add("b.wav", "Beta", "Artist B", "Rock", 16, 44_100, 2_000);
        add("g.wav", "Gamma", "Artist C", "Jazz", 16, 44_100, 3_000);

        let titles = |page: &AlbumPage| {
            page.albums
                .iter()
                .map(|album| album.title.clone())
                .collect::<Vec<_>>()
        };

        let first = store
            .album_page(
                AlbumSortOrder::DateAdded,
                &AlbumQueryFilter::All,
                None,
                2,
                0,
            )
            .unwrap();
        assert_eq!(first.total_count, 3);
        assert_eq!(titles(&first), ["Gamma", "Beta"]);

        let second = store
            .album_page(
                AlbumSortOrder::DateAdded,
                &AlbumQueryFilter::All,
                None,
                2,
                2,
            )
            .unwrap();
        assert_eq!(titles(&second), ["Alpha"]);

        let beyond = store
            .album_page(
                AlbumSortOrder::DateAdded,
                &AlbumQueryFilter::All,
                None,
                2,
                10,
            )
            .unwrap();
        assert_eq!(beyond.total_count, 3);
        assert!(beyond.albums.is_empty());

        let hi_res = store
            .album_page(
                AlbumSortOrder::DateAdded,
                &AlbumQueryFilter::HiRes,
                None,
                10,
                0,
            )
            .unwrap();
        assert_eq!(hi_res.total_count, 1);
        assert_eq!(titles(&hi_res), ["Alpha"]);

        let recent = store
            .album_page(
                AlbumSortOrder::DateAdded,
                &AlbumQueryFilter::AddedSince(2_000),
                None,
                10,
                0,
            )
            .unwrap();
        assert_eq!(recent.total_count, 2);
        assert_eq!(titles(&recent), ["Gamma", "Beta"]);

        let genre = store
            .album_page(
                AlbumSortOrder::DateAdded,
                &AlbumQueryFilter::Genre("jazz".to_string()),
                None,
                10,
                0,
            )
            .unwrap();
        assert_eq!(genre.total_count, 2);
        assert_eq!(titles(&genre), ["Gamma", "Alpha"]);
    }

    #[test]
    fn album_pages_keep_a_total_order_for_tied_sort_keys() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        // Three albums tied on every DateAdded/Duration sort key.
        for (index, album) in ["Tie C", "Tie A", "Tie B"].into_iter().enumerate() {
            let file = test_file(&root, &format!("{album}.wav"), 10, 100);
            let metadata = test_metadata("Track", "Artist", Some(album), Some("Artist"));
            let transaction = store.connection.transaction().unwrap();
            upsert_track(&transaction, root.id, &file, &metadata, 1_000).unwrap();
            transaction.commit().unwrap();
            let _ = index;
        }

        let mut seen = Vec::new();
        for offset in [0, 1, 2] {
            let page = store
                .album_page(
                    AlbumSortOrder::DateAdded,
                    &AlbumQueryFilter::All,
                    None,
                    1,
                    offset,
                )
                .unwrap();
            assert_eq!(page.total_count, 3);
            assert_eq!(page.albums.len(), 1);
            seen.push(page.albums[0].title.clone());
        }
        seen.sort();
        assert_eq!(
            seen,
            ["Tie A", "Tie B", "Tie C"],
            "paging tied albums must neither duplicate nor drop a group"
        );
    }

    #[test]
    fn album_artist_filter_composes_with_every_album_filter() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        let mut add = |file: &str,
                       album: &str,
                       album_artist: &str,
                       genre: &str,
                       bit_depth: u8,
                       sample_rate_hz: u32,
                       added_at_ms: i64| {
            let mut metadata = test_metadata(
                file,
                "Track Artist feat. Guest",
                Some(album),
                Some(album_artist),
            );
            metadata.genre = Some(genre.to_string());
            metadata.bit_depth = Some(bit_depth);
            metadata.sample_rate_hz = Some(sample_rate_hz);
            let file = test_file(&root, file, added_at_ms, 10);
            let transaction = store.connection.transaction().unwrap();
            upsert_track(&transaction, root.id, &file, &metadata, added_at_ms).unwrap();
            transaction.commit().unwrap();
        };
        add("alpha.wav", "Alpha", "Artist A", "Jazz", 24, 96_000, 3_000);
        add("beta.wav", "Beta", "Artist A", "Rock", 16, 44_100, 1_000);
        add("gamma.wav", "Gamma", "Artist B", "Jazz", 24, 96_000, 3_000);

        let titles = |filter: AlbumQueryFilter| {
            store
                .album_page(AlbumSortOrder::Title, &filter, Some("Artist A"), 10, 0)
                .unwrap()
                .albums
                .into_iter()
                .map(|album| album.title)
                .collect::<Vec<_>>()
        };

        assert_eq!(titles(AlbumQueryFilter::All), ["Alpha", "Beta"]);
        assert_eq!(titles(AlbumQueryFilter::HiRes), ["Alpha"]);
        assert_eq!(
            titles(AlbumQueryFilter::Genre("jazz".to_string())),
            ["Alpha"]
        );
        assert_eq!(titles(AlbumQueryFilter::AddedSince(2_000)), ["Alpha"]);
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
}
