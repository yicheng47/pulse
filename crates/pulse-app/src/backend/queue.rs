use std::path::PathBuf;

use super::{Track, TrackId, UNKNOWN_ALBUM, UNKNOWN_ARTIST};

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

impl RepeatMode {
    fn next(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QueueEntry {
    original_position: usize,
    track: TrackRef,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct QueueState {
    entries: Vec<QueueEntry>,
    index: Option<usize>,
    started: bool,
    skipped: usize,
    consecutive_failures: usize,
    shuffle_enabled: bool,
    repeat_mode: RepeatMode,
    history: Vec<usize>,
    true_history: bool,
}

impl QueueState {
    #[cfg(test)]
    pub(crate) fn from_tracks(tracks: &[Track], start_index: usize) -> Self {
        let mut queue = Self::default();
        queue.rebuild(tracks, start_index);
        queue
    }

    pub(crate) fn rebuild(&mut self, tracks: &[Track], start_index: usize) {
        let mut rng = fastrand::Rng::new();
        self.rebuild_with_rng(tracks, start_index, &mut rng);
    }

    pub(crate) fn rebuild_shuffled(&mut self, tracks: &[Track]) {
        let mut rng = fastrand::Rng::new();
        self.rebuild_shuffled_with_rng(tracks, &mut rng);
    }

    fn rebuild_shuffled_with_rng(&mut self, tracks: &[Track], rng: &mut fastrand::Rng) {
        self.shuffle_enabled = true;
        let start_index = if tracks.is_empty() {
            0
        } else {
            rng.usize(..tracks.len())
        };
        self.rebuild_with_rng(tracks, start_index, rng);
    }

    fn rebuild_with_rng(&mut self, tracks: &[Track], start_index: usize, rng: &mut fastrand::Rng) {
        self.entries = tracks
            .iter()
            .enumerate()
            .map(|(original_position, track)| QueueEntry {
                original_position,
                track: TrackRef::from(track),
            })
            .collect();
        self.index = (start_index < self.entries.len()).then_some(start_index);
        if self.shuffle_enabled
            && let Some(index) = self.index
        {
            let current = self.entries.remove(index);
            self.entries.insert(0, current);
            self.index = Some(0);
        }
        self.started = false;
        self.skipped = 0;
        self.consecutive_failures = 0;
        self.history.clear();
        self.true_history = self.shuffle_enabled;
        if self.shuffle_enabled {
            self.shuffle_upcoming(rng);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.index = None;
        self.started = false;
        self.skipped = 0;
        self.consecutive_failures = 0;
        self.history.clear();
        self.true_history = self.shuffle_enabled;
    }

    pub(crate) fn shuffle_enabled(&self) -> bool {
        self.shuffle_enabled
    }

    pub(crate) fn toggle_shuffle(&mut self) {
        self.set_shuffle(!self.shuffle_enabled);
    }

    pub(crate) fn set_shuffle(&mut self, enabled: bool) {
        let mut rng = fastrand::Rng::new();
        self.set_shuffle_with_rng(enabled, &mut rng);
    }

    fn set_shuffle_with_rng(&mut self, enabled: bool, rng: &mut fastrand::Rng) {
        if self.shuffle_enabled == enabled {
            return;
        }
        self.shuffle_enabled = enabled;
        if enabled {
            self.true_history = true;
            self.shuffle_upcoming(rng);
        } else {
            self.restore_upcoming_order();
        }
    }

    pub(crate) fn repeat_mode(&self) -> RepeatMode {
        self.repeat_mode
    }

    pub(crate) fn cycle_repeat(&mut self) {
        self.repeat_mode = self.repeat_mode.next();
    }

    pub(crate) fn mark_started(&mut self) {
        self.started = true;
        self.consecutive_failures = 0;
    }

    pub(crate) fn skipped_count(&self) -> usize {
        self.skipped
    }

    pub(crate) fn note_skipped_ahead(&mut self) {
        self.skipped += 1;
    }

    /// Records the current entry as unplayable and moves to the next one.
    /// Returns `None` at the end of the queue; `nothing_played` then tells a
    /// poison queue (every attempted entry failed) from a partial failure.
    pub(crate) fn skip_failed(&mut self) -> Option<TrackRef> {
        self.skipped += 1;
        self.consecutive_failures += 1;
        if self.consecutive_failures >= self.entries.len() {
            return None;
        }
        let next_index = self.next_index()?;
        self.move_to_index(next_index, false)
    }

    pub(crate) fn nothing_played(&self) -> bool {
        !self.started && self.skipped > 0
    }

    pub(crate) fn current(&self) -> Option<&TrackRef> {
        self.index
            .and_then(|index| self.entries.get(index))
            .map(|entry| &entry.track)
    }

    pub(crate) fn remaining_count(&self) -> usize {
        self.upcoming_indices().count()
    }

    pub(crate) fn can_advance(&self) -> bool {
        self.next_index().is_some()
    }

    pub(crate) fn advance(&mut self) -> Option<TrackRef> {
        let next_index = self.next_index()?;
        self.move_to_index(next_index, true)
    }

    pub(crate) fn advance_on_end(&mut self) -> Option<TrackRef> {
        if self.repeat_mode == RepeatMode::One {
            return self.current().cloned();
        }
        self.advance()
    }

    #[cfg(test)]
    pub(crate) fn peek_advance_on_end(&self) -> Option<&TrackRef> {
        self.peek_advance_on_end_skipping(|_| false)
    }

    pub(crate) fn peek_advance_on_end_skipping(
        &self,
        mut should_skip: impl FnMut(&TrackRef) -> bool,
    ) -> Option<&TrackRef> {
        if self.repeat_mode == RepeatMode::One {
            return self.current().filter(|track| !should_skip(track));
        }
        let current = self.index?;
        let candidate_count = if self.repeat_mode == RepeatMode::All {
            self.entries.len()
        } else {
            self.entries.len().saturating_sub(current + 1)
        };
        (1..=candidate_count)
            .map(|offset| &self.entries[(current + offset) % self.entries.len()].track)
            .find(|track| !should_skip(track))
    }

    fn next_index(&self) -> Option<usize> {
        let current = self.index?;
        if current + 1 < self.entries.len() {
            return Some(current + 1);
        }
        (self.repeat_mode == RepeatMode::All && !self.entries.is_empty()).then_some(0)
    }

    fn move_to_index(&mut self, next_index: usize, record_history: bool) -> Option<TrackRef> {
        let current_index = self.index?;
        let next = self.entries.get(next_index)?.track.clone();
        if record_history && current_index != next_index {
            self.history
                .push(self.entries[current_index].original_position);
        }
        self.index = Some(next_index);
        Some(next)
    }

    fn upcoming_indices(&self) -> impl Iterator<Item = usize> + '_ {
        let index = self.index.unwrap_or(self.entries.len());
        let start = index.saturating_add(1).min(self.entries.len());
        let wrap_end = if self.repeat_mode == RepeatMode::All
            && self.index.is_some()
            && !self.entries.is_empty()
        {
            index
        } else {
            0
        };
        (start..self.entries.len()).chain(0..wrap_end)
    }

    /// Upcoming entries paired with their 1-based queue positions (row
    /// numbering continues past the current track).
    pub(crate) fn upcoming(&self) -> impl Iterator<Item = (usize, &TrackRef)> {
        self.upcoming_indices()
            .map(|index| (index + 1, &self.entries[index].track))
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
        let current_index = self.index?;
        let track = self.entries.get(index)?.track.clone();
        if index == current_index {
            return Some(track);
        }
        self.history
            .push(self.entries[current_index].original_position);
        if self.shuffle_enabled {
            let current_position = self.entries[current_index].original_position;
            let target = self.entries.remove(index);
            let current_index = self
                .entries
                .iter()
                .position(|entry| entry.original_position == current_position)?;
            let target_index = current_index + 1;
            self.entries.insert(target_index, target);
            self.index = Some(target_index);
        } else {
            self.index = Some(index);
        }
        self.consecutive_failures = 0;
        Some(track)
    }

    pub(crate) fn position_on_path(&mut self, path: &std::path::Path) -> bool {
        let index = match self.index {
            Some(current) => (1..=self.entries.len())
                .map(|offset| (current + offset) % self.entries.len())
                .find(|index| self.entries[*index].track.path == path),
            None => self
                .entries
                .iter()
                .position(|entry| entry.track.path == path),
        };
        let Some(index) = index else { return false };
        self.index = Some(index);
        true
    }

    pub(crate) fn track_by_path(&self, path: &std::path::Path) -> Option<&TrackRef> {
        self.entries
            .iter()
            .find(|entry| entry.track.path == path)
            .map(|entry| &entry.track)
    }

    /// Removes the entry at `index`, renumbering the rest; the current track
    /// is never removed.
    pub(crate) fn remove_at(&mut self, index: usize) {
        if index >= self.entries.len() || Some(index) == self.index {
            return;
        }
        let removed = self.entries.remove(index).original_position;
        self.history.retain(|position| *position != removed);
        if let Some(current) = self.index
            && index < current
        {
            self.index = Some(current - 1);
        }
    }

    /// Drops every upcoming entry; the current track and the entries behind
    /// it stay so Previous keeps working.
    pub(crate) fn clear_upcoming(&mut self) {
        let Some(current) = self.index else {
            return;
        };
        if self.repeat_mode == RepeatMode::All {
            let current = self.entries[current].clone();
            self.entries = vec![current];
            self.index = Some(0);
            self.history.clear();
            return;
        }
        self.entries.truncate(current + 1);
        self.history.retain(|position| {
            self.entries
                .iter()
                .any(|entry| entry.original_position == *position)
        });
    }

    pub(crate) fn previous(&mut self, position_ms: u64) -> Option<PreviousAction> {
        let index = self.index?;
        if position_ms > 3_000 {
            return self.current().cloned().map(PreviousAction::Restart);
        }
        if self.true_history {
            while let Some(position) = self.history.pop() {
                if let Some(previous_index) = self
                    .entries
                    .iter()
                    .position(|entry| entry.original_position == position)
                    && previous_index != index
                {
                    let previous = self.entries[previous_index].track.clone();
                    self.index = Some(previous_index);
                    return Some(PreviousAction::PlayPrevious(previous));
                }
            }
            if self.repeat_mode != RepeatMode::All || self.entries.len() <= 1 {
                return self.current().cloned().map(PreviousAction::Restart);
            }
        }
        let previous_index = if index > 0 {
            index - 1
        } else if self.repeat_mode == RepeatMode::All && self.entries.len() > 1 {
            self.entries.len() - 1
        } else {
            return self.current().cloned().map(PreviousAction::Restart);
        };
        let previous = self.entries.get(previous_index)?.track.clone();
        let previous_position = self.entries[previous_index].original_position;
        if self.history.last() == Some(&previous_position) {
            self.history.pop();
        } else {
            self.history.clear();
        }
        self.index = Some(previous_index);
        Some(PreviousAction::PlayPrevious(previous))
    }

    fn shuffle_upcoming(&mut self, rng: &mut fastrand::Rng) {
        let Some(index) = self.index else {
            return;
        };
        rng.shuffle(&mut self.entries[index + 1..]);
    }

    fn restore_upcoming_order(&mut self) {
        let Some(index) = self.index else {
            return;
        };
        self.entries[index + 1..].sort_by_key(|entry| entry.original_position);
    }

    #[cfg(test)]
    fn paths(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|entry| entry.track.path.to_str().unwrap())
            .collect()
    }

    #[cfg(test)]
    fn set_shuffle_with_seed(&mut self, enabled: bool, seed: u64) {
        self.set_shuffle_with_rng(enabled, &mut fastrand::Rng::with_seed(seed));
    }

    #[cfg(test)]
    fn rebuild_with_seed(&mut self, tracks: &[Track], start_index: usize, seed: u64) {
        self.rebuild_with_rng(tracks, start_index, &mut fastrand::Rng::with_seed(seed));
    }

    #[cfg(test)]
    fn rebuild_shuffled_with_seed(&mut self, tracks: &[Track], seed: u64) {
        self.rebuild_shuffled_with_rng(tracks, &mut fastrand::Rng::with_seed(seed));
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
    fn peek_advance_on_end_matches_advancing_a_clone_across_queue_modes() {
        let tracks = [track(1, "first"), track(2, "second"), track(3, "third")];
        for repeat_cycles in 0..3 {
            for shuffle in [false, true] {
                for position in 0..tracks.len() {
                    let mut queue = QueueState::from_tracks(&tracks, 0);
                    if shuffle {
                        queue.set_shuffle_with_seed(true, 7);
                    }
                    for _ in 0..repeat_cycles {
                        queue.cycle_repeat();
                    }
                    queue.index = Some(position);

                    let peeked = queue.peek_advance_on_end().cloned();
                    let advanced = queue.clone().advance_on_end();

                    assert_eq!(
                        peeked,
                        advanced,
                        "repeat={:?}, shuffle={shuffle}, position={position}",
                        queue.repeat_mode()
                    );
                }
            }
        }

        let mut single = QueueState::from_tracks(&[track(1, "only")], 0);
        single.cycle_repeat();
        assert_eq!(
            single.peek_advance_on_end().cloned(),
            single.clone().advance_on_end()
        );
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
    fn repeat_all_stops_after_one_lap_when_every_entry_fails() {
        let tracks = [track(1, "first"), track(2, "second"), track(3, "third")];
        let mut queue = QueueState::from_tracks(&tracks, 0);
        queue.cycle_repeat();

        assert_eq!(queue.skip_failed(), Some(TrackRef::from(&tracks[1])));
        assert_eq!(queue.skip_failed(), Some(TrackRef::from(&tracks[2])));
        assert!(queue.skip_failed().is_none());
        assert_eq!(queue.skipped_count(), tracks.len());
        assert!(queue.nothing_played());
    }

    #[test]
    fn successful_playback_resets_the_repeat_all_failure_lap() {
        let tracks = [track(1, "bad"), track(2, "good")];
        let mut queue = QueueState::from_tracks(&tracks, 0);
        queue.cycle_repeat();

        assert_eq!(queue.skip_failed(), Some(TrackRef::from(&tracks[1])));
        queue.mark_started();
        assert_eq!(queue.advance(), Some(TrackRef::from(&tracks[0])));
        assert_eq!(queue.skip_failed(), Some(TrackRef::from(&tracks[1])));
        assert!(!queue.nothing_played());
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
    fn position_on_path_prefers_the_next_duplicate() {
        let duplicate = track(1, "duplicate");
        let tracks = [duplicate.clone(), track(2, "middle"), duplicate];
        let mut queue = QueueState::from_tracks(&tracks, 0);

        assert!(queue.position_on_path(&tracks[0].path));

        assert_eq!(queue.index, Some(2));
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
    fn shuffle_keeps_current_stable_and_contains_every_upcoming_entry_once() {
        let tracks = [
            track(1, "current"),
            track(2, "second"),
            track(3, "third"),
            track(4, "fourth"),
            track(5, "fifth"),
        ];
        let mut queue = QueueState::from_tracks(&tracks, 0);

        queue.set_shuffle_with_seed(true, 7);

        assert_eq!(queue.current().unwrap().title, "current");
        let mut shuffled = queue
            .upcoming()
            .map(|(_, track)| track.title.clone())
            .collect::<Vec<_>>();
        assert_eq!(shuffled.len(), 4);
        shuffled.sort();
        assert_eq!(shuffled, ["fifth", "fourth", "second", "third"]);
    }

    #[test]
    fn collection_shuffle_randomizes_the_opener_and_includes_every_track_once() {
        let tracks = [
            track(1, "first"),
            track(2, "second"),
            track(3, "third"),
            track(4, "fourth"),
        ];
        let mut queue = QueueState::default();

        queue.rebuild_shuffled_with_seed(&tracks, 7);

        assert!(queue.shuffle_enabled());
        assert_ne!(queue.current().unwrap().title, "first");
        let mut queued = std::iter::once(queue.current().unwrap().title.as_str())
            .chain(queue.upcoming().map(|(_, track)| track.title.as_str()))
            .collect::<Vec<_>>();
        queued.sort_unstable();
        assert_eq!(queued, ["first", "fourth", "second", "third"]);
    }

    #[test]
    fn previous_walks_true_play_history_through_shuffle() {
        let tracks = [
            track(1, "first"),
            track(2, "second"),
            track(3, "third"),
            track(4, "fourth"),
        ];
        let mut queue = QueueState::from_tracks(&tracks, 0);
        queue.set_shuffle_with_seed(true, 11);
        let first_shuffled = queue.advance().unwrap();
        let second_shuffled = queue.advance().unwrap();

        assert_eq!(
            queue.previous(0),
            Some(PreviousAction::PlayPrevious(first_shuffled.clone()))
        );
        assert_eq!(queue.current(), Some(&first_shuffled));
        assert_eq!(
            queue.previous(0),
            Some(PreviousAction::PlayPrevious(TrackRef::from(&tracks[0])))
        );
        assert_ne!(first_shuffled, second_shuffled);
    }

    #[test]
    fn disabling_shuffle_restores_original_order_after_the_current_track() {
        let tracks = [
            track(1, "first"),
            track(2, "second"),
            track(3, "third"),
            track(4, "fourth"),
            track(5, "fifth"),
        ];
        let mut queue = QueueState::from_tracks(&tracks, 0);
        queue.set_shuffle_with_seed(true, 3);
        let current = queue.advance().unwrap();

        queue.set_shuffle_with_seed(false, 0);

        assert_eq!(queue.current(), Some(&current));
        let restored = queue
            .upcoming()
            .map(|(_, track)| track.title.as_str())
            .collect::<Vec<_>>();
        let expected = tracks
            .iter()
            .skip(1)
            .filter(|track| track.id != current.id)
            .map(|track| track.title.as_deref().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(restored, expected);
    }

    #[test]
    fn repeat_one_replays_natural_end_while_manual_next_advances() {
        let tracks = [track(1, "first"), track(2, "second")];
        let mut queue = QueueState::from_tracks(&tracks, 0);
        queue.cycle_repeat();
        queue.cycle_repeat();
        assert_eq!(queue.repeat_mode(), RepeatMode::One);

        assert_eq!(queue.advance_on_end(), Some(TrackRef::from(&tracks[0])));
        assert_eq!(queue.current().unwrap().title, "first");
        assert_eq!(queue.advance(), Some(TrackRef::from(&tracks[1])));
    }

    #[test]
    fn repeat_all_wraps_next_previous_and_the_upcoming_view() {
        let tracks = [track(1, "first"), track(2, "second"), track(3, "third")];
        let mut queue = QueueState::from_tracks(&tracks, 2);
        queue.cycle_repeat();

        assert_eq!(queue.advance(), Some(TrackRef::from(&tracks[0])));
        assert_eq!(
            queue.previous(0),
            Some(PreviousAction::PlayPrevious(TrackRef::from(&tracks[2])))
        );
        assert_eq!(
            queue
                .upcoming()
                .map(|(_, track)| track.title.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );

        let mut from_first = QueueState::from_tracks(&tracks, 0);
        from_first.cycle_repeat();
        assert_eq!(
            from_first.previous(0),
            Some(PreviousAction::PlayPrevious(TrackRef::from(&tracks[2])))
        );
    }

    #[test]
    fn rebuilding_a_shuffled_queue_keeps_modes_and_uses_only_the_new_selection() {
        let old_tracks = [track(1, "old-1"), track(2, "old-2")];
        let new_tracks = [
            track(3, "new-1"),
            track(4, "new-current"),
            track(5, "new-3"),
            track(6, "new-4"),
        ];
        let mut queue = QueueState::from_tracks(&old_tracks, 0);
        queue.set_shuffle_with_seed(true, 5);
        let _ = queue.advance();

        queue.rebuild_with_seed(&new_tracks, 1, 13);

        assert!(queue.shuffle_enabled());
        assert_eq!(queue.current().unwrap().title, "new-current");
        assert!(queue.paths().iter().all(|path| path.contains("new-")));
        let mut upcoming = queue
            .upcoming()
            .map(|(_, track)| track.title.as_str())
            .collect::<Vec<_>>();
        upcoming.sort_unstable();
        assert_eq!(upcoming, ["new-1", "new-3", "new-4"]);
        assert_eq!(
            queue.previous(0),
            Some(PreviousAction::Restart(TrackRef::from(&new_tracks[1])))
        );
    }

    #[test]
    fn off_mode_rebuild_keeps_the_supplied_order_exactly() {
        let first = [track(1, "first"), track(2, "second")];
        let second = [track(3, "third"), track(4, "fourth"), track(5, "fifth")];
        let mut queue = QueueState::from_tracks(&first, 0);

        queue.rebuild(&second, 1);

        assert!(!queue.shuffle_enabled());
        assert_eq!(
            queue.paths(),
            ["/third.flac", "/fourth.flac", "/fifth.flac"]
        );
        assert_eq!(queue.current().unwrap().title, "fourth");
    }

    #[test]
    fn remove_jump_and_clear_stay_coherent_while_shuffled() {
        let tracks = [
            track(1, "current"),
            track(2, "second"),
            track(3, "third"),
            track(4, "fourth"),
            track(5, "fifth"),
        ];
        let mut queue = QueueState::from_tracks(&tracks, 0);
        queue.set_shuffle_with_seed(true, 17);
        let removed_position = queue.upcoming().next().unwrap().0;
        let removed_title = queue.entries[removed_position - 1].track.title.clone();

        queue.remove_at(removed_position - 1);
        assert_eq!(queue.remaining_count(), 3);
        assert!(
            queue
                .paths()
                .iter()
                .all(|path| !path.contains(&removed_title))
        );

        let (jump_position, jump_track) = queue
            .upcoming()
            .next()
            .map(|(position, track)| (position, track.clone()))
            .unwrap();
        assert_eq!(queue.jump_to(jump_position - 1), Some(jump_track.clone()));
        assert_eq!(queue.current(), Some(&jump_track));
        assert_eq!(
            queue.previous(0),
            Some(PreviousAction::PlayPrevious(TrackRef::from(&tracks[0])))
        );

        let _ = queue.advance();
        let current = queue.current().cloned().unwrap();
        queue.clear_upcoming();
        assert_eq!(queue.remaining_count(), 0);
        assert_eq!(queue.current(), Some(&current));
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
