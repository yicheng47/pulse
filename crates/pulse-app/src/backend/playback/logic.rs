use super::*;

pub(crate) fn format_volume_percent(level: f32) -> String {
    let level = level.clamp(0.0, 1.0);
    let percent = if level == 1.0 {
        100
    } else {
        (level * 100.0).round().min(99.0) as u32
    };
    format!("{percent}%")
}

pub(crate) fn displayed_volume_level(level: f32, muted: bool) -> f32 {
    if muted { 0.0 } else { level }
}

pub(crate) fn volume_control_level(level: f32, state: VolumeState) -> f32 {
    if state.domain == pulse_engine::VolumeDomain::Fixed {
        1.0
    } else {
        level
    }
}

pub(crate) fn volume_control_muted(muted: bool, state: VolumeState) -> bool {
    state.domain != pulse_engine::VolumeDomain::Fixed && muted
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SignalPathVerdict {
    BitPerfect,
    Transparent,
    Processed,
}

pub(crate) fn signal_path_verdict(
    bit_perfect_active: bool,
    volume_state: VolumeState,
    volume_level: f32,
    volume_muted: bool,
) -> SignalPathVerdict {
    if bit_perfect_active {
        SignalPathVerdict::BitPerfect
    } else if volume_state.transparent(volume_level, volume_muted) {
        SignalPathVerdict::Transparent
    } else {
        SignalPathVerdict::Processed
    }
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
    mode: StoredOutputMode,
) -> String {
    let sample_rate = format_sample_rate(sample_rate);
    match mode {
        StoredOutputMode::Shared => format!("{sample_rate} source · {device_name}"),
        StoredOutputMode::Exclusive => format!("{sample_rate} · {device_name}"),
    }
}

pub(crate) fn output_mode_meta(mode: StoredOutputMode) -> &'static str {
    match mode {
        StoredOutputMode::Shared => "CoreAudio · Shared",
        StoredOutputMode::Exclusive => "CoreAudio · Exclusive",
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
    if matches!(container.as_str(), "DSF" | "DFF") {
        let dsd = match format.sample_rate {
            176_400 => "DSD64",
            352_800 => "DSD128",
            _ => "DSD",
        };
        return format!("{container} · {dsd}");
    }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DsdPlaybackError {
    NeedsExclusive,
    NoIntegerPath,
    Unreadable {
        detail: String,
    },
    DeviceNotVerified,
    RateCeiling {
        dsd_label: &'static str,
        required_rate_hz: u32,
        device_ceiling_hz: u32,
    },
}

impl DsdPlaybackError {
    pub(crate) fn title(&self) -> String {
        match self {
            Self::NeedsExclusive => "DSD needs Exclusive output".to_string(),
            Self::NoIntegerPath => "This device can't play DSD".to_string(),
            Self::Unreadable { .. } => "Couldn't read this DSD file".to_string(),
            Self::DeviceNotVerified => "Output device not verified yet".to_string(),
            Self::RateCeiling { dsd_label, .. } => {
                format!("This device can't carry {dsd_label}")
            }
        }
    }

    pub(crate) fn body(&self, device_name: Option<&str>) -> String {
        let device_name = device_name.unwrap_or("The active output device");
        match self {
            Self::NeedsExclusive => "DoP only survives Pulse's untouched integer path, and that path needs the device to itself. Switch to Exclusive to play this track.".to_string(),
            Self::NoIntegerPath => format!(
                "{device_name} has no integer path Pulse can trust for DoP — Exclusive here is transparent, not bit-perfect. Choose an output with an integer path."
            ),
            Self::Unreadable { detail } => detail.clone(),
            Self::DeviceNotVerified => {
                format!("Pulse hasn't verified {device_name}'s sample-rate ceiling yet.")
            }
            Self::RateCeiling {
                dsd_label,
                required_rate_hz,
                device_ceiling_hz,
            } => format!(
                "{device_name} supports up to {}; {dsd_label} needs {} for DoP.",
                format_sample_rate(*device_ceiling_hz),
                format_sample_rate(*required_rate_hz),
            ),
        }
    }

    pub(crate) fn needs_exclusive(&self) -> bool {
        matches!(self, Self::NeedsExclusive)
    }
}

pub(crate) fn dsd_playback_error(
    path: &Path,
    engine_kind: EngineKind,
    capabilities: Option<device::OutputDeviceCapabilities>,
) -> Option<DsdPlaybackError> {
    dsd_playback_error_with_sample_rate(path, None, engine_kind, capabilities)
}

pub(crate) fn dsd_playback_error_with_sample_rate(
    path: &Path,
    sample_rate_hz: Option<u32>,
    engine_kind: EngineKind,
    capabilities: Option<device::OutputDeviceCapabilities>,
) -> Option<DsdPlaybackError> {
    let is_dsd = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("dsf") || extension.eq_ignore_ascii_case("dff")
        });
    if !is_dsd {
        return None;
    }
    if engine_kind != EngineKind::Integer {
        return Some(match capabilities {
            Some(capabilities) if capabilities.has_integer_path() => {
                DsdPlaybackError::NeedsExclusive
            }
            Some(_) => DsdPlaybackError::NoIntegerPath,
            None => DsdPlaybackError::DeviceNotVerified,
        });
    }
    let dsd_rate = match sample_rate_hz {
        Some(sample_rate) => sample_rate,
        None => match crate::backend::scan::metadata::dsd_sample_rate(path) {
            Ok(sample_rate) => sample_rate,
            Err(error) => {
                return Some(DsdPlaybackError::Unreadable {
                    detail: format!("Pulse couldn't read the DSD sample rate: {error}."),
                });
            }
        },
    };
    if !matches!(dsd_rate, 2_822_400 | 5_644_800) {
        return Some(DsdPlaybackError::Unreadable {
            detail: format!(
                "Pulse supports DSD64 and DSD128, not a {} DSD stream.",
                format_sample_rate(dsd_rate)
            ),
        });
    }
    let dop_rate = dsd_rate / 16;
    let Some(capabilities) = capabilities else {
        return Some(DsdPlaybackError::DeviceNotVerified);
    };
    if capabilities.max_sample_rate < f64::from(dop_rate) {
        let dsd_label = match dsd_rate {
            2_822_400 => "DSD64",
            5_644_800 => "DSD128",
            _ => "DSD",
        };
        return Some(DsdPlaybackError::RateCeiling {
            dsd_label,
            required_rate_hz: dop_rate,
            device_ceiling_hz: capabilities.max_sample_rate.round() as u32,
        });
    }
    None
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

pub(crate) fn automatic_output_mode(
    capabilities: &Result<device::OutputDeviceCapabilities, EngineError>,
) -> StoredOutputMode {
    match capabilities {
        Ok(capabilities) if capabilities.max_bits_per_channel.is_some() => {
            StoredOutputMode::Exclusive
        }
        Ok(_) | Err(_) => StoredOutputMode::Shared,
    }
}

pub(crate) fn automatic_stored_output_mode(
    capabilities: Option<StoredDeviceCapabilities>,
) -> StoredOutputMode {
    match capabilities {
        Some(capabilities) if capabilities.max_bits_per_channel.is_some() => {
            StoredOutputMode::Exclusive
        }
        Some(_) | None => StoredOutputMode::Shared,
    }
}

pub(crate) fn resolve_engine_kind(
    mode: StoredOutputMode,
    capabilities: Option<device::OutputDeviceCapabilities>,
) -> EngineKind {
    match mode {
        StoredOutputMode::Shared => EngineKind::Universal {
            exclusive_mode: false,
        },
        StoredOutputMode::Exclusive
            if capabilities.is_some_and(|value| value.has_integer_path()) =>
        {
            EngineKind::Integer
        }
        StoredOutputMode::Exclusive => EngineKind::Universal {
            exclusive_mode: true,
        },
    }
}

pub(crate) fn output_mode_for_engine_kind(kind: EngineKind) -> StoredOutputMode {
    match kind {
        EngineKind::Universal {
            exclusive_mode: false,
        } => StoredOutputMode::Shared,
        EngineKind::Universal {
            exclusive_mode: true,
        } => StoredOutputMode::Exclusive,
        EngineKind::Integer => StoredOutputMode::Exclusive,
    }
}

pub(crate) fn merge_managed_devices(
    connected_devices: &[device::Device],
    active_device_uid: Option<&str>,
    saved_output_device_uid: Option<&str>,
    preferences: &OutputModePreferences,
) -> ManagedDeviceGroups {
    let mut merged = BTreeMap::new();
    for (uid, stored) in preferences.devices() {
        let automatic_mode = automatic_stored_output_mode(stored.capabilities);
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
                output_mode: preferences.effective_mode(uid, automatic_mode),
                integer_path_available: stored
                    .capabilities
                    .is_some_and(StoredDeviceCapabilities::has_integer_path),
                hardware_volume_available: false,
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
                output_mode: preferences.effective_mode(&connected.uid, StoredOutputMode::Shared),
                integer_path_available: false,
                hardware_volume_available: false,
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
        integer_wire_formats: Some(capabilities.integer_wire_formats),
        transport: Some(stored_device_transport(capabilities.transport)),
    }
}

pub(crate) fn capabilities_for_sighting(
    stored: Option<StoredDeviceCapabilities>,
    probe: impl FnOnce() -> Result<device::OutputDeviceCapabilities, EngineError>,
) -> Option<StoredDeviceCapabilities> {
    stored.or_else(|| probe().ok().map(stored_device_capabilities))
}

fn stored_device_transport(transport: device::DeviceTransport) -> StoredDeviceTransport {
    match transport {
        device::DeviceTransport::Unknown => StoredDeviceTransport::Unknown,
        device::DeviceTransport::BuiltIn => StoredDeviceTransport::BuiltIn,
        device::DeviceTransport::Aggregate => StoredDeviceTransport::Aggregate,
        device::DeviceTransport::Virtual => StoredDeviceTransport::Virtual,
        device::DeviceTransport::Pci => StoredDeviceTransport::Pci,
        device::DeviceTransport::Usb => StoredDeviceTransport::Usb,
        device::DeviceTransport::FireWire => StoredDeviceTransport::FireWire,
        device::DeviceTransport::Bluetooth => StoredDeviceTransport::Bluetooth,
        device::DeviceTransport::BluetoothLe => StoredDeviceTransport::BluetoothLe,
        device::DeviceTransport::Hdmi => StoredDeviceTransport::Hdmi,
        device::DeviceTransport::DisplayPort => StoredDeviceTransport::DisplayPort,
        device::DeviceTransport::AirPlay => StoredDeviceTransport::AirPlay,
        device::DeviceTransport::Avb => StoredDeviceTransport::Avb,
        device::DeviceTransport::Thunderbolt => StoredDeviceTransport::Thunderbolt,
        device::DeviceTransport::Other => StoredDeviceTransport::Other,
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
