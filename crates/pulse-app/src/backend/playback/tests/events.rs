use super::*;

fn command_sink(row: &mut Playback) -> std::sync::mpsc::Receiver<PlaybackCommand> {
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    row.command_tx = Some(command_tx);
    command_rx
}

fn advanced(path: &Path, attempt: u64) -> PlaybackEvent {
    PlaybackEvent::Advanced {
        attempt,
        source: pulse_engine::PlayableSource {
            path: path.to_path_buf(),
            duration_ms: Some(1_000),
        },
        format: PcmFormat {
            sample_rate: 44_100,
            bits_per_sample: 16,
            channels: 2,
        },
    }
}

fn dropout(attempt: u64, cumulative_frames: u64) -> PlaybackEvent {
    PlaybackEvent::Dropout {
        attempt,
        frames: 1,
        cumulative_frames,
    }
}

fn start_scripted_queue_play(
    row: &mut Playback,
    command_rx: &std::sync::mpsc::Receiver<PlaybackCommand>,
    track: &Track,
) {
    row.record_play_attempt(&track.path);
    assert!(row.send_command(PlaybackCommand::PlayFile {
        path: track.path.clone(),
    }));
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::PlayFile {
            path: track.path.clone()
        }
    );
    row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
    row.handle_event(now_playing(track.path.to_str().unwrap()));
}

#[test]
fn hardware_volume_updates_and_persists_without_sending_volume_back() {
    let directory = tempfile::tempdir().unwrap();
    let settings_path = directory.path().join("settings.json");
    let mut row = Playback::initial();
    row.settings_path = settings_path.clone();
    let command_rx = command_sink(&mut row);

    row.handle_event(PlaybackEvent::HardwareVolume {
        level: 0.5,
        muted: true,
    });

    assert_eq!(row.volume_level, 0.5);
    assert!(row.volume_muted);
    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    let settings = AppSettings::load(&settings_path).unwrap();
    assert_eq!(settings.volume_level, 0.5);
    assert!(settings.volume_muted);
}

#[test]
fn volume_domain_is_exposed_in_the_snapshot() {
    let mut row = Playback::initial();
    let state = VolumeState::new(pulse_engine::VolumeDomain::Fixed);

    row.handle_event(PlaybackEvent::VolumeStateChanged(state));

    assert_eq!(row.snapshot().volume_state, state);
}

#[test]
fn gapless_lookahead_advances_the_queue_without_dispatching_play_file() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["a", "b", "c"]);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    let command_rx = command_sink(&mut row);

    start_scripted_queue_play(&mut row, &command_rx, &tracks[0]);
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: tracks[1].path.clone()
        }
    );
    row.handle_event(now_playing(tracks[0].path.to_str().unwrap()));
    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Playing));

    assert!(row.handle_event(advanced(&tracks[1].path, 1)).is_none());

    assert_eq!(row.playback_state, PlaybackState::Playing);
    assert_eq!(row.source_path.as_ref(), Some(&tracks[1].path));
    assert_eq!(row.position_ms, 0);
    assert_eq!(row.queue.current().unwrap().path, tracks[1].path);
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: tracks[2].path.clone()
        }
    );
    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));

    row.remove_queue_entry(2);
    assert_eq!(command_rx.recv().unwrap(), PlaybackCommand::ClearNext);
}

#[test]
fn unsafe_dsd_lookahead_is_cleared_before_it_can_advance() {
    let temp = tempfile::tempdir().unwrap();
    let first = library_track(1, temp.path().join("first.wav"), "first");
    crate::backend::scan::metadata::write_test_wav(&first.path, "first", "Artist", "Album")
        .unwrap();
    let dff_path = temp.path().join("second.dff");
    let mut second = library_track(2, dff_path.clone(), "second");
    second.sample_rate_hz = Some(2_822_400);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&[first, second], 0));
    row.playback_state = PlaybackState::Playing;
    row.output_mode = StoredOutputMode::Shared;
    row.playback_output_mode = StoredOutputMode::Shared;
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        transport: device::DeviceTransport::Usb,
    });
    row.sent_next = Some(dff_path);
    let command_rx = command_sink(&mut row);

    row.sync_next_source();

    assert_eq!(command_rx.recv().unwrap(), PlaybackCommand::ClearNext);
}

#[test]
fn unsafe_dsd_lookahead_skips_to_the_next_playable_track_with_a_warning() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["first", "ignored", "third"]);
    let mut dsd = tracks[1].clone();
    dsd.path = temp.path().join("second.dff");
    dsd.title = Some("second".to_string());
    dsd.sample_rate_hz = Some(2_822_400);
    dsd.bit_depth = Some(1);
    let queue_tracks = [tracks[0].clone(), dsd, tracks[2].clone()];
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&queue_tracks, 0));
    row.playback_state = PlaybackState::Playing;
    row.dispatched_plays = 1;
    row.playback_output_mode = StoredOutputMode::Shared;
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        transport: device::DeviceTransport::Usb,
    });
    row.sent_next = Some(queue_tracks[1].path.clone());
    let command_rx = command_sink(&mut row);

    row.sync_next_source();
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: queue_tracks[2].path.clone()
        }
    );
    assert!(
        row.handle_event(advanced(&queue_tracks[2].path, 1))
            .is_none()
    );

    assert_eq!(row.queue.current().unwrap().id, queue_tracks[2].id);
    assert_eq!(row.queue.skipped_count(), 1);
    assert_eq!(
        row.toasts.back(),
        Some(&PlaybackToast::warning(
            "DSD track skipped",
            "Skipped “second” — it can't play on the active output."
        ))
    );
}

#[test]
fn an_all_dsd_tail_stops_after_one_pass_without_looping() {
    let temp = tempfile::tempdir().unwrap();
    let first = wav_tracks(temp.path(), &["first"]).remove(0);
    let mut second = library_track(2, temp.path().join("second.dff"), "second");
    second.sample_rate_hz = Some(2_822_400);
    second.bit_depth = Some(1);
    let mut third = library_track(3, temp.path().join("third.dsf"), "third");
    third.sample_rate_hz = Some(5_644_800);
    third.bit_depth = Some(1);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&[first, second, third], 0));
    row.playback_state = PlaybackState::Playing;
    row.dispatched_plays = 1;
    row.playback_output_mode = StoredOutputMode::Shared;
    let command_rx = command_sink(&mut row);

    row.sync_next_source();
    assert_eq!(command_rx.recv().unwrap(), PlaybackCommand::ClearNext);
    assert!(
        row.handle_event(PlaybackEvent::Ended { attempt: 1 })
            .is_none()
    );

    assert_eq!(row.queue.skipped_count(), 2);
    assert_eq!(row.queue.current().unwrap().title, "third");
    assert_eq!(row.toasts.back().unwrap().kind, PlaybackToastKind::Error);
    assert_eq!(row.toasts.back().unwrap().title, "Playback stopped");
}

#[test]
fn safe_dsd_lookahead_uses_the_stored_rate_without_reopening_the_file() {
    let temp = tempfile::tempdir().unwrap();
    let first = library_track(1, temp.path().join("first.wav"), "first");
    crate::backend::scan::metadata::write_test_wav(&first.path, "first", "Artist", "Album")
        .unwrap();
    let dff_path = temp.path().join("not-on-disk.dff");
    let mut second = library_track(2, dff_path.clone(), "second");
    second.sample_rate_hz = Some(2_822_400);
    second.bit_depth = Some(1);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&[first, second], 0));
    row.playback_state = PlaybackState::Playing;
    row.playback_output_mode = StoredOutputMode::BitPerfect;
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        transport: device::DeviceTransport::Usb,
    });
    let command_rx = command_sink(&mut row);

    row.sync_next_source();

    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetNext { path: dff_path }
    );
}

#[test]
fn repeat_modes_resync_the_effective_next_track() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["a", "b", "c"]);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    let command_rx = command_sink(&mut row);
    start_scripted_queue_play(&mut row, &command_rx, &tracks[0]);
    assert!(matches!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetNext { .. }
    ));
    row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Playing));

    row.cycle_repeat();
    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    row.cycle_repeat();
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: tracks[0].path.clone()
        }
    );

    let mut last = Playback::initial();
    last.seed_queue(QueueState::from_tracks(&tracks, 2));
    let last_rx = command_sink(&mut last);
    start_scripted_queue_play(&mut last, &last_rx, &tracks[2]);
    assert!(matches!(
        last_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    last.handle_event(PlaybackEvent::StateChanged(PlaybackState::Playing));
    last.cycle_repeat();
    assert_eq!(
        last_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: tracks[0].path.clone()
        }
    );
}

#[test]
fn repeat_one_resends_lookahead_after_every_advanced_event() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["loop"]);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.queue.cycle_repeat();
    row.queue.cycle_repeat();
    row.playback_state = PlaybackState::Playing;
    row.dispatched_plays = 1;
    row.sent_next = Some(tracks[0].path.clone());
    let command_rx = command_sink(&mut row);

    for _ in 0..2 {
        assert!(row.handle_event(advanced(&tracks[0].path, 1)).is_none());
        assert_eq!(
            command_rx.recv().unwrap(),
            PlaybackCommand::SetNext {
                path: tracks[0].path.clone()
            }
        );
    }
}

#[test]
fn shuffle_resyncs_lookahead() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["a", "b", "c", "d"]);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.playback_state = PlaybackState::Playing;
    row.sent_next = Some(PathBuf::from("stale.flac"));
    let command_rx = command_sink(&mut row);

    row.toggle_shuffle();

    let expected = row.queue.peek_advance_on_end().unwrap().path.clone();
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetNext { path: expected }
    );
}

#[test]
fn queue_navigation_resyncs_after_each_play_file() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["a", "b", "c"]);

    let mut next = Playback::initial();
    next.seed_queue(QueueState::from_tracks(&tracks, 0));
    next.playback_state = PlaybackState::Playing;
    next.source_path = Some(tracks[0].path.clone());
    next.sent_next = Some(tracks[1].path.clone());
    let next_rx = command_sink(&mut next);
    next.next_track();
    assert_eq!(
        next_rx.recv().unwrap(),
        PlaybackCommand::PlayFile {
            path: tracks[1].path.clone()
        }
    );
    assert_eq!(
        next_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: tracks[2].path.clone()
        }
    );

    let mut previous = Playback::initial();
    previous.seed_queue(QueueState::from_tracks(&tracks, 1));
    previous.playback_state = PlaybackState::Playing;
    previous.source_path = Some(tracks[1].path.clone());
    previous.sent_next = Some(tracks[2].path.clone());
    let previous_rx = command_sink(&mut previous);
    previous.previous_track();
    assert_eq!(
        previous_rx.recv().unwrap(),
        PlaybackCommand::PlayFile {
            path: tracks[0].path.clone()
        }
    );
    assert_eq!(
        previous_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: tracks[1].path.clone()
        }
    );

    let mut jump = Playback::initial();
    jump.seed_queue(QueueState::from_tracks(&tracks, 0));
    jump.playback_state = PlaybackState::Playing;
    jump.source_path = Some(tracks[0].path.clone());
    jump.sent_next = Some(tracks[1].path.clone());
    let jump_rx = command_sink(&mut jump);
    jump.jump_to_queue_entry(1);
    assert_eq!(
        jump_rx.recv().unwrap(),
        PlaybackCommand::PlayFile {
            path: tracks[1].path.clone()
        }
    );
    assert_eq!(
        jump_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: tracks[2].path.clone()
        }
    );
}

#[test]
fn advanced_repositions_the_queue_by_source_path() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["a", "b", "c"]);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.playback_state = PlaybackState::Playing;
    row.dispatched_plays = 1;
    row.sent_next = Some(tracks[1].path.clone());
    let command_rx = command_sink(&mut row);

    assert!(row.handle_event(advanced(&tracks[2].path, 1)).is_none());

    assert_eq!(row.queue.current().unwrap().path, tracks[2].path);
    assert_eq!(row.source_path.as_ref(), Some(&tracks[2].path));
    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
}

#[test]
fn buffered_removed_track_plays_without_dropping_its_successor() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["a", "b", "c", "d"]);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.queue.mark_started();
    row.playback_state = PlaybackState::Playing;
    row.dispatched_plays = 1;
    row.source_path = Some(tracks[0].path.clone());
    row.sent_next = Some(tracks[1].path.clone());
    let command_rx = command_sink(&mut row);

    row.remove_queue_entry(1);
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: tracks[2].path.clone()
        }
    );

    row.handle_event(advanced(&tracks[1].path, 1));
    assert_eq!(
        row.queue.current().unwrap().path,
        tracks[0].path,
        "the queue stays on the last entry that actually played"
    );
    assert_eq!(row.source_path.as_ref(), Some(&tracks[1].path));
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: tracks[2].path.clone()
        }
    );

    row.handle_event(advanced(&tracks[2].path, 1));
    assert_eq!(row.queue.current().unwrap().path, tracks[2].path);
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: tracks[3].path.clone()
        }
    );

    row.handle_event(advanced(&tracks[3].path, 1));
    assert_eq!(row.queue.current().unwrap().path, tracks[3].path);
    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
}

#[test]
fn ended_without_a_preload_keeps_the_play_file_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["a", "b"]);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.playback_state = PlaybackState::Playing;
    row.dispatched_plays = 1;
    let command_rx = command_sink(&mut row);

    let next = row
        .handle_event(PlaybackEvent::Ended { attempt: 1 })
        .expect("the fallback advances the queue");
    row.play_queue_track(next);

    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::PlayFile {
            path: tracks[1].path.clone()
        }
    );
    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
}

#[test]
fn rejected_next_is_skipped_for_preload_but_remains_explicitly_playable() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["a", "b", "c"]);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.playback_state = PlaybackState::Playing;
    row.dispatched_plays = 1;
    row.source_path = Some(tracks[0].path.clone());
    row.sent_next = Some(tracks[1].path.clone());
    let command_rx = command_sink(&mut row);

    assert!(
        row.handle_event(PlaybackEvent::NextRejected {
            attempt: 1,
            path: tracks[1].path.clone(),
            message: "decode: unreadable source".to_string(),
        })
        .is_none()
    );

    assert_eq!(row.playback_state, PlaybackState::Playing);
    assert_eq!(row.source_path.as_ref(), Some(&tracks[0].path));
    assert_eq!(row.queue.current().unwrap().path, tracks[0].path);
    assert!(!row.is_track_missing(tracks[1].id));
    assert!(row.rejected_next_track_ids.contains(&tracks[1].id));
    assert_eq!(row.queue.skipped_count(), 1);
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: tracks[2].path.clone()
        }
    );
    row.handle_event(PlaybackEvent::NextRejected {
        attempt: 1,
        path: tracks[2].path.clone(),
        message: "decode: unreadable source".to_string(),
    });
    assert_eq!(command_rx.recv().unwrap(), PlaybackCommand::ClearNext);
    assert_eq!(row.queue.skipped_count(), 2);
    assert_eq!(
        row.toasts.back(),
        Some(&PlaybackToast::warning(
            "Track skipped",
            "Skipped 2 tracks that could not be played."
        ))
    );

    row.jump_to_queue_entry(1);
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::PlayFile {
            path: tracks[1].path.clone()
        }
    );
    row.handle_event(now_playing(tracks[1].path.to_str().unwrap()));
    assert!(!row.rejected_next_track_ids.contains(&tracks[1].id));
}

#[test]
fn lookahead_command_rejections_are_advisory() {
    let mut row = Playback::initial();

    row.handle_event(PlaybackEvent::CommandRejected {
        command: "SetNext",
        state: PlaybackState::Error,
    });
    row.handle_event(PlaybackEvent::CommandRejected {
        command: "ClearNext",
        state: PlaybackState::Idle,
    });

    assert!(row.error.is_none());
}

#[test]
fn stale_advanced_is_ignored() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["a", "b"]);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.playback_state = PlaybackState::Playing;
    row.dispatched_plays = 2;
    row.source_path = Some(tracks[0].path.clone());
    row.sent_next = Some(tracks[1].path.clone());
    let command_rx = command_sink(&mut row);

    assert!(row.handle_event(advanced(&tracks[1].path, 1)).is_none());

    assert_eq!(row.queue.current().unwrap().path, tracks[0].path);
    assert_eq!(row.source_path.as_ref(), Some(&tracks[0].path));
    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
}

#[test]
fn pause_and_resume_keep_the_existing_lookahead_valid() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["a", "b"]);
    let mut row = Playback::initial();
    row.seed_queue(QueueState::from_tracks(&tracks, 0));
    row.playback_state = PlaybackState::Playing;
    row.sent_next = Some(tracks[1].path.clone());
    let command_rx = command_sink(&mut row);

    row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Paused));
    assert!(row.send_command(PlaybackCommand::Resume));
    assert_eq!(command_rx.recv().unwrap(), PlaybackCommand::Resume);
    row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
    row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Playing));

    assert_eq!(row.sent_next.as_ref(), Some(&tracks[1].path));
    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
}

#[test]
fn device_and_exclusive_mode_changes_reset_then_resync_lookahead() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["a", "b"]);

    let mut exclusive = Playback::initial();
    exclusive.seed_queue(QueueState::from_tracks(&tracks, 0));
    exclusive.playback_state = PlaybackState::Playing;
    exclusive.sent_next = Some(tracks[1].path.clone());
    let exclusive_rx = command_sink(&mut exclusive);
    assert!(exclusive.send_command(PlaybackCommand::SetExclusiveMode { enabled: false }));
    assert_eq!(
        exclusive_rx.recv().unwrap(),
        PlaybackCommand::SetExclusiveMode { enabled: false }
    );
    assert_eq!(exclusive.sent_next, None);
    exclusive.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
    exclusive.handle_event(PlaybackEvent::StateChanged(PlaybackState::Playing));
    assert_eq!(
        exclusive_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: tracks[1].path.clone()
        }
    );

    let mut output = Playback::initial();
    output.seed_queue(QueueState::from_tracks(&tracks, 0));
    output.playback_state = PlaybackState::Paused;
    output.sent_next = Some(tracks[1].path.clone());
    let output_rx = command_sink(&mut output);
    assert!(output.send_command(PlaybackCommand::SetOutputDevice {
        device_id: 9,
        kind: EngineKind::Universal {
            exclusive_mode: true,
        },
    }));
    assert_eq!(
        output_rx.recv().unwrap(),
        PlaybackCommand::SetOutputDevice {
            device_id: 9,
            kind: EngineKind::Universal {
                exclusive_mode: true,
            },
        }
    );
    assert_eq!(output.sent_next, None);
    output.handle_event(PlaybackEvent::OutputDeviceChanged {
        device_id: 9,
        kind: EngineKind::Universal {
            exclusive_mode: true,
        },
    });
    assert_eq!(
        output_rx.recv().unwrap(),
        PlaybackCommand::SetNext {
            path: tracks[1].path.clone()
        }
    );
}

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

    let toast = row.toasts.back().expect("expected an error toast");
    assert_eq!(toast.title, "Couldn't play this track");
    assert!(toast.body.contains("“corrupt”"), "{}", toast.body);
    assert!(!toast.body.contains("No track loaded"), "{}", toast.body);
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
        dropout_frames: 0,
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
        dropout_frames: 0,
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
    row.toasts.push_back(PlaybackToast::warning(
        "Track skipped",
        "Skipped “gone” — its file is missing.",
    ));

    let outcome = row.handle_event(PlaybackEvent::Error {
        attempt: 0,
        kind: PlaybackErrorKind::Track,
        message: "decode: backend stop failed".to_string(),
    });

    assert!(outcome.is_none());
    assert_eq!(
        row.toasts.back(),
        Some(&PlaybackToast::warning(
            "Track skipped",
            "Skipped “gone” — its file is missing."
        ))
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
        row.toasts.back(),
        Some(&PlaybackToast::warning(
            "Track skipped",
            "Skipped “gone” — its file is missing."
        ))
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
    // the error toast is truthful.
    assert!(row.active_playback_needs_stop());
    assert_eq!(
        row.toasts.back(),
        Some(&PlaybackToast::error(
            "Playback stopped",
            "Playback stopped — 2 tracks could not be played."
        ))
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

    assert!(
        row.handle_event(PlaybackEvent::Ended { attempt: 0 })
            .is_none()
    );
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
        dropout_frames: 0,
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
fn sustained_dropouts_show_the_existing_notice_after_three_events_in_ten_seconds() {
    let mut row = Playback::initial();
    row.dispatched_plays = 1;
    let started = std::time::Instant::now();

    row.handle_event_at(dropout(1, 1), started);
    row.handle_event_at(dropout(1, 2), started + Duration::from_secs(5));
    assert!(row.notice.is_none());
    row.handle_event_at(dropout(1, 3), started + Duration::from_secs(10));

    assert_eq!(row.dropout_frames, 3);
    assert_eq!(
        row.notice,
        Some(PlaybackNotice::Dropouts {
            text: "Playback is dropping out — the source can't keep up.".to_string(),
        })
    );
}

#[test]
fn dropout_notice_clears_after_thirty_seconds_without_another_dropout() {
    let mut row = Playback::initial();
    row.dispatched_plays = 1;
    let started = std::time::Instant::now();
    row.handle_event_at(dropout(1, 1), started);
    row.handle_event_at(dropout(1, 2), started + Duration::from_secs(1));
    row.handle_event_at(dropout(1, 3), started + Duration::from_secs(2));

    assert!(!row.clear_expired_dropout_notice_at(started + Duration::from_secs(31)));
    assert!(matches!(row.notice, Some(PlaybackNotice::Dropouts { .. })));
    assert!(row.clear_expired_dropout_notice_at(started + Duration::from_secs(32)));
    assert!(row.notice.is_none());
}

#[test]
fn spread_out_dropouts_do_not_show_a_notice() {
    let mut row = Playback::initial();
    row.dispatched_plays = 1;
    let started = std::time::Instant::now();

    row.handle_event_at(dropout(1, 1), started);
    row.handle_event_at(dropout(1, 2), started + Duration::from_secs(11));
    row.handle_event_at(dropout(1, 3), started + Duration::from_secs(22));

    assert!(row.notice.is_none());
    assert_eq!(row.recent_dropouts.len(), 1);
}

#[test]
fn dropout_window_prunes_old_events_before_counting_a_fresh_burst() {
    let mut row = Playback::initial();
    row.dispatched_plays = 1;
    let started = std::time::Instant::now();

    row.handle_event_at(dropout(1, 1), started);
    row.handle_event_at(dropout(1, 2), started + Duration::from_secs(1));
    row.handle_event_at(dropout(1, 3), started + Duration::from_secs(12));
    assert_eq!(row.recent_dropouts.len(), 1);
    assert!(row.notice.is_none());

    row.handle_event_at(dropout(1, 4), started + Duration::from_secs(13));
    row.handle_event_at(dropout(1, 5), started + Duration::from_secs(14));
    assert!(matches!(row.notice, Some(PlaybackNotice::Dropouts { .. })));
}

#[test]
fn audible_track_change_clears_dropout_notice_and_counter() {
    let mut row = Playback::initial();
    row.dispatched_plays = 1;
    let started = std::time::Instant::now();
    row.handle_event_at(dropout(1, 1), started);
    row.handle_event_at(dropout(1, 2), started + Duration::from_secs(1));
    row.handle_event_at(dropout(1, 3), started + Duration::from_secs(2));

    row.handle_event_at(
        advanced(Path::new("/Music/next.flac"), 1),
        started + Duration::from_secs(3),
    );

    assert_eq!(row.dropout_frames, 0);
    assert!(row.notice.is_none());
    assert!(row.recent_dropouts.is_empty());
}

#[test]
fn stopping_and_dismissing_reset_dropout_notice_tracking() {
    let mut row = Playback::initial();
    row.dispatched_plays = 1;
    let started = std::time::Instant::now();
    row.handle_event_at(dropout(1, 1), started);
    row.handle_event_at(dropout(1, 2), started + Duration::from_secs(1));
    row.handle_event_at(dropout(1, 3), started + Duration::from_secs(2));

    row.dismiss_notice();
    row.handle_event_at(dropout(1, 4), started + Duration::from_secs(3));
    assert!(row.notice.is_none());

    row.handle_event_at(dropout(1, 5), started + Duration::from_secs(4));
    row.handle_event_at(dropout(1, 6), started + Duration::from_secs(5));
    assert!(matches!(row.notice, Some(PlaybackNotice::Dropouts { .. })));
    row.handle_event_at(
        PlaybackEvent::StateChanged(PlaybackState::Idle),
        started + Duration::from_secs(6),
    );
    assert!(row.notice.is_none());
    assert!(row.recent_dropouts.is_empty());
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
