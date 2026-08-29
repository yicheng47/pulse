use super::*;

#[test]
fn a_stale_now_playing_does_not_replace_an_unconfirmed_attempt() {
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    // The old track's NowPlaying was already queued when the user
    // dispatched a new file; it must not steal the retry target back.
    row.record_play_attempt(Path::new("/Music/new.flac"));
    let _ = row.handle_event(now_playing("/Music/old.flac"));
    assert_eq!(row.source_path, None, "stale NowPlaying is ignored fully");
    assert_eq!(row.title, "No track loaded");

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
            path: PathBuf::from("/Music/new.flac"),
            position_ms: 0,
        })
    );
    // Transport Play and Try again must agree on the failed file.
    assert_eq!(
        row.prepare_toggle_command(),
        Some(PlaybackCommand::PlayFile {
            path: PathBuf::from("/Music/new.flac")
        })
    );
}

#[test]
fn a_direct_file_decode_failure_before_now_playing_names_the_attempted_file() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["corrupt"]);
    truncate_wav(&tracks[0].path);
    let error = pulse_engine::decode::open(&tracks[0].path)
        .map(|_| ())
        .unwrap_err();
    let kind = PlaybackErrorKind::from(&error);
    let message = error.to_string();
    let mut row = Playback::initial();
    assert_eq!(row.title, "No track loaded");

    // A dropped file fails at decoder open, before any NowPlaying.
    row.record_play_attempt(&tracks[0].path);
    let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
    let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Error));
    let _ = row.handle_event(PlaybackEvent::Error {
        attempt: 0,
        kind,
        message: message.clone(),
    });

    match &row.notice {
        Some(PlaybackNotice::Stopped { text }) => {
            assert!(text.contains("“corrupt”"), "{text}");
            assert!(!text.contains("No track loaded"), "{text}");
        }
        other => panic!("expected a Stopped notice, got {other:?}"),
    }
}

#[test]
fn stale_position_events_do_not_move_an_unconfirmed_attempt() {
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    // A new file was dispatched while the old track's last Position
    // event was still queued; it must not move the new attempt.
    row.record_play_attempt(Path::new("/Music/new.flac"));
    let _ = row.handle_event(PlaybackEvent::Position {
        position_ms: 42_000,
        duration_ms: Some(268_000),
    });
    let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
    let _ = row.handle_event(PlaybackEvent::Error {
        attempt: 0,
        kind: (&EngineError::Hogged(i32::MAX)).into(),
        message: EngineError::Hogged(i32::MAX).to_string(),
    });

    assert_eq!(
        row.retry,
        Some(RetryTarget {
            path: PathBuf::from("/Music/new.flac"),
            position_ms: 0,
        })
    );
}

#[test]
fn losing_the_active_device_mid_playback_stops_with_a_recovery_notice() {
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    row.record_play_attempt(Path::new("/Music/track.flac"));
    let _ = row.handle_event(now_playing("/Music/track.flac"));
    row.playback_state = PlaybackState::Playing;
    let _ = row.handle_event(PlaybackEvent::Position {
        position_ms: 10_000,
        duration_ms: Some(268_000),
    });

    let still_attached = vec![
        output_device(1, "built-in", "Mac Speakers"),
        output_device(9, "matrix", "mini-i Series"),
    ];
    assert!(!row.note_device_loss(still_attached));
    assert!(row.notice.is_none());

    let unplugged = vec![output_device(1, "built-in", "Mac Speakers")];
    assert!(row.note_device_loss(unplugged));
    assert_eq!(
        row.notice,
        Some(PlaybackNotice::DeviceFailure {
            text: "mini-i Series was disconnected. Reconnect it or choose another output, then try again.".to_string()
        })
    );
    assert_eq!(
        row.retry,
        Some(RetryTarget {
            path: PathBuf::from("/Music/track.flac"),
            position_ms: 10_000,
        })
    );
    assert_eq!(row.devices.len(), 1);
}

#[test]
fn advisory_errors_after_teardown_do_not_disturb_the_queue_report() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["only"]);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.playback_state = PlaybackState::Ended;
    row.notice = Some(PlaybackNotice::Skip {
        text: "Skipped “gone” — its file is missing.".to_string(),
    });

    let outcome = row.handle_event(PlaybackEvent::Error {
        attempt: 0,
        kind: PlaybackErrorKind::Track,
        message: "decode: backend stop failed".to_string(),
    });

    assert!(outcome.is_none());
    assert_eq!(
        row.notice,
        Some(PlaybackNotice::Skip {
            text: "Skipped “gone” — its file is missing.".to_string()
        })
    );
    assert_eq!(row.error.as_deref(), Some("decode: backend stop failed"));
}

#[test]
fn a_jump_to_a_marked_missing_entry_skips_and_reports() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["playing", "gone", "after"]);
    std::fs::remove_file(&tracks[1].path).unwrap();
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.queue.mark_started();
    row.seed_missing_track_ids(HashSet::from([2]));

    let target = row.queue.jump_to(1).unwrap();
    let playable = row.prepare_queue_play(target).unwrap();

    assert_eq!(playable, tracks[2].path);
    assert_eq!(row.queue.current().unwrap().title, "after");
    assert!(row.is_track_missing(2));
    assert_eq!(
        row.notice,
        Some(PlaybackNotice::Skip {
            text: "Skipped “gone” — its file is missing.".to_string()
        })
    );
}

#[test]
fn a_jump_into_an_all_missing_tail_stops_the_active_track() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["playing", "gone-1", "gone-2"]);
    std::fs::remove_file(&tracks[1].path).unwrap();
    std::fs::remove_file(&tracks[2].path).unwrap();
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.dispatched_plays = 1;
    row.record_play_attempt(&tracks[0].path);
    let _ = row.handle_event(now_playing(tracks[0].path.to_str().unwrap()));
    row.playback_state = PlaybackState::Playing;

    // The user clicks an up-next row whose file and remaining tail are
    // gone: no new dispatch is possible.
    let target = row.queue.jump_to(1).unwrap();
    assert!(row.prepare_queue_play(target).is_none());

    // `play_queue_track` must stop the still-audible superseded track so
    // the Stopped notice is truthful.
    assert!(row.active_playback_needs_stop());
    assert_eq!(
        row.notice,
        Some(PlaybackNotice::Stopped {
            text: "Playback stopped — 2 tracks could not be played.".to_string()
        })
    );
    assert!(row.is_track_missing(2));
    assert!(row.is_track_missing(3));

    // Until the stop lands, the NOW PLAYING block keeps describing the
    // audible track, not the missing entry the queue index points at.
    assert_eq!(
        row.snapshot().now_playing_lines(),
        Some(("playing".to_string(), "Artist - Album".to_string()))
    );
    let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Idle));
    assert_eq!(row.snapshot().now_playing_lines(), None);

    // An ended/error event from the stopped track is stale bookkeeping,
    // not a queue driver, once state left the active set.
    assert!(
        row.handle_event(PlaybackEvent::Ended { attempt: 1 })
            .is_none()
    );
}

#[test]
fn a_natural_queue_end_does_not_ask_for_a_stop() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["played", "gone"]);
    std::fs::remove_file(&tracks[1].path).unwrap();
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.queue.mark_started();
    row.playback_state = PlaybackState::Ended;

    let next = row
        .handle_event(PlaybackEvent::Ended { attempt: 0 })
        .expect("the queue advances past the ended track");
    assert!(row.prepare_queue_play(next).is_none());
    assert!(
        !row.active_playback_needs_stop(),
        "nothing is audible after a natural end; no Stop command is due"
    );
}

#[test]
fn a_jump_dispatch_supersedes_stale_terminal_events() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["playing", "second", "target", "tail"]);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    // Entry 0 is playing as dispatch 1.
    row.dispatched_plays = 1;
    row.record_play_attempt(&tracks[0].path);
    let _ = row.handle_event(now_playing(tracks[0].path.to_str().unwrap()));
    row.playback_state = PlaybackState::Playing;

    // The user clicks the third up-next row. The jump prepares the play;
    // `play_file`/`send_command` then record the attempt and count the
    // dispatch, simulated here because tests have no command channel.
    let target = row.queue.jump_to(2).unwrap();
    let path = row.prepare_queue_play(target).unwrap();
    assert_eq!(path, tracks[2].path);
    row.record_play_attempt(&path);
    row.dispatched_plays += 1;
    row.playback_state = PlaybackState::Loading;

    // The superseded track's terminal events drain afterwards; the older
    // attempt ordinal keeps them from moving the jumped queue.
    assert!(
        row.handle_event(PlaybackEvent::Ended { attempt: 1 })
            .is_none()
    );
    assert_eq!(row.queue.current().unwrap().title, "target");
    assert!(
        row.handle_event(PlaybackEvent::Error {
            attempt: 1,
            kind: PlaybackErrorKind::Track,
            message: "decode: stale failure".to_string(),
        })
        .is_none()
    );
    assert_eq!(row.queue.current().unwrap().title, "target");
    assert!(row.notice.is_none());

    // The jump's own terminal events still drive the queue.
    let next = row
        .handle_event(PlaybackEvent::Ended { attempt: 2 })
        .unwrap();
    assert_eq!(next.title, "tail");
}

#[test]
fn the_now_playing_block_tracks_playback_state_and_queue_context() {
    let mut row = Playback::initial();
    assert_eq!(row.snapshot().now_playing_lines(), None);

    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["present"]);
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.source_path = Some(tracks[0].path.clone());
    row.playback_state = PlaybackState::Playing;
    assert_eq!(
        row.snapshot().now_playing_lines(),
        Some(("present".to_string(), "Artist · Album".to_string()))
    );

    row.playback_state = PlaybackState::Ended;
    assert_eq!(row.snapshot().now_playing_lines(), None);

    // A dropped file plays without a queue; the row's display strings
    // stand in.
    row.seed_queue(QueueState::default());
    row.playback_state = PlaybackState::Playing;
    row.title = "Dropped".to_string();
    row.secondary = "Folder".to_string();
    assert_eq!(
        row.snapshot().now_playing_lines(),
        Some(("Dropped".to_string(), "Folder".to_string()))
    );
}

#[test]
fn position_events_keep_cached_collection_snapshots_shared() {
    let mut row = Playback::initial();
    let before = row.snapshot();

    row.handle_event(PlaybackEvent::Position {
        position_ms: 42_000,
        duration_ms: Some(84_000),
    });

    let after = row.snapshot();
    assert!(Arc::ptr_eq(&before.queue, &after.queue));
    assert!(Arc::ptr_eq(&before.devices, &after.devices));
    assert!(Arc::ptr_eq(
        &before.missing_track_ids,
        &after.missing_track_ids
    ));
}

#[test]
fn formats_queue_times_without_a_leading_minute_zero() {
    assert_eq!(format_queue_time(0), "0:00");
    assert_eq!(format_queue_time(84_000), "1:24");
    assert_eq!(format_queue_time(239_000), "3:59");
    assert_eq!(format_queue_time(3_661_000), "1:01:01");
}

#[test]
fn formats_the_queue_meta_line_and_omits_unknown_minutes() {
    assert_eq!(format_queue_meta(7, 1_470_000), "7 UP NEXT · 25 MIN");
    assert_eq!(format_queue_meta(1, 29_000), "1 UP NEXT · 1 MIN");
    assert_eq!(format_queue_meta(3, 0), "3 UP NEXT");
}
