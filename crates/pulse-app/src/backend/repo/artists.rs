use std::{collections::BTreeSet, path::PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use super::super::{AlbumSortOrder, Artist, LibraryError, TrackId, UNKNOWN_ALBUM};
use super::{
    EFFECTIVE_ALBUM_ARTIST_SQL, LibraryStore, LibraryTransaction, album_title_sql, albums,
    select_list,
};

const COLUMNS: &[&str] = &[
    "id",
    "name",
    "name_key",
    "album_count",
    "track_count",
    "total_duration_ms",
    "earliest_added_ms",
    "cover_art_path",
    "display_name",
    "hidden",
    "mbid",
    "photo_path",
    "photo_source",
    "enriched_at_ms",
    "created_at_ms",
    "updated_at_ms",
];

struct ArtistRow {
    id: i64,
    name: String,
    name_key: String,
    album_count: i64,
    track_count: i64,
    total_duration_ms: i64,
    earliest_added_ms: i64,
    cover_art_path: Option<String>,
    display_name: Option<String>,
    hidden: Option<bool>,
    mbid: Option<String>,
    photo_path: Option<String>,
    photo_source: Option<String>,
    enriched_at_ms: Option<i64>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl ArtistRow {
    fn into_artist(self, earliest_added_year: Option<i64>) -> Artist {
        Artist {
            id: self.id,
            name: self.name,
            name_key: self.name_key,
            album_count: self.album_count as u64,
            track_count: self.track_count as u64,
            total_duration_ms: self.total_duration_ms as u64,
            earliest_added_ms: self.earliest_added_ms,
            earliest_added_year: earliest_added_year.map(|year| year as u32),
            cover_art_path: self.cover_art_path.map(PathBuf::from),
            display_name: self.display_name,
            hidden: self.hidden,
            mbid: self.mbid,
            photo_path: self.photo_path.map(PathBuf::from),
            photo_source: self.photo_source,
            enriched_at_ms: self.enriched_at_ms,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        }
    }
}

fn derived_artist_upsert_sql(scoped: bool) -> String {
    let album_order = albums::album_order_by(AlbumSortOrder::DateAdded);
    let album_title = album_title_sql("?1");
    let artist_filter = if scoped {
        format!("WHERE {EFFECTIVE_ALBUM_ARTIST_SQL} = ?3")
    } else {
        String::new()
    };
    format!(
        "WITH normalized AS (
             SELECT id, {EFFECTIVE_ALBUM_ARTIST_SQL} AS album_owner,
                    {album_title} AS album_title,
                    year, duration_ms, sample_rate_hz, bit_depth, cover_art_path, added_at_ms
             FROM tracks
             {artist_filter}
         ), grouped_albums AS (
             SELECT album_owner, album_title, MIN(year) AS album_year,
                    COUNT(*) AS track_count,
                    COALESCE(SUM(duration_ms), 0) AS total_duration_ms,
                    MAX(sample_rate_hz) AS max_sample_rate_hz,
                    MAX(bit_depth) AS max_bit_depth,
                    substr(MIN(
                        CASE WHEN cover_art_path IS NOT NULL
                             THEN printf('%020lld%s', id, cover_art_path)
                        END
                    ), 21) AS cover_art_path,
                    MAX(added_at_ms) AS latest_added_at_ms,
                    MIN(added_at_ms) AS earliest_added_at_ms
             FROM normalized
             GROUP BY album_owner, album_title
         ), ranked_albums AS (
             SELECT *, ROW_NUMBER() OVER (
                 PARTITION BY album_owner ORDER BY {album_order}
             ) AS album_position
             FROM grouped_albums
         )
         INSERT INTO artists (
             name, name_key, album_count, track_count, total_duration_ms,
             earliest_added_ms, cover_art_path, created_at_ms, updated_at_ms
         )
         SELECT album_owner, album_owner, COUNT(*) AS album_count,
                SUM(track_count) AS track_count,
                SUM(total_duration_ms) AS total_duration_ms,
                MIN(earliest_added_at_ms) AS earliest_added_ms,
                substr(MIN(
                    CASE WHEN cover_art_path IS NOT NULL
                         THEN printf('%020lld%s', album_position, cover_art_path)
                    END
                ), 21) AS cover_art_path,
                ?2, ?2
         FROM ranked_albums
         WHERE true
         GROUP BY album_owner
         ON CONFLICT(name_key) DO UPDATE SET
             name = excluded.name,
             album_count = excluded.album_count,
             track_count = excluded.track_count,
             total_duration_ms = excluded.total_duration_ms,
             earliest_added_ms = excluded.earliest_added_ms,
             cover_art_path = excluded.cover_art_path,
             updated_at_ms = excluded.updated_at_ms"
    )
}

fn upsert_from_tracks(
    conn: &Connection,
    refreshed_at_ms: i64,
    name_key: Option<&str>,
) -> Result<(), LibraryError> {
    match name_key {
        Some(name_key) => conn.execute(
            &derived_artist_upsert_sql(true),
            params![UNKNOWN_ALBUM, refreshed_at_ms, name_key],
        )?,
        None => conn.execute(
            &derived_artist_upsert_sql(false),
            params![UNKNOWN_ALBUM, refreshed_at_ms],
        )?,
    };
    Ok(())
}

pub(super) fn backfill(
    transaction: &LibraryTransaction<'_>,
    refreshed_at_ms: i64,
) -> Result<(), LibraryError> {
    let transaction = &transaction.inner;
    upsert_from_tracks(transaction, refreshed_at_ms, None)
}

pub(super) fn refresh(
    transaction: &LibraryTransaction<'_>,
    refreshed_at_ms: i64,
) -> Result<(), LibraryError> {
    let transaction = &transaction.inner;
    upsert_from_tracks(transaction, refreshed_at_ms, None)?;
    transaction.execute(
        &format!(
            "DELETE FROM artists
             WHERE NOT EXISTS (
                 SELECT 1 FROM tracks
                 WHERE {EFFECTIVE_ALBUM_ARTIST_SQL} = artists.name_key
             )"
        ),
        [],
    )?;
    Ok(())
}

pub fn refresh_keys(
    transaction: &LibraryTransaction<'_>,
    name_keys: &[String],
    refreshed_at_ms: i64,
) -> Result<(), LibraryError> {
    let transaction = &transaction.inner;
    for name_key in name_keys.iter().collect::<BTreeSet<_>>() {
        upsert_from_tracks(transaction, refreshed_at_ms, Some(name_key))?;
        transaction.execute(
            &format!(
                "DELETE FROM artists
                 WHERE name_key = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM tracks
                       WHERE {EFFECTIVE_ALBUM_ARTIST_SQL} = ?1
                   )"
            ),
            [name_key],
        )?;
    }
    Ok(())
}

pub fn name_key_for_track(
    transaction: &LibraryTransaction<'_>,
    track_id: TrackId,
) -> Result<Option<String>, LibraryError> {
    let conn = &transaction.inner;
    conn.query_row(
        &format!(
            "SELECT {EFFECTIVE_ALBUM_ARTIST_SQL}
             FROM tracks
             WHERE id = ?1"
        ),
        [track_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub fn index(store: &LibraryStore) -> Result<Vec<Artist>, LibraryError> {
    let conn = &store.connection;
    let columns = select_list(COLUMNS);
    let sql = format!(
        "SELECT {columns},
                CAST(strftime(
                    '%Y', earliest_added_ms / 1000, 'unixepoch', 'localtime'
                ) AS INTEGER) AS earliest_added_year
         FROM artists
         ORDER BY name COLLATE NOCASE, name"
    );
    let mut statement = conn.prepare(&sql)?;
    let artists = statement
        .query_map([], |row| {
            let artist = ArtistRow {
                id: row.get(0)?,
                name: row.get(1)?,
                name_key: row.get(2)?,
                album_count: row.get(3)?,
                track_count: row.get(4)?,
                total_duration_ms: row.get(5)?,
                earliest_added_ms: row.get(6)?,
                cover_art_path: row.get(7)?,
                display_name: row.get(8)?,
                hidden: row.get(9)?,
                mbid: row.get(10)?,
                photo_path: row.get(11)?,
                photo_source: row.get(12)?,
                enriched_at_ms: row.get(13)?,
                created_at_ms: row.get(14)?,
                updated_at_ms: row.get(15)?,
            };
            Ok(artist.into_artist(row.get(16)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(artists)
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use tempfile::tempdir;

    use crate::backend::{
        AlbumQueryFilter, AlbumSortOrder, LibraryStore, UNKNOWN_ALBUM,
        repo::{
            schema::EFFECTIVE_ARTIST_INDEX_NAME,
            testing::{insert_track, set_cover, test_file, test_metadata},
        },
    };

    fn refresh(store: &mut LibraryStore, refreshed_at_ms: i64) {
        let transaction = store.transaction().unwrap();
        super::refresh(&transaction, refreshed_at_ms).unwrap();
        transaction.commit().unwrap();
    }

    #[test]
    fn scoped_artist_refresh_uses_the_effective_artist_index() {
        let store = LibraryStore::open_in_memory().unwrap();
        let explain = format!(
            "EXPLAIN QUERY PLAN {}",
            super::derived_artist_upsert_sql(true)
        );
        let mut statement = store.connection.prepare(&explain).unwrap();
        let plan = statement
            .query_map(params![UNKNOWN_ALBUM, 100, "Artist"], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(
            plan.iter().any(|detail| {
                detail.contains("SEARCH") && detail.contains(EFFECTIVE_ARTIST_INDEX_NAME)
            }),
            "scoped refresh must seek the expression index: {plan:?}"
        );
    }

    #[test]
    fn artist_index_uses_effective_album_artist_identity_without_splitting_names() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();

        for (file, title, artist, album, album_artist) in [
            ("one.wav", "One", "王菲", "天空", Some("   ")),
            ("two.wav", "Two", "王菲", "天空 (Remastered)", None),
            (
                "three.wav",
                "Three",
                "Crosby, Stills & Nash",
                "CSN",
                Some("Crosby, Stills & Nash"),
            ),
        ] {
            insert_track(
                &mut store,
                &root,
                &test_file(&root, file, 1, 10),
                &test_metadata(title, artist, Some(album), album_artist),
            );
        }
        store
            .connection
            .execute("UPDATE tracks SET added_at_ms = 1700000000000", [])
            .unwrap();
        refresh(&mut store, 1_700_000_000_000);

        let artists = crate::backend::repo::artists::index(&store).unwrap();
        assert_eq!(artists.len(), 2);
        assert_eq!(artists[0].name, "Crosby, Stills & Nash");
        assert_eq!(artists[0].album_count, 1);
        assert_eq!(artists[1].name, "王菲");
        assert_eq!(artists[1].album_count, 2);
        assert_eq!(artists[1].track_count, 2);
        assert_eq!(artists[1].total_duration_ms, 2_000);
        assert_eq!(artists[1].earliest_added_year, Some(2023));
    }

    #[test]
    fn artist_index_ignores_feat_track_artists_and_groups_reissues() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();

        for (index, (artist, album)) in [
            ("Lead feat. Guest One", "Original"),
            ("Lead feat. Guest Two", "Original"),
            ("Guest Three feat. Lead", "Deluxe Reissue"),
        ]
        .into_iter()
        .enumerate()
        {
            insert_track(
                &mut store,
                &root,
                &test_file(&root, &format!("{index}.wav"), index as i64, 10),
                &test_metadata(&format!("Track {index}"), artist, Some(album), Some("Lead")),
            );
        }

        let artists = crate::backend::repo::artists::index(&store).unwrap();
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].name, "Lead");
        assert_eq!(artists[0].album_count, 2);
        assert_eq!(artists[0].track_count, 3);
    }

    #[test]
    fn artist_index_and_album_filter_share_binary_identity() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();

        for (file, album, album_artist) in [
            ("muse-one.wav", "Muse One", "Muse"),
            ("muse-two.wav", "Muse Two", "Muse"),
            ("upper.wav", "Upper", "MUSE"),
        ] {
            insert_track(
                &mut store,
                &root,
                &test_file(&root, file, 1, 10),
                &test_metadata(file, "Track Artist", Some(album), Some(album_artist)),
            );
        }

        let artists = crate::backend::repo::artists::index(&store).unwrap();
        let muse = artists.iter().find(|artist| artist.name == "Muse").unwrap();
        let upper = artists.iter().find(|artist| artist.name == "MUSE").unwrap();
        assert_eq!((muse.album_count, muse.track_count), (2, 2));
        assert_eq!((upper.album_count, upper.track_count), (1, 1));

        let muse_albums = crate::backend::repo::albums::page(
            &store,
            AlbumSortOrder::Title,
            &AlbumQueryFilter::All,
            Some("Muse"),
            10,
            0,
        )
        .unwrap();
        let upper_albums = crate::backend::repo::albums::page(
            &store,
            AlbumSortOrder::Title,
            &AlbumQueryFilter::All,
            Some("MUSE"),
            10,
            0,
        )
        .unwrap();
        assert_eq!(muse_albums.total_count, muse.album_count as usize);
        assert_eq!(upper_albums.total_count, upper.album_count as usize);
        assert_eq!(
            muse_albums
                .albums
                .into_iter()
                .map(|album| album.title)
                .collect::<Vec<_>>(),
            ["Muse One", "Muse Two"]
        );
        assert_eq!(upper_albums.albums[0].title, "Upper");
    }

    #[test]
    fn artist_avatar_uses_the_first_available_cover_in_album_page_order() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
        let older = insert_track(
            &mut store,
            &root,
            &test_file(&root, "older.wav", 1, 10),
            &test_metadata("Older", "Artist", Some("Older Album"), Some("Artist")),
        );
        let newer = insert_track(
            &mut store,
            &root,
            &test_file(&root, "newer.wav", 2, 10),
            &test_metadata("Newer", "Artist", Some("Newer Album"), Some("Artist")),
        );
        store
            .connection
            .execute(
                "UPDATE tracks SET added_at_ms = CASE id WHEN ?1 THEN 100 WHEN ?2 THEN 200 END",
                rusqlite::params![older, newer],
            )
            .unwrap();
        let older_cover = temp.path().join("older.cover");
        set_cover(&mut store, older, &older_cover);

        let artists = crate::backend::repo::artists::index(&store).unwrap();
        assert_eq!(artists[0].cover_art_path.as_ref(), Some(&older_cover));
    }

    #[test]
    fn refresh_preserves_artist_identity_and_enrichment_seams() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
        insert_track(
            &mut store,
            &root,
            &test_file(&root, "first.wav", 1, 10),
            &test_metadata("First", "Artist", Some("First Album"), Some("Artist")),
        );
        let original = crate::backend::repo::artists::index(&store)
            .unwrap()
            .remove(0);
        store
            .connection
            .execute(
                "UPDATE artists
                 SET display_name = 'Display Artist', hidden = 1, mbid = 'mbid',
                     photo_path = '/photo.jpg', photo_source = 'local', enriched_at_ms = 500
                 WHERE name_key = 'Artist'",
                [],
            )
            .unwrap();

        insert_track(
            &mut store,
            &root,
            &test_file(&root, "second.wav", 2, 10),
            &test_metadata("Second", "Artist", Some("Second Album"), Some("Artist")),
        );

        let refreshed = crate::backend::repo::artists::index(&store)
            .unwrap()
            .remove(0);
        assert_eq!(refreshed.id, original.id);
        assert_eq!(refreshed.created_at_ms, original.created_at_ms);
        assert_eq!((refreshed.album_count, refreshed.track_count), (2, 2));
        assert_eq!(refreshed.display_name.as_deref(), Some("Display Artist"));
        assert_eq!(refreshed.hidden, Some(true));
        assert_eq!(refreshed.mbid.as_deref(), Some("mbid"));
        assert_eq!(
            refreshed.photo_path.as_deref(),
            Some(std::path::Path::new("/photo.jpg"))
        );
        assert_eq!(refreshed.photo_source.as_deref(), Some("local"));
        assert_eq!(refreshed.enriched_at_ms, Some(500));
    }

    #[test]
    fn deleting_tracks_prunes_empty_artists_and_recounts_survivors() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
        let first = insert_track(
            &mut store,
            &root,
            &test_file(&root, "first.wav", 1, 10),
            &test_metadata("First", "Artist", Some("First Album"), Some("Artist")),
        );
        insert_track(
            &mut store,
            &root,
            &test_file(&root, "second.wav", 2, 10),
            &test_metadata("Second", "Artist", Some("Second Album"), Some("Artist")),
        );
        let emptied = insert_track(
            &mut store,
            &root,
            &test_file(&root, "empty.wav", 3, 10),
            &test_metadata("Empty", "Gone", Some("Only Album"), Some("Gone")),
        );

        crate::backend::repo::tracks::delete_tracks(&mut store, &[first, emptied]).unwrap();

        let artists = crate::backend::repo::artists::index(&store).unwrap();
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].name_key, "Artist");
        assert_eq!((artists[0].album_count, artists[0].track_count), (1, 1));
    }

    #[test]
    fn artist_refresh_failure_rolls_back_the_track_delete() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
        let track = insert_track(
            &mut store,
            &root,
            &test_file(&root, "track.wav", 1, 10),
            &test_metadata("Track", "Artist", Some("Album"), Some("Artist")),
        );
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_artist_prune
                 BEFORE DELETE ON artists
                 BEGIN SELECT RAISE(ABORT, 'refresh failed'); END;",
            )
            .unwrap();

        assert!(crate::backend::repo::tracks::delete_tracks(&mut store, &[track]).is_err());
        assert_eq!(
            crate::backend::repo::tracks::for_root(&store, root.id)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            crate::backend::repo::artists::index(&store).unwrap().len(),
            1
        );
    }

    #[test]
    fn artist_index_reads_only_the_materialized_artist_rows() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
        insert_track(
            &mut store,
            &root,
            &test_file(&root, "track.wav", 1, 10),
            &test_metadata("Track", "Before", Some("Album"), Some("Before")),
        );
        store
            .connection
            .execute("UPDATE tracks SET album_artist = 'After'", [])
            .unwrap();

        assert_eq!(
            crate::backend::repo::artists::index(&store).unwrap()[0].name_key,
            "Before"
        );
        refresh(&mut store, 200);
        assert_eq!(
            crate::backend::repo::artists::index(&store).unwrap()[0].name_key,
            "After"
        );
    }
}
