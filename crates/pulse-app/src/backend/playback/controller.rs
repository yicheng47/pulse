use super::*;

impl Playback {
    pub(crate) fn install_controller(&mut self, device_id: device::DeviceId, exclusive_mode: bool) {
        let controller = PlaybackController::spawn(device_id, exclusive_mode);
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
            gain: volume_gain_for_level(self.volume_level),
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
        if self.controller.is_none() {
            self.initialize_output();
            if self.controller.is_none() {
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

    pub(crate) fn send_command(&mut self, command: PlaybackCommand) {
        let Some(command_tx) = &self.command_tx else {
            return;
        };
        let is_play = matches!(command, PlaybackCommand::PlayFile { .. });
        if command_tx.send(command).is_err() {
            self.error = Some("Playback engine disconnected.".to_string());
        } else if is_play {
            self.dispatched_plays += 1;
        }
    }

    pub(crate) fn drain_events(&mut self) -> bool {
        let mut changed = self.pending_saved_output_device_uid.is_some();
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
            if let Some(track) = self.handle_event(event) {
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
    pub(crate) fn handle_event(&mut self, event: PlaybackEvent) -> Option<TrackRef> {
        match event {
            PlaybackEvent::StateChanged(state) => {
                self.playback_state = state;
            }
            PlaybackEvent::NowPlaying { source, format } => {
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
                        });
                    }
                }
                if let Some(track) = self
                    .queue
                    .current()
                    .filter(|track| track.path == source.path)
                    .cloned()
                {
                    self.apply_track_context(&track);
                    self.queue.mark_started();
                    self.refresh_queue_snapshot();
                    self.missing_track_ids.remove(&track.id);
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
            PlaybackEvent::Position {
                position_ms,
                duration_ms,
            } => {
                self.position_ms = position_ms;
                self.duration_ms = duration_ms;
                if let Some(attempt) = &mut self.current_play
                    && attempt.confirmed
                {
                    attempt.target.position_ms = position_ms;
                }
            }
            PlaybackEvent::OutputDeviceChanged {
                device_id,
                exclusive_mode,
            } => {
                self.complete_output_device_change(device_id, exclusive_mode);
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
            PlaybackEvent::CommandRejected { command, state } => {
                self.error = Some(format!(
                    "{command} is unavailable while playback is {}.",
                    playback_state_label(state)
                ));
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
        self.event_rx = None;
        self.command_tx = None;
        self.controller = None;
    }
}
