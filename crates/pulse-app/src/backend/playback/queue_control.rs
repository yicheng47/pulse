use super::*;

impl Playback {
    /// Marks go stale whenever library rows change under them — a completed
    /// scan re-verifies file presence, and a removed root recycles track ids.
    pub(crate) fn clear_missing_marks(&mut self) {
        self.missing_track_ids.clear();
        self.rejected_next_track_ids.clear();
        self.refresh_missing_track_ids_snapshot();
        self.sync_next_source();
    }

    #[cfg(test)]
    pub(super) fn is_track_missing(&self, track_id: TrackId) -> bool {
        self.missing_track_ids.contains(&track_id)
    }

    /// Rows for these tracks were deleted; drop their marks so a future row
    /// recycling one of the ids cannot inherit a stale missing flag.
    pub(crate) fn remove_missing_marks(&mut self, track_ids: &[TrackId]) {
        for track_id in track_ids {
            self.missing_track_ids.remove(track_id);
            self.rejected_next_track_ids.remove(track_id);
        }
        self.refresh_missing_track_ids_snapshot();
        self.sync_next_source();
    }

    pub(crate) fn dismiss_notice(&mut self) {
        self.notice = None;
        self.retry = None;
        self.recent_dropouts.clear();
        self.last_dropout_at = None;
    }

    pub(crate) fn take_toasts(&mut self) -> Vec<PlaybackToast> {
        self.toasts.drain(..).collect()
    }

    pub(crate) fn play_library_tracks(&mut self, tracks: &[Track], start_index: usize) {
        self.queue.rebuild(tracks, start_index);
        self.refresh_queue_snapshot();
        self.notice = None;
        self.retry = None;
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        self.play_queue_track(track);
    }

    pub(crate) fn play_library_tracks_shuffled(&mut self, tracks: &[Track]) {
        self.queue.rebuild_shuffled(tracks);
        self.refresh_queue_snapshot();
        self.notice = None;
        self.retry = None;
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        self.play_queue_track(track);
    }

    pub(crate) fn select_library_tracks(&mut self, tracks: &[Track], start_index: usize) {
        let mut queue = self.queue.clone();
        queue.rebuild(tracks, start_index);
        let Some(track) = queue.current().cloned() else {
            return;
        };
        if self.apply_track_selection(&track) {
            self.queue = queue;
            self.refresh_queue_snapshot();
        }
    }

    pub(crate) fn handle_drop(&mut self, paths: &[PathBuf]) {
        if paths.len() != 1 {
            self.error = Some("Drop one audio file at a time.".to_string());

            return;
        }

        let path = &paths[0];
        if !path.is_file() {
            self.error = Some("Drop a file, not a folder.".to_string());

            return;
        }
        if !is_supported_audio(path) {
            self.error = Some(
                "Unsupported file. Pulse accepts FLAC, ALAC (.m4a), AIFF, and WAV.".to_string(),
            );

            return;
        }

        self.error = None;
        self.notice = None;
        self.retry = None;
        self.pending_seek_ms = None;
        self.cover_art_path = None;
        self.queue.clear();
        self.refresh_queue_snapshot();
        self.play_file(path.clone());
    }

    pub(crate) fn next_track(&mut self) {
        if !self.can_dispatch_track_navigation() {
            return;
        }
        self.notice = None;
        self.retry = None;
        let track = self.queue.advance();
        self.refresh_queue_snapshot();
        if let Some(track) = track {
            self.play_queue_track(track);
        }
    }

    pub(crate) fn toggle_shuffle(&mut self) {
        self.queue.toggle_shuffle();
        self.refresh_queue_snapshot();
        self.sync_next_source();
    }

    pub(crate) fn cycle_repeat(&mut self) {
        self.queue.cycle_repeat();
        self.refresh_queue_snapshot();
        self.sync_next_source();
    }

    pub(crate) fn previous_track(&mut self) {
        if !self.can_dispatch_track_navigation() {
            return;
        }
        self.notice = None;
        self.retry = None;
        let action = self.queue.previous(self.position_ms);
        self.refresh_queue_snapshot();
        match action {
            Some(PreviousAction::Restart(track) | PreviousAction::PlayPrevious(track)) => {
                self.play_queue_track(track);
            }
            None => {
                if let Some(path) = self.source_path.clone() {
                    self.play_file(path);
                }
            }
        }
    }

    pub(crate) fn apply_track_selection(&mut self, track: &TrackRef) -> bool {
        if matches!(
            self.playback_state,
            PlaybackState::Loading
                | PlaybackState::Playing
                | PlaybackState::Paused
                | PlaybackState::Stopping
        ) {
            return false;
        }
        self.apply_track_context(track);
        self.playback_state = PlaybackState::Idle;
        self.format = None;
        self.position_ms = 0;
        self.duration_ms = None;
        self.error = None;
        true
    }

    pub(crate) fn apply_track_context(&mut self, track: &TrackRef) {
        self.title = track.title.clone();
        self.secondary = track.secondary();
        self.source_path = Some(track.path.clone());
        self.cover_art_path = track.cover_art_path.clone();
    }

    pub(crate) fn play_queue_track(&mut self, track: TrackRef) {
        match self.prepare_queue_play(track) {
            Some(path) => self.play_file(path),
            // The queue exhausted through unplayable entries. A user-initiated
            // jump or Next can land here while the superseded track is still
            // audible; stop it so the error toast is truthful.
            None => {
                if self.active_playback_needs_stop() {
                    self.send_command(PlaybackCommand::Stop);
                }
            }
        }
    }

    /// True when the engine is still producing (or about to produce) audio
    /// for a track the queue has already moved past.
    pub(crate) fn active_playback_needs_stop(&self) -> bool {
        matches!(
            self.playback_state,
            PlaybackState::Loading | PlaybackState::Playing | PlaybackState::Paused
        )
    }

    /// Cx-free half of a queue dispatch: resolves the skip-and-report path
    /// for missing files, applies the display context, and returns the path
    /// to hand to `play_file`.
    pub(crate) fn prepare_queue_play(&mut self, track: TrackRef) -> Option<PathBuf> {
        self.pending_seek_ms = None;
        let track = self.next_playable(track)?;
        self.apply_track_context(&track);
        self.error = None;
        Some(track.path)
    }

    /// Queue-popover row click: the index moves to the clicked entry and it
    /// plays; nothing is dropped. A jump to a missing entry follows the
    /// existing skip-and-report path inside `play_queue_track`.
    pub(crate) fn jump_to_queue_entry(&mut self, index: usize) {
        self.notice = None;
        self.retry = None;
        let track = self.queue.jump_to(index);
        self.refresh_queue_snapshot();
        if let Some(track) = track {
            self.play_queue_track(track);
        }
    }

    pub(crate) fn remove_queue_entry(&mut self, index: usize) {
        self.queue.remove_at(index);
        self.refresh_queue_snapshot();
        self.sync_next_source();
    }

    pub(crate) fn clear_upcoming_queue(&mut self) {
        self.queue.clear_upcoming();
        self.refresh_queue_snapshot();
        self.sync_next_source();
    }

    pub(crate) fn next_playable(&mut self, first: TrackRef) -> Option<TrackRef> {
        self.next_playable_for(first, false)
    }

    pub(crate) fn next_auto_playable(&mut self, first: TrackRef) -> Option<TrackRef> {
        self.next_playable_for(first, true)
    }

    fn next_playable_for(&mut self, first: TrackRef, skip_unsafe_dsd: bool) -> Option<TrackRef> {
        let mut candidate = first;
        loop {
            let dsd_error = skip_unsafe_dsd
                .then(|| {
                    dsd_playback_error_with_sample_rate(
                        &candidate.path,
                        candidate.sample_rate_hz,
                        self.dsd_gate_engine_kind(),
                        self.device_capabilities,
                    )
                })
                .flatten();
            if candidate.path.is_file() && dsd_error.is_none() {
                return Some(candidate);
            }
            let reason = if let Some(error) = dsd_error {
                Err(error)
            } else {
                self.missing_track_ids.insert(candidate.id);
                self.refresh_missing_track_ids_snapshot();
                Ok(SkipReason::Missing)
            };
            let next = self.queue.skip_failed();
            self.refresh_queue_snapshot();
            match next {
                Some(next) => {
                    match reason {
                        Ok(reason) => self.note_skip(&candidate, reason),
                        Err(error) => self.note_dsd_skip(&candidate, &error),
                    }
                    candidate = next;
                }
                None => {
                    self.note_queue_stopped(&candidate);
                    return None;
                }
            }
        }
    }

    /// A track-scoped playback failure: mark the entry if its file is gone,
    /// then hand back the next entry to try, or report why the queue stopped.
    pub(crate) fn handle_track_failure(&mut self) -> Option<TrackRef> {
        let failed = self.queue.current().cloned()?;
        let reason = if failed.path.is_file() {
            SkipReason::Undecodable
        } else {
            self.missing_track_ids.insert(failed.id);
            self.refresh_missing_track_ids_snapshot();
            SkipReason::Missing
        };
        let next = self.queue.skip_failed();
        self.refresh_queue_snapshot();
        match next {
            Some(next) => {
                self.note_skip(&failed, reason);
                Some(next)
            }
            None => {
                self.note_queue_stopped(&failed);
                None
            }
        }
    }

    pub(super) fn note_skip(&mut self, track: &TrackRef, reason: SkipReason) {
        let text = if self.queue.skipped_count() > 1 {
            format!(
                "Skipped {} tracks that could not be played.",
                self.queue.skipped_count()
            )
        } else {
            let reason = match reason {
                SkipReason::Missing => "its file is missing",
                SkipReason::Undecodable => "its file could not be decoded",
            };
            format!("Skipped “{}” — {reason}.", track.title)
        };
        if self
            .toasts
            .back()
            .is_some_and(|toast| toast.title == "Track skipped")
        {
            self.toasts.pop_back();
        }
        self.toasts
            .push_back(PlaybackToast::warning("Track skipped", text));
    }

    pub(super) fn note_dsd_skip(&mut self, track: &TrackRef, error: &DsdPlaybackError) {
        if self
            .toasts
            .back()
            .is_some_and(|toast| toast.title == "DSD track skipped")
        {
            self.toasts.pop_back();
        }
        self.toasts.push_back(PlaybackToast::warning(
            "DSD track skipped",
            format!("Skipped “{}” — {}.", track.title, error.title()),
        ));
    }

    pub(super) fn note_dsd_skips(&mut self, tracks: &[TrackRef]) {
        let body = match tracks {
            [track] => format!(
                "Skipped “{}” — it can't play on the active output.",
                track.title
            ),
            tracks => format!(
                "Skipped {} DSD tracks that can't play on the active output.",
                tracks.len()
            ),
        };
        self.toasts
            .push_back(PlaybackToast::warning("DSD track skipped", body));
    }

    pub(crate) fn note_queue_stopped(&mut self, last: &TrackRef) {
        let text = if self.queue.nothing_played() && self.queue.skipped_count() > 1 {
            "Playback stopped — none of the queued tracks could be played.".to_string()
        } else if self.queue.skipped_count() > 1 {
            format!(
                "Playback stopped — {} tracks could not be played.",
                self.queue.skipped_count()
            )
        } else {
            format!("Playback stopped — “{}” could not be played.", last.title)
        };
        self.toasts
            .push_back(PlaybackToast::error("Playback stopped", text));
    }

    pub(crate) fn handle_device_failure(&mut self, message: &str, hog_pid: Option<i32>) {
        let device_name = self
            .active_device
            .as_ref()
            .map(|device| device.name.clone())
            .unwrap_or_else(|| "the output device".to_string());
        let text = match hog_pid {
            Some(pid) => {
                let app = app_name_for_pid(pid).unwrap_or_else(|| "Another app".to_string());
                format!(
                    "{app} is using {device_name} exclusively. Quit it or choose another output, then try again."
                )
            }
            None => format!("Playback stopped on {device_name}: {message}"),
        };
        self.retry = self
            .current_play
            .as_ref()
            .map(|attempt| attempt.target.clone());
        self.notice = Some(PlaybackNotice::DeviceFailure { text });
    }

    pub(crate) fn retry_playback(&mut self) {
        let Some(retry) = self.retry.take() else {
            return;
        };
        self.notice = None;
        self.error = None;
        // Re-resolve the output first: after a disconnect the saved device is
        // gone and refresh falls back to the system default visibly.
        self.refresh_output_devices();
        self.pending_seek_ms = (retry.position_ms > 0).then_some(retry.position_ms);
        self.play_file(retry.path);
    }
}
