use super::*;

#[test]
fn only_not_connected_devices_can_be_forgotten() {
    let mut preferences = app_settings::OutputModePreferences::default();
    preferences.record_sighting("matrix", "mini-i Series", None, 100);
    preferences.record_sighting("airpods", "AirPods Pro", None, 100);

    let groups = merge_managed_devices(
        &[output_device(9, "matrix", "mini-i Series")],
        Some("matrix"),
        None,
        &preferences,
    );

    assert!(!groups.connected[0].can_forget());
    assert!(groups.not_connected[0].can_forget());
}

#[test]
fn set_as_default_moves_the_marker_without_changing_the_active_output() {
    let directory = tempfile::tempdir().unwrap();
    let matrix = output_device(9, "matrix", "mini-i Series");
    let airpods = output_device(10, "airpods", "AirPods Pro");
    let mut row = Playback::initial();
    row.settings_path = directory.path().join("settings.json");
    row.seed_devices(vec![matrix.clone(), airpods]);
    row.active_device = Some(matrix.clone());
    row.settings.saved_output_device_uid = Some(matrix.uid.clone());

    assert!(row.update_saved_output_device_uid("airpods").unwrap());

    assert_eq!(
        row.settings.saved_output_device_uid.as_deref(),
        Some("airpods")
    );
    assert_eq!(
        AppSettings::load(&row.settings_path)
            .unwrap()
            .saved_output_device_uid
            .as_deref(),
        Some("airpods")
    );
    assert_eq!(
        row.active_device.as_ref().map(|device| device.uid.as_str()),
        Some("matrix")
    );
    let groups = row.managed_device_groups();
    let active = groups
        .connected
        .iter()
        .find(|device| device.uid == "matrix")
        .unwrap();
    let saved_default = groups
        .connected
        .iter()
        .find(|device| device.uid == "airpods")
        .unwrap();
    assert!(active.active);
    assert!(!active.saved_default);
    assert!(active.can_set_as_default());
    assert!(saved_default.saved_default);
    assert!(!saved_default.can_set_as_default());
}

#[test]
fn device_page_keeps_legacy_playback_errors_visible() {
    let mut row = Playback::initial();
    row.error = Some("Drop one audio file at a time.".to_string());

    assert_eq!(
        row.device_management_messages(),
        vec![("Drop one audio file at a time.".to_string(), true)]
    );
}

#[test]
fn formats_advertised_output_capabilities_without_playback_claims() {
    assert_eq!(
        format_device_capabilities(device::OutputDeviceCapabilities {
            max_bits_per_channel: Some(24),
            max_sample_rate: 192_000.0,
            integer_wire_formats: true,
            transport: device::DeviceTransport::Usb,
        }),
        "Up to 24-bit / 192 kHz"
    );
    assert_eq!(
        format_device_capabilities(device::OutputDeviceCapabilities {
            max_bits_per_channel: None,
            max_sample_rate: 48_000.0,
            integer_wire_formats: false,
            transport: device::DeviceTransport::Bluetooth,
        }),
        "Up to 48 kHz"
    );
}

#[test]
fn auto_output_mode_uses_the_advertised_integer_depth() {
    assert_eq!(
        automatic_output_mode(&Ok(device::OutputDeviceCapabilities {
            max_bits_per_channel: Some(24),
            max_sample_rate: 192_000.0,
            integer_wire_formats: true,
            transport: device::DeviceTransport::Usb,
        })),
        StoredOutputMode::Exclusive
    );
    assert_eq!(
        automatic_output_mode(&Ok(device::OutputDeviceCapabilities {
            max_bits_per_channel: Some(24),
            max_sample_rate: 192_000.0,
            integer_wire_formats: true,
            transport: device::DeviceTransport::DisplayPort,
        })),
        StoredOutputMode::Exclusive
    );
    assert_eq!(
        automatic_output_mode(&Ok(device::OutputDeviceCapabilities {
            max_bits_per_channel: None,
            max_sample_rate: 48_000.0,
            integer_wire_formats: false,
            transport: device::DeviceTransport::BuiltIn,
        })),
        StoredOutputMode::Shared
    );
    assert_eq!(
        automatic_output_mode(&Err(EngineError::NoOutputCapabilities(9))),
        StoredOutputMode::Shared
    );
}

#[test]
fn dsd_play_file_refuses_before_dispatch_on_an_unsafe_output() {
    let path = PathBuf::from("/Music/test.dff");
    let mut row = Playback::initial();
    row.output_mode = StoredOutputMode::Shared;
    row.playback_output_mode = StoredOutputMode::Shared;
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        integer_wire_formats: true,
        transport: device::DeviceTransport::Usb,
    });
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    row.command_tx = Some(command_tx);

    row.play_file(path.clone());

    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert_eq!(row.playback_state, PlaybackState::Error);
    assert!(row.error.is_none());
    assert_eq!(
        row.toasts.back().unwrap().title,
        "DSD needs Exclusive output"
    );

    row.toggle_playback();

    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert_eq!(row.playback_state, PlaybackState::Error);

    row.current_play = None;
    row.source_path = Some(path);
    row.playback_state = PlaybackState::Ended;
    row.toggle_playback();

    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert_eq!(row.playback_state, PlaybackState::Error);
}

#[test]
fn dsd_resume_stops_instead_of_dispatching_on_an_unsafe_output() {
    let mut row = Playback::initial();
    row.source_path = Some(PathBuf::from("/Music/test.dsf"));
    row.playback_state = PlaybackState::Paused;
    row.playback_output_mode = StoredOutputMode::Shared;
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        integer_wire_formats: true,
        transport: device::DeviceTransport::Usb,
    });
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    row.command_tx = Some(command_tx);

    assert!(!row.send_command(PlaybackCommand::Resume));

    assert_eq!(command_rx.recv().unwrap(), PlaybackCommand::Stop);
    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert_eq!(row.playback_state, PlaybackState::Error);
}

#[test]
fn unsafe_output_mode_change_stops_dsd_before_reconfiguring() {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pulse-engine/tests/fixtures/dsd-interleave.dff"
    ));
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    row.output_mode = StoredOutputMode::Exclusive;
    row.playback_output_mode = StoredOutputMode::Exclusive;
    row.resolved_engine_kind = EngineKind::Integer;
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        integer_wire_formats: true,
        transport: device::DeviceTransport::Usb,
    });
    row.source_path = Some(path.clone());
    row.playback_state = PlaybackState::Playing;
    row.record_play_attempt(&path);
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    row.command_tx = Some(command_tx);

    row.apply_output_mode_if_active("matrix", StoredOutputMode::Shared);

    assert_eq!(command_rx.recv().unwrap(), PlaybackCommand::Stop);
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetOutputDevice {
            device_id: 9,
            kind: EngineKind::Universal {
                exclusive_mode: false,
            },
        }
    );
    assert!(row.error.is_none());
    assert_eq!(
        row.toasts.back().unwrap().title,
        "DSD needs Exclusive output"
    );
}

#[test]
fn dsd_action_persists_exclusive_mode_and_retries_after_integer_reconfiguration() {
    let directory = tempfile::tempdir().unwrap();
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pulse-engine/tests/fixtures/dsd-interleave.dff"
    ));
    let mut row = Playback::initial();
    row.settings_path = directory.path().join("settings.json");
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    row.output_mode = StoredOutputMode::Shared;
    row.playback_output_mode = StoredOutputMode::Shared;
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        integer_wire_formats: true,
        transport: device::DeviceTransport::Usb,
    });
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    row.command_tx = Some(command_tx);

    row.play_file(path.clone());
    assert!(matches!(
        row.toasts.back().unwrap().action,
        Some(PlaybackToastAction::SwitchToExclusive { ref device_uid })
            if device_uid == "matrix"
    ));

    row.switch_to_exclusive_and_retry("matrix".to_string());
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetOutputDevice {
            device_id: 9,
            kind: EngineKind::Integer,
        }
    );
    assert_eq!(
        row.settings
            .output_mode_preferences
            .effective_mode("matrix", StoredOutputMode::Shared),
        StoredOutputMode::Exclusive
    );

    row.handle_event(PlaybackEvent::OutputDeviceChanged {
        device_id: 9,
        kind: EngineKind::Integer,
    });
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::PlayFile { path }
    );
    assert!(row.retry.is_none());
}

#[test]
fn dsd_refusal_has_no_action_when_the_device_has_no_integer_path() {
    let path = PathBuf::from("/Music/test.dff");
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "bluetooth", "Bluetooth DAC"));
    row.output_mode = StoredOutputMode::Shared;
    row.playback_output_mode = StoredOutputMode::Shared;
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        integer_wire_formats: true,
        transport: device::DeviceTransport::Bluetooth,
    });
    let (command_tx, _command_rx) = std::sync::mpsc::channel();
    row.command_tx = Some(command_tx);

    row.play_file(path);

    assert_eq!(
        row.toasts.back().unwrap().title,
        "This device can't play DSD"
    );
    assert!(row.toasts.back().unwrap().action.is_none());
}

#[test]
fn dsd_mode_retry_is_consumed_by_a_universal_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pulse-engine/tests/fixtures/dsd-interleave.dff"
    ));
    let mut row = Playback::initial();
    row.settings_path = directory.path().join("settings.json");
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    row.output_mode = StoredOutputMode::Shared;
    row.playback_output_mode = StoredOutputMode::Shared;
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        integer_wire_formats: true,
        transport: device::DeviceTransport::Usb,
    });
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    row.command_tx = Some(command_tx);

    row.play_file(path);
    row.switch_to_exclusive_and_retry("matrix".to_string());
    assert!(matches!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetOutputDevice {
            device_id: 9,
            kind: EngineKind::Integer,
        }
    ));

    row.handle_event(PlaybackEvent::OutputDeviceChanged {
        device_id: 9,
        kind: EngineKind::Universal {
            exclusive_mode: false,
        },
    });
    assert!(!row.retry_after_output_mode_change);

    row.handle_event(PlaybackEvent::OutputDeviceChanged {
        device_id: 9,
        kind: EngineKind::Integer,
    });
    assert!(matches!(
        command_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
}

#[test]
fn exclusive_fallback_notice_names_the_device_and_marks_playback_shared() {
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    row.playback_output_mode = StoredOutputMode::Exclusive;

    row.handle_event(PlaybackEvent::ExclusiveModeFallback { device_id: 9 });

    assert_eq!(row.playback_output_mode, StoredOutputMode::Shared);
    assert_eq!(
        row.resolved_engine_kind,
        EngineKind::Universal {
            exclusive_mode: false,
        }
    );
    assert_eq!(
        row.toasts.back(),
        Some(&PlaybackToast::warning(
            "Exclusive mode unavailable",
            "mini-i Series could not start in exclusive mode. Playback continues in shared mode."
        ))
    );
}

#[test]
fn repeated_capability_refresh_replaces_its_message() {
    let mut row = Playback::initial();
    let airpods = output_device(9, "airpods", "AirPods Pro");
    row.device_message = Some(DeviceMessage {
        text: "Using the saved output device.".to_string(),
        is_error: false,
    });

    row.apply_device_capabilities_result(
        &airpods,
        Err(EngineError::NoOutputCapabilities(airpods.id)),
    );
    row.apply_device_capabilities_result(
        &airpods,
        Err(EngineError::NoOutputCapabilities(airpods.id)),
    );

    assert_eq!(
        row.displayed_device_message().unwrap().text,
        "Using the saved output device. AirPods Pro does not advertise a signed-integer PCM physical format Pulse can use."
    );
}

#[test]
fn applies_a_confirmed_output_device_and_its_success_message() {
    let mut row = Playback::initial();
    let selected = output_device(9, "matrix", "mini-i Series");

    let applied = row.apply_completed_output_device_change(
        PendingDeviceChange {
            device: selected.clone(),
            persist: false,
            success_message: Some(DeviceMessage {
                text: "Using the system default.".to_string(),
                is_error: false,
            }),
            capabilities: Ok(device::OutputDeviceCapabilities {
                max_bits_per_channel: Some(24),
                max_sample_rate: 192_000.0,
                integer_wire_formats: true,
                transport: device::DeviceTransport::Usb,
            }),
            automatic_mode: StoredOutputMode::Exclusive,
            output_mode: StoredOutputMode::Exclusive,
            engine_kind: EngineKind::Integer,
        },
        EngineKind::Integer,
    );

    assert_eq!(applied.id, selected.id);
    assert_eq!(row.active_device.as_ref().unwrap().uid, selected.uid);
    assert_eq!(row.resolved_engine_kind, EngineKind::Integer);
    assert_eq!(
        row.device_message.as_ref().unwrap().text,
        "Using the system default."
    );
}

#[test]
fn attributes_a_device_change_error_and_clears_the_pending_change() {
    let mut row = Playback::initial();
    row.pending_device_change = Some(PendingDeviceChange {
        device: output_device(9, "matrix", "mini-i Series"),
        persist: false,
        success_message: None,
        capabilities: Ok(device::OutputDeviceCapabilities {
            max_bits_per_channel: Some(24),
            max_sample_rate: 192_000.0,
            integer_wire_formats: true,
            transport: device::DeviceTransport::Usb,
        }),
        automatic_mode: StoredOutputMode::Exclusive,
        output_mode: StoredOutputMode::Exclusive,
        engine_kind: EngineKind::Integer,
    });

    row.handle_event(PlaybackEvent::Error {
        attempt: 0,
        kind: PlaybackErrorKind::Device { hog_pid: Some(42) },
        message: "device hogged by pid 42".to_string(),
    });

    assert!(row.pending_device_change.is_none());
    assert_eq!(
        row.resolved_engine_kind,
        EngineKind::Universal {
            exclusive_mode: false,
        }
    );
    assert_eq!(
        row.device_message.as_ref().unwrap().text,
        "Could not switch to mini-i Series: device hogged by pid 42"
    );
    assert_eq!(
        row.notice,
        Some(PlaybackNotice::DeviceFailure {
            text: "Playback stopped on mini-i Series: device hogged by pid 42".to_string(),
        })
    );
}

#[test]
fn bit_perfect_verdict_is_engine_owned_not_derived_from_the_mode() {
    let mut row = Playback::initial();
    row.output_mode = StoredOutputMode::Exclusive;
    row.playback_output_mode = StoredOutputMode::Exclusive;

    assert!(!row.snapshot().bit_perfect_active);

    row.handle_event(PlaybackEvent::BitPerfectStateChanged { active: true });
    assert!(row.snapshot().bit_perfect_active);

    row.handle_event(PlaybackEvent::BitPerfectStateChanged { active: false });
    assert!(!row.snapshot().bit_perfect_active);
}

#[test]
fn active_hardware_volume_is_exposed_on_the_managed_device() {
    let mut row = Playback::initial();
    let matrix = output_device(9, "matrix", "mini-i Series");
    row.seed_devices(vec![matrix.clone()]);
    row.active_device = Some(matrix);

    row.handle_event(PlaybackEvent::VolumeStateChanged(VolumeState::new(
        pulse_engine::VolumeDomain::Device,
    )));

    assert!(row.managed_device_groups().connected[0].hardware_volume_available);

    row.handle_event(PlaybackEvent::VolumeStateChanged(VolumeState::new(
        pulse_engine::VolumeDomain::Software,
    )));

    assert!(!row.managed_device_groups().connected[0].hardware_volume_available);
}

#[test]
fn changing_the_active_mode_dispatches_a_resolved_output_device_command() {
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        integer_wire_formats: true,
        transport: device::DeviceTransport::Usb,
    });
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    row.command_tx = Some(command_tx);

    row.apply_output_mode_if_active("matrix", StoredOutputMode::Exclusive);

    assert_eq!(row.output_mode, StoredOutputMode::Exclusive);
    assert_eq!(
        row.resolved_engine_kind,
        EngineKind::Universal {
            exclusive_mode: false,
        }
    );
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetOutputDevice {
            device_id: 9,
            kind: EngineKind::Integer,
        }
    );

    row.handle_event(PlaybackEvent::OutputDeviceChanged {
        device_id: 9,
        kind: EngineKind::Integer,
    });

    assert_eq!(row.resolved_engine_kind, EngineKind::Integer);
}

#[test]
fn pending_integer_switch_keeps_dsd_off_auhal_until_confirmation() {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pulse-engine/tests/fixtures/dsd-interleave.dff"
    ));
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        integer_wire_formats: true,
        transport: device::DeviceTransport::Usb,
    });
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    row.command_tx = Some(command_tx);

    row.apply_output_mode_if_active("matrix", StoredOutputMode::Exclusive);
    assert!(matches!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetOutputDevice {
            kind: EngineKind::Integer,
            ..
        }
    ));

    row.play_file(path.clone());
    assert_eq!(
        row.toasts.back().unwrap().title,
        "DSD needs Exclusive output"
    );
    assert!(command_rx.try_recv().is_err());

    row.handle_event(PlaybackEvent::OutputDeviceChanged {
        device_id: 9,
        kind: EngineKind::Integer,
    });
    row.play_file(path.clone());

    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::PlayFile { path }
    );
}

#[test]
fn pending_auhal_switch_refuses_dsd_before_confirmation() {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pulse-engine/tests/fixtures/dsd-interleave.dff"
    ));
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    row.output_mode = StoredOutputMode::Exclusive;
    row.playback_output_mode = StoredOutputMode::Exclusive;
    row.resolved_engine_kind = EngineKind::Integer;
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        integer_wire_formats: true,
        transport: device::DeviceTransport::Usb,
    });
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    row.command_tx = Some(command_tx);

    row.apply_output_mode_if_active("matrix", StoredOutputMode::Shared);
    assert!(matches!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetOutputDevice {
            kind: EngineKind::Universal {
                exclusive_mode: false
            },
            ..
        }
    ));

    row.play_file(path);

    assert_eq!(
        row.toasts.back().unwrap().title,
        "DSD needs Exclusive output"
    );
    assert!(command_rx.try_recv().is_err());
}

#[test]
fn failed_output_mode_change_leaves_the_confirmed_engine_kind() {
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        integer_wire_formats: true,
        transport: device::DeviceTransport::Usb,
    });
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    row.command_tx = Some(command_tx);

    row.apply_output_mode_if_active("matrix", StoredOutputMode::Exclusive);
    assert!(matches!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetOutputDevice {
            kind: EngineKind::Integer,
            ..
        }
    ));

    row.handle_event(PlaybackEvent::Error {
        attempt: 0,
        kind: PlaybackErrorKind::Device { hog_pid: None },
        message: "no matching physical format".to_string(),
    });

    assert_eq!(
        row.resolved_engine_kind,
        EngineKind::Universal {
            exclusive_mode: false,
        }
    );
}

#[test]
fn exclusive_uses_auhal_when_the_device_has_no_integer_path() {
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "display", "DELL U3223QE"));
    row.device_capabilities = Some(device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate: 192_000.0,
        integer_wire_formats: false,
        transport: device::DeviceTransport::DisplayPort,
    });
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    row.command_tx = Some(command_tx);

    row.apply_output_mode_if_active("display", StoredOutputMode::Exclusive);

    assert_eq!(
        row.resolved_engine_kind,
        EngineKind::Universal {
            exclusive_mode: false,
        }
    );
    assert_eq!(
        command_rx.recv().unwrap(),
        PlaybackCommand::SetOutputDevice {
            device_id: 9,
            kind: EngineKind::Universal {
                exclusive_mode: true,
            },
        }
    );

    row.handle_event(PlaybackEvent::OutputDeviceChanged {
        device_id: 9,
        kind: EngineKind::Universal {
            exclusive_mode: true,
        },
    });

    assert_eq!(
        row.resolved_engine_kind,
        EngineKind::Universal {
            exclusive_mode: true,
        }
    );
}

#[test]
fn derives_row_state_from_playback_events() {
    let mut row = Playback::initial();
    row.error = Some("old error".to_string());

    row.handle_event(PlaybackEvent::NowPlaying {
        source: pulse_engine::PlayableSource {
            path: PathBuf::from("/Music/Blonde/Nights.flac"),
            duration_ms: Some(268_000),
        },
        format: PcmFormat {
            sample_rate: 44_100,
            bits_per_sample: 16,
            channels: 2,
        },
    });
    row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Playing));
    row.handle_event(PlaybackEvent::Position {
        position_ms: 134_000,
        duration_ms: Some(268_000),
        dropout_frames: 0,
    });

    assert_eq!(row.title, "Nights");
    assert_eq!(row.secondary, "Blonde");
    assert_eq!(row.playback_state, PlaybackState::Playing);
    assert_eq!(row.snapshot().displayed_fraction(), 0.5);
    assert!(row.error.is_none());

    row.handle_event(PlaybackEvent::Ended { attempt: 0 });
    assert_eq!(row.playback_state, PlaybackState::Ended);
    assert_eq!(row.position_ms, 268_000);
}

#[test]
fn maps_real_transport_states_to_toggle_commands() {
    let mut row = Playback::initial();
    assert_eq!(row.toggle_command(), None);

    row.playback_state = PlaybackState::Playing;
    assert_eq!(row.toggle_command(), Some(PlaybackCommand::Pause));

    row.playback_state = PlaybackState::Paused;
    assert_eq!(row.toggle_command(), Some(PlaybackCommand::Resume));

    row.source_path = Some(PathBuf::from("/Music/track.flac"));
    row.playback_state = PlaybackState::Ended;
    assert_eq!(
        row.toggle_command(),
        Some(PlaybackCommand::PlayFile {
            path: PathBuf::from("/Music/track.flac")
        })
    );
}

#[test]
fn selecting_a_library_track_loads_the_idle_row_for_playback() {
    let mut row = Playback::initial();
    let path = PathBuf::from("/Music/Blonde/Nights.flac");
    let cover = PathBuf::from("/Cache/nights.cover");
    let track = TrackRef {
        id: 1,
        path: path.clone(),
        title: "Nights".to_string(),
        artist: "Frank Ocean".to_string(),
        album: "Blonde".to_string(),
        duration_ms: Some(268_000),
        sample_rate_hz: Some(44_100),
        cover_art_path: Some(cover.clone()),
    };

    assert!(row.apply_track_selection(&track));

    assert_eq!(row.source_path.as_ref(), Some(&path));
    assert_eq!(row.cover_art_path.as_ref(), Some(&cover));
    assert_eq!(row.title, "Nights");
    assert_eq!(row.secondary, "Frank Ocean - Blonde");
    assert_eq!(row.playback_state, PlaybackState::Idle);
    assert_eq!(
        row.toggle_command(),
        Some(PlaybackCommand::PlayFile { path })
    );
}

#[test]
fn selecting_a_row_does_not_replace_the_active_playback_source() {
    let mut row = Playback::initial();
    let playing = PathBuf::from("/Music/Blonde/Nights.flac");
    let playing_cover = PathBuf::from("/Cache/nights.cover");
    row.source_path = Some(playing.clone());
    row.cover_art_path = Some(playing_cover.clone());
    row.title = "Nights".to_string();
    row.playback_state = PlaybackState::Playing;
    let selection = TrackRef {
        id: 2,
        path: PathBuf::from("/Music/Blonde/Solo.flac"),
        title: "Solo".to_string(),
        artist: "Frank Ocean".to_string(),
        album: "Blonde".to_string(),
        duration_ms: None,
        sample_rate_hz: Some(44_100),
        cover_art_path: Some(PathBuf::from("/Cache/solo.cover")),
    };

    assert!(!row.apply_track_selection(&selection));

    assert_eq!(row.source_path.as_ref(), Some(&playing));
    assert_eq!(row.cover_art_path.as_ref(), Some(&playing_cover));
    assert_eq!(row.title, "Nights");
    assert_eq!(row.toggle_command(), Some(PlaybackCommand::Pause));
}

#[test]
fn now_playing_keeps_library_artist_and_album_metadata() {
    let mut row = Playback::initial();
    let track = Track {
        id: 1,
        storage_root_id: 1,
        path: PathBuf::from("/Music/菲靡靡之音/天空.flac"),
        title: Some("天空".to_string()),
        artist: Some("王菲".to_string()),
        album: Some("菲靡靡之音".to_string()),
        album_artist: None,
        year: Some(1995),
        genre: None,
        track_number: Some(1),
        disc_number: Some(1),
        duration_ms: Some(240_000),
        sample_rate_hz: Some(44_100),
        bit_depth: Some(16),
        channels: Some(2),
        file_size_bytes: 1,
        modified_at_ns: 1,
        cover_art_path: Some(PathBuf::from("/Cache/cover")),
        cover_art_mime_type: Some("image/jpeg".to_string()),
        added_at_ms: 1,
        updated_at_ms: 1,
    };
    row.seed_queue(QueueState::from_tracks(std::slice::from_ref(&track), 0));

    row.handle_event(PlaybackEvent::NowPlaying {
        source: pulse_engine::PlayableSource {
            path: track.path,
            duration_ms: Some(240_000),
        },
        format: PcmFormat {
            sample_rate: 44_100,
            bits_per_sample: 16,
            channels: 2,
        },
    });

    assert_eq!(row.title, "天空");
    assert_eq!(row.secondary, "王菲 - 菲靡靡之音");
    assert_eq!(row.cover_art_path, Some(PathBuf::from("/Cache/cover")));
}

#[test]
fn a_real_truncated_wav_decode_error_maps_to_a_track_scoped_kind() {
    let temp = tempfile::tempdir().unwrap();
    let tracks = wav_tracks(temp.path(), &["corrupt"]);
    truncate_wav(&tracks[0].path);

    let error = pulse_engine::decode::open(&tracks[0].path)
        .map(|_| ())
        .unwrap_err();
    assert_eq!(PlaybackErrorKind::from(&error), PlaybackErrorKind::Track);
}
