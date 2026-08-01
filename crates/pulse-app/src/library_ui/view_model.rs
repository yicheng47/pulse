use std::path::Path;

use crate::library::{
    Album, LibrarySearchResults, ScanHistoryEntry, ScanOutcome, ScanProgress, StorageRoot, Track,
    TrackQueryFilter, UNKNOWN_ALBUM, UNKNOWN_ARTIST,
};

const RECENTLY_ADDED_WINDOW_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchSelection {
    Album(usize),
    Track(usize),
    Playlist(usize),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchViewModel {
    pub results: LibrarySearchResults,
    selected_index: Option<usize>,
}

impl SearchViewModel {
    pub fn set_results(&mut self, results: LibrarySearchResults) {
        self.results = results;
        self.selected_index = (self.result_count() > 0).then_some(0);
    }

    pub fn clear(&mut self) {
        self.results = LibrarySearchResults::default();
        self.selected_index = None;
    }

    pub fn result_count(&self) -> usize {
        self.results.albums.len() + self.results.tracks.len() + self.results.playlists.len()
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    pub fn selected(&self) -> Option<SearchSelection> {
        let index = self.selected_index?;
        if index < self.results.albums.len() {
            return Some(SearchSelection::Album(index));
        }
        let index = index - self.results.albums.len();
        if index < self.results.tracks.len() {
            return Some(SearchSelection::Track(index));
        }
        let index = index - self.results.tracks.len();
        (index < self.results.playlists.len()).then_some(SearchSelection::Playlist(index))
    }

    pub fn move_next(&mut self) {
        let count = self.result_count();
        if count == 0 {
            self.selected_index = None;
            return;
        }
        self.selected_index = Some(self.selected_index.map_or(0, |index| (index + 1) % count));
    }

    pub fn move_previous(&mut self) {
        let count = self.result_count();
        if count == 0 {
            self.selected_index = None;
            return;
        }
        self.selected_index = Some(match self.selected_index {
            Some(0) | None => count - 1,
            Some(index) => index - 1,
        });
    }

    pub fn album_index(&self, index: usize) -> usize {
        index
    }

    pub fn track_index(&self, index: usize) -> usize {
        self.results.albums.len() + index
    }

    pub fn playlist_index(&self, index: usize) -> usize {
        self.results.albums.len() + self.results.tracks.len() + index
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FilterChip {
    All,
    HiRes,
    RecentlyAdded,
    Genre(String),
}

impl FilterChip {
    pub fn label(&self) -> &str {
        match self {
            Self::All => "All Albums",
            Self::HiRes => "Hi-Res",
            Self::RecentlyAdded => "Recently Added",
            Self::Genre(genre) => genre,
        }
    }

    pub fn track_query_filter(&self, now_ms: i64) -> TrackQueryFilter {
        match self {
            Self::All => TrackQueryFilter::All,
            Self::HiRes => TrackQueryFilter::HiRes,
            Self::RecentlyAdded => {
                TrackQueryFilter::AddedSince(now_ms.saturating_sub(RECENTLY_ADDED_WINDOW_MS))
            }
            Self::Genre(genre) => TrackQueryFilter::Genre(genre.clone()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaginationItem {
    Page(usize),
    Ellipsis,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pagination {
    page: usize,
    page_size: usize,
    total_items: usize,
}

impl Pagination {
    pub fn new(page_size: usize) -> Self {
        assert!(page_size > 0, "page size must be positive");
        Self {
            page: 0,
            page_size,
            total_items: 0,
        }
    }

    pub fn current_page(&self) -> usize {
        self.page
    }

    pub fn page_size(&self) -> usize {
        self.page_size
    }

    pub fn total_items(&self) -> usize {
        self.total_items
    }

    pub fn page_count(&self) -> usize {
        self.total_items.div_ceil(self.page_size)
    }

    pub fn offset(&self) -> usize {
        self.page * self.page_size
    }

    pub fn range(&self) -> Option<(usize, usize)> {
        if self.total_items == 0 {
            return None;
        }
        let start = self.offset() + 1;
        Some((start, (start + self.page_size - 1).min(self.total_items)))
    }

    pub fn can_previous(&self) -> bool {
        self.page > 0
    }

    pub fn can_next(&self) -> bool {
        self.page + 1 < self.page_count()
    }

    pub fn reset(&mut self) {
        self.page = 0;
    }

    pub fn set_total_items(&mut self, total_items: usize) {
        self.total_items = total_items;
        self.page = self.page.min(self.page_count().saturating_sub(1));
    }

    pub fn set_page(&mut self, page: usize) -> bool {
        if page >= self.page_count() || page == self.page {
            return false;
        }
        self.page = page;
        true
    }

    pub fn items(&self) -> Vec<PaginationItem> {
        let page_count = self.page_count();
        if page_count <= 7 {
            return (0..page_count).map(PaginationItem::Page).collect();
        }
        if self.page <= 3 {
            return (0..4)
                .map(PaginationItem::Page)
                .chain([
                    PaginationItem::Ellipsis,
                    PaginationItem::Page(page_count - 1),
                ])
                .collect();
        }
        if self.page + 4 >= page_count {
            return [PaginationItem::Page(0), PaginationItem::Ellipsis]
                .into_iter()
                .chain((page_count - 4..page_count).map(PaginationItem::Page))
                .collect();
        }
        [PaginationItem::Page(0), PaginationItem::Ellipsis]
            .into_iter()
            .chain((self.page - 1..=self.page + 1).map(PaginationItem::Page))
            .chain([
                PaginationItem::Ellipsis,
                PaginationItem::Page(page_count - 1),
            ])
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RootRowState {
    Online,
    Offline,
    Scanning,
    Failed { message: String },
    CompletedWithErrors { error_count: u64 },
}

pub fn root_row_state(
    root: &StorageRoot,
    latest_scan: Option<&ScanHistoryEntry>,
    live_scan: bool,
) -> RootRowState {
    if live_scan {
        return RootRowState::Scanning;
    }
    if !root.is_reachable {
        return RootRowState::Offline;
    }
    match latest_scan {
        Some(scan) if scan.outcome == Some(ScanOutcome::Failed) => RootRowState::Failed {
            message: scan
                .error_message
                .clone()
                .unwrap_or_else(|| "Scan failed".to_string()),
        },
        Some(scan) if scan.outcome == Some(ScanOutcome::CompletedWithErrors) => {
            RootRowState::CompletedWithErrors {
                error_count: scan.error_count,
            }
        }
        _ => RootRowState::Online,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanProgressView {
    pub percent: Option<u8>,
    pub count_line: String,
}

impl ScanProgressView {
    pub fn from_progress(progress: &ScanProgress) -> Self {
        match progress {
            ScanProgress::Discovering {
                discovered_files, ..
            } => Self {
                percent: None,
                count_line: format!("{discovered_files} files found"),
            },
            ScanProgress::Processing {
                processed_files,
                total_files,
                ..
            } => {
                let percent = if *total_files == 0 {
                    0
                } else {
                    processed_files.saturating_mul(100) / total_files
                }
                .min(100) as u8;
                Self {
                    percent: Some(percent),
                    count_line: format!("{processed_files} of ~{total_files} files"),
                }
            }
            ScanProgress::Finished { .. } => Self {
                percent: Some(100),
                count_line: "Scan complete".to_string(),
            },
        }
    }
}

pub fn format_label(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("flac") => "FLAC",
        Some("m4a") => "ALAC",
        Some("aif" | "aiff") => "AIFF",
        Some("wav") => "WAV",
        _ => "PCM",
    }
}

pub fn quality_label(bit_depth: Option<u8>, sample_rate_hz: Option<u32>) -> Option<String> {
    Some(format!(
        "{}/{}",
        bit_depth?,
        compact_sample_rate(sample_rate_hz?)
    ))
}

pub fn is_hi_res(bit_depth: Option<u8>, sample_rate_hz: Option<u32>) -> bool {
    bit_depth.is_some_and(|depth| depth > 16) || sample_rate_hz.is_some_and(|rate| rate > 48_000)
}

pub fn filter_albums<'a>(albums: &'a [Album], chip: &FilterChip, now_ms: i64) -> Vec<&'a Album> {
    albums
        .iter()
        .filter(|album| match chip {
            FilterChip::All => true,
            FilterChip::HiRes => is_hi_res(album.max_bit_depth, album.max_sample_rate_hz),
            FilterChip::RecentlyAdded => {
                album.latest_added_at_ms >= now_ms.saturating_sub(RECENTLY_ADDED_WINDOW_MS)
            }
            FilterChip::Genre(genre) => album
                .genres
                .iter()
                .any(|album_genre| album_genre.eq_ignore_ascii_case(genre)),
        })
        .collect()
}

#[cfg(test)]
pub fn filter_tracks<'a>(
    tracks: &'a [Track],
    chip: &FilterChip,
    artist_filter: Option<&str>,
    now_ms: i64,
) -> Vec<&'a Track> {
    tracks
        .iter()
        .filter(|track| {
            artist_filter.is_none_or(|artist| track_artist(track).eq_ignore_ascii_case(artist))
        })
        .filter(|track| match chip {
            FilterChip::All => true,
            FilterChip::HiRes => is_hi_res(track.bit_depth, track.sample_rate_hz),
            FilterChip::RecentlyAdded => {
                track.added_at_ms >= now_ms.saturating_sub(RECENTLY_ADDED_WINDOW_MS)
            }
            FilterChip::Genre(genre) => track
                .genre
                .as_deref()
                .is_some_and(|track_genre| track_genre.eq_ignore_ascii_case(genre)),
        })
        .collect()
}

pub fn track_title(track: &Track) -> String {
    track
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .or_else(|| {
            track
                .path
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| track.path.display().to_string())
}

pub fn track_artist(track: &Track) -> &str {
    track
        .artist
        .as_deref()
        .map(str::trim)
        .filter(|artist| !artist.trim().is_empty())
        .unwrap_or(UNKNOWN_ARTIST)
}

pub fn track_album(track: &Track) -> &str {
    track
        .album
        .as_deref()
        .map(str::trim)
        .filter(|album| !album.trim().is_empty())
        .unwrap_or(UNKNOWN_ALBUM)
}

pub fn format_duration(duration_ms: Option<i64>) -> String {
    let total_seconds = duration_ms.unwrap_or_default().max(0) as u64 / 1_000;
    format!("{}:{:02}", total_seconds / 60, total_seconds % 60)
}

pub fn format_bytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / GIB)
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / MIB)
    } else {
        format!("{bytes} B")
    }
}

fn compact_sample_rate(sample_rate_hz: u32) -> String {
    if sample_rate_hz.is_multiple_of(1_000) {
        (sample_rate_hz / 1_000).to_string()
    } else {
        format!("{:.1}", sample_rate_hz as f64 / 1_000.0)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::library::{Playlist, PlaylistSummary, ScanProgressAction, StorageRootId};

    fn root(is_reachable: bool) -> StorageRoot {
        StorageRoot {
            id: 1,
            path: PathBuf::from("/music"),
            display_name: "Music".to_string(),
            added_at_ms: 1,
            last_scan_at_ms: None,
            is_reachable,
            is_case_sensitive: true,
        }
    }

    fn scan(outcome: ScanOutcome, error_count: u64) -> ScanHistoryEntry {
        ScanHistoryEntry {
            id: 1,
            storage_root_id: 1,
            started_at_ms: 1,
            finished_at_ms: Some(2),
            added_count: 0,
            updated_count: 0,
            removed_count: 0,
            unsupported_count: 0,
            error_count,
            removals_suppressed: false,
            outcome: Some(outcome),
            error_message: Some("permission denied".to_string()),
        }
    }

    fn track(
        id: StorageRootId,
        artist: &str,
        genre: Option<&str>,
        bit_depth: u8,
        sample_rate_hz: u32,
        added_at_ms: i64,
    ) -> Track {
        Track {
            id,
            storage_root_id: 1,
            path: PathBuf::from(format!("/{id}.flac")),
            title: Some(format!("Track {id}")),
            artist: Some(artist.to_string()),
            album: Some("Album".to_string()),
            album_artist: None,
            year: Some(2024),
            genre: genre.map(str::to_string),
            track_number: Some(id as u32),
            disc_number: Some(1),
            duration_ms: Some(60_000),
            sample_rate_hz: Some(sample_rate_hz),
            bit_depth: Some(bit_depth),
            channels: Some(2),
            file_size_bytes: 1,
            modified_at_ns: 1,
            cover_art_path: None,
            cover_art_mime_type: None,
            added_at_ms,
            updated_at_ms: added_at_ms,
        }
    }

    #[test]
    fn maps_every_storage_root_row_state() {
        assert_eq!(
            root_row_state(&root(true), None, false),
            RootRowState::Online
        );
        assert_eq!(
            root_row_state(&root(false), Some(&scan(ScanOutcome::Completed, 0)), false),
            RootRowState::Offline
        );
        assert_eq!(
            root_row_state(&root(false), Some(&scan(ScanOutcome::Offline, 1)), true),
            RootRowState::Scanning
        );
        assert_eq!(
            root_row_state(&root(true), Some(&scan(ScanOutcome::Failed, 1)), false),
            RootRowState::Failed {
                message: "permission denied".to_string()
            }
        );
        assert_eq!(
            root_row_state(
                &root(true),
                Some(&scan(ScanOutcome::CompletedWithErrors, 3)),
                false
            ),
            RootRowState::CompletedWithErrors { error_count: 3 }
        );
        assert_eq!(
            root_row_state(&root(true), Some(&scan(ScanOutcome::Completed, 0)), false),
            RootRowState::Online
        );
    }

    #[test]
    fn formats_container_quality_and_hi_res_threshold() {
        assert_eq!(format_label(Path::new("track.flac")), "FLAC");
        assert_eq!(format_label(Path::new("track.m4a")), "ALAC");
        assert_eq!(format_label(Path::new("track.aiff")), "AIFF");
        assert_eq!(format_label(Path::new("track.wav")), "WAV");
        assert_eq!(
            quality_label(Some(24), Some(96_000)).as_deref(),
            Some("24/96")
        );
        assert_eq!(
            quality_label(Some(16), Some(44_100)).as_deref(),
            Some("16/44.1")
        );
        assert!(!is_hi_res(Some(16), Some(48_000)));
        assert!(is_hi_res(Some(24), Some(48_000)));
        assert!(is_hi_res(Some(16), Some(96_000)));
    }

    #[test]
    fn derives_scan_percent_and_count_lines() {
        let processing = ScanProgressView::from_progress(&ScanProgress::Processing {
            processed_files: 25,
            total_files: 100,
            current_path: PathBuf::from("/music/track.flac"),
            action: ScanProgressAction::Added,
        });
        assert_eq!(processing.percent, Some(25));
        assert_eq!(processing.count_line, "25 of ~100 files");

        let discovering = ScanProgressView::from_progress(&ScanProgress::Discovering {
            discovered_files: 42,
            current_path: PathBuf::from("/music"),
        });
        assert_eq!(discovering.percent, None);
        assert_eq!(discovering.count_line, "42 files found");
    }

    #[test]
    fn filters_tracks_by_chip_and_artist_facet() {
        let now = 100 * 24 * 60 * 60 * 1_000;
        let recent = now - 10 * 24 * 60 * 60 * 1_000;
        let old = now - 60 * 24 * 60 * 60 * 1_000;
        let mut tracks = vec![
            track(1, "Artist A", Some("Jazz"), 24, 96_000, recent),
            track(2, "Artist B", Some("Rock"), 16, 44_100, old),
        ];
        tracks[0].artist = Some(" Artist A ".to_string());
        tracks[0].album = Some(" Album ".to_string());

        assert_eq!(
            filter_tracks(&tracks, &FilterChip::HiRes, None, now).len(),
            1
        );
        assert_eq!(
            filter_tracks(&tracks, &FilterChip::RecentlyAdded, None, now).len(),
            1
        );
        assert_eq!(
            filter_tracks(&tracks, &FilterChip::Genre("jazz".to_string()), None, now).len(),
            1
        );
        assert_eq!(
            filter_tracks(&tracks, &FilterChip::All, Some("artist b"), now).len(),
            1
        );
        assert_eq!(
            filter_tracks(&tracks, &FilterChip::All, Some("artist a"), now).len(),
            1
        );
        assert_eq!(track_artist(&tracks[0]), "Artist A");
        assert_eq!(track_album(&tracks[0]), "Album");
    }

    #[test]
    fn filters_albums_by_quality_recency_and_genre_chips() {
        let now = 100 * 24 * 60 * 60 * 1_000;
        let albums = vec![
            Album {
                title: "Recent Jazz".to_string(),
                artist: "Artist A".to_string(),
                year: Some(2024),
                track_count: 1,
                total_duration_ms: 60_000,
                max_sample_rate_hz: Some(96_000),
                max_bit_depth: Some(24),
                cover_art_path: None,
                latest_added_at_ms: now - 10 * 24 * 60 * 60 * 1_000,
                genres: vec!["Jazz".to_string()],
            },
            Album {
                title: "Old Rock".to_string(),
                artist: "Artist B".to_string(),
                year: Some(1999),
                track_count: 1,
                total_duration_ms: 60_000,
                max_sample_rate_hz: Some(44_100),
                max_bit_depth: Some(16),
                cover_art_path: None,
                latest_added_at_ms: now - 60 * 24 * 60 * 60 * 1_000,
                genres: vec!["Rock".to_string()],
            },
        ];

        assert_eq!(filter_albums(&albums, &FilterChip::HiRes, now).len(), 1);
        assert_eq!(
            filter_albums(&albums, &FilterChip::RecentlyAdded, now).len(),
            1
        );
        assert_eq!(
            filter_albums(&albums, &FilterChip::Genre("jazz".to_string()), now).len(),
            1
        );
    }

    #[test]
    fn search_selection_flattens_groups_and_moves_with_wraparound() {
        let album = |title: &str| Album {
            title: title.to_string(),
            artist: "Artist".to_string(),
            year: Some(2024),
            track_count: 1,
            total_duration_ms: 60_000,
            max_sample_rate_hz: Some(44_100),
            max_bit_depth: Some(16),
            cover_art_path: None,
            latest_added_at_ms: 1,
            genres: Vec::new(),
        };
        let playlist = |id, name: &str| PlaylistSummary {
            playlist: Playlist {
                id,
                name: name.to_string(),
                created_at_ms: 1,
                updated_at_ms: 1,
            },
            track_count: 0,
            total_duration_ms: 0,
            cover_art_path: None,
        };
        let mut model = SearchViewModel::default();
        model.set_results(LibrarySearchResults {
            albums: vec![album("First"), album("Second")],
            tracks: vec![track(1, "Artist", None, 16, 44_100, 1)],
            playlists: vec![playlist(1, "First"), playlist(2, "Second")],
        });

        assert_eq!(model.selected(), Some(SearchSelection::Album(0)));
        model.move_next();
        assert_eq!(model.selected(), Some(SearchSelection::Album(1)));
        model.move_next();
        assert_eq!(model.selected(), Some(SearchSelection::Track(0)));
        model.move_next();
        assert_eq!(model.selected(), Some(SearchSelection::Playlist(0)));
        model.move_previous();
        assert_eq!(model.selected(), Some(SearchSelection::Track(0)));

        model.move_previous();
        model.move_previous();
        model.move_previous();
        assert_eq!(model.selected(), Some(SearchSelection::Playlist(1)));
        assert_eq!(model.selected_index(), Some(4));
    }

    #[test]
    fn search_selection_clears_for_empty_results() {
        let mut model = SearchViewModel::default();
        model.set_results(LibrarySearchResults::default());
        model.move_next();
        model.move_previous();

        assert_eq!(model.result_count(), 0);
        assert_eq!(model.selected(), None);
    }

    #[test]
    fn pagination_clamps_ranges_and_moves_between_pages() {
        let mut pagination = Pagination::new(50);
        pagination.set_total_items(206);

        assert_eq!(pagination.page_count(), 5);
        assert_eq!(pagination.range(), Some((1, 50)));
        assert!(!pagination.can_previous());
        assert!(pagination.can_next());
        assert!(pagination.set_page(4));
        assert_eq!(pagination.offset(), 200);
        assert_eq!(pagination.range(), Some((201, 206)));
        assert!(pagination.can_previous());
        assert!(!pagination.can_next());

        pagination.set_total_items(12);
        assert_eq!(pagination.current_page(), 0);
        assert_eq!(pagination.range(), Some((1, 12)));
    }

    #[test]
    fn pagination_items_match_the_compact_design() {
        let mut pagination = Pagination::new(50);
        pagination.set_total_items(12_482);

        assert_eq!(
            pagination.items(),
            vec![
                PaginationItem::Page(0),
                PaginationItem::Page(1),
                PaginationItem::Page(2),
                PaginationItem::Page(3),
                PaginationItem::Ellipsis,
                PaginationItem::Page(249),
            ]
        );
        assert!(pagination.set_page(124));
        assert_eq!(
            pagination.items(),
            vec![
                PaginationItem::Page(0),
                PaginationItem::Ellipsis,
                PaginationItem::Page(123),
                PaginationItem::Page(124),
                PaginationItem::Page(125),
                PaginationItem::Ellipsis,
                PaginationItem::Page(249),
            ]
        );
    }
}
