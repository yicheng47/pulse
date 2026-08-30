use super::*;

const DROPOUT_NOTICE_THRESHOLD: usize = 3;
const DROPOUT_NOTICE_WINDOW: Duration = Duration::from_secs(10);
const DROPOUT_NOTICE_CLEAR_AFTER: Duration = Duration::from_secs(30);

impl Playback {
    pub(crate) fn install_controller(&mut self, device_id: device::DeviceId, exclusive_mode: bool) {
        let controller = PlaybackController::spawn(device_id, exclusive_mode);
        self.sent_next = None;
        self.event_rx = Some(controller.subscribe());
        let command_tx = controller.command_sender();
        if command_tx.send(self.volume_command()).is_err() {
            self.error = Some("Playback engine disconnected.".to_string());
        }
        self.command_tx = Some(command_tx);
        self.controller = Some(controller);
    }

    pub(crate) fn volume_command(&self) -> PlaybackCommand {
        PlaybackCommand::SetVolume {
            level: self.volume_level,
            muted: self.volume_muted,
        }
    }

    pub(crate) fn toggle_volume_mute(&mut self) {
        let muted = !self.volume_muted;
        if let Err(error) = self.update_settings(|settings| settings.volume_muted = muted) {
            self.error = Some(format!(
                "Could not save the volume mute preference: {error}"
            ));

            return;
        }
        self.volume_muted = muted;
        self.send_command(self.volume_command());
    }

    pub(crate) fn set_volume_level(&mut self, level: f32) {
        let level = level.clamp(0.0, 1.0);
        if self.volume_level == level && !self.volume_muted {
            return;
        }
        self.volume_level = level;
        self.volume_muted = false;
        self.send_command(self.volume_command());
    }

    pub(crate) fn persist_volume(&mut self) {
        let volume_level = self.volume_level;
        let volume_muted = self.volume_muted;
        if let Err(error) = self.update_settings(|settings| {
            settings.volume_level = volume_level;
            settings.volume_muted = volume_muted;
        }) {
            self.error = Some(format!("Could not save the volume preference: {error}"));
        }
    }

    pub(crate) fn play_file(&mut self, path: PathBuf) {
        if self.command_tx.is_none() {
            self.initialize_output();
            if self.command_tx.is_none() {
                self.error = Some("No output device is available.".to_string());

                return;
            }
        }

        self.record_play_attempt(&path);
        self.send_command(PlaybackCommand::PlayFile { path });
    }

    /// Tracks what the controller is actually working on, so a device-failure
    /// retry replays the attempted file — `source_path`/`position_ms` still
    /// describe the previous track while a new one is loading.
    pub(crate) fn record_play_attempt(&mut self, path: &Path) {
        self.current_play = Some(PlayAttempt {
            target: RetryTarget {
                path: path.to_path_buf(),
                position_ms: self.pending_seek_ms.unwrap_or(0),
            },
            confirmed: false,
            load: false,
        });
    }

    pub(crate) fn toggle_playback(&mut self) {
        if let Some(command) = self.prepare_toggle_command() {
            self.send_command(command);
        }
    }

    /// Transport play from Idle/Ended/Error dispatches a `PlayFile` restart,
    /// which must be recorded as a fresh attempt like every other dispatch.
    pub(crate) fn prepare_toggle_command(&mut self) -> Option<PlaybackCommand> {
        let command = self.toggle_command()?;
        if let PlaybackCommand::PlayFile { path } = &command {
            self.pending_seek_ms = None;
            let path = path.clone();
            self.record_play_attempt(&path);
        }
        Some(command)
    }

    pub(crate) fn toggle_command(&self) -> Option<PlaybackCommand> {
        match self.playback_state {
            PlaybackState::Playing => Some(PlaybackCommand::Pause),
            PlaybackState::Paused => Some(PlaybackCommand::Resume),
            PlaybackState::Idle | PlaybackState::Ended | PlaybackState::Error => self
                .restart_path()
                .map(|path| PlaybackCommand::PlayFile { path }),
            PlaybackState::Loading | PlaybackState::Stopping => None,
        }
    }

    pub(super) fn can_dispatch_track_navigation(&self) -> bool {
        !matches!(
            self.playback_state,
            PlaybackState::Loading | PlaybackState::Stopping
        )
    }

    /// The file transport Play restarts. In Error state the failed attempt is
    /// the target, keeping Play and Try again in agreement when a new file
    /// failed before any NowPlaying updated `source_path`.
    pub(crate) fn restart_path(&self) -> Option<PathBuf> {
        if self.playback_state == PlaybackState::Error
            && let Some(attempt) = &self.current_play
        {
            return Some(attempt.target.path.clone());
        }
        self.source_path.clone()
    }

    pub(crate) fn send_command(&mut self, command: PlaybackCommand) -> bool {
        let reset_next = matches!(
            &command,
            PlaybackCommand::PlayFile { .. }
                | PlaybackCommand::Load { .. }
                | PlaybackCommand::Stop
                | PlaybackCommand::SetOutputDevice { .. }
                | PlaybackCommand::SetExclusiveMode { .. }
        );
        let resync_next = matches!(&command, PlaybackCommand::PlayFile { .. })
            || matches!(&command, PlaybackCommand::SetExclusiveMode { .. })
                && self.playback_state != PlaybackState::Playing;
        if reset_next {
            self.sent_next = None;
        }
        let Some(command_tx) = &self.command_tx else {
            return false;
        };
        let is_attempt = matches!(
            command,
            PlaybackCommand::PlayFile { .. } | PlaybackCommand::Load { .. }
        );
        if command_tx.send(command).is_err() {
            self.error = Some("Playback engine disconnected.".to_string());
            return false;
        }
        if is_attempt {
            self.open_launch_session_save_gate();
            self.dispatched_plays += 1;
        }
        if resync_next {
            self.sync_next_source();
        }
        true
    }

    pub(crate) fn sync_next_source(&mut self) {
        if !matches!(
            self.playback_state,
            PlaybackState::Loading | PlaybackState::Playing | PlaybackState::Paused
        ) {
            return;
        }
        let next = self.effective_next_track().map(|track| track.path.clone());
        if next == self.sent_next {
            return;
        }
        let command = match &next {
            Some(path) => PlaybackCommand::SetNext { path: path.clone() },
            None => PlaybackCommand::ClearNext,
        };
        let Some(command_tx) = &self.command_tx else {
            return;
        };
        if command_tx.send(command).is_err() {
            self.error = Some("Playback engine disconnected.".to_string());
        } else {
            self.sent_next = next;
        }
    }

    pub(crate) fn drain_events(&mut self) -> bool {
        let now = Instant::now();
        let dropout_notice_changed = self.clear_expired_dropout_notice_at(now);
        let settings_write_finished = self.apply_settings_write_results();
        let mut changed = self.pending_saved_output_device_uid.is_some()
            || dropout_notice_changed
            || settings_write_finished;
        loop {
            let event = match self.event_rx.as_ref().map(Receiver::try_recv) {
                Some(Ok(event)) => event,
                Some(Err(TryRecvError::Empty)) | None => break,
                Some(Err(TryRecvError::Disconnected)) => {
                    self.event_rx = None;
                    self.error = Some("Playback engine disconnected.".to_string());
                    changed = true;
                    break;
                }
            };
            changed = true;
            if let Some(track) = self.handle_event_at(event, now) {
                self.play_queue_track(track);
            }
            if self.playback_state == PlaybackState::Playing
                && let Some(position_ms) = self.pending_seek_ms.take()
            {
                self.send_command(PlaybackCommand::Seek { position_ms });
            }
        }
        self.persist_pending_saved_output_device_uid();
        changed
    }

    /// Applies one controller event; returns the next queue entry to play
    /// when the event calls for an advance (track ended or failed).
    #[cfg(test)]
    pub(crate) fn handle_event(&mut self, event: PlaybackEvent) -> Option<TrackRef> {
        self.handle_event_at(event, Instant::now())
    }

    pub(super) fn handle_event_at(
        &mut self,
        event: PlaybackEvent,
        now: Instant,
    ) -> Option<TrackRef> {
        match event {
            PlaybackEvent::StateChanged(state) => {
                self.playback_state = state;
                if state == PlaybackState::Playing
                    && let Some(attempt) = &mut self.current_play
                {
                    attempt.load = false;
                }
                if state == PlaybackState::Idle {
                    self.clear_dropout_notice_tracking();
                }
                if matches!(
                    state,
                    PlaybackState::Idle | PlaybackState::Ended | PlaybackState::Error
                ) {
                    self.sent_next = None;
                } else if state == PlaybackState::Playing {
                    self.sync_next_source();
                }
                if state == PlaybackState::Paused {
                    self.persist_session_or_record_error();
                }
            }
            PlaybackEvent::NowPlaying { source, format } => {
                let loaded_for_restore = self
                    .current_play
                    .as_ref()
                    .is_some_and(|attempt| attempt.load);
                match &mut self.current_play {
                    Some(attempt) if attempt.target.path == source.path => {
                        attempt.confirmed = true;
                    }
                    // A mismatched NowPlaying against an unconfirmed attempt
                    // is a stale event from a superseded play — ignore it
                    // entirely so display state and the retry target stay on
                    // the dispatched attempt.
                    Some(attempt) if !attempt.confirmed => return None,
                    _ => {
                        self.current_play = Some(PlayAttempt {
                            target: RetryTarget {
                                path: source.path.clone(),
                                position_ms: 0,
                            },
                            confirmed: true,
                            load: false,
                        });
                    }
                }
                self.start_dropout_track();
                self.apply_playing_source(source, format, !loaded_for_restore);
                if !loaded_for_restore {
                    self.sync_next_source();
                    self.persist_session_or_record_error();
                }
            }
            PlaybackEvent::Advanced {
                attempt,
                source,
                format,
            } => {
                if attempt != self.dispatched_plays {
                    return None;
                }
                self.start_dropout_track();
                self.sent_next = None;
                let expected_next = self.effective_next_track().map(|track| track.path.clone());
                if expected_next.as_ref() == Some(&source.path) {
                    let advanced = self.queue.advance_on_end();
                    if advanced
                        .as_ref()
                        .is_some_and(|track| track.path != source.path)
                    {
                        self.queue.position_on_path(&source.path);
                    }
                } else {
                    self.queue.position_on_path(&source.path);
                }
                self.refresh_queue_snapshot();
                self.current_play = Some(PlayAttempt {
                    target: RetryTarget {
                        path: source.path.clone(),
                        position_ms: 0,
                    },
                    confirmed: true,
                    load: false,
                });
                self.playback_state = PlaybackState::Playing;
                self.apply_playing_source(source, format, true);
                self.sync_next_source();
                self.persist_session_or_record_error();
            }
            PlaybackEvent::Position {
                position_ms,
                duration_ms,
                dropout_frames,
            } => {
                self.position_ms = position_ms;
                self.duration_ms = duration_ms;
                self.dropout_frames = dropout_frames;
                if let Some(attempt) = &mut self.current_play
                    && attempt.confirmed
                {
                    attempt.target.position_ms = position_ms;
                }
                self.persist_position_if_due(position_ms);
            }
            PlaybackEvent::Dropout {
                attempt,
                frames: _,
                cumulative_frames,
            } => {
                if attempt != self.dispatched_plays {
                    return None;
                }
                self.dropout_frames = cumulative_frames;
                self.record_dropout_at(now);
            }
            PlaybackEvent::OutputDeviceChanged {
                device_id,
                exclusive_mode,
            } => {
                self.complete_output_device_change(device_id, exclusive_mode);
                self.sync_next_source();
            }
            PlaybackEvent::ExclusiveModeFallback { device_id } => {
                self.playback_exclusive_mode = false;
                let device_name = self
                    .pending_device_change
                    .as_ref()
                    .filter(|pending| pending.device.id == device_id)
                    .map(|pending| pending.device.name.as_str())
                    .or_else(|| {
                        self.active_device
                            .as_ref()
                            .filter(|device| device.id == device_id)
                            .map(|device| device.name.as_str())
                    })
                    .or_else(|| {
                        self.devices
                            .iter()
                            .find(|device| device.id == device_id)
                            .map(|device| device.name.as_str())
                    })
                    .unwrap_or("The output device");
                self.notice = Some(PlaybackNotice::ExclusiveFallback {
                    text: format!(
                        "{device_name} could not start in exclusive mode. Playback continues in shared mode."
                    ),
                });
            }
            PlaybackEvent::HardwareVolume { level, muted } => {
                self.volume_level = level;
                self.volume_muted = muted;
                if let Err(error) = self.update_settings(|settings| {
                    settings.volume_level = level;
                    settings.volume_muted = muted;
                }) {
                    self.error = Some(format!("Could not save the volume preference: {error}"));
                }
            }
            PlaybackEvent::Ended { attempt } => {
                if attempt != self.dispatched_plays {
                    return None;
                }
                self.playback_state = PlaybackState::Ended;
                if let Some(duration_ms) = self.duration_ms {
                    self.position_ms = duration_ms;
                }
                let next = self.queue.advance_on_end();
                self.refresh_queue_snapshot();
                return next;
            }
            PlaybackEvent::CommandRejected {
                command: "SetNext" | "ClearNext",
                ..
            } => {}
            PlaybackEvent::CommandRejected { command, state } => {
                self.error = Some(format!(
                    "{command} is unavailable while playback is {}.",
                    playback_state_label(state)
                ));
            }
            PlaybackEvent::NextRejected {
                attempt,
                path,
                message: _,
            } => {
                if attempt != self.dispatched_plays {
                    return None;
                }
                self.sent_next = Some(path.clone());
                if let Some(track) = self.queue.track_by_path(&path).cloned() {
                    let (newly_rejected, reason) = if path.is_file() {
                        (
                            self.rejected_next_track_ids.insert(track.id),
                            SkipReason::Undecodable,
                        )
                    } else {
                        let inserted = self.missing_track_ids.insert(track.id);
                        self.refresh_missing_track_ids_snapshot();
                        (inserted, SkipReason::Missing)
                    };
                    if newly_rejected {
                        self.queue.note_skipped_ahead();
                        self.refresh_queue_snapshot();
                        self.note_skip(&track, reason);
                    }
                }
                self.sync_next_source();
            }
            PlaybackEvent::Error {
                attempt,
                kind,
                message,
            } => {
                // Output-device change failures are not play-scoped; handle
                // them before the attempt staleness guard.
                if let Some(pending) = self.pending_device_change.take() {
                    self.device_message = Some(DeviceMessage {
                        text: format!("Could not switch to {}: {message}", pending.device.name),
                        is_error: true,
                    });
                    self.error = Some(message);
                    return None;
                }
                if attempt != self.dispatched_plays {
                    return None;
                }
                if self.playback_state == PlaybackState::Idle
                    && self
                        .current_play
                        .as_ref()
                        .is_some_and(|attempt| attempt.load && !attempt.confirmed)
                {
                    self.error = None;
                    return None;
                }
                // Advisory: teardown already reached Idle/Ended, playback is
                // not stopping because of this (see PlaybackEvent::Error docs).
                if matches!(
                    self.playback_state,
                    PlaybackState::Idle | PlaybackState::Ended
                ) {
                    self.error = Some(message);
                    return None;
                }
                match kind {
                    PlaybackErrorKind::Track if self.queue.current().is_some() => {
                        self.error = None;
                        return self.handle_track_failure();
                    }
                    PlaybackErrorKind::Track => {
                        // Before NowPlaying, `title` still describes the
                        // previous row; the attempted file is the one that
                        // failed.
                        let name = self
                            .current_play
                            .as_ref()
                            .map(|attempt| track_title(&attempt.target.path))
                            .unwrap_or_else(|| self.title.clone());
                        self.notice = Some(PlaybackNotice::Stopped {
                            text: format!("Could not play “{name}” — {message}."),
                        });
                        self.error = Some(message);
                    }
                    PlaybackErrorKind::Device { hog_pid } => {
                        self.handle_device_failure(&message, hog_pid);
                        self.error = Some(message);
                    }
                }
            }
        }
        None
    }

    fn record_dropout_at(&mut self, now: Instant) {
        while self
            .recent_dropouts
            .front()
            .is_some_and(|dropout| now.duration_since(*dropout) > DROPOUT_NOTICE_WINDOW)
        {
            self.recent_dropouts.pop_front();
        }
        self.recent_dropouts.push_back(now);
        self.last_dropout_at = Some(now);
        if self.recent_dropouts.len() >= DROPOUT_NOTICE_THRESHOLD {
            self.notice = Some(PlaybackNotice::Dropouts {
                text: "Playback is dropping out — the source can't keep up.".to_string(),
            });
        }
    }

    pub(super) fn clear_expired_dropout_notice_at(&mut self, now: Instant) -> bool {
        let Some(last_dropout_at) = self.last_dropout_at else {
            return false;
        };
        if now.duration_since(last_dropout_at) < DROPOUT_NOTICE_CLEAR_AFTER {
            return false;
        }
        let had_dropout_notice = matches!(self.notice, Some(PlaybackNotice::Dropouts { .. }));
        self.clear_dropout_notice_tracking();
        had_dropout_notice
    }

    fn start_dropout_track(&mut self) {
        self.dropout_frames = 0;
        self.clear_dropout_notice_tracking();
    }

    fn clear_dropout_notice_tracking(&mut self) {
        self.recent_dropouts.clear();
        self.last_dropout_at = None;
        if matches!(self.notice, Some(PlaybackNotice::Dropouts { .. })) {
            self.notice = None;
        }
    }

    pub(crate) fn complete_output_device_change(
        &mut self,
        device_id: device::DeviceId,
        playback_exclusive_mode: bool,
    ) {
        let Some(pending) = self.pending_device_change.take() else {
            return;
        };
        if pending.device.id != device_id {
            return;
        }

        let persist = pending.persist;
        let output_device =
            self.apply_completed_output_device_change(pending, playback_exclusive_mode);

        if persist {
            self.pending_saved_output_device_uid = Some(output_device.uid);
        }
    }

    pub(crate) fn persist_pending_saved_output_device_uid(&mut self) {
        let Some(device_uid) = self.pending_saved_output_device_uid.take() else {
            return;
        };
        match self.update_settings(|settings| {
            settings.saved_output_device_uid = Some(device_uid.clone());
        }) {
            Ok(_) => {}
            Err(error) => {
                let device_name = self
                    .active_device
                    .as_ref()
                    .map(|device| device.name.as_str())
                    .unwrap_or("the output device");
                self.device_message = Some(DeviceMessage {
                    text: format!("Could not save {device_name} as the output device: {error}"),
                    is_error: true,
                });
            }
        }
    }

    pub(super) fn apply_completed_output_device_change(
        &mut self,
        pending: PendingDeviceChange,
        playback_exclusive_mode: bool,
    ) -> device::Device {
        let PendingDeviceChange {
            device: output_device,
            success_message,
            capabilities,
            default_exclusive_mode,
            exclusive_mode,
            ..
        } = pending;
        self.active_device = Some(output_device.clone());
        self.device_message = success_message;
        self.default_exclusive_mode = default_exclusive_mode;
        self.exclusive_mode = exclusive_mode;
        self.playback_exclusive_mode = playback_exclusive_mode;
        self.apply_device_capabilities_result(&output_device, capabilities);
        output_device
    }

    pub(crate) fn seek(&mut self, position_ms: u64) {
        self.position_ms = position_ms;
        self.send_command(PlaybackCommand::Seek { position_ms });
    }

    pub(crate) fn shutdown(&mut self) {
        if let Err(error) = self.flush_settings_writer() {
            self.record_settings_error(error);
        }
        self.apply_settings_write_results();
        self.persist_session_or_record_error();
        if let Err(error) = self.flush_settings_writer() {
            self.record_settings_error(error);
        }
        self.apply_settings_write_results();
        self.sent_next = None;
        self.event_rx = None;
        self.command_tx = None;
        self.controller = None;
    }

    fn apply_playing_source(
        &mut self,
        source: PlayableSource,
        format: PcmFormat,
        mark_started: bool,
    ) {
        if let Some(track) = self
            .queue
            .current()
            .filter(|track| track.path == source.path)
            .cloned()
        {
            self.apply_track_context(&track);
            if mark_started {
                self.queue.mark_started();
            }
            self.refresh_queue_snapshot();
            self.missing_track_ids.remove(&track.id);
            self.rejected_next_track_ids.remove(&track.id);
            self.refresh_missing_track_ids_snapshot();
        } else {
            self.title = track_title(&source.path);
            self.secondary = track_secondary(&source.path);
        }
        self.source_path = Some(source.path);
        self.duration_ms = source.duration_ms;
        self.format = Some(format);
        self.position_ms = 0;
        self.error = None;
    }

    fn effective_next_track(&self) -> Option<&TrackRef> {
        self.queue.peek_advance_on_end_skipping(|track| {
            self.missing_track_ids.contains(&track.id)
                || self.rejected_next_track_ids.contains(&track.id)
        })
    }
}
