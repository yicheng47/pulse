use super::*;

pub(crate) fn format_volume_percent(level: f32) -> String {
    format!("{:.0}%", level.clamp(0.0, 1.0) * 100.0)
}

pub(crate) fn displayed_volume_level(level: f32, muted: bool) -> f32 {
    if muted { 0.0 } else { level }
}

pub(crate) fn volume_icon_state(level: f32, muted: bool) -> VolumeIconState {
    if muted || level == 0.0 {
        VolumeIconState::Muted
    } else if level >= 0.5 {
        VolumeIconState::High
    } else {
        VolumeIconState::Low
    }
}

pub(crate) fn scrub_position_ms(fraction: f32, duration_ms: u64) -> u64 {
    (duration_ms as f64 * f64::from(fraction)).round() as u64
}

pub(crate) fn playback_state_label(state: PlaybackState) -> &'static str {
    match state {
        PlaybackState::Idle => "idle",
        PlaybackState::Loading => "loading",
        PlaybackState::Playing => "playing",
        PlaybackState::Paused => "paused",
        PlaybackState::Stopping => "stopping",
        PlaybackState::Ended => "ended",
        PlaybackState::Error => "in error",
    }
}

/// Header meta line: "N UP NEXT · M MIN". The minutes are the rounded sum of
/// the known upcoming durations; with no known durations the minutes are
/// omitted rather than shown as zero.
pub(crate) fn format_queue_meta(count: usize, total_ms: u64) -> String {
    if total_ms == 0 {
        return format!("{count} UP NEXT");
    }
    let minutes = ((total_ms + 30_000) / 60_000).max(1);
    format!("{count} UP NEXT · {minutes} MIN")
}

/// Queue-popover times drop the leading zero on minutes ("1:24", "3:59").
pub(crate) fn format_queue_time(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    if minutes >= 60 {
        format!("{}:{:02}:{seconds:02}", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

pub(crate) fn format_time(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    if minutes >= 60 {
        format!("{}:{:02}:{seconds:02}", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

pub(crate) fn format_output_device(
    sample_rate: u32,
    device_name: &str,
    exclusive_mode: bool,
) -> String {
    let sample_rate = format_sample_rate(sample_rate);
    if exclusive_mode {
        format!("{sample_rate} · {device_name}")
    } else {
        format!("{sample_rate} source · {device_name}")
    }
}

pub(crate) fn format_sample_rate(sample_rate: u32) -> String {
    if sample_rate.is_multiple_of(1_000) {
        format!("{} kHz", sample_rate / 1_000)
    } else {
        format!("{:.1} kHz", sample_rate as f64 / 1_000.0)
    }
}

pub(crate) fn format_quality(path: Option<&Path>, format: PcmFormat) -> String {
    let container = path
        .and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_uppercase)
        .unwrap_or_else(|| "PCM".to_string());
    format!("{container} · {}-bit", format.bits_per_sample)
}

pub(crate) fn track_secondary(path: &Path) -> String {
    path.parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub(crate) fn track_title(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub(crate) fn is_supported_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

pub(crate) fn format_capability_ceiling(
    max_bits_per_channel: Option<u32>,
    max_sample_rate: u32,
) -> String {
    let sample_rate = format_sample_rate(max_sample_rate);
    match max_bits_per_channel {
        Some(bits) => format!("Up to {bits}-bit / {sample_rate}"),
        None => format!("Up to {sample_rate}"),
    }
}

pub(crate) fn format_stored_device_capabilities(capabilities: StoredDeviceCapabilities) -> String {
    format_capability_ceiling(
        capabilities.max_bits_per_channel,
        capabilities.max_sample_rate,
    )
}

pub(crate) fn format_device_capabilities(capabilities: device::OutputDeviceCapabilities) -> String {
    format_capability_ceiling(
        capabilities.max_bits_per_channel,
        capabilities.max_sample_rate.round() as u32,
    )
}

pub(crate) fn default_exclusive_mode(
    capabilities: &Result<device::OutputDeviceCapabilities, EngineError>,
) -> bool {
    capabilities
        .as_ref()
        .is_ok_and(|capabilities| capabilities.max_bits_per_channel.is_some())
}

pub(crate) fn merge_managed_devices(
    connected_devices: &[device::Device],
    active_device_uid: Option<&str>,
    saved_output_device_uid: Option<&str>,
    preferences: &ExclusiveModePreferences,
) -> ManagedDeviceGroups {
    let mut merged = BTreeMap::new();
    for (uid, stored) in preferences.devices() {
        let default_exclusive_mode = stored
            .capabilities
            .is_some_and(|capabilities| capabilities.max_bits_per_channel.is_some());
        merged.insert(
            uid.to_string(),
            ManagedDevice {
                uid: uid.to_string(),
                name: stored.name.clone().unwrap_or_else(|| uid.to_string()),
                capabilities: stored.capabilities,
                last_seen_unix_seconds: stored.last_seen_unix_seconds,
                connected: false,
                active: false,
                saved_default: saved_output_device_uid == Some(uid),
                default_exclusive_mode,
                exclusive_mode: preferences.effective_mode(uid, default_exclusive_mode),
                automatic: stored.exclusive_mode_override().is_none(),
            },
        );
    }
    for connected in connected_devices {
        let managed = merged
            .entry(connected.uid.clone())
            .or_insert_with(|| ManagedDevice {
                uid: connected.uid.clone(),
                name: connected.name.clone(),
                capabilities: None,
                last_seen_unix_seconds: None,
                connected: true,
                active: false,
                saved_default: saved_output_device_uid == Some(connected.uid.as_str()),
                default_exclusive_mode: false,
                exclusive_mode: preferences.effective_mode(&connected.uid, false),
                automatic: !preferences.is_overridden(&connected.uid),
            });
        managed.name = connected.name.clone();
        managed.connected = true;
        managed.active = active_device_uid == Some(connected.uid.as_str());
        managed.saved_default = saved_output_device_uid == Some(connected.uid.as_str());
    }

    let mut groups = ManagedDeviceGroups::default();
    for managed in merged.into_values() {
        if managed.connected {
            groups.connected.push(managed);
        } else {
            groups.not_connected.push(managed);
        }
    }
    groups.connected.sort_by_cached_key(|managed| {
        (
            !managed.active,
            managed.name.to_lowercase(),
            managed.uid.clone(),
        )
    });
    groups
        .not_connected
        .sort_by_cached_key(|managed| (managed.name.to_lowercase(), managed.uid.clone()));
    groups
}

pub(crate) fn stored_device_capabilities(
    capabilities: device::OutputDeviceCapabilities,
) -> StoredDeviceCapabilities {
    StoredDeviceCapabilities {
        max_bits_per_channel: capabilities.max_bits_per_channel,
        max_sample_rate: capabilities.max_sample_rate.round() as u32,
    }
}

pub(crate) fn resolve_output_device(
    devices: &[device::Device],
    system_default: &device::Device,
    preferred_uid: Option<&str>,
) -> device::Device {
    preferred_uid
        .and_then(|uid| devices.iter().find(|device| device.uid == uid))
        .cloned()
        .unwrap_or_else(|| system_default.clone())
}

pub(crate) fn fraction_at_x(origin_x: f32, width: f32, x: f32) -> f32 {
    if width <= 0.0 {
        return 0.0;
    }
    ((x - origin_x) / width).clamp(0.0, 1.0)
}

pub(crate) fn fraction_at_y(origin_y: f32, height: f32, y: f32) -> f32 {
    if height <= 0.0 {
        return 0.0;
    }
    ((origin_y + height - y) / height).clamp(0.0, 1.0)
}
