use std::{
    collections::{BTreeMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender, TryRecvError},
    time::{SystemTime, UNIX_EPOCH},
};

use gpui::{Bounds, Context, Pixels, px};
use pulse_engine::{
    EngineError, PcmFormat, PlaybackCommand, PlaybackController, PlaybackErrorKind, PlaybackEvent,
    PlaybackState, device,
};

use crate::{
    app_settings::{AppSettings, ExclusiveModePreferences, StoredDeviceCapabilities},
    app_store::AppStore,
    library::{Track, TrackId},
    queue::{PreviousAction, QueueState, TrackRef},
};

const MIN_AUDIBLE_GAIN: f32 = 0.001;
const SUPPORTED_EXTENSIONS: &[&str] = &["flac", "m4a", "aif", "aiff", "wav"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VolumeIconState {
    High,
    Low,
    Muted,
}

impl VolumeIconState {
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::High => "icons/volume-2.svg",
            Self::Low => "icons/volume-1.svg",
            Self::Muted => "icons/volume-x.svg",
        }
    }
}

struct PendingDeviceChange {
    device: device::Device,
    persist: bool,
    success_message: Option<DeviceMessage>,
    capabilities: Result<device::OutputDeviceCapabilities, EngineError>,
    default_exclusive_mode: bool,
    exclusive_mode: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeviceMessage {
    pub(crate) text: String,
    pub(crate) is_error: bool,
}

/// A visible report shown above the playback row. `Skip` means playback
/// continued past an unplayable queue entry; `Stopped` and `DeviceFailure`
/// mean it stopped.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PlaybackNotice {
    Skip { text: String },
    ExclusiveFallback { text: String },
    Stopped { text: String },
    DeviceFailure { text: String },
}

#[derive(Clone, Copy)]
enum SkipReason {
    Missing,
    Undecodable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RetryTarget {
    path: PathBuf,
    position_ms: u64,
}

/// A dispatched `PlayFile` and its resume position. `confirmed` flips when
/// `NowPlaying` proves the controller is working on this source; until then
/// stale `Position` events from the previous track must not touch it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct PlayAttempt {
    target: RetryTarget,
    confirmed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedDevice {
    pub uid: String,
    pub name: String,
    pub capabilities: Option<StoredDeviceCapabilities>,
    pub last_seen_unix_seconds: Option<u64>,
    pub connected: bool,
    pub active: bool,
    pub saved_default: bool,
    pub default_exclusive_mode: bool,
    pub exclusive_mode: bool,
    pub automatic: bool,
}

impl ManagedDevice {
    pub fn can_forget(&self) -> bool {
        !self.connected
    }

    pub fn can_set_as_default(&self) -> bool {
        self.connected && !self.saved_default
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ManagedDeviceGroups {
    pub connected: Vec<ManagedDevice>,
    pub not_connected: Vec<ManagedDevice>,
}

#[derive(Clone)]
pub(crate) struct PlaybackSnapshot {
    pub(crate) playback_state: PlaybackState,
    pub(crate) source_path: Option<PathBuf>,
    pub(crate) cover_art_path: Option<PathBuf>,
    pub(crate) queue: QueueState,
    pub(crate) title: String,
    pub(crate) secondary: String,
    pub(crate) format: Option<PcmFormat>,
    pub(crate) devices: Vec<device::Device>,
    pub(crate) active_device: Option<device::Device>,
    pub(crate) device_capabilities: Option<device::OutputDeviceCapabilities>,
    pub(crate) device_message: Option<DeviceMessage>,
    pub(crate) default_exclusive_mode: bool,
    pub(crate) exclusive_mode: bool,
    pub(crate) playback_exclusive_mode: bool,
    pub(crate) exclusive_mode_automatic: bool,
    pub(crate) volume_level: f32,
    pub(crate) volume_muted: bool,
    pub(crate) position_ms: u64,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) error: Option<String>,
    pub(crate) notice: Option<PlaybackNotice>,
    pub(crate) missing_track_ids: HashSet<TrackId>,
}

impl PlaybackSnapshot {
    /// Title and `artist · album` for the popover's NOW PLAYING block, absent
    /// when nothing is playing.
    pub(crate) fn now_playing_lines(&self) -> Option<(String, String)> {
        if !matches!(
            self.playback_state,
            PlaybackState::Loading | PlaybackState::Playing | PlaybackState::Paused
        ) {
            return None;
        }
        match self.queue.current() {
            Some(track) if self.source_path.as_deref() == Some(track.path.as_path()) => Some((
                track.title.clone(),
                format!("{} · {}", track.artist, track.album),
            )),
            // A dropped file plays without a queue, and after an exhausted
            // jump the index points at an entry that never played; the row's
            // display strings describe what is actually audible.
            _ => Some((self.title.clone(), self.secondary.clone())),
        }
    }

    pub(crate) fn displayed_fraction(&self) -> f32 {
        self.duration_ms
            .filter(|duration| *duration > 0)
            .map(|duration| (self.position_ms as f64 / duration as f64) as f32)
            .unwrap_or(0.0)
            .clamp(0.0, 1.0)
    }
}

pub(crate) enum PlaybackAction {
    ToggleVolumeMute,
    SetVolumeLevel(f32),
    PersistVolume,
    TogglePlayback,
    NextTrack,
    PreviousTrack,
    ToggleShuffle,
    CycleRepeat,
    PlayLibraryTracks {
        tracks: Vec<Track>,
        start_index: usize,
    },
    PlayLibraryTracksShuffled(Vec<Track>),
    SelectLibraryTracks {
        tracks: Vec<Track>,
        start_index: usize,
    },
    PlayDroppedPaths(Vec<PathBuf>),
    JumpToQueueEntry(usize),
    RemoveQueueEntry(usize),
    ClearUpcomingQueue,
    RetryPlayback,
    DismissNotice,
    Seek(u64),
    RefreshOutputDevices,
    SelectOutputDevice(device::Device),
    ToggleDeviceExclusiveMode {
        device_uid: String,
        default: bool,
    },
    ResetDeviceExclusiveMode {
        device_uid: String,
        default: bool,
    },
    ForgetManagedDevice(String),
    SetManagedDeviceAsDefault(String),
    ClearMissingMarks,
    RemoveMissingMarks(Vec<TrackId>),
}

#[cfg(target_os = "macos")]
fn app_name_for_pid(pid: i32) -> Option<String> {
    let app = objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
    app.localizedName().map(|name| name.to_string())
}

#[cfg(not(target_os = "macos"))]
fn app_name_for_pid(_pid: i32) -> Option<String> {
    None
}

pub(crate) struct Playback {
    controller: Option<PlaybackController>,
    command_tx: Option<Sender<PlaybackCommand>>,
    event_rx: Option<Receiver<PlaybackEvent>>,
    pub(crate) playback_state: PlaybackState,
    pub(crate) source_path: Option<PathBuf>,
    pub(crate) cover_art_path: Option<PathBuf>,
    pub(crate) queue: QueueState,
    pub(crate) title: String,
    pub(crate) secondary: String,
    pub(crate) format: Option<PcmFormat>,
    pub(crate) devices: Vec<device::Device>,
    pub(crate) active_device: Option<device::Device>,
    pub(crate) device_capabilities: Option<device::OutputDeviceCapabilities>,
    pub(crate) device_capability_message: Option<DeviceMessage>,
    pending_device_change: Option<PendingDeviceChange>,
    pending_saved_output_device_uid: Option<String>,
    pub(crate) device_message: Option<DeviceMessage>,
    device_sightings_writable: bool,
    pub(crate) default_exclusive_mode: bool,
    pub(crate) exclusive_mode: bool,
    pub(crate) playback_exclusive_mode: bool,
    pub(crate) volume_level: f32,
    pub(crate) volume_muted: bool,
    pub(crate) position_ms: u64,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) error: Option<String>,
    pub(crate) notice: Option<PlaybackNotice>,
    pub(crate) missing_track_ids: HashSet<TrackId>,
    dispatched_plays: u64,
    current_play: Option<PlayAttempt>,
    retry: Option<RetryTarget>,
    pending_seek_ms: Option<u64>,
    settings: AppSettings,
    settings_path: PathBuf,
}

impl Playback {
    pub(crate) fn new(settings_path: PathBuf, settings: AppSettings) -> Self {
        let mut playback = Self {
            controller: None,
            command_tx: None,
            event_rx: None,
            playback_state: PlaybackState::Idle,
            source_path: None,
            cover_art_path: None,
            queue: QueueState::default(),
            title: "No track loaded".to_string(),
            secondary: "Choose a track from your library".to_string(),
            format: None,
            devices: Vec::new(),
            active_device: None,
            device_capabilities: None,
            device_capability_message: None,
            pending_device_change: None,
            pending_saved_output_device_uid: None,
            device_message: None,
            device_sightings_writable: true,
            default_exclusive_mode: true,
            exclusive_mode: true,
            playback_exclusive_mode: true,
            volume_level: settings.volume_level,
            volume_muted: settings.volume_muted,
            position_ms: 0,
            duration_ms: None,
            error: None,
            notice: None,
            missing_track_ids: HashSet::new(),
            dispatched_plays: 0,
            current_play: None,
            retry: None,
            pending_seek_ms: None,
            settings,
            settings_path,
        };
        playback.initialize_output_inner();
        playback
    }

    #[cfg(test)]
    fn initial() -> Self {
        Self {
            controller: None,
            command_tx: None,
            event_rx: None,
            playback_state: PlaybackState::Idle,
            source_path: None,
            cover_art_path: None,
            queue: QueueState::default(),
            title: "No track loaded".to_string(),
            secondary: "Choose a track from your library".to_string(),
            format: None,
            devices: Vec::new(),
            active_device: None,
            device_capabilities: None,
            device_capability_message: None,
            pending_device_change: None,
            pending_saved_output_device_uid: None,
            device_message: None,
            device_sightings_writable: false,
            default_exclusive_mode: true,
            exclusive_mode: true,
            playback_exclusive_mode: true,
            volume_level: 1.0,
            volume_muted: false,
            position_ms: 0,
            duration_ms: None,
            error: None,
            notice: None,
            missing_track_ids: HashSet::new(),
            dispatched_plays: 0,
            current_play: None,
            retry: None,
            pending_seek_ms: None,
            settings: AppSettings::default(),
            settings_path: PathBuf::new(),
        }
    }

    pub(crate) fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub(crate) fn snapshot(&self) -> PlaybackSnapshot {
        PlaybackSnapshot {
            playback_state: self.playback_state,
            source_path: self.source_path.clone(),
            cover_art_path: self.cover_art_path.clone(),
            queue: self.queue.clone(),
            title: self.title.clone(),
            secondary: self.secondary.clone(),
            format: self.format,
            devices: self.devices.clone(),
            active_device: self.active_device.clone(),
            device_capabilities: self.device_capabilities,
            device_message: self.displayed_device_message(),
            default_exclusive_mode: self.default_exclusive_mode,
            exclusive_mode: self.exclusive_mode,
            playback_exclusive_mode: self.playback_exclusive_mode,
            exclusive_mode_automatic: self.exclusive_mode_is_automatic(),
            volume_level: self.volume_level,
            volume_muted: self.volume_muted,
            position_ms: self.position_ms,
            duration_ms: self.duration_ms,
            error: self.error.clone(),
            notice: self.notice.clone(),
            missing_track_ids: self.missing_track_ids.clone(),
        }
    }

    fn update_settings(&mut self, update: impl FnOnce(&mut AppSettings)) -> io::Result<bool> {
        let mut settings = self.settings.clone();
        update(&mut settings);
        settings.normalize();
        if settings == self.settings {
            return Ok(false);
        }
        settings.save(&self.settings_path)?;
        self.settings = settings;
        Ok(true)
    }

    fn migrate_legacy_exclusive_mode(&mut self, active_device_uid: &str) -> io::Result<bool> {
        let Some(disabled) = self.settings.legacy_exclusive_mode_disabled else {
            return Ok(false);
        };
        self.update_settings(|settings| {
            settings.legacy_exclusive_mode_disabled = None;
            settings
                .exclusive_mode_preferences
                .set_override(active_device_uid, !disabled);
        })
    }

    fn set_exclusive_mode_preferences(
        &mut self,
        preferences: ExclusiveModePreferences,
    ) -> io::Result<bool> {
        self.update_settings(|settings| {
            settings.exclusive_mode_preferences = preferences;
            settings.legacy_exclusive_mode_disabled = None;
        })
    }

    fn forget_device_settings(&mut self, device_uid: &str) -> io::Result<bool> {
        if !self
            .settings
            .exclusive_mode_preferences
            .devices()
            .any(|(uid, _)| uid == device_uid)
        {
            return Ok(false);
        }
        self.update_settings(|settings| {
            settings.exclusive_mode_preferences.forget(device_uid);
            if settings.saved_output_device_uid.as_deref() == Some(device_uid) {
                settings.saved_output_device_uid = None;
            }
        })
    }

    fn initialize_output_inner(&mut self) {
        self.device_message = None;
        self.device_capability_message = None;
        let devices = match device::list_output_devices() {
            Ok(devices) => devices,
            Err(error) => {
                self.device_message = Some(DeviceMessage {
                    text: format!("Could not list output devices: {error}"),
                    is_error: true,
                });
                return;
            }
        };
        let system_default = match device::default_output_device() {
            Ok(device) => device,
            Err(error) => {
                self.devices = devices;
                self.device_message = Some(DeviceMessage {
                    text: error.to_string(),
                    is_error: true,
                });
                return;
            }
        };
        let preferred_uid = self.settings.saved_output_device_uid.clone();
        let active_device =
            resolve_output_device(&devices, &system_default, preferred_uid.as_deref());
        self.active_device = Some(active_device.clone());
        if self.settings.legacy_exclusive_mode_disabled.is_some()
            && let Err(error) = self.migrate_legacy_exclusive_mode(&active_device.uid)
        {
            self.device_message = Some(DeviceMessage {
                text: format!("Could not load exclusive-mode preferences: {error}"),
                is_error: true,
            });
            self.device_sightings_writable = false;
        }
        self.record_device_sightings_inner(&devices);
        self.devices = devices;
        let capabilities = device::output_device_capabilities(active_device.id);
        self.default_exclusive_mode = default_exclusive_mode(&capabilities);
        self.exclusive_mode = self
            .settings
            .exclusive_mode_preferences
            .effective_mode(&active_device.uid, self.default_exclusive_mode);
        self.playback_exclusive_mode = self.exclusive_mode;
        self.apply_device_capabilities_result(&active_device, capabilities);
        self.install_controller(active_device.id, self.exclusive_mode);
    }

    fn initialize_output(&mut self, _cx: &mut Context<AppStore>) {
        self.initialize_output_inner();
    }

    pub(crate) fn install_controller(&mut self, device_id: device::DeviceId, exclusive_mode: bool) {
        let controller = PlaybackController::spawn(device_id, exclusive_mode);
        self.event_rx = Some(controller.subscribe());
        let command_tx = controller.command_sender();
        if command_tx.send(self.volume_command()).is_err() {
            self.error = Some("Playback engine disconnected.".to_string());
        }
        self.command_tx = Some(command_tx);
        self.controller = Some(controller);
    }

    pub(crate) fn volume_command(&self) -> PlaybackCommand {
        PlaybackCommand::SetVolume {
            gain: volume_gain_for_level(self.volume_level),
            muted: self.volume_muted,
        }
    }

    pub(crate) fn toggle_volume_mute(&mut self, cx: &mut Context<AppStore>) {
        let muted = !self.volume_muted;
        if let Err(error) = self.update_settings(|settings| settings.volume_muted = muted) {
            self.error = Some(format!(
                "Could not save the volume mute preference: {error}"
            ));

            return;
        }
        self.volume_muted = muted;
        self.send_command(self.volume_command(), cx);
    }

    pub(crate) fn set_volume_level(&mut self, level: f32, cx: &mut Context<AppStore>) {
        let level = level.clamp(0.0, 1.0);
        if self.volume_level == level && !self.volume_muted {
            return;
        }
        self.volume_level = level;
        self.volume_muted = false;
        self.send_command(self.volume_command(), cx);
    }

    pub(crate) fn persist_volume(&mut self, _cx: &mut Context<AppStore>) {
        let volume_level = self.volume_level;
        let volume_muted = self.volume_muted;
        if let Err(error) = self.update_settings(|settings| {
            settings.volume_level = volume_level;
            settings.volume_muted = volume_muted;
        }) {
            self.error = Some(format!("Could not save the volume preference: {error}"));
        }
    }

    pub(crate) fn update_device_capabilities(&mut self, output_device: &device::Device) {
        self.apply_device_capabilities_result(
            output_device,
            device::output_device_capabilities(output_device.id),
        );
    }

    pub(crate) fn apply_device_capabilities_result(
        &mut self,
        output_device: &device::Device,
        result: Result<device::OutputDeviceCapabilities, EngineError>,
    ) {
        self.device_capability_message = None;
        match result {
            Ok(capabilities) => {
                self.device_capabilities = Some(capabilities);
            }
            Err(EngineError::NoOutputCapabilities(_)) => {
                self.device_capabilities = None;
                self.device_capability_message = Some(DeviceMessage {
                    text: format!(
                        "{} does not advertise a signed-integer PCM physical format Pulse can use.",
                        output_device.name
                    ),
                    is_error: false,
                });
            }
            Err(error) => {
                self.device_capabilities = None;
                self.device_capability_message = Some(DeviceMessage {
                    text: format!(
                        "Could not query {} capabilities: {error}",
                        output_device.name
                    ),
                    is_error: true,
                });
            }
        }
    }

    pub(crate) fn record_device_sightings(
        &mut self,
        devices: &[device::Device],
        _cx: &mut Context<AppStore>,
    ) {
        self.record_device_sightings_inner(devices);
    }

    fn record_device_sightings_inner(&mut self, devices: &[device::Device]) {
        if !self.device_sightings_writable {
            return;
        }
        let seen_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut updated = self.settings.exclusive_mode_preferences.clone();
        for output_device in devices {
            let capabilities = updated.stored_capabilities(&output_device.uid).or_else(|| {
                device::output_device_capabilities(output_device.id)
                    .ok()
                    .map(stored_device_capabilities)
            });
            updated.record_sighting(
                &output_device.uid,
                &output_device.name,
                capabilities,
                seen_at,
            );
        }
        if let Err(error) = self.set_exclusive_mode_preferences(updated) {
            self.device_message = Some(DeviceMessage {
                text: format!("Could not save output device details: {error}"),
                is_error: true,
            });
        }
    }

    pub(crate) fn displayed_device_message(&self) -> Option<DeviceMessage> {
        match (&self.device_message, &self.device_capability_message) {
            (Some(message), Some(capability)) => Some(DeviceMessage {
                text: format!("{} {}", message.text, capability.text),
                is_error: message.is_error || capability.is_error,
            }),
            (Some(message), None) => Some(message.clone()),
            (None, Some(capability)) => Some(capability.clone()),
            (None, None) => None,
        }
    }

    pub(crate) fn active_output_device(&self) -> Option<&device::Device> {
        self.active_device.as_ref()
    }

    pub(crate) fn managed_device_groups(&self) -> ManagedDeviceGroups {
        let mut groups = merge_managed_devices(
            &self.devices,
            self.active_device
                .as_ref()
                .map(|device| device.uid.as_str()),
            self.settings.saved_output_device_uid.as_deref(),
            &self.settings.exclusive_mode_preferences,
        );
        if let Some(active) = groups.connected.iter_mut().find(|device| device.active) {
            if let Some(capabilities) = self.device_capabilities {
                active.capabilities = Some(stored_device_capabilities(capabilities));
            }
            active.default_exclusive_mode = self.default_exclusive_mode;
            active.exclusive_mode = self.exclusive_mode;
            active.automatic = self.exclusive_mode_is_automatic();
        }
        groups
    }

    pub(crate) fn device_management_messages(&self) -> Vec<(String, bool)> {
        let mut messages = Vec::new();
        if let Some(message) = self.displayed_device_message() {
            messages.push((message.text, message.is_error));
        }
        if let Some(error) = &self.error {
            messages.push((error.clone(), true));
        }
        messages
    }

    pub(crate) fn exclusive_mode_is_automatic(&self) -> bool {
        self.active_device.as_ref().is_none_or(|device| {
            !self
                .settings
                .exclusive_mode_preferences
                .is_overridden(&device.uid)
        })
    }

    pub(crate) fn toggle_device_exclusive_mode(
        &mut self,
        device_uid: String,
        default: bool,
        cx: &mut Context<AppStore>,
    ) {
        let enabled = !self
            .settings
            .exclusive_mode_preferences
            .effective_mode(&device_uid, default);
        let mut updated_preferences = self.settings.exclusive_mode_preferences.clone();
        updated_preferences.set_override(&device_uid, enabled);
        if let Err(error) = self.set_exclusive_mode_preferences(updated_preferences) {
            self.device_message = Some(DeviceMessage {
                text: format!("Could not save the exclusive-mode preference: {error}"),
                is_error: true,
            });

            return;
        }
        self.device_sightings_writable = true;
        self.apply_exclusive_mode_if_active(&device_uid, enabled, cx);
    }

    pub(crate) fn reset_device_exclusive_mode_to_auto(
        &mut self,
        device_uid: String,
        default: bool,
        cx: &mut Context<AppStore>,
    ) {
        let mut updated_preferences = self.settings.exclusive_mode_preferences.clone();
        updated_preferences.clear_override(&device_uid);
        if let Err(error) = self.set_exclusive_mode_preferences(updated_preferences) {
            self.device_message = Some(DeviceMessage {
                text: format!("Could not save the exclusive-mode preference: {error}"),
                is_error: true,
            });

            return;
        }
        self.device_sightings_writable = true;
        self.apply_exclusive_mode_if_active(&device_uid, default, cx);
    }

    pub(crate) fn apply_exclusive_mode_if_active(
        &mut self,
        device_uid: &str,
        enabled: bool,
        cx: &mut Context<AppStore>,
    ) {
        if !self
            .active_device
            .as_ref()
            .is_some_and(|device| device.uid == device_uid)
        {
            return;
        }
        self.exclusive_mode = enabled;
        self.playback_exclusive_mode = enabled;
        self.send_command(PlaybackCommand::SetExclusiveMode { enabled }, cx);
    }

    pub(crate) fn forget_managed_device(
        &mut self,
        device_uid: &str,
        _cx: &mut Context<AppStore>,
    ) -> bool {
        if self.devices.iter().any(|device| device.uid == device_uid) {
            return false;
        }
        match self.forget_device_settings(device_uid) {
            Ok(true) => {
                self.device_message = None;

                true
            }
            Ok(false) => false,
            Err(error) => {
                self.device_message = Some(DeviceMessage {
                    text: format!("Could not forget the output device: {error}"),
                    is_error: true,
                });

                false
            }
        }
    }

    pub(crate) fn set_managed_device_as_default(
        &mut self,
        device_uid: &str,
        _cx: &mut Context<AppStore>,
    ) {
        match self.update_saved_output_device_uid(device_uid) {
            Ok(true) => {
                self.device_message = None;
            }
            Ok(false) => {}
            Err(error) => {
                self.device_message = Some(DeviceMessage {
                    text: format!("Could not save the default output device: {error}"),
                    is_error: true,
                });
            }
        }
    }

    pub(crate) fn update_saved_output_device_uid(
        &mut self,
        device_uid: &str,
    ) -> std::io::Result<bool> {
        if !self.devices.iter().any(|device| device.uid == device_uid) {
            return Ok(false);
        }
        let device_uid = device_uid.to_string();
        self.update_settings(|settings| settings.saved_output_device_uid = Some(device_uid))
    }

    /// Marks go stale whenever library rows change under them — a completed
    /// scan re-verifies file presence, and a removed root recycles track ids.
    pub(crate) fn clear_missing_marks(&mut self) {
        self.missing_track_ids.clear();
    }

    #[cfg(test)]
    fn is_track_missing(&self, track_id: TrackId) -> bool {
        self.missing_track_ids.contains(&track_id)
    }

    /// Rows for these tracks were deleted; drop their marks so a future row
    /// recycling one of the ids cannot inherit a stale missing flag.
    pub(crate) fn remove_missing_marks(&mut self, track_ids: &[TrackId]) {
        for track_id in track_ids {
            self.missing_track_ids.remove(track_id);
        }
    }

    pub(crate) fn dismiss_notice(&mut self, _cx: &mut Context<AppStore>) {
        self.notice = None;
        self.retry = None;
    }

    pub(crate) fn play_library_tracks(
        &mut self,
        tracks: &[Track],
        start_index: usize,
        cx: &mut Context<AppStore>,
    ) {
        self.queue.rebuild(tracks, start_index);
        self.notice = None;
        self.retry = None;
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        self.play_queue_track(track, cx);
    }

    pub(crate) fn play_library_tracks_shuffled(
        &mut self,
        tracks: &[Track],
        cx: &mut Context<AppStore>,
    ) {
        self.queue.rebuild_shuffled(tracks);
        self.notice = None;
        self.retry = None;
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        self.play_queue_track(track, cx);
    }

    pub(crate) fn select_library_tracks(
        &mut self,
        tracks: &[Track],
        start_index: usize,
        _cx: &mut Context<AppStore>,
    ) {
        let mut queue = self.queue.clone();
        queue.rebuild(tracks, start_index);
        let Some(track) = queue.current().cloned() else {
            return;
        };
        if self.apply_track_selection(&track) {
            self.queue = queue;
        }
    }

    pub(crate) fn handle_drop(&mut self, paths: &[PathBuf], cx: &mut Context<AppStore>) {
        if paths.len() != 1 {
            self.error = Some("Drop one audio file at a time.".to_string());

            return;
        }

        let path = &paths[0];
        if !path.is_file() {
            self.error = Some("Drop a file, not a folder.".to_string());

            return;
        }
        if !is_supported_audio(path) {
            self.error = Some(
                "Unsupported file. Pulse accepts FLAC, ALAC (.m4a), AIFF, and WAV.".to_string(),
            );

            return;
        }

        self.error = None;
        self.notice = None;
        self.retry = None;
        self.pending_seek_ms = None;
        self.cover_art_path = None;
        self.queue.clear();
        self.play_file(path.clone(), cx);
    }

    pub(crate) fn play_file(&mut self, path: PathBuf, cx: &mut Context<AppStore>) {
        if self.controller.is_none() {
            self.initialize_output(cx);
            if self.controller.is_none() {
                self.error = Some("No output device is available.".to_string());

                return;
            }
        }

        self.record_play_attempt(&path);
        self.send_command(PlaybackCommand::PlayFile { path }, cx);
    }

    /// Tracks what the controller is actually working on, so a device-failure
    /// retry replays the attempted file — `source_path`/`position_ms` still
    /// describe the previous track while a new one is loading.
    pub(crate) fn record_play_attempt(&mut self, path: &Path) {
        self.current_play = Some(PlayAttempt {
            target: RetryTarget {
                path: path.to_path_buf(),
                position_ms: self.pending_seek_ms.unwrap_or(0),
            },
            confirmed: false,
        });
    }

    pub(crate) fn toggle_playback(&mut self, cx: &mut Context<AppStore>) {
        if let Some(command) = self.prepare_toggle_command() {
            self.send_command(command, cx);
        }
    }

    /// Transport play from Idle/Ended/Error dispatches a `PlayFile` restart,
    /// which must be recorded as a fresh attempt like every other dispatch.
    pub(crate) fn prepare_toggle_command(&mut self) -> Option<PlaybackCommand> {
        let command = self.toggle_command()?;
        if let PlaybackCommand::PlayFile { path } = &command {
            self.pending_seek_ms = None;
            let path = path.clone();
            self.record_play_attempt(&path);
        }
        Some(command)
    }

    pub(crate) fn next_track(&mut self, cx: &mut Context<AppStore>) {
        self.notice = None;
        self.retry = None;
        if let Some(track) = self.queue.advance() {
            self.play_queue_track(track, cx);
        }
    }

    pub(crate) fn toggle_shuffle(&mut self, _cx: &mut Context<AppStore>) {
        self.queue.toggle_shuffle();
    }

    pub(crate) fn cycle_repeat(&mut self, _cx: &mut Context<AppStore>) {
        self.queue.cycle_repeat();
    }

    pub(crate) fn previous_track(&mut self, cx: &mut Context<AppStore>) {
        self.notice = None;
        self.retry = None;
        match self.queue.previous(self.position_ms) {
            Some(PreviousAction::Restart(track) | PreviousAction::PlayPrevious(track)) => {
                self.play_queue_track(track, cx);
            }
            None => {
                if let Some(path) = self.source_path.clone() {
                    self.play_file(path, cx);
                }
            }
        }
    }

    pub(crate) fn toggle_command(&self) -> Option<PlaybackCommand> {
        match self.playback_state {
            PlaybackState::Playing => Some(PlaybackCommand::Pause),
            PlaybackState::Paused => Some(PlaybackCommand::Resume),
            PlaybackState::Idle | PlaybackState::Ended | PlaybackState::Error => self
                .restart_path()
                .map(|path| PlaybackCommand::PlayFile { path }),
            PlaybackState::Loading | PlaybackState::Stopping => None,
        }
    }

    /// The file transport Play restarts. In Error state the failed attempt is
    /// the target, keeping Play and Try again in agreement when a new file
    /// failed before any NowPlaying updated `source_path`.
    pub(crate) fn restart_path(&self) -> Option<PathBuf> {
        if self.playback_state == PlaybackState::Error
            && let Some(attempt) = &self.current_play
        {
            return Some(attempt.target.path.clone());
        }
        self.source_path.clone()
    }

    pub(crate) fn apply_track_selection(&mut self, track: &TrackRef) -> bool {
        if matches!(
            self.playback_state,
            PlaybackState::Loading
                | PlaybackState::Playing
                | PlaybackState::Paused
                | PlaybackState::Stopping
        ) {
            return false;
        }
        self.apply_track_context(track);
        self.playback_state = PlaybackState::Idle;
        self.format = None;
        self.position_ms = 0;
        self.duration_ms = None;
        self.error = None;
        true
    }

    pub(crate) fn apply_track_context(&mut self, track: &TrackRef) {
        self.title = track.title.clone();
        self.secondary = track.secondary();
        self.source_path = Some(track.path.clone());
        self.cover_art_path = track.cover_art_path.clone();
    }

    pub(crate) fn play_queue_track(&mut self, track: TrackRef, cx: &mut Context<AppStore>) {
        match self.prepare_queue_play(track) {
            Some(path) => self.play_file(path, cx),
            // The queue exhausted through unplayable entries. A user-initiated
            // jump or Next can land here while the superseded track is still
            // audible; stop it so the Stopped notice is truthful.
            None => {
                if self.active_playback_needs_stop() {
                    self.send_command(PlaybackCommand::Stop, cx);
                }
            }
        }
    }

    /// True when the engine is still producing (or about to produce) audio
    /// for a track the queue has already moved past.
    pub(crate) fn active_playback_needs_stop(&self) -> bool {
        matches!(
            self.playback_state,
            PlaybackState::Loading | PlaybackState::Playing | PlaybackState::Paused
        )
    }

    /// Cx-free half of a queue dispatch: resolves the skip-and-report path
    /// for missing files, applies the display context, and returns the path
    /// to hand to `play_file`.
    pub(crate) fn prepare_queue_play(&mut self, track: TrackRef) -> Option<PathBuf> {
        self.pending_seek_ms = None;
        let track = self.next_playable(track)?;
        self.apply_track_context(&track);
        self.error = None;
        Some(track.path)
    }

    /// Queue-popover row click: the index moves to the clicked entry and it
    /// plays; nothing is dropped. A jump to a missing entry follows the
    /// existing skip-and-report path inside `play_queue_track`.
    pub(crate) fn jump_to_queue_entry(&mut self, index: usize, cx: &mut Context<AppStore>) {
        self.notice = None;
        self.retry = None;
        if let Some(track) = self.queue.jump_to(index) {
            self.play_queue_track(track, cx);
        }
    }

    pub(crate) fn remove_queue_entry(&mut self, index: usize, _cx: &mut Context<AppStore>) {
        self.queue.remove_at(index);
    }

    pub(crate) fn clear_upcoming_queue(&mut self, _cx: &mut Context<AppStore>) {
        self.queue.clear_upcoming();
    }

    pub(crate) fn next_playable(&mut self, first: TrackRef) -> Option<TrackRef> {
        let mut candidate = first;
        loop {
            if candidate.path.is_file() {
                return Some(candidate);
            }
            self.missing_track_ids.insert(candidate.id);
            match self.queue.skip_failed() {
                Some(next) => {
                    self.note_skip(&candidate, SkipReason::Missing);
                    candidate = next;
                }
                None => {
                    self.note_queue_stopped(&candidate);
                    return None;
                }
            }
        }
    }

    /// A track-scoped playback failure: mark the entry if its file is gone,
    /// then hand back the next entry to try, or report why the queue stopped.
    pub(crate) fn handle_track_failure(&mut self) -> Option<TrackRef> {
        let failed = self.queue.current().cloned()?;
        let reason = if failed.path.is_file() {
            SkipReason::Undecodable
        } else {
            self.missing_track_ids.insert(failed.id);
            SkipReason::Missing
        };
        match self.queue.skip_failed() {
            Some(next) => {
                self.note_skip(&failed, reason);
                Some(next)
            }
            None => {
                self.note_queue_stopped(&failed);
                None
            }
        }
    }

    fn note_skip(&mut self, track: &TrackRef, reason: SkipReason) {
        let text = if self.queue.skipped_count() > 1 {
            format!(
                "Skipped {} tracks that could not be played.",
                self.queue.skipped_count()
            )
        } else {
            let reason = match reason {
                SkipReason::Missing => "its file is missing",
                SkipReason::Undecodable => "its file could not be decoded",
            };
            format!("Skipped “{}” — {reason}.", track.title)
        };
        self.notice = Some(PlaybackNotice::Skip { text });
    }

    pub(crate) fn note_queue_stopped(&mut self, last: &TrackRef) {
        let text = if self.queue.nothing_played() && self.queue.skipped_count() > 1 {
            "Playback stopped — none of the queued tracks could be played.".to_string()
        } else if self.queue.skipped_count() > 1 {
            format!(
                "Playback stopped — {} tracks could not be played.",
                self.queue.skipped_count()
            )
        } else {
            format!("Playback stopped — “{}” could not be played.", last.title)
        };
        self.notice = Some(PlaybackNotice::Stopped { text });
    }

    pub(crate) fn handle_device_failure(&mut self, message: &str, hog_pid: Option<i32>) {
        let device_name = self
            .active_device
            .as_ref()
            .map(|device| device.name.clone())
            .unwrap_or_else(|| "the output device".to_string());
        let text = match hog_pid {
            Some(pid) => {
                let app = app_name_for_pid(pid).unwrap_or_else(|| "Another app".to_string());
                format!(
                    "{app} is using {device_name} exclusively. Quit it or choose another output, then try again."
                )
            }
            None => format!("Playback stopped on {device_name}: {message}"),
        };
        self.retry = self
            .current_play
            .as_ref()
            .map(|attempt| attempt.target.clone());
        self.notice = Some(PlaybackNotice::DeviceFailure { text });
    }

    pub(crate) fn retry_playback(&mut self, cx: &mut Context<AppStore>) {
        let Some(retry) = self.retry.take() else {
            return;
        };
        self.notice = None;
        self.error = None;
        // Re-resolve the output first: after a disconnect the saved device is
        // gone and refresh falls back to the system default visibly.
        self.refresh_output_devices(cx);
        self.pending_seek_ms = (retry.position_ms > 0).then_some(retry.position_ms);
        self.play_file(retry.path, cx);
    }

    pub(crate) fn send_command(&mut self, command: PlaybackCommand, _cx: &mut Context<AppStore>) {
        let Some(command_tx) = &self.command_tx else {
            return;
        };
        let is_play = matches!(command, PlaybackCommand::PlayFile { .. });
        if command_tx.send(command).is_err() {
            self.error = Some("Playback engine disconnected.".to_string());
        } else if is_play {
            self.dispatched_plays += 1;
        }
    }

    pub(crate) fn drain_events(&mut self, cx: &mut Context<AppStore>) -> bool {
        let mut changed = self.pending_saved_output_device_uid.is_some();
        loop {
            let event = match self.event_rx.as_ref().map(Receiver::try_recv) {
                Some(Ok(event)) => event,
                Some(Err(TryRecvError::Empty)) | None => break,
                Some(Err(TryRecvError::Disconnected)) => {
                    self.event_rx = None;
                    self.error = Some("Playback engine disconnected.".to_string());
                    changed = true;
                    break;
                }
            };
            changed = true;
            if let Some(track) = self.handle_event(event) {
                self.play_queue_track(track, cx);
            }
            if self.playback_state == PlaybackState::Playing
                && let Some(position_ms) = self.pending_seek_ms.take()
            {
                self.send_command(PlaybackCommand::Seek { position_ms }, cx);
            }
        }
        self.persist_pending_saved_output_device_uid(cx);
        changed
    }

    /// Applies one controller event; returns the next queue entry to play
    /// when the event calls for an advance (track ended or failed).
    pub(crate) fn handle_event(&mut self, event: PlaybackEvent) -> Option<TrackRef> {
        match event {
            PlaybackEvent::StateChanged(state) => {
                self.playback_state = state;
            }
            PlaybackEvent::NowPlaying { source, format } => {
                match &mut self.current_play {
                    Some(attempt) if attempt.target.path == source.path => {
                        attempt.confirmed = true;
                    }
                    // A mismatched NowPlaying against an unconfirmed attempt
                    // is a stale event from a superseded play — ignore it
                    // entirely so display state and the retry target stay on
                    // the dispatched attempt.
                    Some(attempt) if !attempt.confirmed => return None,
                    _ => {
                        self.current_play = Some(PlayAttempt {
                            target: RetryTarget {
                                path: source.path.clone(),
                                position_ms: 0,
                            },
                            confirmed: true,
                        });
                    }
                }
                if let Some(track) = self
                    .queue
                    .current()
                    .filter(|track| track.path == source.path)
                    .cloned()
                {
                    self.apply_track_context(&track);
                    self.queue.mark_started();
                    self.missing_track_ids.remove(&track.id);
                } else {
                    self.title = track_title(&source.path);
                    self.secondary = track_secondary(&source.path);
                }
                self.source_path = Some(source.path);
                self.duration_ms = source.duration_ms;
                self.format = Some(format);
                self.position_ms = 0;
                self.error = None;
            }
            PlaybackEvent::Position {
                position_ms,
                duration_ms,
            } => {
                self.position_ms = position_ms;
                self.duration_ms = duration_ms;
                if let Some(attempt) = &mut self.current_play
                    && attempt.confirmed
                {
                    attempt.target.position_ms = position_ms;
                }
            }
            PlaybackEvent::OutputDeviceChanged {
                device_id,
                exclusive_mode,
            } => {
                self.complete_output_device_change(device_id, exclusive_mode);
            }
            PlaybackEvent::ExclusiveModeFallback { device_id } => {
                self.playback_exclusive_mode = false;
                let device_name = self
                    .pending_device_change
                    .as_ref()
                    .filter(|pending| pending.device.id == device_id)
                    .map(|pending| pending.device.name.as_str())
                    .or_else(|| {
                        self.active_device
                            .as_ref()
                            .filter(|device| device.id == device_id)
                            .map(|device| device.name.as_str())
                    })
                    .or_else(|| {
                        self.devices
                            .iter()
                            .find(|device| device.id == device_id)
                            .map(|device| device.name.as_str())
                    })
                    .unwrap_or("The output device");
                self.notice = Some(PlaybackNotice::ExclusiveFallback {
                    text: format!(
                        "{device_name} could not start in exclusive mode. Playback continues in shared mode."
                    ),
                });
            }
            PlaybackEvent::Ended { attempt } => {
                if attempt != self.dispatched_plays {
                    return None;
                }
                self.playback_state = PlaybackState::Ended;
                if let Some(duration_ms) = self.duration_ms {
                    self.position_ms = duration_ms;
                }
                return self.queue.advance_on_end();
            }
            PlaybackEvent::CommandRejected { command, state } => {
                self.error = Some(format!(
                    "{command} is unavailable while playback is {}.",
                    playback_state_label(state)
                ));
            }
            PlaybackEvent::Error {
                attempt,
                kind,
                message,
            } => {
                // Output-device change failures are not play-scoped; handle
                // them before the attempt staleness guard.
                if let Some(pending) = self.pending_device_change.take() {
                    self.device_message = Some(DeviceMessage {
                        text: format!("Could not switch to {}: {message}", pending.device.name),
                        is_error: true,
                    });
                    self.error = Some(message);
                    return None;
                }
                if attempt != self.dispatched_plays {
                    return None;
                }
                // Advisory: teardown already reached Idle/Ended, playback is
                // not stopping because of this (see PlaybackEvent::Error docs).
                if matches!(
                    self.playback_state,
                    PlaybackState::Idle | PlaybackState::Ended
                ) {
                    self.error = Some(message);
                    return None;
                }
                match kind {
                    PlaybackErrorKind::Track if self.queue.current().is_some() => {
                        self.error = None;
                        return self.handle_track_failure();
                    }
                    PlaybackErrorKind::Track => {
                        // Before NowPlaying, `title` still describes the
                        // previous row; the attempted file is the one that
                        // failed.
                        let name = self
                            .current_play
                            .as_ref()
                            .map(|attempt| track_title(&attempt.target.path))
                            .unwrap_or_else(|| self.title.clone());
                        self.notice = Some(PlaybackNotice::Stopped {
                            text: format!("Could not play “{name}” — {message}."),
                        });
                        self.error = Some(message);
                    }
                    PlaybackErrorKind::Device { hog_pid } => {
                        self.handle_device_failure(&message, hog_pid);
                        self.error = Some(message);
                    }
                }
            }
        }
        None
    }

    pub(crate) fn complete_output_device_change(
        &mut self,
        device_id: device::DeviceId,
        playback_exclusive_mode: bool,
    ) {
        let Some(pending) = self.pending_device_change.take() else {
            return;
        };
        if pending.device.id != device_id {
            return;
        }

        let persist = pending.persist;
        let output_device =
            self.apply_completed_output_device_change(pending, playback_exclusive_mode);

        if persist {
            self.pending_saved_output_device_uid = Some(output_device.uid);
        }
    }

    pub(crate) fn persist_pending_saved_output_device_uid(&mut self, _cx: &mut Context<AppStore>) {
        let Some(device_uid) = self.pending_saved_output_device_uid.take() else {
            return;
        };
        match self.update_settings(|settings| {
            settings.saved_output_device_uid = Some(device_uid.clone());
        }) {
            Ok(_) => {}
            Err(error) => {
                let device_name = self
                    .active_device
                    .as_ref()
                    .map(|device| device.name.as_str())
                    .unwrap_or("the output device");
                self.device_message = Some(DeviceMessage {
                    text: format!("Could not save {device_name} as the output device: {error}"),
                    is_error: true,
                });
            }
        }
    }

    fn apply_completed_output_device_change(
        &mut self,
        pending: PendingDeviceChange,
        playback_exclusive_mode: bool,
    ) -> device::Device {
        let PendingDeviceChange {
            device: output_device,
            success_message,
            capabilities,
            default_exclusive_mode,
            exclusive_mode,
            ..
        } = pending;
        self.active_device = Some(output_device.clone());
        self.device_message = success_message;
        self.default_exclusive_mode = default_exclusive_mode;
        self.exclusive_mode = exclusive_mode;
        self.playback_exclusive_mode = playback_exclusive_mode;
        self.apply_device_capabilities_result(&output_device, capabilities);
        output_device
    }

    pub(crate) fn check_active_device_presence(&mut self, cx: &mut Context<AppStore>) -> bool {
        if !matches!(
            self.playback_state,
            PlaybackState::Playing | PlaybackState::Loading
        ) {
            return false;
        }
        let Ok(devices) = device::list_output_devices() else {
            return false;
        };
        let sightings_changed = self.devices.len() != devices.len()
            || self
                .devices
                .iter()
                .zip(&devices)
                .any(|(known, current)| known.uid != current.uid || known.name != current.name);
        if sightings_changed {
            self.record_device_sightings(&devices, cx);
        }
        let devices_changed = self.devices.len() != devices.len()
            || self.devices.iter().zip(&devices).any(|(known, current)| {
                known.id != current.id || known.uid != current.uid || known.name != current.name
            });
        let device_lost = self.note_device_loss(devices);
        if device_lost {
            self.send_command(PlaybackCommand::Stop, cx);
        }
        devices_changed || device_lost
    }

    /// Returns true when the active output device is no longer attached; the
    /// engine has no mid-playback device-loss signal, so the row watches for
    /// it while playing.
    pub(crate) fn note_device_loss(&mut self, devices: Vec<device::Device>) -> bool {
        let Some(active) = self.active_device.clone() else {
            self.devices = devices;
            return false;
        };
        let attached = devices.iter().any(|device| device.uid == active.uid);
        self.devices = devices;
        if attached {
            return false;
        }
        self.retry = self
            .current_play
            .as_ref()
            .map(|attempt| attempt.target.clone());
        self.notice = Some(PlaybackNotice::DeviceFailure {
            text: format!(
                "{} was disconnected. Reconnect it or choose another output, then try again.",
                active.name
            ),
        });
        true
    }

    pub(crate) fn refresh_output_devices(&mut self, cx: &mut Context<AppStore>) {
        let devices = match device::list_output_devices() {
            Ok(devices) => devices,
            Err(error) => {
                self.device_message = Some(DeviceMessage {
                    text: format!("Could not refresh output devices: {error}"),
                    is_error: true,
                });
                return;
            }
        };
        self.record_device_sightings(&devices, cx);

        let Some(active_device) = self.active_device.clone() else {
            self.initialize_output(cx);
            return;
        };

        if let Some(current_device) = devices
            .iter()
            .find(|device| device.uid == active_device.uid)
            .cloned()
        {
            self.devices = devices;
            if current_device.id != active_device.id {
                self.request_output_device_change(current_device, false, None, cx);
            } else {
                self.active_device = Some(current_device.clone());
                self.update_device_capabilities(&current_device);
            }
            return;
        }

        self.devices = devices;
        let system_default = match device::default_output_device() {
            Ok(device) => device,
            Err(error) => {
                self.device_message = Some(DeviceMessage {
                    text: format!(
                        "{} is no longer available, and no system default could be resolved: {error}",
                        active_device.name
                    ),
                    is_error: true,
                });
                return;
            }
        };
        let message = DeviceMessage {
            text: format!(
                "{} is no longer available. Using system default: {}.",
                active_device.name, system_default.name
            ),
            is_error: false,
        };
        self.request_output_device_change(system_default, false, Some(message), cx);
    }

    pub(crate) fn select_output_device(
        &mut self,
        output_device: device::Device,
        cx: &mut Context<AppStore>,
    ) {
        self.request_output_device_change(output_device, true, None, cx);
    }

    pub(crate) fn request_output_device_change(
        &mut self,
        output_device: device::Device,
        persist: bool,
        success_message: Option<DeviceMessage>,
        _cx: &mut Context<AppStore>,
    ) {
        // The popover stays open on selection so the current-device card
        // (capability line, mode control) reflects the switch in place.
        if persist {
            self.notice = None;
            self.retry = None;
        }
        self.error = None;
        self.device_message = None;
        self.device_capability_message = None;
        let capabilities = device::output_device_capabilities(output_device.id);
        let default_exclusive_mode = default_exclusive_mode(&capabilities);
        let exclusive_mode = self
            .settings
            .exclusive_mode_preferences
            .effective_mode(&output_device.uid, default_exclusive_mode);
        self.pending_device_change = Some(PendingDeviceChange {
            device: output_device.clone(),
            persist,
            success_message,
            capabilities,
            default_exclusive_mode,
            exclusive_mode,
        });

        let Some(command_tx) = &self.command_tx else {
            self.install_controller(output_device.id, exclusive_mode);
            self.complete_output_device_change(output_device.id, exclusive_mode);

            return;
        };
        if command_tx
            .send(PlaybackCommand::SetOutputDevice {
                device_id: output_device.id,
                exclusive_mode,
            })
            .is_err()
        {
            self.pending_device_change = None;
            self.device_message = Some(DeviceMessage {
                text: "Playback engine disconnected while changing output devices.".to_string(),
                is_error: true,
            });
        }
    }

    pub(crate) fn seek(&mut self, position_ms: u64, cx: &mut Context<AppStore>) {
        self.position_ms = position_ms;
        self.send_command(PlaybackCommand::Seek { position_ms }, cx);
    }

    pub(crate) fn shutdown(&mut self) {
        self.event_rx = None;
        self.command_tx = None;
        self.controller = None;
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

pub(crate) fn stored_device_capabilities(
    capabilities: device::OutputDeviceCapabilities,
) -> StoredDeviceCapabilities {
    StoredDeviceCapabilities {
        max_bits_per_channel: capabilities.max_bits_per_channel,
        max_sample_rate: capabilities.max_sample_rate.round() as u32,
    }
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

pub(crate) fn default_exclusive_mode(
    capabilities: &Result<device::OutputDeviceCapabilities, EngineError>,
) -> bool {
    capabilities
        .as_ref()
        .is_ok_and(|capabilities| capabilities.max_bits_per_channel.is_some())
}

pub(crate) fn format_device_capabilities(capabilities: device::OutputDeviceCapabilities) -> String {
    format_capability_ceiling(
        capabilities.max_bits_per_channel,
        capabilities.max_sample_rate.round() as u32,
    )
}

pub(crate) fn format_stored_device_capabilities(capabilities: StoredDeviceCapabilities) -> String {
    format_capability_ceiling(
        capabilities.max_bits_per_channel,
        capabilities.max_sample_rate,
    )
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

pub(crate) fn is_supported_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

pub(crate) fn track_title(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub(crate) fn track_secondary(path: &Path) -> String {
    path.parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub(crate) fn format_quality(path: Option<&Path>, format: PcmFormat) -> String {
    let container = path
        .and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_uppercase)
        .unwrap_or_else(|| "PCM".to_string());
    format!("{container} · {}-bit", format.bits_per_sample)
}

pub(crate) fn format_sample_rate(sample_rate: u32) -> String {
    if sample_rate.is_multiple_of(1_000) {
        format!("{} kHz", sample_rate / 1_000)
    } else {
        format!("{:.1} kHz", sample_rate as f64 / 1_000.0)
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

pub(crate) fn scrub_position_ms(fraction: f32, duration_ms: u64) -> u64 {
    (duration_ms as f64 * f64::from(fraction)).round() as u64
}

pub(crate) fn volume_gain_for_level(level: f32) -> f32 {
    let level = level.clamp(0.0, 1.0);
    if level == 0.0 {
        return 0.0;
    }
    (level * level * level).max(MIN_AUDIBLE_GAIN)
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

pub(crate) fn displayed_volume_level(level: f32, muted: bool) -> f32 {
    if muted { 0.0 } else { level }
}

pub(crate) fn format_volume_percent(level: f32) -> String {
    format!("{:.0}%", level.clamp(0.0, 1.0) * 100.0)
}

pub(crate) fn fraction_at_x(bounds: Bounds<Pixels>, x: Pixels) -> f32 {
    if bounds.size.width <= px(0.) {
        return 0.0;
    }
    ((x - bounds.origin.x) / bounds.size.width).clamp(0.0, 1.0)
}

pub(crate) fn fraction_at_y(bounds: Bounds<Pixels>, y: Pixels) -> f32 {
    if bounds.size.height <= px(0.) {
        return 0.0;
    }
    ((bounds.origin.y + bounds.size.height - y) / bounds.size.height).clamp(0.0, 1.0)
}
#[cfg(test)]
mod tests {
    use gpui::{bounds, point, px, size};

    use super::*;
    use crate::app_settings;

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
                .exclusive_mode_preferences
                .is_overridden("airpods")
        );
        assert!(
            !playback
                .settings
                .exclusive_mode_preferences
                .effective_mode("airpods", true)
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
        playback
            .settings
            .exclusive_mode_preferences
            .record_sighting(
                "matrix",
                "mini-i Series",
                Some(StoredDeviceCapabilities {
                    max_bits_per_channel: Some(24),
                    max_sample_rate: 192_000,
                }),
                100,
            );

        assert!(playback.forget_device_settings("matrix").unwrap());

        let saved = AppSettings::load(&path).unwrap();
        assert_eq!(saved.saved_output_device_uid, None);
        assert!(
            !saved
                .exclusive_mode_preferences
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
            .exclusive_mode_preferences
            .set_override("matrix", false);

        playback
            .update_settings(|settings| {
                settings.volume_level = 0.25;
                settings.volume_muted = false;
            })
            .unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.saved_output_device_uid.as_deref(), Some("matrix"));
        assert!(
            !loaded
                .exclusive_mode_preferences
                .effective_mode("matrix", true)
        );
        assert_eq!(loaded.legacy_exclusive_mode_disabled, Some(true));
        assert_eq!(loaded.volume_level, 0.25);
        assert!(!loaded.volume_muted);
    }

    #[test]
    fn explicit_exclusive_mode_write_clears_pending_legacy_intent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let mut playback = Playback::initial();
        playback.settings_path = path.clone();
        playback.settings.legacy_exclusive_mode_disabled = Some(true);
        let mut exclusive_modes = ExclusiveModePreferences::default();
        exclusive_modes.set_override("matrix", true);

        playback
            .set_exclusive_mode_preferences(exclusive_modes)
            .unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.legacy_exclusive_mode_disabled, None);
        assert!(
            loaded
                .exclusive_mode_preferences
                .effective_mode("matrix", false)
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
        ] {
            assert!(is_supported_audio(Path::new(path)), "{path}");
        }
        for path in ["track.mp3", "track.aac", "track"] {
            assert!(!is_supported_audio(Path::new(path)), "{path}");
        }
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
        let track_bounds = bounds(point(px(100.), px(20.)), size(px(200.), px(4.)));
        assert_eq!(fraction_at_x(track_bounds, px(50.)), 0.0);
        assert_eq!(fraction_at_x(track_bounds, px(200.)), 0.5);
        assert_eq!(fraction_at_x(track_bounds, px(350.)), 1.0);
        assert_eq!(
            fraction_at_x(
                bounds(point(px(100.), px(20.)), size(px(0.), px(4.))),
                px(100.)
            ),
            0.0
        );
    }

    #[test]
    fn maps_vertical_volume_positions_bottom_to_top() {
        let slider_bounds = bounds(point(px(20.), px(100.)), size(px(28.), px(120.)));
        assert_eq!(fraction_at_y(slider_bounds, px(220.)), 0.0);
        assert_eq!(fraction_at_y(slider_bounds, px(160.)), 0.5);
        assert_eq!(fraction_at_y(slider_bounds, px(100.)), 1.0);
        assert_eq!(fraction_at_y(slider_bounds, px(250.)), 0.0);
        assert_eq!(fraction_at_y(slider_bounds, px(50.)), 1.0);
        assert_eq!(
            fraction_at_y(
                bounds(point(px(20.), px(100.)), size(px(28.), px(0.))),
                px(100.)
            ),
            0.0
        );
    }

    #[test]
    fn maps_scrub_fraction_to_position() {
        assert_eq!(scrub_position_ms(0.0, 268_000), 0);
        assert_eq!(scrub_position_ms(0.5, 268_000), 134_000);
        assert_eq!(scrub_position_ms(1.0, 268_000), 268_000);

        let bounds = bounds(point(px(100.), px(20.)), size(px(200.), px(4.)));
        assert_eq!(
            scrub_position_ms(fraction_at_x(bounds, px(250.)), 268_000),
            201_000
        );
    }

    #[test]
    fn maps_volume_level_to_a_perceptual_gain_curve() {
        assert_eq!(volume_gain_for_level(0.0), 0.0);
        assert_eq!(volume_gain_for_level(0.05), MIN_AUDIBLE_GAIN);
        assert_eq!(volume_gain_for_level(0.5), 0.125);
        assert_eq!(volume_gain_for_level(1.0), 1.0);
        assert!(volume_gain_for_level(0.25) < volume_gain_for_level(0.5));
        assert!(volume_gain_for_level(0.5) < volume_gain_for_level(0.75));
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
        assert_eq!(format_volume_percent(1.0), "100%");
    }

    #[test]
    fn default_volume_command_is_unity_and_unmuted() {
        assert_eq!(
            Playback::initial().volume_command(),
            PlaybackCommand::SetVolume {
                gain: 1.0,
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
    }

    #[test]
    fn shared_output_labels_the_track_rate_as_source_metadata() {
        assert_eq!(
            format_output_device(44_100, "AirPods Pro", false),
            "44.1 kHz source · AirPods Pro"
        );
        assert_eq!(
            format_output_device(44_100, "mini-i Series", true),
            "44.1 kHz · mini-i Series"
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
        let mut preferences = app_settings::ExclusiveModePreferences::default();
        preferences.record_sighting(
            "matrix",
            "mini-i Series",
            Some(app_settings::StoredDeviceCapabilities {
                max_bits_per_channel: Some(24),
                max_sample_rate: 192_000,
            }),
            100,
        );
        preferences.record_sighting(
            "airpods",
            "AirPods Pro",
            Some(app_settings::StoredDeviceCapabilities {
                max_bits_per_channel: None,
                max_sample_rate: 48_000,
            }),
            90,
        );
        let connected = vec![
            output_device(9, "matrix", "mini-i Series"),
            output_device(1, "built-in", "Mac Speakers"),
        ];

        let groups =
            merge_managed_devices(&connected, Some("matrix"), Some("matrix"), &preferences);
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
    fn managed_device_group_moves_keep_the_stored_override() {
        let mut preferences = app_settings::ExclusiveModePreferences::default();
        preferences.record_sighting(
            "matrix",
            "mini-i Series",
            Some(app_settings::StoredDeviceCapabilities {
                max_bits_per_channel: Some(24),
                max_sample_rate: 192_000,
            }),
            100,
        );
        preferences.set_override("matrix", false);

        let disconnected = merge_managed_devices(&[], None, None, &preferences);
        assert!(!disconnected.not_connected[0].exclusive_mode);
        assert!(!disconnected.not_connected[0].automatic);

        let connected = merge_managed_devices(
            &[output_device(9, "matrix", "mini-i Series")],
            Some("matrix"),
            None,
            &preferences,
        );
        assert!(!connected.connected[0].exclusive_mode);
        assert!(!connected.connected[0].automatic);
        assert!(preferences.is_overridden("matrix"));
    }

    #[test]
    fn managed_device_groups_sort_active_first_then_alphabetically() {
        let mut preferences = app_settings::ExclusiveModePreferences::default();
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
        row.devices = vec![matrix.clone(), airpods];
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
        row.queue = QueueState::from_tracks(std::slice::from_ref(&track), 0);

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

    fn output_device(id: device::DeviceId, uid: &str, name: &str) -> device::Device {
        device::Device {
            id,
            uid: uid.to_string(),
            name: name.to_string(),
        }
    }

    fn library_track(id: crate::library::TrackId, path: PathBuf, title: &str) -> Track {
        Track {
            id,
            storage_root_id: 1,
            path,
            title: Some(title.to_string()),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            album_artist: None,
            year: None,
            genre: None,
            track_number: None,
            disc_number: None,
            duration_ms: Some(1_000),
            sample_rate_hz: Some(44_100),
            bit_depth: Some(16),
            channels: Some(2),
            file_size_bytes: 1,
            modified_at_ns: 1,
            cover_art_path: None,
            cover_art_mime_type: None,
            added_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn wav_tracks(directory: &Path, names: &[&str]) -> Vec<Track> {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let path = directory.join(format!("{name}.wav"));
                crate::library::metadata::write_test_wav(&path, name, "Artist", "Album").unwrap();
                library_track(index as i64 + 1, path, name)
            })
            .collect()
    }

    fn truncate_wav(path: &Path) {
        let bytes = std::fs::read(path).unwrap();
        std::fs::write(path, &bytes[..20]).unwrap();
    }

    fn now_playing(path: &str) -> PlaybackEvent {
        PlaybackEvent::NowPlaying {
            source: pulse_engine::PlayableSource {
                path: PathBuf::from(path),
                duration_ms: Some(268_000),
            },
            format: PcmFormat {
                sample_rate: 44_100,
                bits_per_sample: 16,
                channels: 2,
            },
        }
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

    #[test]
    fn engine_errors_map_to_the_kinds_that_drive_queue_behavior() {
        assert_eq!(
            PlaybackErrorKind::from(&EngineError::Hogged(i32::MAX)),
            PlaybackErrorKind::Device {
                hog_pid: Some(i32::MAX)
            }
        );
        // Audio-unit/device start failures must never skip the queue.
        assert_eq!(
            PlaybackErrorKind::from(&EngineError::AudioUnit(
                "the requested device was not found".into()
            )),
            PlaybackErrorKind::Device { hog_pid: None }
        );
        assert_eq!(
            PlaybackErrorKind::from(&EngineError::Os {
                call: "AudioUnitRender",
                status: -10863
            }),
            PlaybackErrorKind::Device { hog_pid: None }
        );
        // Format problems belong to the track and are skippable.
        assert_eq!(
            PlaybackErrorKind::from(&EngineError::UnsupportedFormat(
                "20-bit PCM is not supported by the AUHAL packer".into()
            )),
            PlaybackErrorKind::Track
        );
    }

    #[test]
    fn a_decode_failure_mid_queue_skips_to_the_next_entry_and_reports() {
        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["corrupt", "good", "later"]);
        truncate_wav(&tracks[0].path);
        let mut row = Playback::initial();
        row.queue = QueueState::from_tracks(&tracks, 0);
        let error = pulse_engine::decode::open(&tracks[0].path)
            .map(|_| ())
            .unwrap_err();
        let kind = PlaybackErrorKind::from(&error);
        let message = error.to_string();

        assert!(
            row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading))
                .is_none()
        );
        assert!(
            row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Error))
                .is_none()
        );
        let next = row
            .handle_event(PlaybackEvent::Error {
                attempt: 0,
                kind,
                message,
            })
            .unwrap();

        assert_eq!(next.title, "good");
        assert_eq!(row.next_playable(next).unwrap().title, "good");
        assert_eq!(
            row.notice,
            Some(PlaybackNotice::Skip {
                text: "Skipped “corrupt” — its file could not be decoded.".to_string()
            })
        );
        assert!(!row.is_track_missing(1), "a corrupt file is not missing");
        assert!(row.error.is_none(), "the queue keeps playing");
    }

    #[test]
    fn missing_files_are_marked_and_skipped_at_play_time() {
        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["gone-1", "gone-2", "present"]);
        std::fs::remove_file(&tracks[0].path).unwrap();
        std::fs::remove_file(&tracks[1].path).unwrap();
        let mut row = Playback::initial();
        row.queue = QueueState::from_tracks(&tracks, 0);

        let first = row.queue.current().cloned().unwrap();
        let playable = row.next_playable(first).unwrap();

        assert_eq!(playable.title, "present");
        assert_eq!(row.queue.current().unwrap().title, "present");
        assert!(row.is_track_missing(1));
        assert!(row.is_track_missing(2));
        assert!(!row.is_track_missing(3));
        assert_eq!(
            row.notice,
            Some(PlaybackNotice::Skip {
                text: "Skipped 2 tracks that could not be played.".to_string()
            })
        );
    }

    #[test]
    fn a_queue_where_every_file_is_gone_stops_with_a_poison_message() {
        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["gone-1", "gone-2"]);
        std::fs::remove_file(&tracks[0].path).unwrap();
        std::fs::remove_file(&tracks[1].path).unwrap();
        let mut row = Playback::initial();
        row.queue = QueueState::from_tracks(&tracks, 0);

        let first = row.queue.current().cloned().unwrap();
        assert!(row.next_playable(first).is_none());
        assert!(row.is_track_missing(1));
        assert!(row.is_track_missing(2));
        assert_eq!(
            row.notice,
            Some(PlaybackNotice::Stopped {
                text: "Playback stopped — none of the queued tracks could be played.".to_string()
            })
        );
    }

    #[test]
    fn a_trailing_failure_after_played_tracks_is_not_reported_as_poison() {
        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["played", "gone"]);
        std::fs::remove_file(&tracks[1].path).unwrap();
        let mut row = Playback::initial();
        row.queue = QueueState::from_tracks(&tracks, 0);
        row.queue.mark_started();

        let next = row
            .handle_event(PlaybackEvent::Ended { attempt: 0 })
            .expect("the queue advances past the ended track");
        assert!(row.next_playable(next).is_none());
        assert_eq!(
            row.notice,
            Some(PlaybackNotice::Stopped {
                text: "Playback stopped — “gone” could not be played.".to_string()
            })
        );
    }

    #[test]
    fn now_playing_clears_the_missing_mark_for_a_recovered_queue_track() {
        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["recovered"]);
        let mut row = Playback::initial();
        row.queue = QueueState::from_tracks(&tracks, 0);
        row.missing_track_ids.insert(1);

        let _ = row.handle_event(PlaybackEvent::NowPlaying {
            source: pulse_engine::PlayableSource {
                path: tracks[0].path.clone(),
                duration_ms: Some(1_000),
            },
            format: PcmFormat {
                sample_rate: 44_100,
                bits_per_sample: 16,
                channels: 2,
            },
        });

        assert!(!row.is_track_missing(1));
        assert!(!row.queue.nothing_played());
    }

    #[test]
    fn a_hogged_device_reports_plain_language_with_a_retry_target() {
        let mut row = Playback::initial();
        row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
        row.record_play_attempt(Path::new("/Music/track.flac"));
        let _ = row.handle_event(now_playing("/Music/track.flac"));
        row.playback_state = PlaybackState::Playing;
        let _ = row.handle_event(PlaybackEvent::Position {
            position_ms: 42_000,
            duration_ms: Some(268_000),
        });

        let outcome = row.handle_event(PlaybackEvent::Error {
            attempt: 0,
            kind: (&EngineError::Hogged(i32::MAX)).into(),
            message: EngineError::Hogged(i32::MAX).to_string(),
        });

        assert!(outcome.is_none(), "device failures never skip the queue");
        assert_eq!(
            row.notice,
            Some(PlaybackNotice::DeviceFailure {
                text: "Another app is using mini-i Series exclusively. Quit it or choose another output, then try again.".to_string()
            })
        );
        assert_eq!(
            row.retry,
            Some(RetryTarget {
                path: PathBuf::from("/Music/track.flac"),
                position_ms: 42_000,
            })
        );
    }

    #[test]
    fn a_device_failure_while_loading_a_new_file_retries_that_file_from_zero() {
        let mut row = Playback::initial();
        row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
        // A previous track was playing at 42 s when the user dropped a new
        // file; source_path and position_ms still describe the old track
        // while the new one loads.
        row.source_path = Some(PathBuf::from("/Music/old.flac"));
        row.position_ms = 42_000;
        row.record_play_attempt(Path::new("/Music/dropped.flac"));
        row.playback_state = PlaybackState::Loading;

        let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Error));
        let outcome = row.handle_event(PlaybackEvent::Error {
            attempt: 0,
            kind: (&EngineError::Hogged(i32::MAX)).into(),
            message: EngineError::Hogged(i32::MAX).to_string(),
        });

        assert!(outcome.is_none());
        assert_eq!(
            row.retry,
            Some(RetryTarget {
                path: PathBuf::from("/Music/dropped.flac"),
                position_ms: 0,
            })
        );
        // Transport Play must restart the same file Try again targets.
        assert_eq!(
            row.prepare_toggle_command(),
            Some(PlaybackCommand::PlayFile {
                path: PathBuf::from("/Music/dropped.flac")
            })
        );
    }

    #[test]
    fn a_device_failure_during_auto_advance_retries_the_next_track_from_zero() {
        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["finished", "next"]);
        let mut row = Playback::initial();
        row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
        row.queue = QueueState::from_tracks(&tracks, 0);
        row.record_play_attempt(&tracks[0].path);
        let _ = row.handle_event(now_playing(tracks[0].path.to_str().unwrap()));
        row.playback_state = PlaybackState::Playing;
        let _ = row.handle_event(PlaybackEvent::Position {
            position_ms: 180_000,
            duration_ms: Some(180_000),
        });

        let next = row
            .handle_event(PlaybackEvent::Ended { attempt: 0 })
            .expect("the queue advances");
        row.record_play_attempt(&next.path);
        let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));

        let outcome = row.handle_event(PlaybackEvent::Error {
            attempt: 0,
            kind: (&EngineError::Hogged(i32::MAX)).into(),
            message: EngineError::Hogged(i32::MAX).to_string(),
        });

        assert!(outcome.is_none());
        assert_eq!(
            row.retry,
            Some(RetryTarget {
                path: tracks[1].path.clone(),
                position_ms: 0,
            })
        );
    }

    #[test]
    fn transport_play_from_an_idle_selection_records_the_attempt() {
        let mut row = Playback::initial();
        row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
        let track = TrackRef {
            id: 1,
            path: PathBuf::from("/Music/selected.flac"),
            title: "Selected".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            duration_ms: None,
            cover_art_path: None,
        };
        assert!(row.apply_track_selection(&track));

        let command = row.prepare_toggle_command().unwrap();
        assert_eq!(
            command,
            PlaybackCommand::PlayFile {
                path: PathBuf::from("/Music/selected.flac")
            }
        );
        assert_eq!(
            row.current_play,
            Some(PlayAttempt {
                target: RetryTarget {
                    path: PathBuf::from("/Music/selected.flac"),
                    position_ms: 0,
                },
                confirmed: false,
            })
        );

        let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
        let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Error));
        let _ = row.handle_event(PlaybackEvent::Error {
            attempt: 0,
            kind: (&EngineError::Hogged(i32::MAX)).into(),
            message: EngineError::Hogged(i32::MAX).to_string(),
        });
        assert_eq!(
            row.retry,
            Some(RetryTarget {
                path: PathBuf::from("/Music/selected.flac"),
                position_ms: 0,
            })
        );
    }

    #[test]
    fn replaying_an_ended_track_restarts_the_attempt_from_zero() {
        let mut row = Playback::initial();
        row.record_play_attempt(Path::new("/Music/track.flac"));
        let _ = row.handle_event(now_playing("/Music/track.flac"));
        row.playback_state = PlaybackState::Playing;
        let _ = row.handle_event(PlaybackEvent::Position {
            position_ms: 268_000,
            duration_ms: Some(268_000),
        });
        let _ = row.handle_event(PlaybackEvent::Ended { attempt: 0 });

        let command = row.prepare_toggle_command().unwrap();
        assert!(matches!(command, PlaybackCommand::PlayFile { .. }));
        assert_eq!(
            row.current_play,
            Some(PlayAttempt {
                target: RetryTarget {
                    path: PathBuf::from("/Music/track.flac"),
                    position_ms: 0,
                },
                confirmed: false,
            })
        );
    }

    #[test]
    fn stale_terminal_events_from_a_superseded_play_are_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["b1", "b2"]);
        let mut row = Playback::initial();
        row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
        // Play A happened (attempt 1), then the user started queue B
        // (attempt 2) before A's terminal events drained.
        row.queue = QueueState::from_tracks(&tracks, 0);
        row.dispatched_plays = 2;
        row.playback_state = PlaybackState::Loading;

        let stale_track_error = row.handle_event(PlaybackEvent::Error {
            attempt: 1,
            kind: PlaybackErrorKind::Track,
            message: "decode: stale failure from A".to_string(),
        });
        assert!(stale_track_error.is_none(), "stale error must not skip B");
        assert_eq!(row.queue.current().unwrap().title, "b1");
        assert!(row.notice.is_none());

        let stale_ended = row.handle_event(PlaybackEvent::Ended { attempt: 1 });
        assert!(stale_ended.is_none(), "stale Ended must not advance B");
        assert_eq!(row.queue.current().unwrap().title, "b1");
        assert_eq!(row.playback_state, PlaybackState::Loading);

        let stale_device_error = row.handle_event(PlaybackEvent::Error {
            attempt: 1,
            kind: PlaybackErrorKind::Device { hog_pid: None },
            message: "stale device failure from A".to_string(),
        });
        assert!(stale_device_error.is_none());
        assert!(
            row.notice.is_none(),
            "no failure notice for a superseded play"
        );
        assert!(row.retry.is_none());

        let current = row.handle_event(PlaybackEvent::Error {
            attempt: 2,
            kind: PlaybackErrorKind::Track,
            message: "decode: real failure for B".to_string(),
        });
        assert_eq!(
            current.unwrap().title,
            "b2",
            "current-attempt errors still drive the queue"
        );
    }

    #[test]
    fn a_stale_now_playing_does_not_replace_an_unconfirmed_attempt() {
        let mut row = Playback::initial();
        row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
        // The old track's NowPlaying was already queued when the user
        // dispatched a new file; it must not steal the retry target back.
        row.record_play_attempt(Path::new("/Music/new.flac"));
        let _ = row.handle_event(now_playing("/Music/old.flac"));
        assert_eq!(row.source_path, None, "stale NowPlaying is ignored fully");
        assert_eq!(row.title, "No track loaded");

        let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
        let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Error));
        let _ = row.handle_event(PlaybackEvent::Error {
            attempt: 0,
            kind: (&EngineError::Hogged(i32::MAX)).into(),
            message: EngineError::Hogged(i32::MAX).to_string(),
        });

        assert_eq!(
            row.retry,
            Some(RetryTarget {
                path: PathBuf::from("/Music/new.flac"),
                position_ms: 0,
            })
        );
        // Transport Play and Try again must agree on the failed file.
        assert_eq!(
            row.prepare_toggle_command(),
            Some(PlaybackCommand::PlayFile {
                path: PathBuf::from("/Music/new.flac")
            })
        );
    }

    #[test]
    fn a_direct_file_decode_failure_before_now_playing_names_the_attempted_file() {
        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["corrupt"]);
        truncate_wav(&tracks[0].path);
        let error = pulse_engine::decode::open(&tracks[0].path)
            .map(|_| ())
            .unwrap_err();
        let kind = PlaybackErrorKind::from(&error);
        let message = error.to_string();
        let mut row = Playback::initial();
        assert_eq!(row.title, "No track loaded");

        // A dropped file fails at decoder open, before any NowPlaying.
        row.record_play_attempt(&tracks[0].path);
        let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
        let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Error));
        let _ = row.handle_event(PlaybackEvent::Error {
            attempt: 0,
            kind,
            message: message.clone(),
        });

        match &row.notice {
            Some(PlaybackNotice::Stopped { text }) => {
                assert!(text.contains("“corrupt”"), "{text}");
                assert!(!text.contains("No track loaded"), "{text}");
            }
            other => panic!("expected a Stopped notice, got {other:?}"),
        }
    }

    #[test]
    fn stale_position_events_do_not_move_an_unconfirmed_attempt() {
        let mut row = Playback::initial();
        row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
        // A new file was dispatched while the old track's last Position
        // event was still queued; it must not move the new attempt.
        row.record_play_attempt(Path::new("/Music/new.flac"));
        let _ = row.handle_event(PlaybackEvent::Position {
            position_ms: 42_000,
            duration_ms: Some(268_000),
        });
        let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Loading));
        let _ = row.handle_event(PlaybackEvent::Error {
            attempt: 0,
            kind: (&EngineError::Hogged(i32::MAX)).into(),
            message: EngineError::Hogged(i32::MAX).to_string(),
        });

        assert_eq!(
            row.retry,
            Some(RetryTarget {
                path: PathBuf::from("/Music/new.flac"),
                position_ms: 0,
            })
        );
    }

    #[test]
    fn losing_the_active_device_mid_playback_stops_with_a_recovery_notice() {
        let mut row = Playback::initial();
        row.active_device = Some(output_device(9, "matrix", "mini-i Series"));
        row.record_play_attempt(Path::new("/Music/track.flac"));
        let _ = row.handle_event(now_playing("/Music/track.flac"));
        row.playback_state = PlaybackState::Playing;
        let _ = row.handle_event(PlaybackEvent::Position {
            position_ms: 10_000,
            duration_ms: Some(268_000),
        });

        let still_attached = vec![
            output_device(1, "built-in", "Mac Speakers"),
            output_device(9, "matrix", "mini-i Series"),
        ];
        assert!(!row.note_device_loss(still_attached));
        assert!(row.notice.is_none());

        let unplugged = vec![output_device(1, "built-in", "Mac Speakers")];
        assert!(row.note_device_loss(unplugged));
        assert_eq!(
            row.notice,
            Some(PlaybackNotice::DeviceFailure {
                text: "mini-i Series was disconnected. Reconnect it or choose another output, then try again.".to_string()
            })
        );
        assert_eq!(
            row.retry,
            Some(RetryTarget {
                path: PathBuf::from("/Music/track.flac"),
                position_ms: 10_000,
            })
        );
        assert_eq!(row.devices.len(), 1);
    }

    #[test]
    fn advisory_errors_after_teardown_do_not_disturb_the_queue_report() {
        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["only"]);
        let mut row = Playback::initial();
        row.queue = QueueState::from_tracks(&tracks, 0);
        row.playback_state = PlaybackState::Ended;
        row.notice = Some(PlaybackNotice::Skip {
            text: "Skipped “gone” — its file is missing.".to_string(),
        });

        let outcome = row.handle_event(PlaybackEvent::Error {
            attempt: 0,
            kind: PlaybackErrorKind::Track,
            message: "decode: backend stop failed".to_string(),
        });

        assert!(outcome.is_none());
        assert_eq!(
            row.notice,
            Some(PlaybackNotice::Skip {
                text: "Skipped “gone” — its file is missing.".to_string()
            })
        );
        assert_eq!(row.error.as_deref(), Some("decode: backend stop failed"));
    }

    #[test]
    fn a_jump_to_a_marked_missing_entry_skips_and_reports() {
        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["playing", "gone", "after"]);
        std::fs::remove_file(&tracks[1].path).unwrap();
        let mut row = Playback::initial();
        row.queue = QueueState::from_tracks(&tracks, 0);
        row.queue.mark_started();
        row.missing_track_ids.insert(2);

        let target = row.queue.jump_to(1).unwrap();
        let playable = row.prepare_queue_play(target).unwrap();

        assert_eq!(playable, tracks[2].path);
        assert_eq!(row.queue.current().unwrap().title, "after");
        assert!(row.is_track_missing(2));
        assert_eq!(
            row.notice,
            Some(PlaybackNotice::Skip {
                text: "Skipped “gone” — its file is missing.".to_string()
            })
        );
    }

    #[test]
    fn a_jump_into_an_all_missing_tail_stops_the_active_track() {
        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["playing", "gone-1", "gone-2"]);
        std::fs::remove_file(&tracks[1].path).unwrap();
        std::fs::remove_file(&tracks[2].path).unwrap();
        let mut row = Playback::initial();
        row.queue = QueueState::from_tracks(&tracks, 0);
        row.dispatched_plays = 1;
        row.record_play_attempt(&tracks[0].path);
        let _ = row.handle_event(now_playing(tracks[0].path.to_str().unwrap()));
        row.playback_state = PlaybackState::Playing;

        // The user clicks an up-next row whose file and remaining tail are
        // gone: no new dispatch is possible.
        let target = row.queue.jump_to(1).unwrap();
        assert!(row.prepare_queue_play(target).is_none());

        // `play_queue_track` must stop the still-audible superseded track so
        // the Stopped notice is truthful.
        assert!(row.active_playback_needs_stop());
        assert_eq!(
            row.notice,
            Some(PlaybackNotice::Stopped {
                text: "Playback stopped — 2 tracks could not be played.".to_string()
            })
        );
        assert!(row.is_track_missing(2));
        assert!(row.is_track_missing(3));

        // Until the stop lands, the NOW PLAYING block keeps describing the
        // audible track, not the missing entry the queue index points at.
        assert_eq!(
            row.snapshot().now_playing_lines(),
            Some(("playing".to_string(), "Artist - Album".to_string()))
        );
        let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Idle));
        assert_eq!(row.snapshot().now_playing_lines(), None);

        // An ended/error event from the stopped track is stale bookkeeping,
        // not a queue driver, once state left the active set.
        assert!(
            row.handle_event(PlaybackEvent::Ended { attempt: 1 })
                .is_none()
        );
    }

    #[test]
    fn a_natural_queue_end_does_not_ask_for_a_stop() {
        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["played", "gone"]);
        std::fs::remove_file(&tracks[1].path).unwrap();
        let mut row = Playback::initial();
        row.queue = QueueState::from_tracks(&tracks, 0);
        row.queue.mark_started();
        row.playback_state = PlaybackState::Ended;

        let next = row
            .handle_event(PlaybackEvent::Ended { attempt: 0 })
            .expect("the queue advances past the ended track");
        assert!(row.prepare_queue_play(next).is_none());
        assert!(
            !row.active_playback_needs_stop(),
            "nothing is audible after a natural end; no Stop command is due"
        );
    }

    #[test]
    fn a_jump_dispatch_supersedes_stale_terminal_events() {
        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["playing", "second", "target", "tail"]);
        let mut row = Playback::initial();
        row.queue = QueueState::from_tracks(&tracks, 0);
        // Entry 0 is playing as dispatch 1.
        row.dispatched_plays = 1;
        row.record_play_attempt(&tracks[0].path);
        let _ = row.handle_event(now_playing(tracks[0].path.to_str().unwrap()));
        row.playback_state = PlaybackState::Playing;

        // The user clicks the third up-next row. The jump prepares the play;
        // `play_file`/`send_command` then record the attempt and count the
        // dispatch, simulated here because tests have no command channel.
        let target = row.queue.jump_to(2).unwrap();
        let path = row.prepare_queue_play(target).unwrap();
        assert_eq!(path, tracks[2].path);
        row.record_play_attempt(&path);
        row.dispatched_plays += 1;
        row.playback_state = PlaybackState::Loading;

        // The superseded track's terminal events drain afterwards; the older
        // attempt ordinal keeps them from moving the jumped queue.
        assert!(
            row.handle_event(PlaybackEvent::Ended { attempt: 1 })
                .is_none()
        );
        assert_eq!(row.queue.current().unwrap().title, "target");
        assert!(
            row.handle_event(PlaybackEvent::Error {
                attempt: 1,
                kind: PlaybackErrorKind::Track,
                message: "decode: stale failure".to_string(),
            })
            .is_none()
        );
        assert_eq!(row.queue.current().unwrap().title, "target");
        assert!(row.notice.is_none());

        // The jump's own terminal events still drive the queue.
        let next = row
            .handle_event(PlaybackEvent::Ended { attempt: 2 })
            .unwrap();
        assert_eq!(next.title, "tail");
    }

    #[test]
    fn the_now_playing_block_tracks_playback_state_and_queue_context() {
        let mut row = Playback::initial();
        assert_eq!(row.snapshot().now_playing_lines(), None);

        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["present"]);
        row.queue = QueueState::from_tracks(&tracks, 0);
        row.source_path = Some(tracks[0].path.clone());
        row.playback_state = PlaybackState::Playing;
        assert_eq!(
            row.snapshot().now_playing_lines(),
            Some(("present".to_string(), "Artist · Album".to_string()))
        );

        row.playback_state = PlaybackState::Ended;
        assert_eq!(row.snapshot().now_playing_lines(), None);

        // A dropped file plays without a queue; the row's display strings
        // stand in.
        row.queue = QueueState::default();
        row.playback_state = PlaybackState::Playing;
        row.title = "Dropped".to_string();
        row.secondary = "Folder".to_string();
        assert_eq!(
            row.snapshot().now_playing_lines(),
            Some(("Dropped".to_string(), "Folder".to_string()))
        );
    }

    #[test]
    fn formats_queue_times_without_a_leading_minute_zero() {
        assert_eq!(format_queue_time(0), "0:00");
        assert_eq!(format_queue_time(84_000), "1:24");
        assert_eq!(format_queue_time(239_000), "3:59");
        assert_eq!(format_queue_time(3_661_000), "1:01:01");
    }

    #[test]
    fn formats_the_queue_meta_line_and_omits_unknown_minutes() {
        assert_eq!(format_queue_meta(7, 1_470_000), "7 UP NEXT · 25 MIN");
        assert_eq!(format_queue_meta(1, 29_000), "1 UP NEXT · 1 MIN");
        assert_eq!(format_queue_meta(3, 0), "3 UP NEXT");
    }
}
