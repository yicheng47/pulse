use super::*;

#[test]
fn engine_errors_map_to_the_kinds_that_drive_queue_behavior() {
    assert_eq!(
        PlaybackErrorKind::from(&EngineError::Hogged(i32::MAX)),
        PlaybackErrorKind::Device {
            hog_pid: Some(i32::MAX)
        }
    );
    // Audio-unit/device start failures must never skip the queue.
    assert_eq!(
        PlaybackErrorKind::from(&EngineError::AudioUnit(
            "the requested device was not found".into()
        )),
        PlaybackErrorKind::Device { hog_pid: None }
    );
    assert_eq!(
        PlaybackErrorKind::from(&EngineError::Os {
            call: "AudioUnitRender",
            status: -10863
        }),
        PlaybackErrorKind::Device { hog_pid: None }
    );
    // Format problems belong to the track and are skippable.
    assert_eq!(
        PlaybackErrorKind::from(&EngineError::UnsupportedFormat(
            "20-bit PCM is not supported by the AUHAL packer".into()
        )),
        PlaybackErrorKind::Track
    );
}

#[test]
fn failed_launch_load_keeps_the_queue_and_retries_normally_on_play() {
    let directory = tempfile::tempdir().unwrap();
    let tracks = [
        library_track(1, PathBuf::from("/1.flac"), "Track 1"),
        library_track(2, PathBuf::from("/2.flac"), "Track 2"),
        library_track(3, PathBuf::from("/3.flac"), "Track 3"),
    ];
    let session = SessionState {
        queue_track_ids: vec![1, 2, 3],
        queue_original_positions: vec![0, 1, 2],
        current_index: Some(1),
        position_ms: 12_345,
        ..SessionState::default()
    };
    let mut playback = Playback::initial();
    playback.settings_path = directory.path().join("settings.json");
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    playback.command_tx = Some(command_tx);
    playback.restore_session(&session, tracks.iter().cloned().map(Some).collect());
    command_rx.recv().unwrap();

    playback.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
    playback.handle_event(PlaybackEvent::StateChanged(PlaybackState::Idle));
    playback.handle_event(PlaybackEvent::Error {
        attempt: 1,
        kind: PlaybackErrorKind::Track,
        message: "unreadable source".to_string(),
    });

    assert_eq!(playback.queue.current().unwrap().id, 2);
    assert_eq!(playback.position_ms, 12_345);
    assert!(playback.error.is_none());
    assert_eq!(
        playback.prepare_toggle_command(),
        Some(PlaybackCommand::PlayFile {
            path: PathBuf::from("/2.flac"),
        })
    );
}

#[test]
fn gate_released_play_advance_pause_and_next_each_persist_the_expected_save_point() {
    let directory = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(directory.path(), &["first", "second", "third"]);
    let mut playback = Playback::initial();
    playback.settings_path = directory.path().join("settings.json");
    playback.launch_session_pending = false;
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    playback.command_tx = Some(command_tx);
    playback.play_library_tracks(&tracks, 0);
    assert_eq!(playback.dispatched_plays, 1);
    command_rx.recv().unwrap();

    let format = PcmFormat {
        sample_rate: 44_100,
        bits_per_sample: 16,
        channels: 2,
    };
    playback.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
    playback.handle_event(PlaybackEvent::NowPlaying {
        source: PlayableSource {
            path: tracks[0].path.clone(),
            duration_ms: Some(60_000),
        },
        format,
    });
    assert_eq!(playback.next_settings_generation, 1);
    playback.handle_event(PlaybackEvent::StateChanged(PlaybackState::Playing));
    playback.handle_event(PlaybackEvent::Advanced {
        attempt: 1,
        source: PlayableSource {
            path: tracks[1].path.clone(),
            duration_ms: Some(60_000),
        },
        format,
    });
    assert_eq!(playback.next_settings_generation, 2);
    playback.handle_event(PlaybackEvent::Position {
        position_ms: 2_345,
        duration_ms: Some(60_000),
        dropout_frames: 0,
    });
    assert_eq!(playback.next_settings_generation, 2);
    playback.handle_event(PlaybackEvent::StateChanged(PlaybackState::Paused));
    assert_eq!(playback.next_settings_generation, 3);

    while command_rx.try_recv().is_ok() {}
    playback.next_track();
    let commands = command_rx.try_iter().collect::<Vec<_>>();
    assert!(commands.contains(&PlaybackCommand::PlayFile {
        path: tracks[2].path.clone(),
    }));
    assert_eq!(playback.next_settings_generation, 3);
    playback.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
    playback.handle_event(PlaybackEvent::NowPlaying {
        source: PlayableSource {
            path: tracks[2].path.clone(),
            duration_ms: Some(60_000),
        },
        format,
    });
    assert_eq!(playback.next_settings_generation, 4);

    playback.flush_settings_writer().unwrap();
    playback.drain_events();
    let saved = AppSettings::load(&playback.settings_path)
        .unwrap()
        .session
        .unwrap();
    assert_eq!(saved.current_index, Some(2));
    assert_eq!(saved.position_ms, 0);
    assert_eq!(saved.queue_track_ids, [1, 2, 3]);
}

#[test]
fn a_decode_failure_mid_queue_skips_to_the_next_entry_and_reports() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["corrupt", "good", "later"]);
    truncate_wav(&tracks[0].path);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    let error = pulse_engine::decode::open(&tracks[0].path)
        .map(|_| ())
        .unwrap_err();
    let kind = PlaybackErrorKind::from(&error);
    let message = error.to_string();

    assert!(
        row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading))
            .is_none()
    );
    assert!(
        row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Error))
            .is_none()
    );
    let next = row
        .handle_event(PlaybackEvent::Error {
            attempt: 0,
            kind,
            message,
        })
        .unwrap();

    assert_eq!(next.title, "good");
    assert_eq!(row.next_playable(next).unwrap().title, "good");
    assert_eq!(
        row.toasts.back(),
        Some(&PlaybackToast::warning(
            "Track skipped",
            "Skipped “corrupt” — its file could not be decoded."
        ))
    );
    assert!(!row.is_track_missing(1), "a corrupt file is not missing");
    assert!(row.error.is_none(), "the queue keeps playing");
}

#[test]
fn next_track_during_loading_does_not_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["current", "next"]);
    let mut playback = Playback::initial();
    playback.seed_queue(QueueState::from_tracks(&tracks, 0));
    playback.source_path = Some(tracks[0].path.clone());
    playback.playback_state = PlaybackState::Loading;
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    playback.command_tx = Some(command_tx);

    playback.next_track();

    assert_eq!(playback.queue.current().unwrap().title, "current");
    assert!(playback.current_play.is_none());
    assert_eq!(playback.dispatched_plays, 0);
    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
}

#[test]
fn previous_track_during_loading_does_not_restart_the_source() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["current"]);
    let mut playback = Playback::initial();
    playback.source_path = Some(tracks[0].path.clone());
    playback.playback_state = PlaybackState::Loading;
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    playback.command_tx = Some(command_tx);

    playback.previous_track();

    assert!(playback.current_play.is_none());
    assert_eq!(playback.dispatched_plays, 0);
    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
}

#[test]
fn missing_files_are_marked_and_skipped_at_play_time() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["gone-1", "gone-2", "present"]);
    std::fs::remove_file(&tracks[0].path).unwrap();
    std::fs::remove_file(&tracks[1].path).unwrap();
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));

    let first = row.queue.current().cloned().unwrap();
    let playable = row.next_playable(first).unwrap();

    assert_eq!(playable.title, "present");
    assert_eq!(row.queue.current().unwrap().title, "present");
    assert!(row.is_track_missing(1));
    assert!(row.is_track_missing(2));
    assert!(!row.is_track_missing(3));
    assert_eq!(
        row.toasts.back(),
        Some(&PlaybackToast::warning(
            "Track skipped",
            "Skipped 2 tracks that could not be played."
        ))
    );
}

#[test]
fn a_queue_where_every_file_is_gone_stops_with_a_poison_message() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["gone-1", "gone-2"]);
    std::fs::remove_file(&tracks[0].path).unwrap();
    std::fs::remove_file(&tracks[1].path).unwrap();
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));

    let first = row.queue.current().cloned().unwrap();
    assert!(row.next_playable(first).is_none());
    assert!(row.is_track_missing(1));
    assert!(row.is_track_missing(2));
    assert_eq!(
        row.toasts.back(),
        Some(&PlaybackToast::error(
            "Playback stopped",
            "Playback stopped — none of the queued tracks could be played."
        ))
    );
}

#[test]
fn a_trailing_failure_after_played_tracks_is_not_reported_as_poison() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["played", "gone"]);
    std::fs::remove_file(&tracks[1].path).unwrap();
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.queue.mark_started();

    assert!(
        row.handle_event(PlaybackEvent::Ended { attempt: 0 })
            .is_none()
    );
    assert_eq!(
        row.toasts.back(),
        Some(&PlaybackToast::error(
            "Playback stopped",
            "Playback stopped — “gone” could not be played."
        ))
    );
}

#[test]
fn now_playing_clears_the_missing_mark_for_a_recovered_queue_track() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["recovered"]);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.seed_missing_track_ids(HashSet::from([1]));

    let _ = row.handle_event(PlaybackEvent::NowPlaying {
        source: pulse_engine::PlayableSource {
            path: tracks[0].path.clone(),
            duration_ms: Some(1_000),
        },
        format: PcmFormat {
            sample_rate: 44_100,
            bits_per_sample: 16,
            channels: 2,
        },
    });

    assert!(!row.is_track_missing(1));
    assert!(!row.queue.nothing_played());
}

#[test]
fn a_hogged_device_reports_plain_language_with_a_retry_target() {
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    row.record_play_attempt(Path::new("/Music/track.flac"));
    let _ = row.handle_event(now_playing("/Music/track.flac"));
    row.playback_state = PlaybackState::Playing;
    let _ = row.handle_event(PlaybackEvent::Position {
        position_ms: 42_000,
        duration_ms: Some(268_000),
        dropout_frames: 0,
    });

    let outcome = row.handle_event(PlaybackEvent::Error {
        attempt: 0,
        kind: (&EngineError::Hogged(i32::MAX)).into(),
        message: EngineError::Hogged(i32::MAX).to_string(),
    });

    assert!(outcome.is_none(), "device failures never skip the queue");
    assert_eq!(
        row.notice,
        Some(PlaybackNotice::DeviceFailure {
            text: "Another app is using mini-i Series exclusively. Quit it or choose another output, then try again.".to_string()
        })
    );
    assert_eq!(
        row.retry,
        Some(RetryTarget {
            path: PathBuf::from("/Music/track.flac"),
            position_ms: 42_000,
        })
    );
}

#[test]
fn a_device_failure_while_loading_a_new_file_retries_that_file_from_zero() {
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    // A previous track was playing at 42 s when the user dropped a new
    // file; source_path and position_ms still describe the old track
    // while the new one loads.
    row.source_path = Some(PathBuf::from("/Music/old.flac"));
    row.position_ms = 42_000;
    row.record_play_attempt(Path::new("/Music/dropped.flac"));
    row.playback_state = PlaybackState::Loading;

    let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Error));
    let outcome = row.handle_event(PlaybackEvent::Error {
        attempt: 0,
        kind: (&EngineError::Hogged(i32::MAX)).into(),
        message: EngineError::Hogged(i32::MAX).to_string(),
    });

    assert!(outcome.is_none());
    assert_eq!(
        row.retry,
        Some(RetryTarget {
            path: PathBuf::from("/Music/dropped.flac"),
            position_ms: 0,
        })
    );
    // Transport Play must restart the same file Try again targets.
    assert_eq!(
        row.prepare_toggle_command(),
        Some(PlaybackCommand::PlayFile {
            path: PathBuf::from("/Music/dropped.flac")
        })
    );
}

#[test]
fn a_device_failure_during_auto_advance_retries_the_next_track_from_zero() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["finished", "next"]);
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.record_play_attempt(&tracks[0].path);
    let _ = row.handle_event(now_playing(tracks[0].path.to_str().unwrap()));
    row.playback_state = PlaybackState::Playing;
    let _ = row.handle_event(PlaybackEvent::Position {
        position_ms: 180_000,
        duration_ms: Some(180_000),
        dropout_frames: 0,
    });

    let next = row
        .handle_event(PlaybackEvent::Ended { attempt: 0 })
        .expect("the queue advances");
    row.record_play_attempt(&next.path);
    let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));

    let outcome = row.handle_event(PlaybackEvent::Error {
        attempt: 0,
        kind: (&EngineError::Hogged(i32::MAX)).into(),
        message: EngineError::Hogged(i32::MAX).to_string(),
    });

    assert!(outcome.is_none());
    assert_eq!(
        row.retry,
        Some(RetryTarget {
            path: tracks[1].path.clone(),
            position_ms: 0,
        })
    );
}

#[test]
fn transport_play_from_an_idle_selection_records_the_attempt() {
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    let track = TrackRef {
        id: 1,
        path: PathBuf::from("/Music/selected.flac"),
        title: "Selected".to_string(),
        artist: "Artist".to_string(),
        album: "Album".to_string(),
        duration_ms: None,
        sample_rate_hz: Some(44_100),
        cover_art_path: None,
    };
    assert!(row.apply_track_selection(&track));

    let command = row.prepare_toggle_command().unwrap();
    assert_eq!(
        command,
        PlaybackCommand::PlayFile {
            path: PathBuf::from("/Music/selected.flac")
        }
    );
    assert_eq!(
        row.current_play,
        Some(PlayAttempt {
            target: RetryTarget {
                path: PathBuf::from("/Music/selected.flac"),
                position_ms: 0,
            },
            confirmed: false,
            load: false,
        })
    );

    let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
    let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Error));
    let _ = row.handle_event(PlaybackEvent::Error {
        attempt: 0,
        kind: (&EngineError::Hogged(i32::MAX)).into(),
        message: EngineError::Hogged(i32::MAX).to_string(),
    });
    assert_eq!(
        row.retry,
        Some(RetryTarget {
            path: PathBuf::from("/Music/selected.flac"),
            position_ms: 0,
        })
    );
}

#[test]
fn replaying_an_ended_track_restarts_the_attempt_from_zero() {
    let mut row = Playback::initial();
    row.record_play_attempt(Path::new("/Music/track.flac"));
    let _ = row.handle_event(now_playing("/Music/track.flac"));
    row.playback_state = PlaybackState::Playing;
    let _ = row.handle_event(PlaybackEvent::Position {
        position_ms: 268_000,
        duration_ms: Some(268_000),
        dropout_frames: 0,
    });
    let _ = row.handle_event(PlaybackEvent::Ended { attempt: 0 });

    let command = row.prepare_toggle_command().unwrap();
    assert!(matches!(command, PlaybackCommand::PlayFile { .. }));
    assert_eq!(
        row.current_play,
        Some(PlayAttempt {
            target: RetryTarget {
                path: PathBuf::from("/Music/track.flac"),
                position_ms: 0,
            },
            confirmed: false,
            load: false,
        })
    );
}

#[test]
fn stale_terminal_events_from_a_superseded_play_are_ignored() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["b1", "b2"]);
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    // Play A happened (attempt 1), then the user started queue B
    // (attempt 2) before A's terminal events drained.
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.dispatched_plays = 2;
    row.playback_state = PlaybackState::Loading;

    let stale_track_error = row.handle_event(PlaybackEvent::Error {
        attempt: 1,
        kind: PlaybackErrorKind::Track,
        message: "decode: stale failure from A".to_string(),
    });
    assert!(stale_track_error.is_none(), "stale error must not skip B");
    assert_eq!(row.queue.current().unwrap().title, "b1");
    assert!(row.notice.is_none());

    let stale_ended = row.handle_event(PlaybackEvent::Ended { attempt: 1 });
    assert!(stale_ended.is_none(), "stale Ended must not advance B");
    assert_eq!(row.queue.current().unwrap().title, "b1");
    assert_eq!(row.playback_state, PlaybackState::Loading);

    let stale_device_error = row.handle_event(PlaybackEvent::Error {
        attempt: 1,
        kind: PlaybackErrorKind::Device { hog_pid: None },
        message: "stale device failure from A".to_string(),
    });
    assert!(stale_device_error.is_none());
    assert!(
        row.notice.is_none(),
        "no failure notice for a superseded play"
    );
    assert!(row.retry.is_none());

    let current = row.handle_event(PlaybackEvent::Error {
        attempt: 2,
        kind: PlaybackErrorKind::Track,
        message: "decode: real failure for B".to_string(),
    });
    assert_eq!(
        current.unwrap().title,
        "b2",
        "current-attempt errors still drive the queue"
    );
}
