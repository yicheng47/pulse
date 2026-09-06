use super::*;

#[test]
fn volume_survives_pause_resume_seek_track_changes_and_device_switches() {
    let (controller, log) = fake_controller();
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::SetVolume {
            level: 0.25,
            muted: true,
        })
        .unwrap();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("first.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
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
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
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

    commands
        .send(PlaybackCommand::SetOutputDevice {
            device_id: 9,
            kind: EngineKind::Universal {
                exclusive_mode: false,
            },
        })
        .unwrap();
    wait_for(&events, |event| {
        *event
            == PlaybackEvent::OutputDeviceChanged {
                device_id: 9,
                kind: EngineKind::Universal {
                    exclusive_mode: false,
                },
            }
    });

    assert_eq!(
        log.lock().unwrap().started_volumes,
        [(crate::volume_gain_for_level(0.25), true); 5]
    );
}

#[test]
fn hogged_controllable_device_writes_hardware_and_keeps_software_gain_at_unity() {
    let (controller, log) = fake_controller();
    {
        let mut log = log.lock().unwrap();
        log.hardware_volume = Some((0.4, false));
        log.hardware_volume_settable = true;
    }
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event
            == PlaybackEvent::HardwareVolume {
                level: 0.4,
                muted: false,
            }
    });
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    {
        let log = log.lock().unwrap();
        assert_eq!(log.volume_writes, [(0.4, false)]);
        assert!(log.software_volume_writes.is_empty());
        assert_eq!(log.hardware_volume_writes, [(0.4, false)]);
        assert_eq!(log.started_volumes, [(1.0, false)]);
    }

    commands
        .send(PlaybackCommand::SetVolume {
            level: 0.8,
            muted: true,
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.hardware_volume_writes == [(0.4, false), (0.8, true)]
    });

    let log = log.lock().unwrap();
    assert!(log.software_volume_writes.is_empty());
    assert_eq!(log.started_volumes, [(1.0, false)]);
}

#[test]
fn first_hardware_adoption_preserves_the_apps_mute() {
    let (controller, log) = fake_controller();
    {
        let mut log = log.lock().unwrap();
        log.hardware_volume = Some((0.4, false));
        log.hardware_volume_settable = true;
    }
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::SetVolume {
            level: 0.7,
            muted: true,
        })
        .unwrap();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();

    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::HardwareVolume { .. }
        )),
        PlaybackEvent::HardwareVolume {
            level: 0.4,
            muted: true,
        }
    );
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    let log = log.lock().unwrap();
    assert_eq!(log.hardware_volume_writes, [(0.4, true)]);
    assert_eq!(log.started_volumes, [(1.0, false)]);
}

#[test]
fn later_hog_reapplies_the_app_level_without_emitting_another_hardware_event() {
    let (controller, log) = fake_controller();
    {
        let mut log = log.lock().unwrap();
        log.hardware_volume = Some((0.5, false));
        log.hardware_volume_on_release = Some((0.5, false));
        log.hardware_volume_settable = true;
    }
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(event, PlaybackEvent::HardwareVolume { .. })
    });
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetVolume {
            level: 0.2,
            muted: false,
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.hardware_volume_writes.last() == Some(&(0.2, false))
    });

    commands.send(PlaybackCommand::Pause).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
    });
    assert_eq!(log.lock().unwrap().hardware_volume, Some((0.5, false)));
    commands.send(PlaybackCommand::Resume).unwrap();
    let mut emitted_hardware_volume = false;
    loop {
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            PlaybackEvent::HardwareVolume { .. } => emitted_hardware_volume = true,
            PlaybackEvent::StateChanged(PlaybackState::Playing) => break,
            _ => {}
        }
    }

    assert!(!emitted_hardware_volume);
    let log = log.lock().unwrap();
    assert_eq!(log.hardware_volume, Some((0.2, false)));
    assert_eq!(
        log.hardware_volume_writes,
        [(0.5, false), (0.2, false), (0.2, false)]
    );
    assert_eq!(log.started_volumes, [(1.0, false), (1.0, false)]);
}

#[test]
fn hogged_device_without_hardware_control_uses_software_gain() {
    let (controller, log) = fake_controller();
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::SetVolume {
            level: 0.5,
            muted: false,
        })
        .unwrap();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    let log = log.lock().unwrap();
    assert_eq!(log.started_volumes, [(0.125, false)]);
    assert_eq!(log.volume_writes, [(0.5, false)]);
    assert_eq!(log.software_volume_writes, [(0.125, false)]);
    assert!(log.hardware_volume_writes.is_empty());
}

#[test]
fn shared_mode_uses_software_gain_even_when_the_device_has_hardware_volume() {
    let (controller, log) = fake_controller_with_exclusive_mode(false);
    {
        let mut log = log.lock().unwrap();
        log.hardware_volume = Some((0.4, false));
        log.hardware_volume_settable = true;
    }
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::SetVolume {
            level: 0.5,
            muted: false,
        })
        .unwrap();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    let log = log.lock().unwrap();
    assert_eq!(log.started_volumes, [(0.125, false)]);
    assert_eq!(log.volume_writes, [(0.5, false)]);
    assert_eq!(log.software_volume_writes, [(0.125, false)]);
    assert!(log.hardware_volume_writes.is_empty());
}

#[test]
fn reports_device_software_fallback_and_fixed_volume_domains() {
    let (device_controller, device_log) = fake_controller();
    {
        let mut log = device_log.lock().unwrap();
        log.hardware_volume = Some((0.4, false));
        log.hardware_volume_settable = true;
    }
    let device_events = device_controller.subscribe();
    device_controller
        .command_sender()
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("device.flac"),
        })
        .unwrap();
    assert_eq!(
        wait_for(&device_events, |event| matches!(
            event,
            PlaybackEvent::VolumeStateChanged(_)
        )),
        PlaybackEvent::VolumeStateChanged(VolumeState::new(VolumeDomain::Device))
    );

    device_log.lock().unwrap().fail_exclusive_open_device = Some(9);
    device_controller
        .command_sender()
        .send(PlaybackCommand::SetOutputDevice {
            device_id: 9,
            kind: EngineKind::Universal {
                exclusive_mode: true,
            },
        })
        .unwrap();
    assert_eq!(
        wait_for(&device_events, |event| matches!(
            event,
            PlaybackEvent::VolumeStateChanged(_)
        )),
        PlaybackEvent::VolumeStateChanged(VolumeState::new(VolumeDomain::Software))
    );
    wait_for(&device_events, |event| {
        matches!(event, PlaybackEvent::ExclusiveModeFallback { device_id: 9 })
    });

    let (fixed_controller, _) = fake_controller_with_kind(EngineKind::Integer);
    let fixed_events = fixed_controller.subscribe();
    fixed_controller
        .command_sender()
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("fixed.flac"),
        })
        .unwrap();
    assert_eq!(
        wait_for(&fixed_events, |event| matches!(
            event,
            PlaybackEvent::VolumeStateChanged(_)
        )),
        PlaybackEvent::VolumeStateChanged(VolumeState::new(VolumeDomain::Fixed))
    );
    fixed_controller
        .command_sender()
        .send(PlaybackCommand::Stop)
        .unwrap();
    assert_eq!(
        wait_for(&fixed_events, |event| matches!(
            event,
            PlaybackEvent::VolumeStateChanged(_)
        )),
        PlaybackEvent::VolumeStateChanged(VolumeState::new(VolumeDomain::Software))
    );
}

#[test]
fn software_domain_does_not_change_for_steady_volume_or_pause_fades() {
    let (controller, log) = fake_controller_with_exclusive_mode(false);
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

    commands
        .send(PlaybackCommand::SetVolume {
            level: 0.7,
            muted: false,
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.software_volume_writes.last() == Some(&(0.343, false))
    });
    commands
        .send(PlaybackCommand::SetVolume {
            level: 1.0,
            muted: false,
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.software_volume_writes.last() == Some(&(1.0, false))
    });
    assert_no_matching_event(&events, Duration::from_millis(20), |event| {
        matches!(event, PlaybackEvent::VolumeStateChanged(_))
    });

    commands.send(PlaybackCommand::Pause).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
    });
    assert_no_matching_event(&events, Duration::from_millis(20), |event| {
        matches!(event, PlaybackEvent::VolumeStateChanged(_))
    });
}

#[test]
fn controllable_hog_emits_hardware_volume_once() {
    let (controller, log) = fake_controller();
    {
        let mut log = log.lock().unwrap();
        log.hardware_volume = Some((0.4, true));
        log.hardware_volume_settable = true;
    }
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("first.flac"),
        })
        .unwrap();
    assert_eq!(
        wait_for(&events, |event| matches!(
            event,
            PlaybackEvent::HardwareVolume { .. }
        )),
        PlaybackEvent::HardwareVolume {
            level: 0.4,
            muted: false,
        }
    );
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });

    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("second.flac"),
        })
        .unwrap();
    wait_for(
        &events,
        |event| matches!(event, PlaybackEvent::NowPlaying { source, .. } if source.path == Path::new("second.flac")),
    );
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    assert_no_matching_event(&events, Duration::from_millis(20), |event| {
        matches!(event, PlaybackEvent::HardwareVolume { .. })
    });
}

#[test]
fn switching_from_controllable_hog_to_shared_reapplies_last_level_as_software_gain() {
    let (controller, log) = fake_controller();
    {
        let mut log = log.lock().unwrap();
        log.hardware_volume = Some((0.5, false));
        log.hardware_volume_settable = true;
    }
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(event, PlaybackEvent::HardwareVolume { .. })
    });
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
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
        *event
            == PlaybackEvent::OutputDeviceChanged {
                device_id: 9,
                kind: EngineKind::Universal {
                    exclusive_mode: false,
                },
            }
    });

    assert_eq!(
        log.lock().unwrap().started_volumes,
        [(1.0, false), (0.125, false)]
    );
}

#[test]
fn device_round_trip_reapplies_the_adopted_level_without_readopting() {
    let (controller, log) = fake_controller();
    {
        let mut log = log.lock().unwrap();
        log.hardware_volume = Some((0.5, false));
        log.hardware_volume_settable = true;
    }
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(event, PlaybackEvent::HardwareVolume { .. })
    });
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
    });
    commands
        .send(PlaybackCommand::SetVolume {
            level: 0.2,
            muted: false,
        })
        .unwrap();
    wait_for_log(&log, |log| {
        log.hardware_volume_writes.last() == Some(&(0.2, false))
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
    commands
        .send(PlaybackCommand::SetOutputDevice {
            device_id: 7,
            kind: EngineKind::Universal {
                exclusive_mode: true,
            },
        })
        .unwrap();
    let mut emitted_hardware_volume = false;
    loop {
        match events.recv_timeout(Duration::from_secs(1)).unwrap() {
            PlaybackEvent::HardwareVolume { .. } => emitted_hardware_volume = true,
            PlaybackEvent::OutputDeviceChanged { device_id: 7, .. } => break,
            _ => {}
        }
    }

    assert!(!emitted_hardware_volume);
    assert_eq!(log.lock().unwrap().hardware_volume, Some((0.2, false)));
}

#[test]
fn coalesces_queued_volume_commands_before_transport() {
    let (command_tx, command_rx) = mpsc::channel();
    command_tx
        .send(PlaybackCommand::SetVolume {
            level: 0.2,
            muted: false,
        })
        .unwrap();
    command_tx
        .send(PlaybackCommand::SetVolume {
            level: 0.7,
            muted: true,
        })
        .unwrap();
    command_tx.send(PlaybackCommand::Pause).unwrap();

    let PlaybackCommand::SetVolume { level, muted } = command_rx.recv().unwrap() else {
        panic!("first queued command must set volume");
    };
    assert_eq!(
        coalesce_volume_commands(&command_rx, level, muted),
        (0.7, true, Some(PlaybackCommand::Pause))
    );
}
