mod controller;
mod devices;
mod logic;
mod queue_control;

pub(crate) use logic::*;

use std::{
    collections::{BTreeMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{Receiver, Sender, TryRecvError},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use pulse_engine::{
    EngineError, PcmFormat, PlayableSource, PlaybackCommand, PlaybackController, PlaybackErrorKind,
    PlaybackEvent, PlaybackState, device,
};

use super::{
    Track, TrackId,
    queue::{PreviousAction, QueueState, TrackRef},
    settings::{AppSettings, ExclusiveModePreferences, StoredDeviceCapabilities},
};

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
    pub(crate) queue: Arc<QueueState>,
    pub(crate) title: String,
    pub(crate) secondary: String,
    pub(crate) format: Option<PcmFormat>,
    pub(crate) devices: Arc<Vec<device::Device>>,
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
    pub(crate) missing_track_ids: Arc<HashSet<TrackId>>,
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
    queue_snapshot: Arc<QueueState>,
    pub(crate) title: String,
    pub(crate) secondary: String,
    pub(crate) format: Option<PcmFormat>,
    pub(crate) devices: Vec<device::Device>,
    devices_snapshot: Arc<Vec<device::Device>>,
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
    missing_track_ids_snapshot: Arc<HashSet<TrackId>>,
    rejected_next_track_ids: HashSet<TrackId>,
    dispatched_plays: u64,
    /// Mirrors the engine's `next_source`, which the engine consumes at the transition.
    sent_next: Option<PathBuf>,
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
            queue_snapshot: Arc::new(QueueState::default()),
            title: "No track loaded".to_string(),
            secondary: "Choose a track from your library".to_string(),
            format: None,
            devices: Vec::new(),
            devices_snapshot: Arc::new(Vec::new()),
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
            missing_track_ids_snapshot: Arc::new(HashSet::new()),
            rejected_next_track_ids: HashSet::new(),
            dispatched_plays: 0,
            sent_next: None,
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
            queue_snapshot: Arc::new(QueueState::default()),
            title: "No track loaded".to_string(),
            secondary: "Choose a track from your library".to_string(),
            format: None,
            devices: Vec::new(),
            devices_snapshot: Arc::new(Vec::new()),
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
            missing_track_ids_snapshot: Arc::new(HashSet::new()),
            rejected_next_track_ids: HashSet::new(),
            dispatched_plays: 0,
            sent_next: None,
            current_play: None,
            retry: None,
            pending_seek_ms: None,
            settings: AppSettings::default(),
            settings_path: PathBuf::new(),
        }
    }

    #[cfg(test)]
    fn seed_queue(&mut self, queue: QueueState) {
        self.queue = queue;
        self.refresh_queue_snapshot();
    }

    #[cfg(test)]
    fn seed_devices(&mut self, devices: Vec<device::Device>) {
        self.devices = devices;
        self.refresh_devices_snapshot();
    }

    #[cfg(test)]
    fn seed_missing_track_ids(&mut self, missing_track_ids: HashSet<TrackId>) {
        self.missing_track_ids = missing_track_ids;
        self.refresh_missing_track_ids_snapshot();
    }

    pub(crate) fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub(crate) fn snapshot(&self) -> PlaybackSnapshot {
        PlaybackSnapshot {
            playback_state: self.playback_state,
            source_path: self.source_path.clone(),
            cover_art_path: self.cover_art_path.clone(),
            queue: self.queue_snapshot.clone(),
            title: self.title.clone(),
            secondary: self.secondary.clone(),
            format: self.format,
            devices: self.devices_snapshot.clone(),
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
            missing_track_ids: self.missing_track_ids_snapshot.clone(),
        }
    }

    fn refresh_queue_snapshot(&mut self) {
        if self.queue_snapshot.as_ref() != &self.queue {
            self.queue_snapshot = Arc::new(self.queue.clone());
        }
    }

    fn refresh_devices_snapshot(&mut self) {
        let changed = self.devices_snapshot.len() != self.devices.len()
            || self
                .devices_snapshot
                .iter()
                .zip(&self.devices)
                .any(|(before, after)| {
                    let device::Device {
                        id: before_id,
                        uid: before_uid,
                        name: before_name,
                    } = before;
                    let device::Device {
                        id: after_id,
                        uid: after_uid,
                        name: after_name,
                    } = after;
                    before_id != after_id || before_uid != after_uid || before_name != after_name
                });
        if changed {
            self.devices_snapshot = Arc::new(self.devices.clone());
        }
    }

    fn refresh_missing_track_ids_snapshot(&mut self) {
        if self.missing_track_ids_snapshot.as_ref() != &self.missing_track_ids {
            self.missing_track_ids_snapshot = Arc::new(self.missing_track_ids.clone());
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
}

#[cfg(test)]
mod tests;
