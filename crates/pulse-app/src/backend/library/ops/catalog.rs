use super::super::{
    AlbumPage, AlbumQueryFilter, AlbumSortOrder, Artist, ArtistDetail, LibraryError,
    LibrarySearchResults, LibraryStore, LibrarySummary, Track, TrackPage, TrackQueryFilter,
    TrackSortOrder,
};

pub fn album_page(
    store: &LibraryStore,
    sort_order: AlbumSortOrder,
    filter: &AlbumQueryFilter,
    artist_filter: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<AlbumPage, LibraryError> {
    store.album_page(sort_order, filter, artist_filter, limit, offset)
}

pub fn track_page(
    store: &LibraryStore,
    sort_order: TrackSortOrder,
    filter: &TrackQueryFilter,
    artist_filter: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<TrackPage, LibraryError> {
    store.track_page(sort_order, filter, artist_filter, limit, offset)
}

pub fn matching_tracks(
    store: &LibraryStore,
    sort_order: TrackSortOrder,
    filter: &TrackQueryFilter,
    artist_filter: Option<&str>,
) -> Result<Vec<Track>, LibraryError> {
    store.matching_tracks(sort_order, filter, artist_filter)
}

pub fn album_tracks(
    store: &LibraryStore,
    artist: &str,
    title: &str,
) -> Result<Vec<Track>, LibraryError> {
    store.tracks_for_album(artist, title)
}

pub fn artist_index(store: &LibraryStore) -> Result<Vec<Artist>, LibraryError> {
    store.artist_index()
}

pub fn artist_detail(
    store: &LibraryStore,
    artist: Artist,
    album_sort: AlbumSortOrder,
) -> Result<ArtistDetail, LibraryError> {
    let album_count = usize::try_from(artist.album_count)
        .map_err(|_| LibraryError::IntegerOutOfRange("artist album count"))?;
    let albums = album_page(
        store,
        album_sort,
        &AlbumQueryFilter::All,
        Some(&artist.name),
        album_count.max(1),
        0,
    )?
    .albums;
    let mut tracks = Vec::with_capacity(artist.track_count as usize);
    for album in &albums {
        tracks.extend(album_tracks(store, &album.artist, &album.title)?);
    }
    Ok(ArtistDetail {
        artist,
        albums,
        tracks,
    })
}

pub fn artist_filter_counts(store: &LibraryStore) -> Result<Vec<(String, u64)>, LibraryError> {
    store.artists()
}

pub fn genre_album_counts(store: &LibraryStore) -> Result<Vec<(String, u64)>, LibraryError> {
    store.genre_album_counts()
}

pub fn summary(store: &LibraryStore) -> Result<LibrarySummary, LibraryError> {
    store.catalog_summary()
}

pub fn search(store: &LibraryStore, query: &str) -> Result<LibrarySearchResults, LibraryError> {
    store.search(query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::library::{
        LibraryStore,
        repo::testing::{insert_track, test_file, test_metadata},
    };
    use tempfile::tempdir;

    #[test]
    fn catalog_use_cases_return_filtered_pages_details_and_search_results() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();

        let standard = test_metadata("Standard", "Artist A", Some("Standard Album"), None);
        insert_track(
            &mut store,
            &root,
            &test_file(&root, "standard.wav", 1, 10),
            &standard,
        );
        let mut hi_res = test_metadata("Hi Res", "Artist A", Some("Hi Album"), None);
        hi_res.genre = Some("Jazz".to_string());
        hi_res.bit_depth = Some(24);
        hi_res.sample_rate_hz = Some(96_000);
        insert_track(
            &mut store,
            &root,
            &test_file(&root, "hi-res.wav", 2, 10),
            &hi_res,
        );
        let mut other = test_metadata("Other", "Artist B", Some("Other Album"), None);
        other.genre = Some("Classical".to_string());
        insert_track(
            &mut store,
            &root,
            &test_file(&root, "other.wav", 3, 10),
            &other,
        );

        assert_eq!(
            album_page(
                &store,
                AlbumSortOrder::Title,
                &AlbumQueryFilter::All,
                None,
                20,
                0,
            )
            .unwrap()
            .total_count,
            3
        );
        assert_eq!(
            album_page(
                &store,
                AlbumSortOrder::Title,
                &AlbumQueryFilter::HiRes,
                None,
                20,
                0,
            )
            .unwrap()
            .albums
            .iter()
            .map(|album| album.title.as_str())
            .collect::<Vec<_>>(),
            ["Hi Album"]
        );
        assert!(
            album_page(
                &store,
                AlbumSortOrder::Title,
                &AlbumQueryFilter::AddedSince(101),
                None,
                20,
                0,
            )
            .unwrap()
            .albums
            .is_empty()
        );
        assert_eq!(
            album_page(
                &store,
                AlbumSortOrder::Title,
                &AlbumQueryFilter::Genre("Jazz".to_string()),
                None,
                20,
                0,
            )
            .unwrap()
            .albums[0]
                .title,
            "Hi Album"
        );
        assert_eq!(
            album_page(
                &store,
                AlbumSortOrder::Title,
                &AlbumQueryFilter::All,
                Some("Artist A"),
                20,
                0,
            )
            .unwrap()
            .total_count,
            2
        );

        let tracks = track_page(
            &store,
            TrackSortOrder::Title,
            &TrackQueryFilter::All,
            Some("Artist A"),
            20,
            0,
        )
        .unwrap();
        assert_eq!(tracks.total_count, 2);
        assert_eq!(
            matching_tracks(
                &store,
                TrackSortOrder::Title,
                &TrackQueryFilter::Genre("Jazz".to_string()),
                None,
            )
            .unwrap()
            .len(),
            1
        );
        assert_eq!(
            album_tracks(&store, "Artist A", "Hi Album").unwrap().len(),
            1
        );

        let artists = artist_index(&store).unwrap();
        let artist = artists
            .iter()
            .find(|artist| artist.name == "Artist A")
            .unwrap()
            .clone();
        let detail = artist_detail(&store, artist, AlbumSortOrder::Title).unwrap();
        assert_eq!(detail.albums.len(), 2);
        assert_eq!(detail.tracks.len(), 2);
        assert_eq!(artist_filter_counts(&store).unwrap().len(), 2);
        assert!(
            genre_album_counts(&store)
                .unwrap()
                .contains(&("Jazz".to_string(), 1))
        );
        assert_eq!(summary(&store).unwrap().album_count, 3);

        let results = search(&store, "Hi Album").unwrap();
        assert_eq!(results.albums.len(), 1);
        assert_eq!(results.tracks.len(), 1);
    }
}
