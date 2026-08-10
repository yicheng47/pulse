use std::path::PathBuf;

use crate::library::{Track, TrackId, UNKNOWN_ALBUM, UNKNOWN_ARTIST};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrackRef {
    pub id: TrackId,
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<u64>,
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
            duration_ms: track.duration_ms.and_then(|ms| u64::try_from(ms).ok()),
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
    started: bool,
    skipped: usize,
}

impl QueueState {
    pub(crate) fn from_tracks(tracks: &[Track], start_index: usize) -> Self {
        let entries = tracks.iter().map(TrackRef::from).collect::<Vec<_>>();
        let index = (start_index < entries.len()).then_some(start_index);
        Self {
            entries,
            index,
            started: false,
            skipped: 0,
        }
    }

    pub(crate) fn mark_started(&mut self) {
        self.started = true;
    }

    pub(crate) fn skipped_count(&self) -> usize {
        self.skipped
    }

    /// Records the current entry as unplayable and moves to the next one.
    /// Returns `None` at the end of the queue; `nothing_played` then tells a
    /// poison queue (every attempted entry failed) from a partial failure.
    pub(crate) fn skip_failed(&mut self) -> Option<TrackRef> {
        self.skipped += 1;
        self.advance()
    }

    pub(crate) fn nothing_played(&self) -> bool {
        !self.started && self.skipped > 0
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

    fn upcoming_start(&self) -> usize {
        match self.index {
            Some(index) => index + 1,
            None => self.entries.len(),
        }
    }

    /// Upcoming entries paired with their 1-based queue positions (row
    /// numbering continues past the current track).
    pub(crate) fn upcoming(&self) -> impl Iterator<Item = (usize, &TrackRef)> {
        let start = self.upcoming_start().min(self.entries.len());
        self.entries[start..]
            .iter()
            .enumerate()
            .map(move |(offset, track)| (start + offset + 1, track))
    }

    /// Sum of the known upcoming durations; unknown durations contribute
    /// nothing.
    pub(crate) fn upcoming_duration_ms(&self) -> u64 {
        self.upcoming()
            .filter_map(|(_, track)| track.duration_ms)
            .sum()
    }

    /// Moves the queue index to `index` and returns that entry to play.
    /// Nothing is dropped, so Previous still walks back over jumped entries.
    pub(crate) fn jump_to(&mut self, index: usize) -> Option<TrackRef> {
        let track = self.entries.get(index)?.clone();
        self.index = Some(index);
        Some(track)
    }

    /// Removes the entry at `index`, renumbering the rest; the current track
    /// is never removed.
    pub(crate) fn remove_at(&mut self, index: usize) {
        if index >= self.entries.len() || Some(index) == self.index {
            return;
        }
        self.entries.remove(index);
        if let Some(current) = self.index
            && index < current
        {
            self.index = Some(current - 1);
        }
    }

    /// Drops every upcoming entry; the current track and the entries behind
    /// it stay so Previous keeps working.
    pub(crate) fn clear_upcoming(&mut self) {
        let start = self.upcoming_start();
        self.entries.truncate(start);
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
    fn skipping_failed_entries_advances_and_counts_without_marking_playback() {
        let tracks = [track(1, "corrupt"), track(2, "good"), track(3, "last")];
        let mut queue = QueueState::from_tracks(&tracks, 0);

        assert_eq!(queue.skip_failed().unwrap().title, "good");
        assert_eq!(queue.skipped_count(), 1);
        assert!(queue.nothing_played());

        queue.mark_started();
        assert!(!queue.nothing_played());
        assert_eq!(queue.skip_failed().unwrap().title, "last");
        assert_eq!(queue.skipped_count(), 2);
    }

    #[test]
    fn a_queue_where_every_entry_fails_reports_nothing_played() {
        let tracks = [track(1, "first"), track(2, "second")];
        let mut queue = QueueState::from_tracks(&tracks, 0);

        assert!(queue.skip_failed().is_some());
        assert!(queue.skip_failed().is_none());
        assert_eq!(queue.skipped_count(), 2);
        assert!(queue.nothing_played());
    }

    #[test]
    fn a_fresh_queue_with_no_failures_is_not_poisoned() {
        let queue = QueueState::from_tracks(&[track(1, "only")], 0);
        assert!(!queue.nothing_played());
    }

    #[test]
    fn jump_moves_the_index_without_dropping_entries() {
        let tracks = [
            track(1, "first"),
            track(2, "second"),
            track(3, "third"),
            track(4, "fourth"),
        ];
        let mut queue = QueueState::from_tracks(&tracks, 0);

        let target = queue.jump_to(2).unwrap();

        assert_eq!(target.title, "third");
        assert_eq!(queue.current().unwrap().title, "third");
        assert_eq!(queue.remaining_count(), 1);
        assert_eq!(
            queue.paths(),
            ["/first.flac", "/second.flac", "/third.flac", "/fourth.flac"]
        );
        assert_eq!(
            queue.previous(0),
            Some(PreviousAction::PlayPrevious(TrackRef::from(&tracks[1]))),
            "Previous walks back over the jumped entries"
        );
    }

    #[test]
    fn jump_out_of_range_leaves_the_queue_untouched() {
        let tracks = [track(1, "only")];
        let mut queue = QueueState::from_tracks(&tracks, 0);

        assert!(queue.jump_to(1).is_none());
        assert_eq!(queue.current().unwrap().title, "only");
        assert_eq!(queue.remaining_count(), 0);
    }

    #[test]
    fn remove_renumbers_upcoming_entries_and_fixes_the_badge_count() {
        let tracks = [
            track(1, "current"),
            track(2, "doomed"),
            track(3, "third"),
            track(4, "fourth"),
        ];
        let mut queue = QueueState::from_tracks(&tracks, 0);
        assert_eq!(queue.remaining_count(), 3);

        queue.remove_at(1);

        assert_eq!(queue.remaining_count(), 2);
        assert_eq!(
            queue.paths(),
            ["/current.flac", "/third.flac", "/fourth.flac"]
        );
        assert_eq!(
            queue
                .upcoming()
                .map(|(position, _)| position)
                .collect::<Vec<_>>(),
            [2, 3]
        );
        assert_eq!(queue.current().unwrap().title, "current");
    }

    #[test]
    fn remove_never_touches_the_current_track() {
        let tracks = [track(1, "current"), track(2, "next")];
        let mut queue = QueueState::from_tracks(&tracks, 0);

        queue.remove_at(0);
        queue.remove_at(9);

        assert_eq!(queue.paths(), ["/current.flac", "/next.flac"]);
        assert_eq!(queue.current().unwrap().title, "current");
        assert_eq!(queue.remaining_count(), 1);
    }

    #[test]
    fn remove_before_the_current_track_keeps_it_current() {
        let tracks = [track(1, "played"), track(2, "current"), track(3, "next")];
        let mut queue = QueueState::from_tracks(&tracks, 1);

        queue.remove_at(0);

        assert_eq!(queue.current().unwrap().title, "current");
        assert_eq!(queue.remaining_count(), 1);
        assert_eq!(queue.paths(), ["/current.flac", "/next.flac"]);
    }

    #[test]
    fn clear_upcoming_keeps_the_current_track_and_history() {
        let tracks = [
            track(1, "played"),
            track(2, "current"),
            track(3, "doomed-1"),
            track(4, "doomed-2"),
        ];
        let mut queue = QueueState::from_tracks(&tracks, 1);
        assert_eq!(queue.remaining_count(), 2);

        queue.clear_upcoming();

        assert_eq!(queue.remaining_count(), 0);
        assert_eq!(queue.current().unwrap().title, "current");
        assert_eq!(queue.paths(), ["/played.flac", "/current.flac"]);
        assert_eq!(
            queue.previous(0),
            Some(PreviousAction::PlayPrevious(TrackRef::from(&tracks[0]))),
            "history stays for Previous"
        );
    }

    #[test]
    fn upcoming_positions_continue_the_queue_numbering() {
        let tracks = [
            track(1, "first"),
            track(2, "second"),
            track(3, "third"),
            track(4, "fourth"),
        ];
        let queue = QueueState::from_tracks(&tracks, 1);

        let numbered = queue
            .upcoming()
            .map(|(position, track)| (position, track.title.clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            numbered,
            [(3, "third".to_string()), (4, "fourth".to_string())]
        );
    }

    #[test]
    fn upcoming_minutes_total_excludes_unknown_durations() {
        let mut with_duration = track(2, "timed");
        with_duration.duration_ms = Some(200_000);
        let mut unknown = track(3, "untimed");
        unknown.duration_ms = None;
        let mut also_timed = track(4, "also-timed");
        also_timed.duration_ms = Some(40_000);
        let tracks = [track(1, "current"), with_duration, unknown, also_timed];
        let queue = QueueState::from_tracks(&tracks, 0);

        assert_eq!(queue.upcoming_duration_ms(), 240_000);
    }

    #[test]
    fn library_metadata_formats_the_playback_secondary_line() {
        let mut source = track(1, "song");
        source.artist = Some(" 王菲 ".to_string());
        source.album = Some(" 菲靡靡之音 ".to_string());

        assert_eq!(TrackRef::from(&source).secondary(), "王菲 - 菲靡靡之音");
    }
}
