use super::*;

#[test]
fn priming_and_drain_tail_underruns_are_not_counted() {
    let (controller, log) = fake_controller();
    {
        let mut log = log.lock().unwrap();
        log.position_limit = 0;
        log.underrun_frames = 100;
    }
    let events = controller.subscribe();
    controller
        .command_sender()
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.backend_fed_frames == 2_000 && log.decoder_eofs.contains(&PathBuf::from("track.flac"))
    });

    log.lock().unwrap().position_limit = 100;
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 100,
                ..
            }
        )),
        PlaybackEvent::Position {
            position_ms: 100,
            duration_ms: Some(10_000),
            dropout_frames: 0,
        }
    );

    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 107;
        log.position_limit = 200;
    }
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Dropout { .. }
        )),
        PlaybackEvent::Dropout {
            attempt: 1,
            frames: 7,
            cumulative_frames: 7,
        }
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Position {
            position_ms: 200,
            duration_ms: Some(10_000),
            dropout_frames: 7,
        }
    );

    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 111;
        log.position_limit = 2_000;
    }
    wait_for(&events, |event| {
        matches!(event, PlaybackEvent::Ended { .. })
    });
    assert_no_matching_event(&events, Duration::from_millis(100), |event| {
        matches!(event, PlaybackEvent::Dropout { .. })
    });
}

#[test]
fn steady_state_underruns_report_deltas_and_track_cumulative_frames() {
    let (controller, log) = fake_controller();
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    controller
        .command_sender()
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| log.backend_fed_frames == 2_000);
    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 5;
        log.position_limit = 100;
    }
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 100,
                ..
            }
        )
    });

    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 12;
        log.position_limit = 200;
    }
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Dropout { .. }
        )),
        PlaybackEvent::Dropout {
            attempt: 1,
            frames: 7,
            cumulative_frames: 7,
        }
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Position {
            position_ms: 200,
            duration_ms: Some(10_000),
            dropout_frames: 7,
        }
    );

    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 15;
        log.position_limit = 300;
    }
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Dropout { .. }
        )),
        PlaybackEvent::Dropout {
            attempt: 1,
            frames: 3,
            cumulative_frames: 10,
        }
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Position {
            position_ms: 300,
            duration_ms: Some(10_000),
            dropout_frames: 10,
        }
    );
}

#[test]
fn underrun_free_playback_emits_no_dropout_event() {
    let (controller, log) = fake_controller();
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    controller
        .command_sender()
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| log.backend_fed_frames == 2_000);
    log.lock().unwrap().position_limit = 200;

    loop {
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            PlaybackEvent::Dropout { .. } => {
                panic!("underrun-free playback reported a dropout")
            }
            PlaybackEvent::Position {
                position_ms: 200,
                dropout_frames,
                ..
            } => {
                assert_eq!(dropout_frames, 0);
                break;
            }
            _ => {}
        }
    }
}

#[test]
fn seamless_transition_keeps_the_underrun_baseline_and_resets_track_total() {
    let (controller, log) = fake_controller();
    configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
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
    wait_for_log(&log, |log| log.backend_fed_frames == 4_000);
    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 5;
        log.position_limit = 100;
    }
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 100,
                ..
            }
        )
    });
    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 7;
        log.position_limit = 200;
    }
    wait_for(&events, |event| {
        matches!(event, PlaybackEvent::Dropout { .. })
    });
    let _ = events.recv_timeout(Duration::from_secs(1)).unwrap();

    log.lock().unwrap().position_limit = 2_000;
    loop {
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            PlaybackEvent::Dropout { .. } => {
                panic!("gapless boundary emitted a spurious dropout")
            }
            PlaybackEvent::Advanced { .. } => break,
            _ => {}
        }
    }
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Position {
            position_ms: 0,
            duration_ms: Some(2_000),
            dropout_frames: 0,
        }
    );

    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 10;
        log.position_limit = 2_100;
    }
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Dropout { .. }
        )),
        PlaybackEvent::Dropout {
            attempt: 1,
            frames: 3,
            cumulative_frames: 3,
        }
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Position {
            position_ms: 100,
            duration_ms: Some(2_000),
            dropout_frames: 3,
        }
    );
}

#[test]
fn pause_resume_preserves_the_track_tally_and_rebaselines_the_new_sink() {
    let (controller, log) = fake_controller();
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| log.backend_fed_frames == 2_000);
    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 5;
        log.position_limit = 100;
    }
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 100,
                ..
            }
        )
    });
    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 12;
        log.position_limit = 200;
    }
    wait_for(&events, |event| {
        matches!(event, PlaybackEvent::Dropout { .. })
    });
    let _ = events.recv_timeout(Duration::from_secs(1)).unwrap();

    commands.send(PlaybackCommand::Pause).unwrap();
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 200,
                ..
            }
        )),
        PlaybackEvent::Position {
            position_ms: 200,
            duration_ms: Some(10_000),
            dropout_frames: 7,
        }
    );
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
    });

    commands.send(PlaybackCommand::Resume).unwrap();
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 200,
                ..
            }
        )),
        PlaybackEvent::Position {
            position_ms: 200,
            duration_ms: Some(10_000),
            dropout_frames: 7,
        }
    );
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 400,
                ..
            }
        )
    });

    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 15;
        log.position_limit = 300;
    }
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Dropout { .. }
        )),
        PlaybackEvent::Dropout {
            attempt: 1,
            frames: 3,
            cumulative_frames: 10,
        }
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Position {
            position_ms: 500,
            duration_ms: Some(10_000),
            dropout_frames: 10,
        }
    );
}

#[test]
fn gapless_boundary_flushes_pending_dropout_before_resetting_the_incoming_track() {
    let (controller, log, clock) = fake_controller_with_stall_timeout(Duration::MAX);
    configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
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
    wait_for_log(&log, |log| log.backend_fed_frames == 4_000);
    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 5;
        log.position_limit = 100;
    }
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 100,
                ..
            }
        )
    });

    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 8;
        log.position_limit = 150;
    }
    wait_for_worker_pumps(&clock, 2);
    log.lock().unwrap().position_limit = 2_000;

    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Dropout { .. }
        )),
        PlaybackEvent::Dropout {
            attempt: 1,
            frames: 3,
            cumulative_frames: 3,
        }
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Position {
            position_ms: 2_000,
            duration_ms: Some(2_000),
            dropout_frames: 3,
        }
    );
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Advanced { .. }
    ));
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Position {
            position_ms: 0,
            duration_ms: Some(2_000),
            dropout_frames: 0,
        }
    );
}

#[test]
fn finish_flushes_pending_dropout_when_the_position_did_not_move() {
    let (controller, log, clock) = fake_controller_with_stall_timeout(Duration::MAX);
    {
        let mut log = log.lock().unwrap();
        log.position_limit = 0;
        log.decoder_starved_after_first_chunk = true;
    }
    let events = controller.subscribe();
    controller
        .command_sender()
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for_log(&log, |log| log.backend_fed_frames == 2_000);
    {
        let mut log = log.lock().unwrap();
        log.underrun_frames = 5;
        log.position_limit = 100;
    }
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 100,
                ..
            }
        )
    });
    log.lock().unwrap().position_limit = 2_000;
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 2_000,
                ..
            }
        )
    });

    log.lock().unwrap().underrun_frames = 12;
    wait_for_worker_pumps(&clock, 2);
    log.lock().unwrap().decoder_starved_after_first_chunk = false;

    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Dropout { .. }
        )),
        PlaybackEvent::Dropout {
            attempt: 1,
            frames: 7,
            cumulative_frames: 7,
        }
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Position {
            position_ms: 2_000,
            duration_ms: Some(10_000),
            dropout_frames: 7,
        }
    );
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Ended { .. }
        )),
        PlaybackEvent::Ended { attempt: 1 }
    );
}

#[test]
fn stalled_output_emits_device_error_and_engine_remains_reusable() {
    let (controller, log, clock) = fake_controller_with_stall_timeout(TEST_STALL_TIMEOUT);
    log.lock().unwrap().position_limit = 0;
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("stalled.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    clock.advance(TEST_STALL_TIMEOUT * 2);
    wait_for_worker_pumps(&clock, 2);
    assert_no_error_pending(&events);

    log.lock().unwrap().position_limit = 1_000;
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 1_000,
                ..
            }
        )
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

    commands.send(PlaybackCommand::Stop).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
    });

    log.lock().unwrap().position_limit = u64::MAX;
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("recovered.flac"),
        })
        .unwrap();
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Ended { .. }
        )),
        PlaybackEvent::Ended { attempt: 2 }
    );
}

#[test]
fn paused_playback_does_not_trigger_stall_watchdog() {
    let (controller, _, clock) = fake_controller_with_stall_timeout(TEST_STALL_TIMEOUT);
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
    clock.advance(TEST_STALL_TIMEOUT * 2);
    assert_no_matching_event(&events, TEST_STALL_TIMEOUT * 2, |event| {
        matches!(event, PlaybackEvent::Error { .. })
    });
}

#[test]
fn decoder_underrun_does_not_trigger_stall_watchdog() {
    let (controller, log, clock) = fake_controller_with_stall_timeout(TEST_STALL_TIMEOUT);
    {
        let mut log = log.lock().unwrap();
        log.position_limit = u64::MAX;
        log.decoder_starved_after_first_chunk = true;
    }
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
                position_ms: 2_000,
                ..
            }
        )
    });

    clock.advance(TEST_STALL_TIMEOUT * 2);
    wait_for_worker_pumps(&clock, 2);
    assert_no_error_pending(&events);

    commands.send(PlaybackCommand::Stop).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
    });
}
