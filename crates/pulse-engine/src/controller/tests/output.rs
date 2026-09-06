use super::*;

#[test]
fn device_switch_releases_a_backend_retained_by_pause() {
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

    commands
        .send(PlaybackCommand::SetOutputDevice {
            device_id: 9,
            kind: EngineKind::Integer,
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::OutputDeviceChanged { device_id: 9, .. }
        )
    });

    let log = log.lock().unwrap();
    assert_eq!(log.opened_devices, [7]);
    assert_eq!(log.releases, 1);
}

#[test]
fn engine_switch_releases_a_backend_retained_by_pause() {
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

    commands
        .send(PlaybackCommand::SetOutputDevice {
            device_id: 7,
            kind: EngineKind::Universal {
                exclusive_mode: false,
            },
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::OutputDeviceChanged { device_id: 7, .. }
        )
    });

    let log = log.lock().unwrap();
    assert_eq!(log.engine_kinds, [EngineKind::Integer]);
    assert_eq!(log.releases, 1);
}

#[test]
fn exclusive_mode_command_does_not_change_integer_backend() {
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
    commands
        .send(PlaybackCommand::SetExclusiveMode { enabled: false })
        .unwrap();
    commands.send(PlaybackCommand::Resume).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    let log = log.lock().unwrap();
    assert_eq!(log.engine_kinds, [EngineKind::Integer]);
    assert_eq!(log.releases, 0);
}

#[test]
fn changing_output_device_restarts_active_playback() {
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

    commands
        .send(PlaybackCommand::SetOutputDevice {
            device_id: 9,
            kind: EngineKind::Universal {
                exclusive_mode: false,
            },
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    assert_eq!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::OutputDeviceChanged {
            device_id: 9,
            kind: EngineKind::Universal {
                exclusive_mode: false,
            },
        }
    );

    let log = log.lock().unwrap();
    assert_eq!(log.opened_devices, [7, 9]);
    assert_eq!(log.exclusive_modes, [true, false]);
    assert_eq!(log.seek_positions, [1_000]);
}

#[test]
fn changing_exclusive_mode_reopens_the_backend_without_losing_position() {
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

    commands
        .send(PlaybackCommand::SetExclusiveMode { enabled: false })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    let log = log.lock().unwrap();
    assert_eq!(log.opened_devices, [7, 7]);
    assert_eq!(log.exclusive_modes, [true, false]);
    assert_eq!(log.seek_positions, [1_000]);
}

#[test]
fn shared_mode_start_uses_a_shared_backend() {
    let (controller, log) = fake_controller_with_exclusive_mode(false);
    let events = controller.subscribe();
    controller
        .command_sender()
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    assert_eq!(log.lock().unwrap().exclusive_modes, [false]);
}

#[test]
fn integer_start_failure_surfaces_without_float_fallback() {
    let (controller, log) = fake_controller_with_kind(EngineKind::Integer);
    log.lock().unwrap().fail_integer_start_device = Some(7);
    let events = controller.subscribe();
    controller
        .command_sender()
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();

    let mut saw_fallback = false;
    let error = loop {
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            PlaybackEvent::ExclusiveModeFallback { .. } => saw_fallback = true,
            event @ PlaybackEvent::Error { .. } => break event,
            _ => {}
        }
    };

    assert!(!saw_fallback);
    assert_eq!(
        error,
        PlaybackEvent::Error {
            attempt: 1,
            kind: crate::PlaybackErrorKind::Device { hog_pid: None },
            message: "AudioDeviceStart failed (OSStatus -1)".to_string(),
        }
    );
    let log = log.lock().unwrap();
    assert_eq!(log.opened_devices, [7]);
    assert_eq!(log.engine_kinds, [EngineKind::Integer]);
    assert_eq!(log.releases, 1);
}

#[test]
fn integer_restart_failure_clears_the_confirmed_state() {
    let (controller, log) = fake_controller_with_kind(EngineKind::Integer);
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("first.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::BitPerfectStateChanged { active: true }
    });
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    log.lock().unwrap().fail_integer_start_device = Some(7);
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("second.flac"),
        })
        .unwrap();

    assert_eq!(
        wait_for(&events, |event| {
            *event == PlaybackEvent::BitPerfectStateChanged { active: false }
        }),
        PlaybackEvent::BitPerfectStateChanged { active: false }
    );
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::Error { .. }
        )),
        PlaybackEvent::Error {
            attempt: 2,
            kind: crate::PlaybackErrorKind::Device { hog_pid: None },
            message: "AudioDeviceStart failed (OSStatus -1)".to_string(),
        }
    );
    assert_eq!(log.lock().unwrap().releases, 1);
}

#[test]
fn exclusive_open_failure_retries_shared_once_for_the_device_session() {
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
    log.lock().unwrap().fail_exclusive_open_device = Some(9);

    commands
        .send(PlaybackCommand::SetOutputDevice {
            device_id: 9,
            kind: EngineKind::Universal {
                exclusive_mode: true,
            },
        })
        .unwrap();
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::ExclusiveModeFallback { .. }
        )),
        PlaybackEvent::ExclusiveModeFallback { device_id: 9 }
    );
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::OutputDeviceChanged { .. }
        )),
        PlaybackEvent::OutputDeviceChanged {
            device_id: 9,
            kind: EngineKind::Universal {
                exclusive_mode: false,
            },
        }
    );

    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("second.flac"),
        })
        .unwrap();
    wait_for(
        &events,
        |event| matches!(event, PlaybackEvent::NowPlaying { source, .. } if source.path == Path::new("second.flac")),
    );
    while let Ok(event) = events.recv_timeout(Duration::from_millis(50)) {
        assert!(!matches!(
            event,
            PlaybackEvent::ExclusiveModeFallback { .. }
        ));
    }

    let log = log.lock().unwrap();
    assert_eq!(log.opened_devices, [7, 9, 9]);
    assert_eq!(log.exclusive_modes, [true, true, false]);
    assert_eq!(log.seek_positions, [1_000]);
}

#[test]
fn unsupported_exclusive_nominal_rate_retries_shared() {
    let (controller, log) = fake_controller();
    log.lock().unwrap().fail_exclusive_start_device = Some(7);
    let events = controller.subscribe();
    controller
        .command_sender()
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();

    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::ExclusiveModeFallback { .. }
        )),
        PlaybackEvent::ExclusiveModeFallback { device_id: 7 }
    );
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    assert_eq!(log.lock().unwrap().exclusive_modes, [true, false]);
}

#[test]
fn reselecting_exclusive_after_fallback_retries_exclusive() {
    let (controller, log) = fake_controller();
    log.lock().unwrap().fail_exclusive_start_device = Some(7);
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(event, PlaybackEvent::ExclusiveModeFallback { device_id: 7 })
    });
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    log.lock().unwrap().fail_exclusive_start_device = None;
    commands
        .send(PlaybackCommand::SetOutputDevice {
            device_id: 7,
            kind: EngineKind::Universal {
                exclusive_mode: true,
            },
        })
        .unwrap();

    assert_eq!(
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::OutputDeviceChanged {
                    device_id: 7,
                    kind: EngineKind::Universal {
                        exclusive_mode: true,
                    },
                }
            )
        }),
        PlaybackEvent::OutputDeviceChanged {
            device_id: 7,
            kind: EngineKind::Universal {
                exclusive_mode: true,
            },
        }
    );
    assert_eq!(log.lock().unwrap().exclusive_modes, [true, false, true]);
}

#[test]
fn output_device_failure_stops_playback_and_emits_error() {
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
    log.lock().unwrap().fail_all_open_device = Some(9);

    commands
        .send(PlaybackCommand::SetOutputDevice {
            device_id: 9,
            kind: EngineKind::Universal {
                exclusive_mode: true,
            },
        })
        .unwrap();
    let error = wait_for(&events, |event| {
        matches!(event, PlaybackEvent::Error { .. })
    });

    assert_eq!(
        error,
        PlaybackEvent::Error {
            attempt: 1,
            kind: crate::PlaybackErrorKind::Device { hog_pid: None },
            message: "audio unit: output unavailable".to_string()
        }
    );

    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    let log = log.lock().unwrap();
    assert_eq!(log.opened_devices, [7, 9, 9, 7]);
    assert_eq!(log.stops, 1);
}

#[test]
fn backend_stop_failure_emits_error_instead_of_paused() {
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
    log.lock().unwrap().stop_error = true;

    commands.send(PlaybackCommand::Pause).unwrap();
    let error = wait_for(&events, |event| {
        matches!(event, PlaybackEvent::Error { .. })
    });

    assert_eq!(
        error,
        PlaybackEvent::Error {
            attempt: 1,
            kind: crate::PlaybackErrorKind::Track,
            message: "decode: backend stop failed".to_string()
        }
    );
    assert_eq!(log.lock().unwrap().stops, 1);
}
