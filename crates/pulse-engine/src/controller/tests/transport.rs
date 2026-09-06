use super::*;

#[test]
fn load_prepares_a_paused_source_without_opening_or_starting_the_backend() {
    let (controller, log) = fake_controller_with_seek_offset(250);
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::Load {
            path: PathBuf::from("track.flac"),
            position_ms: 5_000,
        })
        .unwrap();

    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::NowPlaying { .. }
        )),
        PlaybackEvent::NowPlaying {
            source: PlayableSource {
                path: PathBuf::from("track.flac"),
                duration_ms: Some(10_000),
            },
            format: TEST_FORMAT,
        }
    );
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Position { .. }
        )),
        PlaybackEvent::Position {
            position_ms: 4_750,
            duration_ms: Some(10_000),
            dropout_frames: 0,
        }
    );
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
    });
    {
        let log = log.lock().unwrap();
        assert!(log.opened_devices.is_empty());
        assert!(log.backend_starts.is_empty());
        assert_eq!(log.seek_positions, [5_000]);
    }

    commands.send(PlaybackCommand::Resume).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    let log = log.lock().unwrap();
    assert_eq!(log.opened_devices, [7]);
    assert_eq!(log.backend_starts, [TEST_FORMAT]);
    assert_eq!(log.seek_positions, [5_000]);
}

#[test]
fn load_is_rejected_while_a_source_is_paused() {
    let (controller, log) = fake_controller();
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::Load {
            path: PathBuf::from("first.flac"),
            position_ms: 0,
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
    });

    commands
        .send(PlaybackCommand::Load {
            path: PathBuf::from("second.flac"),
            position_ms: 0,
        })
        .unwrap();

    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::CommandRejected { .. }
        )),
        PlaybackEvent::CommandRejected {
            command: "Load",
            state: PlaybackState::Paused,
        }
    );
    assert_eq!(
        log.lock().unwrap().decoder_opens,
        [PathBuf::from("first.flac")]
    );
}

#[test]
fn unreadable_loads_return_to_idle_and_increment_attempts() {
    let (controller, log) = fake_controller();
    log.lock()
        .unwrap()
        .unreadable_paths
        .extend([PathBuf::from("first.flac"), PathBuf::from("second.flac")]);
    let events = controller.subscribe();
    let commands = controller.command_sender();

    for (path, attempt) in [("first.flac", 1), ("second.flac", 2)] {
        commands
            .send(PlaybackCommand::Load {
                path: PathBuf::from(path),
                position_ms: 1_000,
            })
            .unwrap();
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Error { .. }
            )),
            PlaybackEvent::Error {
                attempt,
                kind: crate::PlaybackErrorKind::Track,
                message: "decode: unreadable source".to_string(),
            }
        );
    }

    let log = log.lock().unwrap();
    assert!(log.opened_devices.is_empty());
    assert!(log.backend_starts.is_empty());
}

#[test]
fn universal_pause_releases_backend_while_seek_reuses_resumed_backend() {
    let (controller, log) = fake_controller();
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 1_000,
                ..
            }
        )
    });

    commands.send(PlaybackCommand::Pause).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
    });

    commands.send(PlaybackCommand::Resume).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    commands
        .send(PlaybackCommand::Seek { position_ms: 5_000 })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 5_000,
                ..
            }
        )
    });
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    commands.send(PlaybackCommand::Stop).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
    });

    let log = log.lock().unwrap();
    assert_eq!(log.opened_devices, [7, 7]);
    assert_eq!(log.seek_positions, [1_000, 5_000]);
    assert_eq!(log.stops, 3);
    assert_eq!(log.releases, 2);
}

#[test]
fn integer_pause_retains_backend_and_resume_reuses_it() {
    let (controller, log) = fake_controller_with_kind(EngineKind::Integer);
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::BitPerfectStateChanged { active: true }
    });
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 1_000,
                ..
            }
        )
    });

    commands.send(PlaybackCommand::Pause).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
    });
    {
        let log = log.lock().unwrap();
        assert_eq!(log.opened_devices, [7]);
        assert_eq!(log.stops, 1);
        assert_eq!(log.releases, 0);
    }

    commands.send(PlaybackCommand::Resume).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands.send(PlaybackCommand::Stop).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::BitPerfectStateChanged { active: false }
    });
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
    });

    let log = log.lock().unwrap();
    assert_eq!(log.opened_devices, [7]);
    assert_eq!(log.backend_starts, [TEST_FORMAT, TEST_FORMAT]);
    assert_eq!(log.stops, 2);
    assert_eq!(log.releases, 1);
}

#[test]
fn stop_releases_a_backend_retained_by_pause() {
    let (controller, log) = fake_controller_with_kind(EngineKind::Integer);
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 1_000,
                ..
            }
        )
    });
    commands.send(PlaybackCommand::Pause).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
    });

    commands.send(PlaybackCommand::Stop).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
    });

    let log = log.lock().unwrap();
    assert_eq!(log.stops, 1);
    assert_eq!(log.releases, 1);
}

#[test]
fn illegal_command_order_is_rejected_without_changing_idle_state() {
    let (controller, _) = fake_controller();
    let events = controller.subscribe();
    let commands = controller.command_sender();

    commands.send(PlaybackCommand::Pause).unwrap();
    commands.send(PlaybackCommand::Resume).unwrap();
    commands
        .send(PlaybackCommand::Seek { position_ms: 500 })
        .unwrap();

    let mut rejections = Vec::new();
    while rejections.len() < 3 {
        let event = events.recv_timeout(Duration::from_secs(1)).unwrap();
        match event {
            PlaybackEvent::CommandRejected { command, state } => rejections.push((command, state)),
            PlaybackEvent::StateChanged(state) => {
                panic!("illegal command changed state to {state:?}")
            }
            _ => {}
        }
    }

    assert_eq!(
        rejections,
        [
            ("Pause", PlaybackState::Idle),
            ("Resume", PlaybackState::Idle),
            ("Seek", PlaybackState::Idle),
        ]
    );
}

#[test]
fn paused_seek_resumes_from_the_original_target_without_compounding_seek_error() {
    let (controller, log) = fake_controller_with_seek_offset(250);
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 1_000,
                ..
            }
        )
    });
    commands.send(PlaybackCommand::Pause).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
    });

    commands
        .send(PlaybackCommand::Seek { position_ms: 5_000 })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 4_750,
                ..
            }
        )
    });
    commands.send(PlaybackCommand::Resume).unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 4_750,
                ..
            }
        )
    });
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    assert_eq!(log.lock().unwrap().seek_positions, [5_000]);
}

#[test]
fn paused_seek_resume_drains_pending_pcm_from_the_prepared_decoder() {
    let (controller, log) = fake_controller();
    configure_decoder(&log, "track.flac", TEST_FORMAT, 10_000, 2_000);
    log.lock()
        .unwrap()
        .decoder_specs
        .get_mut(Path::new("track.flac"))
        .unwrap()
        .seek_pending_frames = 37;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 1_000,
                ..
            }
        )
    });
    commands.send(PlaybackCommand::Pause).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
    });

    commands
        .send(PlaybackCommand::Seek { position_ms: 5_000 })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 5_000,
                ..
            }
        )
    });
    commands.send(PlaybackCommand::Resume).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    wait_for_log(&log, |log| {
        log.prepared_pcm_drains == [PathBuf::from("track.flac")]
    });

    let log = log.lock().unwrap();
    assert_eq!(
        log.decoder_opens
            .iter()
            .filter(|path| path.as_path() == Path::new("track.flac"))
            .count(),
        2
    );
    assert_eq!(log.seek_positions, [5_000]);
}

#[test]
fn end_of_track_emits_ended_and_supports_stop_from_ended() {
    let (controller, log) = fake_controller();
    log.lock().unwrap().position_limit = u64::MAX;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    let mut ending_events = Vec::new();
    loop {
        let event = events.recv_timeout(Duration::from_secs(1)).unwrap();
        let ended = matches!(event, PlaybackEvent::Ended { .. });
        ending_events.push(event);
        if ended {
            break;
        }
    }
    assert_eq!(
        ending_events,
        [
            PlaybackEvent::Position {
                position_ms: 2_000,
                duration_ms: Some(10_000),
                dropout_frames: 0,
            },
            PlaybackEvent::StateChanged(PlaybackState::Ended),
            PlaybackEvent::Ended { attempt: 1 },
        ]
    );

    commands.send(PlaybackCommand::Resume).unwrap();
    assert_eq!(
        wait_for(&events, |event| {
            matches!(event, PlaybackEvent::CommandRejected { .. })
        }),
        PlaybackEvent::CommandRejected {
            command: "Resume",
            state: PlaybackState::Ended,
        }
    );

    commands.send(PlaybackCommand::Stop).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
    });
    assert_eq!(log.lock().unwrap().stops, 1);
}

#[test]
fn end_of_track_stop_failure_preserves_ended_before_error() {
    let (controller, log) = fake_controller();
    {
        let mut log = log.lock().unwrap();
        log.position_limit = u64::MAX;
        log.stop_error = true;
    }
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();

    wait_for(&events, |event| {
        matches!(event, PlaybackEvent::Ended { .. })
    });
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Error {
            attempt: 1,
            kind: crate::PlaybackErrorKind::Track,
            message: "decode: backend stop failed".to_string(),
        }
    );
    assert_eq!(log.lock().unwrap().stops, 1);
}

#[test]
fn play_file_while_playing_reuses_backend_for_new_track() {
    let (controller, log) = fake_controller();
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("first.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 1_000,
                ..
            }
        )
    });

    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("second.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::NowPlaying { source, .. }
                if source.path == Path::new("second.flac")
        )
    });
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    let log = log.lock().unwrap();
    assert_eq!(log.opened_devices, [7]);
    assert_eq!(log.stops, 1);
}
