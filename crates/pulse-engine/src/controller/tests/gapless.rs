use super::*;

#[test]
fn same_format_preload_advances_only_at_the_fed_frame_boundary() {
    let (controller, log) = fake_controller();
    configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
    configure_decoder(&log, "b.flac", TEST_FORMAT, 4_000, 2_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.backend_fed_frames == 4_000 && log.decoder_eofs.contains(&PathBuf::from("b.flac"))
    });

    log.lock().unwrap().position_limit = 1_999;
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 1_999,
                ..
            }
        )),
        PlaybackEvent::Position {
            position_ms: 1_999,
            duration_ms: Some(2_000),
            dropout_frames: 0,
        }
    );
    assert_no_matching_event(&events, Duration::from_millis(100), |event| {
        matches!(event, PlaybackEvent::Advanced { .. })
    });

    log.lock().unwrap().position_limit = 2_000;
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Advanced { .. }
        )),
        PlaybackEvent::Advanced {
            attempt: 1,
            source: PlayableSource {
                path: PathBuf::from("b.flac"),
                duration_ms: Some(4_000),
            },
            format: TEST_FORMAT,
        }
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Position {
            position_ms: 0,
            duration_ms: Some(4_000),
            dropout_frames: 0,
        }
    );

    log.lock().unwrap().position_limit = 2_500;
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 500,
                ..
            }
        )),
        PlaybackEvent::Position {
            position_ms: 500,
            duration_ms: Some(4_000),
            dropout_frames: 0,
        }
    );
    let log = log.lock().unwrap();
    assert_eq!(log.backend_starts, [TEST_FORMAT]);
    assert_eq!(log.stops, 0);
}

#[test]
fn continuous_progress_across_seamless_boundary_does_not_stall() {
    let (controller, log, clock) = fake_controller_with_stall_timeout(TEST_STALL_TIMEOUT);
    configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
    configure_decoder(&log, "b.flac", TEST_FORMAT, 10_000, 10_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| log.backend_fed_frames == 12_000);

    let mut position = 0;
    let mut advanced = false;
    for _ in 0..8 {
        position += 500;
        log.lock().unwrap().position_limit = position;
        wait_for_worker_pumps(&clock, 2);
        clock.advance(TEST_STALL_TIMEOUT / 4);
        while let Ok(event) = events.try_recv() {
            match event {
                PlaybackEvent::Advanced { .. } => advanced = true,
                PlaybackEvent::Error { message, .. } => {
                    panic!("continuous backend progress stalled: {message}")
                }
                _ => {}
            }
        }
    }
    wait_for_worker_pumps(&clock, 2);
    assert_no_error_pending(&events);
    assert!(advanced, "test progress never crossed the audible boundary");

    commands.send(PlaybackCommand::Stop).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
    });
}

#[test]
fn stalled_output_at_seamless_boundary_still_times_out() {
    let (controller, log, clock) = fake_controller_with_stall_timeout(TEST_STALL_TIMEOUT);
    configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
    configure_decoder(&log, "b.flac", TEST_FORMAT, 10_000, 10_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| log.backend_fed_frames == 12_000);

    log.lock().unwrap().position_limit = 2_000;
    wait_for(&events, |event| {
        matches!(event, PlaybackEvent::Advanced { .. })
    });
    wait_for_worker_pumps(&clock, 2);
    clock.advance(TEST_STALL_TIMEOUT);
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Error { .. }
        )),
        PlaybackEvent::Error {
            attempt: 1,
            kind: crate::PlaybackErrorKind::Device { hog_pid: None },
            message: "timed out waiting for audio output progress".to_string(),
        }
    );
}

#[test]
fn format_mismatch_rebuilds_backend_and_still_advances() {
    let (controller, log) = fake_controller();
    configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
    configure_decoder(&log, "b.flac", ALT_FORMAT, 1_000, 4_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| log.backend_fed_frames == 2_000);

    log.lock().unwrap().position_limit = 2_000;
    let advanced = loop {
        let event = events.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(!matches!(
            event,
            PlaybackEvent::BitPerfectStateChanged { .. } | PlaybackEvent::VolumeStateChanged(_)
        ));
        if matches!(event, PlaybackEvent::Advanced { .. }) {
            break event;
        }
    };
    assert_eq!(
        advanced,
        PlaybackEvent::Advanced {
            attempt: 1,
            source: PlayableSource {
                path: PathBuf::from("b.flac"),
                duration_ms: Some(1_000),
            },
            format: ALT_FORMAT,
        }
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Position {
            position_ms: 0,
            duration_ms: Some(1_000),
            dropout_frames: 0,
        }
    );
    let log = log.lock().unwrap();
    assert_eq!(log.releases, 0);
    assert_eq!(log.opened_devices, [7]);
    assert_eq!(log.backend_starts, [TEST_FORMAT, ALT_FORMAT]);
    assert!(log.stops >= 1);
}

#[test]
fn integer_format_mismatch_reuses_backend_without_state_flicker() {
    let (controller, log) = fake_controller_with_kind(EngineKind::Integer);
    configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
    configure_decoder(&log, "b.flac", ALT_FORMAT, 1_000, 4_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();

    let mut bit_perfect_states = Vec::new();
    let mut volume_state_changes = 0;
    loop {
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            PlaybackEvent::BitPerfectStateChanged { active } => {
                bit_perfect_states.push(active);
            }
            PlaybackEvent::VolumeStateChanged(_) => volume_state_changes += 1,
            PlaybackEvent::StateChanged(PlaybackState::Playing) => break,
            _ => {}
        }
    }

    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| log.backend_fed_frames == 2_000);
    log.lock().unwrap().position_limit = 2_000;

    let advanced = loop {
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            PlaybackEvent::BitPerfectStateChanged { active } => {
                bit_perfect_states.push(active);
            }
            PlaybackEvent::VolumeStateChanged(_) => volume_state_changes += 1,
            event @ PlaybackEvent::Advanced { .. } => break event,
            _ => {}
        }
    };
    assert_eq!(
        advanced,
        PlaybackEvent::Advanced {
            attempt: 1,
            source: PlayableSource {
                path: PathBuf::from("b.flac"),
                duration_ms: Some(1_000),
            },
            format: ALT_FORMAT,
        }
    );
    assert_eq!(bit_perfect_states, [true]);
    assert_eq!(volume_state_changes, 1);

    let log = log.lock().unwrap();
    assert_eq!(log.releases, 0);
    assert_eq!(log.opened_devices, [7]);
    assert_eq!(log.backend_starts, [TEST_FORMAT, ALT_FORMAT]);
    assert!(log.stops >= 1);
}

#[test]
fn integer_boundary_start_failure_releases_the_reused_backend() {
    let (controller, log) = fake_controller_with_kind(EngineKind::Integer);
    configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
    configure_decoder(&log, "b.flac", ALT_FORMAT, 1_000, 4_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::BitPerfectStateChanged { active: true }
    });
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| log.backend_fed_frames == 2_000);
    {
        let mut log = log.lock().unwrap();
        log.fail_integer_start_device = Some(7);
        log.position_limit = 2_000;
    }

    assert_eq!(
        wait_for(&events, |event| {
            *event == PlaybackEvent::BitPerfectStateChanged { active: false }
        }),
        PlaybackEvent::BitPerfectStateChanged { active: false }
    );
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Error)
    });
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Error { .. }
        )),
        PlaybackEvent::Error {
            attempt: 1,
            kind: crate::PlaybackErrorKind::Device { hog_pid: None },
            message: "AudioDeviceStart failed (OSStatus -1)".to_string(),
        }
    );
    {
        let log = log.lock().unwrap();
        assert_eq!(log.releases, 1);
        assert_eq!(log.opened_devices, [7]);
        assert_eq!(log.backend_starts, [TEST_FORMAT]);
    }

    commands.send(PlaybackCommand::Stop).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
    });
    log.lock().unwrap().fail_integer_start_device = None;
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("c.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    assert_eq!(log.lock().unwrap().opened_devices, [7, 7]);
}

#[test]
fn exclusive_boundary_start_failure_falls_back_to_a_fresh_shared_backend() {
    let (controller, log) = fake_controller();
    configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
    configure_decoder(&log, "b.flac", ALT_FORMAT, 1_000, 4_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| log.backend_fed_frames == 2_000);
    {
        let mut log = log.lock().unwrap();
        log.fail_exclusive_start_device = Some(7);
        log.position_limit = 2_000;
    }

    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::ExclusiveModeFallback { .. }
        )),
        PlaybackEvent::ExclusiveModeFallback { device_id: 7 }
    );
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Advanced { .. }
        )),
        PlaybackEvent::Advanced {
            attempt: 1,
            source: PlayableSource {
                path: PathBuf::from("b.flac"),
                duration_ms: Some(1_000),
            },
            format: ALT_FORMAT,
        }
    );

    let log = log.lock().unwrap();
    assert_eq!(log.releases, 1);
    assert_eq!(log.opened_devices, [7, 7]);
    assert_eq!(
        log.engine_kinds,
        [
            EngineKind::Universal {
                exclusive_mode: true,
            },
            EngineKind::Universal {
                exclusive_mode: false,
            },
        ]
    );
    assert_eq!(log.backend_starts, [TEST_FORMAT, ALT_FORMAT]);
}

#[test]
fn unreadable_set_next_is_rejected_and_current_playback_continues() {
    let (controller, log) = fake_controller();
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    log.lock()
        .unwrap()
        .unreadable_paths
        .insert(PathBuf::from("bad.flac"));
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("bad.flac"),
        })
        .unwrap();

    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::NextRejected { .. }
        )),
        PlaybackEvent::NextRejected {
            attempt: 1,
            path: PathBuf::from("bad.flac"),
            message: "decode: unreadable source".to_string(),
        }
    );
    assert_no_matching_event(&events, Duration::from_millis(100), |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Error)
    });
    log.lock().unwrap().position_limit = u64::MAX;
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Ended { .. }
        )),
        PlaybackEvent::Ended { attempt: 1 }
    );
}

#[test]
fn redundant_same_path_set_next_reuses_the_existing_preload() {
    let (controller, log) = fake_controller();
    {
        let mut log = log.lock().unwrap();
        log.position_limit = 0;
        log.decoder_starved_after_first_chunk = true;
    }
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.decoder_opens.contains(&PathBuf::from("b.flac"))
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(20));

    assert_eq!(
        log.lock()
            .unwrap()
            .decoder_opens
            .iter()
            .filter(|path| path.as_path() == Path::new("b.flac"))
            .count(),
        1
    );
}

#[test]
fn clear_next_before_eof_preserves_existing_ended_behavior() {
    let (controller, log) = fake_controller();
    {
        let mut log = log.lock().unwrap();
        log.position_limit = 0;
        log.decoder_starved_after_first_chunk = true;
    }
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    commands.send(PlaybackCommand::ClearNext).unwrap();
    wait_for_log(&log, |log| {
        log.decoder_opens.contains(&PathBuf::from("b.flac"))
    });
    {
        let mut log = log.lock().unwrap();
        log.decoder_starved_after_first_chunk = false;
        log.position_limit = u64::MAX;
    }

    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Ended { .. }
        )),
        PlaybackEvent::Ended { attempt: 1 }
    );
    let log = log.lock().unwrap();
    assert_eq!(log.backend_starts, [TEST_FORMAT]);
    assert_eq!(log.stops, 1);
}

#[test]
fn set_next_during_transition_replaces_the_incoming_tracks_successor() {
    let (controller, log) = fake_controller();
    for path in ["a.flac", "b.flac", "c.flac", "d.flac"] {
        configure_decoder(&log, path, TEST_FORMAT, 2_000, 2_000);
    }
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.decoder_eofs.contains(&PathBuf::from("b.flac"))
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("c.flac"),
        })
        .unwrap();
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("d.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.decoder_opens.contains(&PathBuf::from("d.flac"))
    });

    log.lock().unwrap().position_limit = 2_000;
    assert!(matches!(
        wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac"))),
        PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")
    ));
    log.lock().unwrap().position_limit = 4_000;
    assert!(matches!(
        wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("d.flac"))),
        PlaybackEvent::Advanced { source, .. } if source.path == Path::new("d.flac")
    ));
    assert_no_matching_event(
        &events,
        Duration::from_millis(100),
        |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("c.flac")),
    );
    let log = log.lock().unwrap();
    assert_eq!(log.backend_starts, [TEST_FORMAT]);
    assert_eq!(log.stops, 0);
}

#[test]
fn clear_next_during_transition_keeps_incoming_audio_but_clears_its_successor() {
    let (controller, log) = fake_controller();
    for path in ["a.flac", "b.flac", "c.flac"] {
        configure_decoder(&log, path, TEST_FORMAT, 2_000, 2_000);
    }
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.decoder_eofs.contains(&PathBuf::from("b.flac"))
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("c.flac"),
        })
        .unwrap();
    commands.send(PlaybackCommand::ClearNext).unwrap();

    log.lock().unwrap().position_limit = 2_000;
    assert!(matches!(
        wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac"))),
        PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")
    ));
    log.lock().unwrap().position_limit = 4_000;
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Ended { .. }
        )),
        PlaybackEvent::Ended { attempt: 1 }
    );
    assert_no_matching_event(
        &events,
        Duration::from_millis(100),
        |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("c.flac")),
    );
}

#[test]
fn pause_during_transition_repreloads_incoming_from_zero() {
    let (controller, log) = fake_controller();
    configure_decoder(&log, "a.flac", TEST_FORMAT, 3_000, 2_000);
    configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.decoder_eofs.contains(&PathBuf::from("b.flac"))
    });
    log.lock().unwrap().position_limit = 500;
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 500,
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
        assert_eq!(
            log.decoder_opens
                .iter()
                .filter(|path| path.as_path() == Path::new("b.flac"))
                .count(),
            2
        );
        assert!(log.seek_positions.is_empty());
    }

    log.lock().unwrap().position_limit = 0;
    commands.send(PlaybackCommand::Resume).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    wait_for_log(&log, |log| log.backend_fed_frames == 4_000);
    log.lock().unwrap().position_limit = 2_000;
    assert!(matches!(
        wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac"))),
        PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")
    ));
    assert_eq!(log.lock().unwrap().seek_positions, [500]);
}

#[test]
fn pause_with_unreadable_buffered_source_is_advisory_and_resume_works() {
    let (controller, log) = fake_controller();
    configure_decoder(&log, "a.flac", TEST_FORMAT, 3_000, 2_000);
    configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.decoder_eofs.contains(&PathBuf::from("b.flac"))
    });
    log.lock().unwrap().position_limit = 500;
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 500,
                ..
            }
        )
    });
    log.lock()
        .unwrap()
        .unreadable_paths
        .insert(PathBuf::from("b.flac"));

    commands.send(PlaybackCommand::Pause).unwrap();
    let mut next_rejected = false;
    loop {
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            PlaybackEvent::NextRejected {
                attempt,
                path,
                message,
            } => {
                assert_eq!(attempt, 1);
                assert_eq!(path, PathBuf::from("b.flac"));
                assert_eq!(message, "decode: unreadable source");
                next_rejected = true;
            }
            PlaybackEvent::StateChanged(PlaybackState::Paused) => break,
            PlaybackEvent::StateChanged(PlaybackState::Error) => {
                panic!("unreadable lookahead made pause fatal")
            }
            _ => {}
        }
    }
    assert!(next_rejected);

    log.lock().unwrap().position_limit = 0;
    commands.send(PlaybackCommand::Resume).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    log.lock().unwrap().position_limit = u64::MAX;
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Ended { .. }
        )),
        PlaybackEvent::Ended { attempt: 1 }
    );
}

#[test]
fn pause_after_boundary_crossing_uses_the_incoming_tracks_position() {
    let (controller, log) = fake_controller();
    configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
    configure_decoder(&log, "b.flac", TEST_FORMAT, 3_000, 2_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| log.backend_fed_frames == 4_000);

    log.lock().unwrap().position_limit = 2_500;
    commands.send(PlaybackCommand::Pause).unwrap();
    wait_for(
        &events,
        |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")),
    );
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 500,
                ..
            }
        )),
        PlaybackEvent::Position {
            position_ms: 500,
            duration_ms: Some(3_000),
            dropout_frames: 0,
        }
    );
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
    });
}

#[test]
fn seek_during_transition_repreloads_incoming_from_zero() {
    let (controller, log) = fake_controller();
    configure_decoder(&log, "a.flac", TEST_FORMAT, 4_000, 2_000);
    configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.decoder_eofs.contains(&PathBuf::from("b.flac"))
    });

    commands
        .send(PlaybackCommand::Seek { position_ms: 1_000 })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    wait_for_log(&log, |log| log.backend_fed_frames == 4_000);
    {
        let log = log.lock().unwrap();
        assert_eq!(log.seek_positions, [1_000]);
        assert_eq!(
            log.decoder_opens
                .iter()
                .filter(|path| path.as_path() == Path::new("b.flac"))
                .count(),
            2
        );
    }
    log.lock().unwrap().position_limit = 2_000;
    assert!(matches!(
        wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac"))),
        PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")
    ));
    let log = log.lock().unwrap();
    assert_eq!(log.backend_starts, [TEST_FORMAT, TEST_FORMAT]);
    assert_eq!(log.stops, 1);
}

#[test]
fn output_device_change_drops_buffered_transition_and_successor() {
    let (controller, log) = fake_controller();
    for path in ["a.flac", "b.flac", "c.flac"] {
        configure_decoder(&log, path, TEST_FORMAT, 3_000, 2_000);
    }
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.decoder_eofs.contains(&PathBuf::from("b.flac"))
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("c.flac"),
        })
        .unwrap();
    log.lock().unwrap().position_limit = 500;
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 500,
                ..
            }
        )
    });

    commands
        .send(PlaybackCommand::SetOutputDevice {
            device_id: 9,
            kind: EngineKind::Universal {
                exclusive_mode: false,
            },
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::OutputDeviceChanged { device_id: 9, .. }
        )
    });
    {
        let log = log.lock().unwrap();
        assert_eq!(log.backend_starts, [TEST_FORMAT, TEST_FORMAT]);
        assert_eq!(log.stops, 1);
        assert_eq!(log.seek_positions, [500]);
        assert_eq!(
            log.decoder_opens
                .iter()
                .filter(|path| path.as_path() == Path::new("b.flac"))
                .count(),
            1
        );
    }

    log.lock().unwrap().position_limit = u64::MAX;
    loop {
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            PlaybackEvent::Advanced { source, .. } => {
                panic!("unexpected advance to {}", source.path.display())
            }
            PlaybackEvent::Ended { attempt } => {
                assert_eq!(attempt, 1);
                break;
            }
            _ => {}
        }
    }
}

#[test]
fn unchanged_output_device_keeps_buffered_transition_without_restart() {
    let (controller, log) = fake_controller();
    configure_decoder(&log, "a.flac", TEST_FORMAT, 3_000, 2_000);
    configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.decoder_eofs.contains(&PathBuf::from("b.flac"))
    });

    commands
        .send(PlaybackCommand::SetOutputDevice {
            device_id: 7,
            kind: EngineKind::Universal {
                exclusive_mode: true,
            },
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::OutputDeviceChanged { device_id: 7, .. }
        )
    });
    {
        let log = log.lock().unwrap();
        assert_eq!(log.backend_starts, [TEST_FORMAT]);
        assert_eq!(log.stops, 0);
        assert!(log.seek_positions.is_empty());
    }

    log.lock().unwrap().position_limit = 2_000;
    assert!(matches!(
        wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac"))),
        PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")
    ));
}

#[test]
fn exclusive_mode_change_drops_buffered_transition() {
    let (controller, log) = fake_controller();
    configure_decoder(&log, "a.flac", TEST_FORMAT, 3_000, 2_000);
    configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.decoder_eofs.contains(&PathBuf::from("b.flac"))
    });
    log.lock().unwrap().position_limit = 500;
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 500,
                ..
            }
        )
    });

    commands
        .send(PlaybackCommand::SetExclusiveMode { enabled: false })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    {
        let log = log.lock().unwrap();
        assert_eq!(log.backend_starts, [TEST_FORMAT, TEST_FORMAT]);
        assert_eq!(log.stops, 1);
        assert_eq!(log.seek_positions, [500]);
        assert_eq!(
            log.decoder_opens
                .iter()
                .filter(|path| path.as_path() == Path::new("b.flac"))
                .count(),
            1
        );
    }

    log.lock().unwrap().position_limit = u64::MAX;
    loop {
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            PlaybackEvent::Advanced { source, .. } => {
                panic!("unexpected advance to {}", source.path.display())
            }
            PlaybackEvent::Ended { attempt } => {
                assert_eq!(attempt, 1);
                break;
            }
            _ => {}
        }
    }
}

#[test]
fn unchanged_exclusive_mode_keeps_buffered_transition_without_restart() {
    let (controller, log) = fake_controller();
    configure_decoder(&log, "a.flac", TEST_FORMAT, 3_000, 2_000);
    configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.decoder_eofs.contains(&PathBuf::from("b.flac"))
    });

    commands
        .send(PlaybackCommand::SetExclusiveMode { enabled: true })
        .unwrap();
    commands
        .send(PlaybackCommand::SetOutputDevice {
            device_id: 7,
            kind: EngineKind::Universal {
                exclusive_mode: true,
            },
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::OutputDeviceChanged { device_id: 7, .. }
        )
    });
    {
        let log = log.lock().unwrap();
        assert_eq!(log.backend_starts, [TEST_FORMAT]);
        assert_eq!(log.stops, 0);
        assert!(log.seek_positions.is_empty());
    }

    log.lock().unwrap().position_limit = 2_000;
    assert!(matches!(
        wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac"))),
        PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")
    ));
}

#[test]
fn stop_drops_buffered_transition_before_a_later_play() {
    let (controller, log) = fake_controller();
    for path in ["a.flac", "b.flac", "c.flac"] {
        configure_decoder(&log, path, TEST_FORMAT, 2_000, 2_000);
    }
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.decoder_eofs.contains(&PathBuf::from("b.flac"))
    });

    commands.send(PlaybackCommand::Stop).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
    });
    log.lock().unwrap().position_limit = u64::MAX;
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("c.flac"),
        })
        .unwrap();
    loop {
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            PlaybackEvent::Advanced { source, .. } => {
                panic!("unexpected advance to {}", source.path.display())
            }
            PlaybackEvent::Ended { attempt } => {
                assert_eq!(attempt, 2);
                break;
            }
            _ => {}
        }
    }
}

#[test]
fn play_file_drops_buffered_transition() {
    let (controller, log) = fake_controller();
    for path in ["a.flac", "b.flac", "c.flac"] {
        configure_decoder(&log, path, TEST_FORMAT, 2_000, 2_000);
    }
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("a.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.decoder_eofs.contains(&PathBuf::from("b.flac"))
    });

    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("c.flac"),
        })
        .unwrap();
    wait_for(
        &events,
        |event| matches!(event, PlaybackEvent::NowPlaying { source, .. } if source.path == Path::new("c.flac")),
    );
    log.lock().unwrap().position_limit = u64::MAX;
    loop {
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            PlaybackEvent::Advanced { source, .. } => {
                panic!("unexpected advance to {}", source.path.display())
            }
            PlaybackEvent::Ended { attempt } => {
                assert_eq!(attempt, 2);
                break;
            }
            _ => {}
        }
    }
    let log = log.lock().unwrap();
    assert_eq!(log.backend_starts, [TEST_FORMAT, TEST_FORMAT]);
    assert_eq!(log.stops, 2);
}

#[test]
fn next_source_commands_are_rejected_when_nothing_can_follow() {
    let (controller, _) = fake_controller();
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::SetNext {
            path: PathBuf::from("b.flac"),
        })
        .unwrap();
    commands.send(PlaybackCommand::ClearNext).unwrap();

    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::CommandRejected {
                command: "SetNext",
                ..
            }
        )),
        PlaybackEvent::CommandRejected {
            command: "SetNext",
            state: PlaybackState::Idle,
        }
    );
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::CommandRejected {
                command: "ClearNext",
                ..
            }
        )),
        PlaybackEvent::CommandRejected {
            command: "ClearNext",
            state: PlaybackState::Idle,
        }
    );
}
