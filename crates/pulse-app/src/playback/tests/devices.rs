use super::*;

#[test]
fn only_not_connected_devices_can_be_forgotten() {
    let mut preferences = app_settings::ExclusiveModePreferences::default();
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
        }),
        "Up to 24-bit / 192 kHz"
    );
    assert_eq!(
        format_device_capabilities(device::OutputDeviceCapabilities {
            max_bits_per_channel: None,
            max_sample_rate: 48_000.0,
        }),
        "Up to 48 kHz"
    );
}

#[test]
fn device_capabilities_choose_the_unset_exclusive_mode_default() {
    assert!(default_exclusive_mode(&Ok(
        device::OutputDeviceCapabilities {
            max_bits_per_channel: Some(24),
            max_sample_rate: 192_000.0,
        }
    )));
    assert!(!default_exclusive_mode(&Ok(
        device::OutputDeviceCapabilities {
            max_bits_per_channel: None,
            max_sample_rate: 48_000.0,
        }
    )));
    assert!(!default_exclusive_mode(&Err(
        EngineError::NoOutputCapabilities(9)
    )));
}

#[test]
fn exclusive_fallback_notice_names_the_device_and_marks_playback_shared() {
    let mut row = Playback::initial();
    row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
    row.playback_exclusive_mode = true;

    row.handle_event(PlaybackEvent::ExclusiveModeFallback { device_id: 9 });

    assert!(!row.playback_exclusive_mode);
    assert_eq!(
        row.notice,
        Some(PlaybackNotice::ExclusiveFallback {
            text: "mini-i Series could not start in exclusive mode. Playback continues in shared mode."
                .to_string(),
        })
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
            }),
            default_exclusive_mode: true,
            exclusive_mode: true,
        },
        true,
    );

    assert_eq!(applied.id, selected.id);
    assert_eq!(row.active_device.as_ref().unwrap().uid, selected.uid);
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
        }),
        default_exclusive_mode: true,
        exclusive_mode: true,
    });

    row.handle_event(PlaybackEvent::Error {
        attempt: 0,
        kind: PlaybackErrorKind::Device { hog_pid: Some(42) },
        message: "device hogged by pid 42".to_string(),
    });

    assert!(row.pending_device_change.is_none());
    assert_eq!(
        row.device_message.as_ref().unwrap().text,
        "Could not switch to mini-i Series: device hogged by pid 42"
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
