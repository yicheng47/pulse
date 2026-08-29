use std::{io, path::PathBuf};

use thiserror::Error;

pub type StorageRootId = i64;
pub type TrackId = i64;
pub type PlaylistId = i64;

pub const UNKNOWN_ALBUM: &str = "Unknown Album";
pub const UNKNOWN_ARTIST: &str = "Unknown Artist";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageRoot {
    pub id: StorageRootId,
    pub path: PathBuf,
    pub display_name: String,
    pub added_at_ms: i64,
    pub last_scan_at_ms: Option<i64>,
    pub is_reachable: bool,
    pub is_case_sensitive: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Track {
    pub id: TrackId,
    pub storage_root_id: StorageRootId,
    pub path: PathBuf,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration_ms: Option<i64>,
    pub sample_rate_hz: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    pub file_size_bytes: u64,
    pub modified_at_ns: i64,
    pub cover_art_path: Option<PathBuf>,
    pub cover_art_mime_type: Option<String>,
    pub added_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrackQueryFilter {
    All,
    HiRes,
    AddedSince(i64),
    Genre(String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TrackPage {
    pub tracks: Vec<Track>,
    pub total_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AlbumQueryFilter {
    All,
    HiRes,
    AddedSince(i64),
    Genre(String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AlbumPage {
    pub albums: Vec<Album>,
    pub total_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub year: Option<u32>,
    pub track_count: u64,
    pub total_duration_ms: u64,
    pub max_sample_rate_hz: Option<u32>,
    pub max_bit_depth: Option<u8>,
    pub cover_art_path: Option<PathBuf>,
    pub latest_added_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Artist {
    pub id: i64,
    pub name: String,
    pub name_key: String,
    pub album_count: u64,
    pub track_count: u64,
    pub total_duration_ms: u64,
    pub earliest_added_ms: i64,
    pub earliest_added_year: Option<u32>,
    pub cover_art_path: Option<PathBuf>,
    pub display_name: Option<String>,
    pub hidden: Option<bool>,
    pub mbid: Option<String>,
    pub photo_path: Option<PathBuf>,
    pub photo_source: Option<String>,
    pub enriched_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtistDetail {
    pub artist: Artist,
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaylistSummary {
    pub playlist: Playlist,
    pub track_count: u64,
    pub total_duration_ms: u64,
    pub cover_art_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaylistTrack {
    pub position: usize,
    pub track: Track,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LibrarySearchResults {
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
    pub playlists: Vec<PlaylistSummary>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AlbumSortOrder {
    #[default]
    Title,
    Artist,
    DateAdded,
    ReleaseYear,
    Duration,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TrackSortOrder {
    #[default]
    Title,
    Artist,
    Album,
    DateAdded,
    ReleaseYear,
    Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanOutcome {
    Completed,
    CompletedWithErrors,
    Offline,
    Failed,
}

impl ScanOutcome {
    pub(super) fn as_db_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::CompletedWithErrors => "completed_with_errors",
            Self::Offline => "offline",
            Self::Failed => "failed",
        }
    }

    pub(super) fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "completed" => Some(Self::Completed),
            "completed_with_errors" => Some(Self::CompletedWithErrors),
            "offline" => Some(Self::Offline),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanFileError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanReport {
    pub scan_id: i64,
    pub storage_root_id: StorageRootId,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub discovered: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unsupported: usize,
    pub skipped: usize,
    pub removals_suppressed: bool,
    pub errors: Vec<ScanFileError>,
    pub outcome: ScanOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanProgressAction {
    Added,
    Updated,
    Unsupported,
    Skipped,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScanProgress {
    Discovering {
        discovered_files: usize,
        current_path: PathBuf,
    },
    Processing {
        processed_files: usize,
        total_files: usize,
        current_path: PathBuf,
        action: ScanProgressAction,
    },
    Finished {
        outcome: ScanOutcome,
        added: usize,
        updated: usize,
        removed: usize,
        unsupported: usize,
        skipped: usize,
        removals_suppressed: bool,
        error_count: usize,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LibrarySummary {
    pub album_count: u64,
    pub track_count: u64,
    pub file_size_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanHistoryEntry {
    pub id: i64,
    pub storage_root_id: StorageRootId,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub added_count: u64,
    pub updated_count: u64,
    pub removed_count: u64,
    pub unsupported_count: u64,
    pub error_count: u64,
    pub removals_suppressed: bool,
    pub outcome: Option<ScanOutcome>,
    pub error_message: Option<String>,
}

/// What deleting an album accomplished. File deletion and the database
/// update cannot be atomic across the filesystem boundary, so the outcome
/// reports each side separately: files already unlinked stay unlinked even
/// when the row cleanup fails, and the caller must say so.
pub struct DeleteAlbumOutcome {
    pub deleted_ids: Vec<TrackId>,
    pub deleted_files: usize,
    pub total_files: usize,
    pub failures: Vec<String>,
    pub db_error: Option<String>,
}

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("SQLite error: {0}")]
    Database(String),
    #[error("I/O error at {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("path is not valid Unicode: {}", .0.display())]
    NonUnicodePath(PathBuf),
    #[error("storage root is not a directory: {}", .0.display())]
    NotDirectory(PathBuf),
    #[error("unsupported library schema version {0}")]
    UnsupportedSchemaVersion(i64),
    #[error("schema migration found rows violating foreign keys in table {0}")]
    MigrationIntegrity(String),
    #[error("storage root {0} was not found")]
    StorageRootNotFound(StorageRootId),
    #[error("playlist {0} was not found")]
    PlaylistNotFound(PlaylistId),
    #[error("playlist {playlist_id} has no entry at position {position}")]
    PlaylistEntryNotFound {
        playlist_id: PlaylistId,
        position: usize,
    },
    #[error("file modified time is too large to store: {}", .0.display())]
    FileTimestampOutOfRange(PathBuf),
    #[error("{0} is too large to store")]
    IntegerOutOfRange(&'static str),
}
