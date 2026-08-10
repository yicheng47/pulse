use rusqlite::{Connection, params};

use super::super::{LibraryError, LibrarySearchResults, UNKNOWN_ALBUM, UNKNOWN_ARTIST};
use super::{albums, playlists, tracks};

pub fn search(conn: &Connection, query: &str) -> Result<LibrarySearchResults, LibraryError> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(LibrarySearchResults::default());
    }
    let pattern = like_pattern(query);

    let mut albums_statement = conn.prepare(
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
            albums::album_from_row,
        )?
        .collect::<Result<Vec<_>, _>>()?;

    let tracks_sql = format!(
        "SELECT {}
         FROM tracks
         WHERE COALESCE(title, '') LIKE ?1 ESCAPE '\\'
            OR COALESCE(artist, '') LIKE ?1 ESCAPE '\\'
            OR COALESCE(album, '') LIKE ?1 ESCAPE '\\'
            OR COALESCE(album_artist, '') LIKE ?1 ESCAPE '\\'
         ORDER BY COALESCE(NULLIF(trim(title), ''), path) COLLATE NOCASE, path_key
         LIMIT 5",
        tracks::TRACK_COLUMNS
    );
    let mut tracks_statement = conn.prepare(&tracks_sql)?;
    let matching_tracks = tracks_statement
        .query_map([&pattern], tracks::track_from_row)?
        .collect::<Result<Vec<_>, _>>()?;

    let playlists_sql = format!(
        "{}
         WHERE p.name LIKE ?1 ESCAPE '\\'
         GROUP BY p.id
         ORDER BY p.name COLLATE NOCASE, p.id
         LIMIT 3",
        playlists::PLAYLIST_SUMMARY_SELECT
    );
    let mut playlists_statement = conn.prepare(&playlists_sql)?;
    let matching_playlists = playlists_statement
        .query_map([&pattern], playlists::playlist_summary_from_row)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(LibrarySearchResults {
        albums,
        tracks: matching_tracks,
        playlists: matching_playlists,
    })
}

fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::library::{
        LibrarySearchResults, LibraryStore,
        store::testing::{insert_track, test_file, test_metadata},
    };

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
}
