use std::path::PathBuf;

use crate::library::{Track, TrackId, UNKNOWN_ALBUM, UNKNOWN_ARTIST};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrackRef {
    pub id: TrackId,
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover_art_path: Option<PathBuf>,
}

impl TrackRef {
    pub(crate) fn secondary(&self) -> String {
        format!("{} - {}", self.artist, self.album)
    }
}

impl From<&Track> for TrackRef {
    fn from(track: &Track) -> Self {
        Self {
            id: track.id,
            path: track.path.clone(),
            title: track
                .title
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| path_title(&track.path)),
            artist: track
                .artist
                .as_deref()
                .map(str::trim)
                .filter(|artist| !artist.is_empty())
                .unwrap_or(UNKNOWN_ARTIST)
                .to_string(),
            album: track
                .album
                .as_deref()
                .map(str::trim)
                .filter(|album| !album.is_empty())
                .unwrap_or(UNKNOWN_ALBUM)
                .to_string(),
            cover_art_path: track.cover_art_path.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PreviousAction {
    Restart(TrackRef),
    PlayPrevious(TrackRef),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct QueueState {
    entries: Vec<TrackRef>,
    index: Option<usize>,
}

impl QueueState {
    pub(crate) fn from_tracks(tracks: &[Track], start_index: usize) -> Self {
        let entries = tracks.iter().map(TrackRef::from).collect::<Vec<_>>();
        let index = (start_index < entries.len()).then_some(start_index);
        Self { entries, index }
    }

    pub(crate) fn current(&self) -> Option<&TrackRef> {
        self.index.and_then(|index| self.entries.get(index))
    }

    pub(crate) fn remaining_count(&self) -> usize {
        self.index
            .map(|index| self.entries.len().saturating_sub(index + 1))
            .unwrap_or(0)
    }

    pub(crate) fn can_advance(&self) -> bool {
        self.remaining_count() > 0
    }

    pub(crate) fn advance(&mut self) -> Option<TrackRef> {
        let next_index = self.index?.checked_add(1)?;
        let next = self.entries.get(next_index)?.clone();
        self.index = Some(next_index);
        Some(next)
    }

    pub(crate) fn previous(&mut self, position_ms: u64) -> Option<PreviousAction> {
        let index = self.index?;
        if position_ms > 3_000 || index == 0 {
            return self.current().cloned().map(PreviousAction::Restart);
        }
        let previous_index = index - 1;
        let previous = self.entries.get(previous_index)?.clone();
        self.index = Some(previous_index);
        Some(PreviousAction::PlayPrevious(previous))
    }

    #[cfg(test)]
    fn paths(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|entry| entry.path.to_str().unwrap())
            .collect()
    }
}

fn path_title(path: &std::path::Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: TrackId, name: &str) -> Track {
        Track {
            id,
            storage_root_id: 1,
            path: PathBuf::from(format!("/{name}.flac")),
            title: Some(name.to_string()),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            album_artist: None,
            year: None,
            genre: None,
            track_number: None,
            disc_number: None,
            duration_ms: Some(1_000),
            sample_rate_hz: Some(44_100),
            bit_depth: Some(16),
            channels: Some(2),
            file_size_bytes: 1,
            modified_at_ns: 1,
            cover_art_path: None,
            cover_art_mime_type: None,
            added_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn builds_album_queue_in_the_supplied_disc_track_order() {
        let tracks = [track(1, "disc-1-track-1"), track(2, "disc-2-track-1")];
        let queue = QueueState::from_tracks(&tracks, 0);

        assert_eq!(
            queue.paths(),
            ["/disc-1-track-1.flac", "/disc-2-track-1.flac"]
        );
    }

    #[test]
    fn builds_playlist_queue_in_order_with_duplicate_entries() {
        let duplicate = track(1, "duplicate");
        let tracks = [duplicate.clone(), track(2, "middle"), duplicate];
        let queue = QueueState::from_tracks(&tracks, 1);

        assert_eq!(
            queue.paths(),
            ["/duplicate.flac", "/middle.flac", "/duplicate.flac"]
        );
        assert_eq!(queue.current().unwrap().title, "middle");
    }

    #[test]
    fn builds_tracks_queue_in_the_active_visible_sort_and_filter_order() {
        let visible = [track(3, "recent"), track(1, "older")];
        let queue = QueueState::from_tracks(&visible, 1);

        assert_eq!(queue.paths(), ["/recent.flac", "/older.flac"]);
        assert_eq!(queue.current().unwrap().title, "older");
    }

    #[test]
    fn advances_counts_remaining_and_stops_at_the_end() {
        let tracks = [track(1, "first"), track(2, "second"), track(3, "third")];
        let mut queue = QueueState::from_tracks(&tracks, 0);

        assert_eq!(queue.remaining_count(), 2);
        assert!(queue.can_advance());
        assert_eq!(queue.advance().unwrap().title, "second");
        assert_eq!(queue.remaining_count(), 1);
        assert_eq!(queue.advance().unwrap().title, "third");
        assert_eq!(queue.remaining_count(), 0);
        assert!(!queue.can_advance());
        assert!(queue.advance().is_none());
        assert_eq!(queue.current().unwrap().title, "third");
    }

    #[test]
    fn previous_restarts_after_three_seconds_otherwise_moves_back() {
        let tracks = [track(1, "first"), track(2, "second")];
        let mut queue = QueueState::from_tracks(&tracks, 1);

        assert_eq!(
            queue.previous(3_001),
            Some(PreviousAction::Restart(TrackRef::from(&tracks[1])))
        );
        assert_eq!(queue.current().unwrap().title, "second");
        assert_eq!(
            queue.previous(3_000),
            Some(PreviousAction::PlayPrevious(TrackRef::from(&tracks[0])))
        );
        assert_eq!(queue.current().unwrap().title, "first");
        assert_eq!(
            queue.previous(0),
            Some(PreviousAction::Restart(TrackRef::from(&tracks[0])))
        );
    }

    #[test]
    fn library_metadata_formats_the_playback_secondary_line() {
        let mut source = track(1, "song");
        source.artist = Some(" 王菲 ".to_string());
        source.album = Some(" 菲靡靡之音 ".to_string());

        assert_eq!(TrackRef::from(&source).secondary(), "王菲 - 菲靡靡之音");
    }
}
