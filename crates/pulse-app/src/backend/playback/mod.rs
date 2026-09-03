mod controller;
mod devices;
mod logic;
mod queue_control;
mod session;

pub(crate) use logic::*;

use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{Receiver, Sender, TryRecvError},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use pulse_engine::{
    EngineError, EngineKind, PcmFormat, PlayableSource, PlaybackCommand, PlaybackController,
    PlaybackErrorKind, PlaybackEvent, PlaybackState, VolumeState, device,
};

use super::{
    Track, TrackId,
    queue::{PreviousAction, QueueState, TrackRef},
    settings::{
        AppSettings, OutputModePreferences, SessionRoute, SessionState, StoredDeviceCapabilities,
        StoredDeviceTransport, StoredOutputMode,
    },
};

use session::SessionSaveCadence;

const SUPPORTED_EXTENSIONS: &[&str] = &["flac", "m4a", "aif", "aiff", "wav", "dsf", "dff"];

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
    automatic_mode: StoredOutputMode,
    output_mode: StoredOutputMode,
    engine_kind: EngineKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeviceMessage {
    pub(crate) text: String,
    pub(crate) is_error: bool,
}

/// A standing playback condition rendered above the row with persistent recovery controls.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PlaybackNotice {
    Dropouts { text: String },
    DeviceFailure { text: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlaybackToastKind {
    Error,
    Warning,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PlaybackToastAction {
    SwitchToExclusive { device_uid: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlaybackToast {
    pub(crate) kind: PlaybackToastKind,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) action: Option<PlaybackToastAction>,
}

impl PlaybackToast {
    fn error(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            kind: PlaybackToastKind::Error,
            title: title.into(),
            body: body.into(),
            action: None,
        }
    }

    fn warning(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            kind: PlaybackToastKind::Warning,
            title: title.into(),
            body: body.into(),
            action: None,
        }
    }

    fn error_with_action(
        title: impl Into<String>,
        body: impl Into<String>,
        action: PlaybackToastAction,
    ) -> Self {
        Self {
            kind: PlaybackToastKind::Error,
            title: title.into(),
            body: body.into(),
            action: Some(action),
        }
    }
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
    load: bool,
}

struct StagedSettings {
    generation: Option<u64>,
    settings: AppSettings,
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
    pub output_mode: StoredOutputMode,
    pub integer_path_available: bool,
    pub hardware_volume_available: bool,
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
    pub(crate) output_mode: StoredOutputMode,
    pub(crate) playback_output_mode: StoredOutputMode,
    pub(crate) resolved_engine_kind: EngineKind,
    pub(crate) bit_perfect_active: bool,
    pub(crate) volume_state: VolumeState,
    pub(crate) volume_level: f32,
    pub(crate) volume_muted: bool,
    pub(crate) position_ms: u64,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) dropout_frames: u64,
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
    SetDeviceOutputMode {
        device_uid: String,
        mode: StoredOutputMode,
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

trait EngineController {
    fn shutdown(&mut self) -> Result<(), EngineError>;
}

impl EngineController for PlaybackController {
    fn shutdown(&mut self) -> Result<(), EngineError> {
        PlaybackController::shutdown(self)
    }
}

pub(crate) struct Playback {
    controller: Option<Box<dyn EngineController>>,
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
    pending_output_mode_engine_kind: Option<EngineKind>,
    pending_saved_output_device_uid: Option<String>,
    pub(crate) device_message: Option<DeviceMessage>,
    device_sightings_writable: bool,
    pub(crate) automatic_output_mode: StoredOutputMode,
    pub(crate) output_mode: StoredOutputMode,
    pub(crate) playback_output_mode: StoredOutputMode,
    pub(crate) resolved_engine_kind: EngineKind,
    pub(crate) bit_perfect_active: bool,
    pub(crate) volume_state: VolumeState,
    pub(crate) volume_level: f32,
    pub(crate) volume_muted: bool,
    pub(crate) position_ms: u64,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) dropout_frames: u64,
    pub(crate) error: Option<String>,
    pub(crate) notice: Option<PlaybackNotice>,
    toasts: VecDeque<PlaybackToast>,
    pub(crate) missing_track_ids: HashSet<TrackId>,
    missing_track_ids_snapshot: Arc<HashSet<TrackId>>,
    rejected_next_track_ids: HashSet<TrackId>,
    dispatched_plays: u64,
    /// Mirrors the engine's `next_source`, which the engine consumes at the transition.
    sent_next: Option<PathBuf>,
    pending_dsd_skips: Vec<TrackRef>,
    current_play: Option<PlayAttempt>,
    retry: Option<RetryTarget>,
    retry_after_output_mode_change: bool,
    pending_seek_ms: Option<u64>,
    recent_dropouts: VecDeque<Instant>,
    last_dropout_at: Option<Instant>,
    session_save_cadence: SessionSaveCadence,
    /// Protects an existing launch blob until restore concludes or the first
    /// successful PlayFile/Load dispatch proves that live playback replaced it.
    launch_session_pending: bool,
    settings: AppSettings,
    settings_path: PathBuf,
    settings_writer: Option<super::settings::SettingsWriter>,
    staged_settings: Option<StagedSettings>,
    next_settings_generation: u64,
    last_settings_error: Option<String>,
    shutdown_complete: bool,
}

impl Playback {
    pub(crate) fn new(settings_path: PathBuf, settings: AppSettings) -> Self {
        let saved_position_ms = settings
            .session
            .as_ref()
            .map_or(0, |session| session.position_ms);
        let launch_session_pending = settings.session.is_some();
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
            pending_output_mode_engine_kind: None,
            pending_saved_output_device_uid: None,
            device_message: None,
            device_sightings_writable: true,
            automatic_output_mode: StoredOutputMode::Shared,
            output_mode: StoredOutputMode::Shared,
            playback_output_mode: StoredOutputMode::Shared,
            resolved_engine_kind: EngineKind::Universal {
                exclusive_mode: false,
            },
            bit_perfect_active: false,
            volume_state: VolumeState::default(),
            volume_level: settings.volume_level,
            volume_muted: settings.volume_muted,
            position_ms: 0,
            duration_ms: None,
            dropout_frames: 0,
            error: None,
            notice: None,
            toasts: VecDeque::new(),
            missing_track_ids: HashSet::new(),
            missing_track_ids_snapshot: Arc::new(HashSet::new()),
            rejected_next_track_ids: HashSet::new(),
            dispatched_plays: 0,
            sent_next: None,
            pending_dsd_skips: Vec::new(),
            current_play: None,
            retry: None,
            retry_after_output_mode_change: false,
            pending_seek_ms: None,
            recent_dropouts: VecDeque::new(),
            last_dropout_at: None,
            session_save_cadence: SessionSaveCadence::new(saved_position_ms),
            launch_session_pending,
            settings,
            settings_path,
            settings_writer: None,
            staged_settings: None,
            next_settings_generation: 0,
            last_settings_error: None,
            shutdown_complete: false,
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
            pending_output_mode_engine_kind: None,
            pending_saved_output_device_uid: None,
            device_message: None,
            device_sightings_writable: false,
            automatic_output_mode: StoredOutputMode::Shared,
            output_mode: StoredOutputMode::Shared,
            playback_output_mode: StoredOutputMode::Shared,
            resolved_engine_kind: EngineKind::Universal {
                exclusive_mode: false,
            },
            bit_perfect_active: false,
            volume_state: VolumeState::default(),
            volume_level: 1.0,
            volume_muted: false,
            position_ms: 0,
            duration_ms: None,
            dropout_frames: 0,
            error: None,
            notice: None,
            toasts: VecDeque::new(),
            missing_track_ids: HashSet::new(),
            missing_track_ids_snapshot: Arc::new(HashSet::new()),
            rejected_next_track_ids: HashSet::new(),
            dispatched_plays: 0,
            sent_next: None,
            pending_dsd_skips: Vec::new(),
            current_play: None,
            retry: None,
            retry_after_output_mode_change: false,
            pending_seek_ms: None,
            recent_dropouts: VecDeque::new(),
            last_dropout_at: None,
            session_save_cadence: SessionSaveCadence::new(0),
            // General playback tests do not own a settings path; session tests release this gate.
            launch_session_pending: true,
            settings: AppSettings::default(),
            settings_path: PathBuf::new(),
            settings_writer: None,
            staged_settings: None,
            next_settings_generation: 0,
            last_settings_error: None,
            shutdown_complete: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(settings_path: PathBuf, settings: AppSettings) -> Self {
        let saved_position_ms = settings
            .session
            .as_ref()
            .map_or(0, |session| session.position_ms);
        let launch_session_pending = settings.session.is_some();
        let mut playback = Self::initial();
        playback.settings_path = settings_path;
        playback.settings = settings;
        playback.session_save_cadence = SessionSaveCadence::new(saved_position_ms);
        playback.launch_session_pending = launch_session_pending;
        playback
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

    pub(crate) fn set_interface_scale(&mut self, interface_scale: f32) -> bool {
        match self.update_settings(|settings| settings.interface_scale = interface_scale) {
            Ok(changed) => changed,
            Err(error) => {
                self.error = Some(format!("Could not save the interface scale: {error}"));
                false
            }
        }
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
            output_mode: self.output_mode,
            playback_output_mode: self.playback_output_mode,
            resolved_engine_kind: self.resolved_engine_kind,
            bit_perfect_active: self.bit_perfect_active,
            volume_state: self.volume_state,
            volume_level: self.volume_level,
            volume_muted: self.volume_muted,
            position_ms: self.position_ms,
            duration_ms: self.duration_ms,
            dropout_frames: self.dropout_frames,
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
        let mut settings = self.desired_settings().clone();
        update(&mut settings);
        settings.normalize();
        if settings == *self.desired_settings() {
            return Ok(false);
        }
        let result = self.ensure_settings_writer()?.save(settings.clone());
        if let Err(error) = result {
            self.apply_settings_write_results();
            return Err(error);
        }
        if let Some(writer) = &self.settings_writer {
            let _ = writer.take_results();
        }
        self.settings = settings;
        self.staged_settings = None;
        self.last_settings_error = None;
        Ok(true)
    }

    fn update_settings_in_background(
        &mut self,
        update: impl FnOnce(&mut AppSettings),
    ) -> io::Result<bool> {
        let mut settings = self.desired_settings().clone();
        update(&mut settings);
        settings.normalize();
        let changed = settings != *self.desired_settings();
        let retry = !changed
            && self
                .staged_settings
                .as_ref()
                .is_some_and(|staged| staged.generation.is_none());
        if !changed && !retry {
            return Ok(false);
        }
        self.next_settings_generation = self.next_settings_generation.wrapping_add(1);
        let generation = self.next_settings_generation;
        match self
            .ensure_settings_writer()
            .and_then(|writer| writer.save_in_background(generation, settings.clone()))
        {
            Ok(()) => {
                self.staged_settings = Some(StagedSettings {
                    generation: Some(generation),
                    settings,
                });
                Ok(changed)
            }
            Err(error) => {
                self.staged_settings = Some(StagedSettings {
                    generation: None,
                    settings,
                });
                Err(error)
            }
        }
    }

    fn desired_settings(&self) -> &AppSettings {
        self.staged_settings
            .as_ref()
            .map_or(&self.settings, |staged| &staged.settings)
    }

    fn apply_settings_write_results(&mut self) -> bool {
        let results = self
            .settings_writer
            .as_ref()
            .map(super::settings::SettingsWriter::take_results)
            .unwrap_or_default();
        let changed = !results.is_empty();
        for result in results {
            match result.result {
                Ok(()) => {
                    self.settings = result.settings;
                    if self
                        .staged_settings
                        .as_ref()
                        .is_some_and(|staged| staged.generation == Some(result.generation))
                    {
                        self.staged_settings = None;
                        self.last_settings_error = None;
                    }
                }
                Err(error) => {
                    if let Some(staged) = &mut self.staged_settings
                        && staged.generation == Some(result.generation)
                    {
                        staged.generation = None;
                    }
                    self.record_settings_error(error);
                }
            }
        }
        changed
    }

    fn record_settings_error(&mut self, error: io::Error) {
        let message = format!("Could not save the launch state: {error}");
        if self.last_settings_error.as_deref() != Some(&message) {
            eprintln!("{message}");
        }
        self.last_settings_error = Some(message);
    }

    fn ensure_settings_writer(&mut self) -> io::Result<&super::settings::SettingsWriter> {
        if self.settings_writer.is_none() {
            self.settings_writer = Some(super::settings::SettingsWriter::spawn(
                self.settings_path.clone(),
            )?);
        }
        Ok(self
            .settings_writer
            .as_ref()
            .expect("settings writer was initialized"))
    }

    fn flush_settings_writer(&self) -> io::Result<()> {
        match &self.settings_writer {
            Some(writer) => writer.flush(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
