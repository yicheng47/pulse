use super::*;

const POSITION_SAVE_INTERVAL_MS: u64 = 5_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SessionSaveCadence {
    last_saved_position_ms: u64,
}

impl SessionSaveCadence {
    pub(super) fn new(position_ms: u64) -> Self {
        Self {
            last_saved_position_ms: position_ms,
        }
    }

    fn should_save_progress(self, position_ms: u64) -> bool {
        position_ms < self.last_saved_position_ms
            || position_ms - self.last_saved_position_ms >= POSITION_SAVE_INTERVAL_MS
    }

    fn record_saved(&mut self, position_ms: u64) {
        self.last_saved_position_ms = position_ms;
    }
}

struct ResolvedSessionQueue {
    tracks: Vec<Track>,
    original_positions: Vec<usize>,
    current_index: usize,
    position_ms: u64,
}

impl Playback {
    pub(crate) fn launch_session(&self) -> Option<SessionState> {
        self.desired_settings().session.clone()
    }

    pub(crate) fn set_session_route(&mut self, route: SessionRoute) {
        if self.launch_session_pending {
            return;
        }
        let mut session = self.session_snapshot();
        session.route = route;
        if let Err(error) =
            self.update_settings_in_background(|settings| settings.session = Some(session))
        {
            self.record_settings_error(error);
        } else {
            self.session_save_cadence.record_saved(self.position_ms);
        }
    }

    pub(crate) fn restore_session(
        &mut self,
        session: &SessionState,
        resolved_tracks: Vec<Option<Track>>,
    ) {
        self.open_launch_session_save_gate();
        if self.playback_state != PlaybackState::Idle || self.current_play.is_some() {
            self.persist_session_or_record_error();
            return;
        }
        let Some(restored) = resolve_session_queue(session, &resolved_tracks) else {
            self.queue.clear();
            self.refresh_queue_snapshot();
            self.persist_session_or_record_error();
            return;
        };

        self.queue.restore(
            &restored.tracks,
            &restored.original_positions,
            restored.current_index,
            session.shuffle_enabled,
            session.repeat_mode,
        );
        self.refresh_queue_snapshot();
        let track = self
            .queue
            .current()
            .cloned()
            .expect("restored queue has a current track");
        self.apply_track_context(&track);
        self.playback_state = PlaybackState::Idle;
        self.format = None;
        self.position_ms = restored.position_ms;
        self.duration_ms = track.duration_ms;
        self.dropout_frames = 0;
        self.error = None;
        self.notice = None;
        self.sent_next = None;
        self.pending_seek_ms = None;
        self.persist_session_or_record_error();

        self.current_play = Some(PlayAttempt {
            target: RetryTarget {
                path: track.path.clone(),
                position_ms: restored.position_ms,
            },
            confirmed: false,
            load: true,
        });
        self.send_command(PlaybackCommand::Load {
            path: track.path,
            position_ms: restored.position_ms,
        });
    }

    pub(crate) fn abandon_launch_session_restore(&mut self) {}

    pub(super) fn open_launch_session_save_gate(&mut self) {
        self.launch_session_pending = false;
    }

    pub(super) fn persist_position_if_due(&mut self, position_ms: u64) {
        if self.playback_state == PlaybackState::Playing
            && self.session_save_cadence.should_save_progress(position_ms)
        {
            self.persist_session_or_record_error();
        }
    }

    pub(super) fn persist_session_or_record_error(&mut self) {
        if self.launch_session_pending {
            return;
        }
        let session = self.session_snapshot();
        match self.update_settings_in_background(|settings| settings.session = Some(session)) {
            Ok(_) => self.session_save_cadence.record_saved(self.position_ms),
            Err(error) => self.record_settings_error(error),
        }
    }

    fn session_snapshot(&self) -> SessionState {
        let current_index = self.queue.current_index();
        SessionState {
            version: super::super::settings::SESSION_STATE_VERSION,
            queue_track_ids: self.queue.track_ids(),
            queue_original_positions: self.queue.original_positions(),
            current_index,
            position_ms: current_index.map_or(0, |_| self.position_ms),
            shuffle_enabled: self.queue.shuffle_enabled(),
            repeat_mode: self.queue.repeat_mode(),
            route: self
                .desired_settings()
                .session
                .as_ref()
                .map(|session| session.route.clone())
                .unwrap_or_default(),
        }
    }
}

fn resolve_session_queue(
    session: &SessionState,
    resolved_tracks: &[Option<Track>],
) -> Option<ResolvedSessionQueue> {
    let saved_current = session.current_index?;
    let current_survives = resolved_tracks
        .get(saved_current)
        .is_some_and(Option::is_some);
    let target_original_index = if current_survives {
        Some(saved_current)
    } else {
        resolved_tracks
            .iter()
            .enumerate()
            .skip(saved_current.saturating_add(1))
            .find_map(|(index, track)| track.as_ref().map(|_| index))
            .or_else(|| {
                resolved_tracks
                    .iter()
                    .enumerate()
                    .take(saved_current)
                    .rev()
                    .find_map(|(index, track)| track.as_ref().map(|_| index))
            })
    }?;
    let current_index = resolved_tracks[..target_original_index]
        .iter()
        .filter(|track| track.is_some())
        .count();
    let (tracks, original_positions) = resolved_tracks
        .iter()
        .zip(&session.queue_original_positions)
        .filter_map(|(track, original_position)| {
            track
                .as_ref()
                .map(|track| (track.clone(), *original_position))
        })
        .unzip();
    Some(ResolvedSessionQueue {
        tracks,
        original_positions,
        current_index,
        position_ms: if current_survives {
            session.position_ms
        } else {
            0
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::RepeatMode;

    fn track(id: TrackId) -> Track {
        Track {
            id,
            storage_root_id: 1,
            path: PathBuf::from(format!("/{id}.flac")),
            title: Some(format!("Track {id}")),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            album_artist: None,
            year: None,
            genre: None,
            track_number: None,
            disc_number: None,
            duration_ms: Some(60_000),
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

    fn session(current_index: usize) -> SessionState {
        SessionState {
            queue_track_ids: vec![1, 2, 3],
            queue_original_positions: vec![0, 2, 1],
            current_index: Some(current_index),
            position_ms: 12_345,
            shuffle_enabled: true,
            repeat_mode: RepeatMode::All,
            ..SessionState::default()
        }
    }

    #[test]
    fn restore_preserves_an_all_present_queue_and_position() {
        let resolved = [Some(track(1)), Some(track(2)), Some(track(3))];
        let restored = resolve_session_queue(&session(1), &resolved).unwrap();

        assert_eq!(
            restored
                .tracks
                .iter()
                .map(|track| track.id)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(restored.current_index, 1);
        assert_eq!(restored.position_ms, 12_345);
        assert_eq!(restored.original_positions, [0, 2, 1]);
    }

    #[test]
    fn restore_drops_deleted_non_current_tracks_without_reordering() {
        let resolved = [None, Some(track(2)), Some(track(3))];
        let restored = resolve_session_queue(&session(1), &resolved).unwrap();

        assert_eq!(
            restored
                .tracks
                .iter()
                .map(|track| track.id)
                .collect::<Vec<_>>(),
            [2, 3]
        );
        assert_eq!(restored.current_index, 0);
        assert_eq!(restored.position_ms, 12_345);
        assert_eq!(restored.original_positions, [2, 1]);
    }

    #[test]
    fn restore_uses_the_next_surviving_track_at_zero_when_current_was_deleted() {
        let resolved = [Some(track(1)), None, Some(track(3))];
        let restored = resolve_session_queue(&session(1), &resolved).unwrap();

        assert_eq!(
            restored
                .tracks
                .iter()
                .map(|track| track.id)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(restored.current_index, 1);
        assert_eq!(restored.position_ms, 0);
        assert_eq!(restored.original_positions, [0, 1]);
    }

    #[test]
    fn restore_dispatches_load_without_probing_the_track_path() {
        let directory = tempfile::tempdir().unwrap();
        let mut playback = Playback::initial();
        playback.settings_path = directory.path().join("settings.json");
        let (command_tx, command_rx) = std::sync::mpsc::channel();
        playback.command_tx = Some(command_tx);
        let session = session(1);

        playback.restore_session(
            &session,
            vec![Some(track(1)), Some(track(2)), Some(track(3))],
        );

        assert_eq!(
            command_rx.recv().unwrap(),
            PlaybackCommand::Load {
                path: PathBuf::from("/2.flac"),
                position_ms: 12_345,
            }
        );
        assert_eq!(playback.queue.current().unwrap().id, 2);
        assert!(playback.queue.shuffle_enabled());
        assert_eq!(playback.queue.repeat_mode(), RepeatMode::All);
    }

    #[test]
    fn restore_refuses_unsafe_dsd_before_loading_or_restarting_it() {
        let directory = tempfile::tempdir().unwrap();
        let mut playback = Playback::initial();
        playback.settings_path = directory.path().join("settings.json");
        playback.playback_output_mode = StoredOutputMode::Shared;
        playback.device_capabilities = Some(device::OutputDeviceCapabilities {
            max_bits_per_channel: Some(24),
            max_sample_rate: 192_000.0,
            integer_wire_formats: true,
            transport: device::DeviceTransport::Usb,
        });
        let (command_tx, command_rx) = std::sync::mpsc::channel();
        playback.command_tx = Some(command_tx);
        let mut dsd = track(2);
        dsd.path = PathBuf::from("/2.dff");
        dsd.sample_rate_hz = Some(2_822_400);
        dsd.bit_depth = Some(1);

        playback.restore_session(&session(1), vec![Some(track(1)), Some(dsd), Some(track(3))]);

        assert!(matches!(
            command_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        assert_eq!(playback.playback_state, PlaybackState::Error);
        assert!(playback.error.is_none());
        assert_eq!(
            playback.toasts.back().unwrap().title,
            "DSD needs Exclusive output"
        );

        playback.toggle_playback();

        assert!(matches!(
            command_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn late_restore_does_not_replace_user_started_playback_or_shift_attempts() {
        let directory = tempfile::tempdir().unwrap();
        let live_path = directory.path().join("live.wav");
        crate::backend::scan::metadata::write_test_wav(&live_path, "Live", "Artist", "Album")
            .unwrap();
        let mut playback = Playback::initial();
        playback.settings_path = directory.path().join("settings.json");
        let (command_tx, command_rx) = std::sync::mpsc::channel();
        playback.command_tx = Some(command_tx);
        playback.handle_drop(std::slice::from_ref(&live_path));
        assert_eq!(
            command_rx.recv().unwrap(),
            PlaybackCommand::PlayFile {
                path: live_path.clone(),
            }
        );
        let format = PcmFormat {
            sample_rate: 44_100,
            bits_per_sample: 16,
            channels: 2,
        };
        playback.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
        playback.handle_event(PlaybackEvent::NowPlaying {
            source: PlayableSource {
                path: live_path.clone(),
                duration_ms: Some(60_000),
            },
            format,
        });
        playback.handle_event(PlaybackEvent::StateChanged(PlaybackState::Playing));

        playback.restore_session(
            &session(1),
            vec![Some(track(1)), Some(track(2)), Some(track(3))],
        );

        assert!(!playback.launch_session_pending);
        assert_eq!(playback.dispatched_plays, 1);
        assert_eq!(playback.playback_state, PlaybackState::Playing);
        assert_eq!(playback.source_path.as_deref(), Some(live_path.as_path()));
        assert!(matches!(
            command_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        let advanced_path = directory.path().join("advanced.wav");
        playback.handle_event(PlaybackEvent::Advanced {
            attempt: 1,
            source: PlayableSource {
                path: advanced_path.clone(),
                duration_ms: Some(60_000),
            },
            format,
        });
        assert_eq!(
            playback.source_path.as_deref(),
            Some(advanced_path.as_path())
        );
        playback.handle_event(PlaybackEvent::Error {
            attempt: 1,
            kind: PlaybackErrorKind::Track,
            message: "later failure".to_string(),
        });
        assert_eq!(playback.error.as_deref(), Some("later failure"));
    }

    #[test]
    fn abandoning_launch_restore_keeps_the_gate_closed_and_the_saved_session_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let mut playback = Playback::initial();
        playback.settings_path = directory.path().join("settings.json");
        playback.settings.session = Some(session(1));
        playback.settings.save(&playback.settings_path).unwrap();

        playback.abandon_launch_session_restore();
        playback.flush_settings_writer().unwrap();
        playback.apply_settings_write_results();

        assert!(playback.launch_session_pending);
        assert_eq!(
            AppSettings::load(&playback.settings_path)
                .unwrap()
                .session
                .unwrap()
                .queue_track_ids,
            [1, 2, 3]
        );
    }

    #[test]
    fn failed_background_save_keeps_persisted_settings_and_retries_without_hiding_playback_error() {
        let directory = tempfile::tempdir().unwrap();
        let blocked_parent = directory.path().join("blocked");
        std::fs::write(&blocked_parent, "not a directory").unwrap();
        let mut playback = Playback::initial();
        playback.settings_path = blocked_parent.join("settings.json");
        playback.launch_session_pending = false;
        playback
            .queue
            .restore(&[track(1)], &[0], 0, false, RepeatMode::Off);
        playback.refresh_queue_snapshot();
        playback.error = Some("playback failure".to_string());

        playback.persist_session_or_record_error();
        playback.flush_settings_writer().unwrap();
        playback.drain_events();

        assert!(playback.settings.session.is_none());
        assert!(playback.staged_settings.is_some());
        assert_eq!(playback.error.as_deref(), Some("playback failure"));
        assert!(playback.last_settings_error.is_some());

        std::fs::remove_file(&blocked_parent).unwrap();
        std::fs::create_dir(&blocked_parent).unwrap();
        playback.persist_session_or_record_error();
        playback.flush_settings_writer().unwrap();
        playback.drain_events();

        assert_eq!(
            playback.settings.session.as_ref().unwrap().queue_track_ids,
            [1]
        );
        assert!(playback.staged_settings.is_none());
        assert!(playback.last_settings_error.is_none());
        assert_eq!(playback.error.as_deref(), Some("playback failure"));
    }

    #[test]
    fn position_saves_are_coalesced_to_five_seconds_of_progress() {
        let mut cadence = SessionSaveCadence::new(10_000);
        let positions = [10_001, 12_000, 14_999, 15_000, 19_999, 20_000];
        let mut saves = Vec::new();
        for position in positions {
            if cadence.should_save_progress(position) {
                saves.push(position);
                cadence.record_saved(position);
            }
        }

        assert_eq!(saves, [15_000, 20_000]);
    }

    #[test]
    fn playback_position_events_write_only_at_the_coalesced_save_points() {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        let mut playback = Playback::initial();
        playback.settings_path = settings_path.clone();
        playback.launch_session_pending = false;
        playback.session_save_cadence = SessionSaveCadence::new(0);
        playback
            .queue
            .restore(&[track(1)], &[0], 0, false, RepeatMode::Off);
        playback.refresh_queue_snapshot();
        playback.playback_state = PlaybackState::Playing;

        for position_ms in [1_000, 4_999] {
            playback.handle_event(PlaybackEvent::Position {
                position_ms,
                duration_ms: Some(60_000),
                dropout_frames: 0,
            });
        }
        assert!(!settings_path.exists());

        let first_save_started = Instant::now();
        playback.handle_event(PlaybackEvent::Position {
            position_ms: 5_000,
            duration_ms: Some(60_000),
            dropout_frames: 0,
        });
        eprintln!(
            "launch-state drain-side save dispatch: {:?}",
            first_save_started.elapsed()
        );
        playback.flush_settings_writer().unwrap();
        assert_eq!(
            AppSettings::load(&settings_path)
                .unwrap()
                .session
                .unwrap()
                .position_ms,
            5_000
        );

        playback.handle_event(PlaybackEvent::Position {
            position_ms: 9_999,
            duration_ms: Some(60_000),
            dropout_frames: 0,
        });
        assert_eq!(
            AppSettings::load(&settings_path)
                .unwrap()
                .session
                .unwrap()
                .position_ms,
            5_000
        );

        playback.handle_event(PlaybackEvent::Position {
            position_ms: 10_000,
            duration_ms: Some(60_000),
            dropout_frames: 0,
        });
        playback.flush_settings_writer().unwrap();
        assert_eq!(
            AppSettings::load(&settings_path)
                .unwrap()
                .session
                .unwrap()
                .position_ms,
            10_000
        );
    }
}
