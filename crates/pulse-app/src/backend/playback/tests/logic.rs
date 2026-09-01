use super::*;

#[test]
fn no_op_settings_update_does_not_save() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let mut playback = Playback::initial();
    playback.settings_path = path.clone();

    assert!(
        !playback
            .update_settings(|settings| settings.volume_level = 1.0)
            .unwrap()
    );
    assert!(!path.exists());
}

#[test]
fn interface_scale_uses_the_existing_atomic_settings_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let mut playback = Playback::initial();
    playback.settings_path = path.clone();

    assert!(playback.set_interface_scale(1.25));
    assert_eq!(playback.settings().interface_scale, 1.25);
    assert_eq!(AppSettings::load(&path).unwrap().interface_scale, 1.25);
    assert!(!playback.set_interface_scale(1.25));
}

#[test]
fn legacy_disabled_marker_becomes_one_active_device_override() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let mut playback = Playback::initial();
    playback.settings_path = path.clone();
    playback.settings.legacy_exclusive_mode_disabled = Some(true);

    assert!(playback.migrate_legacy_exclusive_mode("airpods").unwrap());

    assert!(
        playback
            .settings
            .output_mode_preferences
            .is_pinned("airpods")
    );
    assert_eq!(
        playback
            .settings
            .output_mode_preferences
            .effective_mode("airpods", StoredOutputMode::BitPerfect),
        StoredOutputMode::Shared
    );
    assert_eq!(
        AppSettings::load(&path)
            .unwrap()
            .legacy_exclusive_mode_disabled,
        None
    );
}

#[test]
fn forgetting_a_saved_device_updates_both_json_fields_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let mut playback = Playback::initial();
    playback.settings_path = path.clone();
    playback.settings.saved_output_device_uid = Some("matrix".to_string());
    playback.settings.output_mode_preferences.record_sighting(
        "matrix",
        "mini-i Series",
        Some(StoredDeviceCapabilities {
            max_bits_per_channel: Some(24),
            max_sample_rate: 192_000,
            transport: Some(StoredDeviceTransport::Usb),
        }),
        100,
    );

    assert!(playback.forget_device_settings("matrix").unwrap());

    let saved = AppSettings::load(&path).unwrap();
    assert_eq!(saved.saved_output_device_uid, None);
    assert!(
        !saved
            .output_mode_preferences
            .devices()
            .any(|(uid, _)| uid == "matrix")
    );
}

#[test]
fn combined_volume_update_preserves_unrelated_settings_fields() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let mut playback = Playback::initial();
    playback.settings_path = path.clone();
    playback.settings.saved_output_device_uid = Some("matrix".to_string());
    playback.settings.legacy_exclusive_mode_disabled = Some(true);
    playback
        .settings
        .output_mode_preferences
        .set_mode("matrix", StoredOutputMode::Shared);

    playback
        .update_settings(|settings| {
            settings.volume_level = 0.25;
            settings.volume_muted = false;
        })
        .unwrap();

    let loaded = AppSettings::load(&path).unwrap();
    assert_eq!(loaded.saved_output_device_uid.as_deref(), Some("matrix"));
    assert_eq!(
        loaded
            .output_mode_preferences
            .effective_mode("matrix", StoredOutputMode::BitPerfect),
        StoredOutputMode::Shared
    );
    assert_eq!(loaded.legacy_exclusive_mode_disabled, Some(true));
    assert_eq!(loaded.volume_level, 0.25);
    assert!(!loaded.volume_muted);
}

#[test]
fn explicit_output_mode_write_clears_pending_legacy_intent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let mut playback = Playback::initial();
    playback.settings_path = path.clone();
    playback.settings.legacy_exclusive_mode_disabled = Some(true);
    let mut output_modes = OutputModePreferences::default();
    output_modes.set_mode("matrix", StoredOutputMode::Exclusive);

    playback.set_output_mode_preferences(output_modes).unwrap();

    let loaded = AppSettings::load(&path).unwrap();
    assert_eq!(loaded.legacy_exclusive_mode_disabled, None);
    assert_eq!(
        loaded
            .output_mode_preferences
            .effective_mode("matrix", StoredOutputMode::Shared),
        StoredOutputMode::Exclusive
    );
}

#[test]
fn accepts_only_supported_audio_extensions() {
    for path in [
        "track.flac",
        "track.FLAC",
        "track.m4a",
        "track.aif",
        "track.aiff",
        "track.wav",
        "track.dsf",
        "track.DFF",
    ] {
        assert!(is_supported_audio(Path::new(path)), "{path}");
    }
    for path in ["track.mp3", "track.aac", "track"] {
        assert!(!is_supported_audio(Path::new(path)), "{path}");
    }
}

#[test]
fn dsd_gate_requires_bit_perfect_mode_and_the_dop_rate() {
    let dff = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pulse-engine/tests/fixtures/dsd-interleave.dff"
    ));
    let capabilities = |max_sample_rate| device::OutputDeviceCapabilities {
        max_bits_per_channel: Some(24),
        max_sample_rate,
        transport: device::DeviceTransport::Usb,
    };

    let wrong_mode =
        dsd_playback_error(dff, StoredOutputMode::Shared, Some(capabilities(192_000.0))).unwrap();
    assert_eq!(wrong_mode.title(), "DSD needs Bit-perfect output");
    assert!(wrong_mode.needs_bit_perfect());

    let rate_ceiling = dsd_playback_error_with_sample_rate(
        Path::new("track.dsf"),
        Some(5_644_800),
        StoredOutputMode::BitPerfect,
        Some(capabilities(192_000.0)),
    )
    .unwrap();
    assert_eq!(rate_ceiling.title(), "This device can't carry DSD128");
    assert_eq!(
        rate_ceiling.body(Some("Matrix mini-i")),
        "Matrix mini-i supports up to 192 kHz; DSD128 needs 352.8 kHz for DoP."
    );

    let unknown = dsd_playback_error(dff, StoredOutputMode::BitPerfect, None).unwrap();
    assert_eq!(unknown.title(), "Output device not verified yet");

    let unreadable = dsd_playback_error(
        Path::new("missing.dff"),
        StoredOutputMode::BitPerfect,
        Some(capabilities(192_000.0)),
    )
    .unwrap();
    assert_eq!(unreadable.title(), "Couldn't read this DSD file");

    assert_eq!(
        dsd_playback_error(
            dff,
            StoredOutputMode::BitPerfect,
            Some(capabilities(192_000.0))
        ),
        None
    );
}

#[test]
fn derives_display_text_from_the_path() {
    let path = Path::new("/Music/Album/Track 01.flac");
    assert_eq!(track_title(path), "Track 01");
    assert_eq!(track_secondary(path), "Album");
}

#[test]
fn formats_transport_time_without_a_leading_minute_zero() {
    assert_eq!(format_time(0), "0:00");
    assert_eq!(format_time(185_999), "3:05");
    assert_eq!(format_time(3_661_000), "1:01:01");
}

#[test]
fn clamps_progress_positions_to_the_track() {
    assert_eq!(fraction_at_x(100.0, 200.0, 50.0), 0.0);
    assert_eq!(fraction_at_x(100.0, 200.0, 200.0), 0.5);
    assert_eq!(fraction_at_x(100.0, 200.0, 350.0), 1.0);
    assert_eq!(fraction_at_x(100.0, 0.0, 100.0), 0.0);
}

#[test]
fn maps_vertical_volume_positions_bottom_to_top() {
    assert_eq!(fraction_at_y(100.0, 120.0, 220.0), 0.0);
    assert_eq!(fraction_at_y(100.0, 120.0, 160.0), 0.5);
    assert_eq!(fraction_at_y(100.0, 120.0, 100.0), 1.0);
    assert_eq!(fraction_at_y(100.0, 120.0, 250.0), 0.0);
    assert_eq!(fraction_at_y(100.0, 120.0, 50.0), 1.0);
    assert_eq!(fraction_at_y(100.0, 0.0, 100.0), 0.0);
}

#[test]
fn maps_scrub_fraction_to_position() {
    assert_eq!(scrub_position_ms(0.0, 268_000), 0);
    assert_eq!(scrub_position_ms(0.5, 268_000), 134_000);
    assert_eq!(scrub_position_ms(1.0, 268_000), 268_000);

    assert_eq!(
        scrub_position_ms(fraction_at_x(100.0, 200.0, 250.0), 268_000),
        201_000
    );
}

#[test]
fn volume_icon_and_fill_follow_the_designed_states() {
    assert_eq!(volume_icon_state(1.0, false), VolumeIconState::High);
    assert_eq!(volume_icon_state(0.5, false), VolumeIconState::High);
    assert_eq!(volume_icon_state(0.49, false), VolumeIconState::Low);
    assert_eq!(volume_icon_state(0.0, false), VolumeIconState::Muted);
    assert_eq!(volume_icon_state(0.75, true), VolumeIconState::Muted);
    assert_eq!(displayed_volume_level(0.75, false), 0.75);
    assert_eq!(displayed_volume_level(0.75, true), 0.0);
    assert_eq!(format_volume_percent(0.0), "0%");
    assert_eq!(format_volume_percent(0.7), "70%");
    assert_eq!(format_volume_percent(0.996), "99%");
    assert_eq!(format_volume_percent(1.0), "100%");
}

#[test]
fn fixed_volume_pins_the_control_and_verdicts_follow_the_transparent_path() {
    let fixed = VolumeState::new(pulse_engine::VolumeDomain::Fixed);
    assert_eq!(volume_control_level(0.2, fixed), 1.0);
    assert!(!volume_control_muted(true, fixed));
    assert_eq!(
        signal_path_verdict(false, fixed, 0.2, true),
        SignalPathVerdict::Transparent
    );
    assert_eq!(
        signal_path_verdict(true, fixed, 0.2, true),
        SignalPathVerdict::BitPerfect
    );

    let software = VolumeState::new(pulse_engine::VolumeDomain::Software);
    assert_eq!(
        signal_path_verdict(false, software, 1.0, false),
        SignalPathVerdict::Transparent
    );
    assert_eq!(
        signal_path_verdict(false, software, 0.7, false),
        SignalPathVerdict::Processed
    );
    assert_eq!(
        signal_path_verdict(false, software, 1.0, true),
        SignalPathVerdict::Processed
    );
}

#[test]
fn default_volume_command_is_unity_and_unmuted() {
    assert_eq!(
        Playback::initial().volume_command(),
        PlaybackCommand::SetVolume {
            level: 1.0,
            muted: false,
        }
    );
}

#[test]
fn formats_reported_pcm_without_inventing_codec_details() {
    let format = PcmFormat {
        sample_rate: 44_100,
        bits_per_sample: 24,
        channels: 2,
    };
    assert_eq!(
        format_quality(Some(Path::new("track.flac")), format),
        "FLAC · 24-bit"
    );
    assert_eq!(
        format_quality(Some(Path::new("track.m4a")), format),
        "M4A · 24-bit"
    );
    assert_eq!(format_sample_rate(format.sample_rate), "44.1 kHz");
    assert_eq!(
        format_quality(
            Some(Path::new("track.dff")),
            PcmFormat {
                sample_rate: 176_400,
                bits_per_sample: 24,
                channels: 2,
            }
        ),
        "DFF · DSD64"
    );
    assert_eq!(
        format_quality(
            Some(Path::new("track.dsf")),
            PcmFormat {
                sample_rate: 352_800,
                bits_per_sample: 24,
                channels: 2,
            }
        ),
        "DSF · DSD128"
    );
}

#[test]
fn shared_output_labels_the_track_rate_as_source_metadata() {
    assert_eq!(
        format_output_device(44_100, "AirPods Pro", StoredOutputMode::Shared),
        "44.1 kHz source · AirPods Pro"
    );
    assert_eq!(
        format_output_device(44_100, "mini-i Series", StoredOutputMode::Exclusive),
        "44.1 kHz · mini-i Series"
    );
    assert_eq!(
        output_mode_meta(StoredOutputMode::Shared),
        "CoreAudio · Shared"
    );
    assert_eq!(
        output_mode_meta(StoredOutputMode::Exclusive),
        "CoreAudio · Exclusive"
    );
    assert_eq!(
        output_mode_meta(StoredOutputMode::BitPerfect),
        "CoreAudio · Bit-perfect"
    );
}

#[test]
fn every_resolved_output_mode_maps_to_one_controller_kind() {
    assert_eq!(
        engine_kind_for_output_mode(StoredOutputMode::Shared),
        EngineKind::Universal {
            exclusive_mode: false,
        }
    );
    assert_eq!(
        engine_kind_for_output_mode(StoredOutputMode::Exclusive),
        EngineKind::Universal {
            exclusive_mode: true,
        }
    );
    assert_eq!(
        engine_kind_for_output_mode(StoredOutputMode::BitPerfect),
        EngineKind::BitPerfect
    );
}

#[test]
fn a_missing_stored_transport_runs_the_capability_probe() {
    let mut probe_called = false;
    let capabilities = capabilities_for_sighting(None, || {
        probe_called = true;
        Ok(device::OutputDeviceCapabilities {
            max_bits_per_channel: Some(24),
            max_sample_rate: 192_000.0,
            transport: device::DeviceTransport::Usb,
        })
    });

    assert!(probe_called);
    assert_eq!(
        capabilities.unwrap().transport,
        Some(StoredDeviceTransport::Usb)
    );
}

#[test]
fn resolves_saved_output_by_uid_and_falls_back_silently() {
    let system_default = output_device(1, "built-in", "Mac Speakers");
    let dac = output_device(9, "matrix", "mini-i Series");
    let devices = vec![system_default.clone(), dac.clone()];

    let selected = resolve_output_device(&devices, &system_default, Some("matrix"));
    assert_eq!(selected.id, dac.id);

    let selected = resolve_output_device(&devices, &system_default, Some("unplugged"));
    assert_eq!(selected.id, system_default.id);
}

#[test]
fn managed_devices_merge_connected_and_stored_rows_without_duplicates() {
    let mut preferences = app_settings::OutputModePreferences::default();
    preferences.record_sighting(
        "matrix",
        "mini-i Series",
        Some(app_settings::StoredDeviceCapabilities {
            max_bits_per_channel: Some(24),
            max_sample_rate: 192_000,
            transport: Some(StoredDeviceTransport::Usb),
        }),
        100,
    );
    preferences.record_sighting(
        "airpods",
        "AirPods Pro",
        Some(app_settings::StoredDeviceCapabilities {
            max_bits_per_channel: None,
            max_sample_rate: 48_000,
            transport: Some(StoredDeviceTransport::Bluetooth),
        }),
        90,
    );
    let connected = vec![
        output_device(9, "matrix", "mini-i Series"),
        output_device(1, "built-in", "Mac Speakers"),
    ];

    let groups = merge_managed_devices(&connected, Some("matrix"), Some("matrix"), &preferences);
    let uids = groups
        .connected
        .iter()
        .chain(&groups.not_connected)
        .map(|device| device.uid.as_str())
        .collect::<HashSet<_>>();

    assert_eq!(groups.connected.len(), 2);
    assert_eq!(groups.not_connected.len(), 1);
    assert_eq!(uids.len(), 3);
    assert_eq!(groups.not_connected[0].uid, "airpods");
}

#[test]
fn managed_device_group_moves_keep_the_stored_pin() {
    let mut preferences = app_settings::OutputModePreferences::default();
    preferences.record_sighting(
        "matrix",
        "mini-i Series",
        Some(app_settings::StoredDeviceCapabilities {
            max_bits_per_channel: Some(24),
            max_sample_rate: 192_000,
            transport: Some(StoredDeviceTransport::Usb),
        }),
        100,
    );
    preferences.set_mode("matrix", StoredOutputMode::Shared);

    let disconnected = merge_managed_devices(&[], None, None, &preferences);
    assert_eq!(
        disconnected.not_connected[0].output_mode,
        StoredOutputMode::Shared
    );
    assert!(!disconnected.not_connected[0].automatic);

    let connected = merge_managed_devices(
        &[output_device(9, "matrix", "mini-i Series")],
        Some("matrix"),
        None,
        &preferences,
    );
    assert_eq!(connected.connected[0].output_mode, StoredOutputMode::Shared);
    assert!(!connected.connected[0].automatic);
    assert!(preferences.is_pinned("matrix"));
}

#[test]
fn managed_device_groups_sort_active_first_then_alphabetically() {
    let mut preferences = app_settings::OutputModePreferences::default();
    for (uid, name) in [
        ("delta", "Delta"),
        ("charlie", "charlie"),
        ("alpha", "alpha"),
        ("zulu", "Zulu"),
        ("beta", "Beta"),
    ] {
        preferences.record_sighting(uid, name, None, 100);
    }
    let connected = vec![
        output_device(1, "zulu", "Zulu"),
        output_device(2, "beta", "Beta"),
        output_device(3, "alpha", "alpha"),
    ];

    let groups = merge_managed_devices(&connected, Some("beta"), None, &preferences);

    assert_eq!(
        groups
            .connected
            .iter()
            .map(|device| device.uid.as_str())
            .collect::<Vec<_>>(),
        vec!["beta", "alpha", "zulu"]
    );
    assert_eq!(
        groups
            .not_connected
            .iter()
            .map(|device| device.uid.as_str())
            .collect::<Vec<_>>(),
        vec!["charlie", "delta"]
    );
}
