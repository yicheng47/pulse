use std::{
    cell::Cell,
    collections::HashSet,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc::{Receiver, Sender, TryRecvError},
    time::Duration,
};

use gpui::{
    AnyElement, Bounds, Context, ExternalPaths, FocusHandle, FontWeight, IntoElement, KeyDownEvent,
    ListSizingBehavior, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit,
    Pixels, Render, Window, canvas, div, img, prelude::*, px, relative, svg, uniform_list,
};
use pulse_engine::{
    EngineError, PcmFormat, PlaybackCommand, PlaybackController, PlaybackErrorKind, PlaybackEvent,
    PlaybackState, device,
};

use crate::{
    components,
    library::{Track, TrackId},
    preferences,
    queue::{PreviousAction, QueueState, TrackRef},
    theme,
};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(16);
/// Polls between active-device presence checks while playing (~2 s at 16 ms).
const DEVICE_WATCH_POLLS: u32 = 125;
const MIN_AUDIBLE_GAIN: f32 = 0.001;
const SUPPORTED_EXTENSIONS: &[&str] = &["flac", "m4a", "aif", "aiff", "wav"];

struct PendingDeviceChange {
    device: device::Device,
    persist: bool,
    success_message: Option<DeviceMessage>,
}

#[derive(Clone)]
struct DeviceMessage {
    text: String,
    is_error: bool,
}

/// A visible report shown above the playback row. `Skip` means playback
/// continued past an unplayable queue entry; `Stopped` and `DeviceFailure`
/// mean it stopped.
#[derive(Clone, Debug, Eq, PartialEq)]
enum PlaybackNotice {
    Skip { text: String },
    Stopped { text: String },
    DeviceFailure { text: String },
    UpdateAvailable { version: String, url: String },
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

#[derive(Clone, Copy, Eq, PartialEq)]
enum PlaybackSurface {
    Transport,
    SettingsOutputPicker,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VolumeIconState {
    High,
    Low,
    Muted,
}

impl VolumeIconState {
    fn path(self) -> &'static str {
        match self {
            Self::High => "icons/volume-2.svg",
            Self::Low => "icons/volume-1.svg",
            Self::Muted => "icons/volume-x.svg",
        }
    }
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

pub struct PlaybackRow {
    controller: Option<PlaybackController>,
    command_tx: Option<Sender<PlaybackCommand>>,
    event_rx: Option<Receiver<PlaybackEvent>>,
    playback_state: PlaybackState,
    source_path: Option<PathBuf>,
    cover_art_path: Option<PathBuf>,
    queue: QueueState,
    title: String,
    secondary: String,
    format: Option<PcmFormat>,
    devices: Vec<device::Device>,
    active_device: Option<device::Device>,
    device_capabilities: Option<device::OutputDeviceCapabilities>,
    device_capability_message: Option<DeviceMessage>,
    pending_device_change: Option<PendingDeviceChange>,
    device_message: Option<DeviceMessage>,
    exclusive_mode: bool,
    volume_level: f32,
    volume_muted: bool,
    surface: PlaybackSurface,
    volume_popover_open: bool,
    volume_toggle_press_closed_popover: bool,
    output_popover_open: bool,
    output_toggle_press_closed_popover: bool,
    queue_popover_open: bool,
    queue_toggle_press_closed_popover: bool,
    /// Absolute queue index of the hovered up-next row; drives the row fill
    /// and the ✕ remove affordance.
    hovered_upcoming: Option<usize>,
    /// Present only when built with a window context; tests construct the row
    /// without one.
    volume_popover_focus: Option<FocusHandle>,
    queue_popover_focus: Option<FocusHandle>,
    position_ms: u64,
    duration_ms: Option<u64>,
    error: Option<String>,
    notice: Option<PlaybackNotice>,
    missing_track_ids: HashSet<TrackId>,
    /// PlayFile commands sent to the controller; terminal events stamped with
    /// an older attempt belong to a superseded play and are dropped.
    dispatched_plays: u64,
    current_play: Option<PlayAttempt>,
    retry: Option<RetryTarget>,
    pending_seek_ms: Option<u64>,
    track_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    scrubbing: bool,
    scrub_fraction: Option<f32>,
    volume_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    volume_dragging: bool,
}

impl PlaybackRow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut row = Self::initial();
        row.volume_popover_focus = Some(cx.focus_handle());
        row.queue_popover_focus = Some(cx.focus_handle());
        row.exclusive_mode = preferences::load_exclusive_mode().unwrap_or(true);
        row.volume_level = preferences::load_volume_level().unwrap_or_else(|error| {
            eprintln!("Could not load the volume level preference: {error}");
            1.0
        });
        row.volume_muted = preferences::load_volume_muted().unwrap_or_else(|error| {
            eprintln!("Could not load the volume mute preference: {error}");
            false
        });
        row.initialize_output();

        cx.spawn(async move |this, cx| {
            let mut polls: u32 = 0;
            loop {
                cx.background_executor().timer(EVENT_POLL_INTERVAL).await;
                polls = polls.wrapping_add(1);
                let watch_device = polls.is_multiple_of(DEVICE_WATCH_POLLS);
                let update = this.update(cx, |this, cx| {
                    this.drain_events(cx);
                    if watch_device {
                        this.check_active_device_presence(cx);
                    }
                });
                if update.is_err() {
                    break;
                }
            }
        })
        .detach();

        row
    }

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
            device_message: None,
            exclusive_mode: true,
            volume_level: 1.0,
            volume_muted: false,
            surface: PlaybackSurface::Transport,
            volume_popover_open: false,
            volume_toggle_press_closed_popover: false,
            output_popover_open: false,
            output_toggle_press_closed_popover: false,
            queue_popover_open: false,
            queue_toggle_press_closed_popover: false,
            hovered_upcoming: None,
            volume_popover_focus: None,
            queue_popover_focus: None,
            position_ms: 0,
            duration_ms: None,
            error: None,
            notice: None,
            missing_track_ids: HashSet::new(),
            dispatched_plays: 0,
            current_play: None,
            retry: None,
            pending_seek_ms: None,
            track_bounds: Rc::new(Cell::new(None)),
            scrubbing: false,
            scrub_fraction: None,
            volume_bounds: Rc::new(Cell::new(None)),
            volume_dragging: false,
        }
    }

    fn initialize_output(&mut self) {
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
        let preferred_uid = match preferences::load_output_device_uid() {
            Ok(uid) => uid,
            Err(error) => {
                self.device_message = Some(DeviceMessage {
                    text: format!(
                        "Could not read the saved output device. Using {}: {error}",
                        system_default.name
                    ),
                    is_error: true,
                });
                None
            }
        };
        let (active_device, saved_device_missing) =
            resolve_output_device(&devices, &system_default, preferred_uid.as_deref());

        self.devices = devices;
        self.active_device = Some(active_device.clone());
        if saved_device_missing {
            self.device_message = Some(DeviceMessage {
                text: format!(
                    "Saved output device is unavailable. Using system default: {}.",
                    active_device.name
                ),
                is_error: false,
            });
        }
        self.update_device_capabilities(&active_device);
        self.install_controller(active_device.id);
    }

    fn install_controller(&mut self, device_id: device::DeviceId) {
        let controller = PlaybackController::spawn(device_id, self.exclusive_mode);
        self.event_rx = Some(controller.subscribe());
        let command_tx = controller.command_sender();
        if command_tx.send(self.volume_command()).is_err() {
            self.error = Some("Playback engine disconnected.".to_string());
        }
        self.command_tx = Some(command_tx);
        self.controller = Some(controller);
    }

    fn volume_command(&self) -> PlaybackCommand {
        PlaybackCommand::SetVolume {
            gain: volume_gain_for_level(self.volume_level),
            muted: self.volume_muted,
        }
    }

    fn toggle_volume_mute(&mut self, cx: &mut Context<Self>) {
        let muted = !self.volume_muted;
        if let Err(error) = preferences::save_volume_muted(muted) {
            self.error = Some(format!(
                "Could not save the volume mute preference: {error}"
            ));
            cx.notify();
            return;
        }
        self.volume_muted = muted;
        self.send_command(self.volume_command(), cx);
        cx.notify();
    }

    fn set_volume_level(&mut self, level: f32, cx: &mut Context<Self>) {
        let level = level.clamp(0.0, 1.0);
        if self.volume_level == level && !self.volume_muted {
            return;
        }
        self.volume_level = level;
        self.volume_muted = false;
        self.send_command(self.volume_command(), cx);
        cx.notify();
    }

    fn persist_volume(&mut self, cx: &mut Context<Self>) {
        let result = preferences::save_volume_level(self.volume_level)
            .and_then(|()| preferences::save_volume_muted(self.volume_muted));
        if let Err(error) = result {
            self.error = Some(format!("Could not save the volume preference: {error}"));
            cx.notify();
        }
    }

    fn update_device_capabilities(&mut self, output_device: &device::Device) {
        self.apply_device_capabilities_result(
            output_device,
            device::output_device_capabilities(output_device.id),
        );
    }

    fn apply_device_capabilities_result(
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

    fn displayed_device_message(&self) -> Option<DeviceMessage> {
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

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn active_output_device(&self) -> Option<&device::Device> {
        self.active_device.as_ref()
    }

    pub(crate) fn exclusive_mode(&self) -> bool {
        self.exclusive_mode
    }

    pub(crate) fn output_popover_open(&self) -> bool {
        self.output_popover_open
    }

    pub(crate) fn enter_settings(&mut self, cx: &mut Context<Self>) {
        self.surface = PlaybackSurface::SettingsOutputPicker;
        self.volume_popover_open = false;
        self.output_popover_open = false;
        self.queue_popover_open = false;
        cx.notify();
    }

    pub(crate) fn leave_settings(&mut self, cx: &mut Context<Self>) {
        self.surface = PlaybackSurface::Transport;
        self.volume_popover_open = false;
        self.output_popover_open = false;
        cx.notify();
    }

    pub(crate) fn close_output_popover(&mut self, cx: &mut Context<Self>) {
        if self.output_popover_open {
            self.output_popover_open = false;
            cx.notify();
        }
    }

    pub(crate) fn toggle_settings_output_popover(&mut self, cx: &mut Context<Self>) {
        self.toggle_output_popover(cx);
    }

    pub(crate) fn toggle_exclusive_mode(&mut self, cx: &mut Context<Self>) {
        let enabled = !self.exclusive_mode;
        if let Err(error) = preferences::save_exclusive_mode(enabled) {
            self.device_message = Some(DeviceMessage {
                text: format!("Could not save the exclusive-mode preference: {error}"),
                is_error: true,
            });
            cx.notify();
            return;
        }
        self.exclusive_mode = enabled;
        self.send_command(PlaybackCommand::SetExclusiveMode { enabled }, cx);
        cx.notify();
    }

    /// Library rows marked unplayable because their file was gone at play
    /// time. Marks are runtime-only; when rows are deleted (root removal,
    /// Delete Album), their marks must be dropped too — track ids are
    /// recyclable INTEGER PRIMARY KEYs.
    pub(crate) fn is_track_missing(&self, track_id: TrackId) -> bool {
        self.missing_track_ids.contains(&track_id)
    }

    /// Marks go stale whenever library rows change under them — a completed
    /// scan re-verifies file presence, and a removed root recycles track ids.
    pub(crate) fn clear_missing_marks(&mut self) {
        self.missing_track_ids.clear();
    }

    /// Rows for these tracks were deleted; drop their marks so a future row
    /// recycling one of the ids cannot inherit a stale missing flag.
    pub(crate) fn remove_missing_marks(&mut self, track_ids: &[TrackId]) {
        for track_id in track_ids {
            self.missing_track_ids.remove(track_id);
        }
    }

    pub(crate) fn show_update_available(
        &mut self,
        version: String,
        url: String,
        cx: &mut Context<Self>,
    ) {
        if self.notice.is_none() {
            self.notice = Some(PlaybackNotice::UpdateAvailable { version, url });
            cx.notify();
        }
    }

    fn dismiss_notice(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        self.retry = None;
        cx.notify();
    }

    pub(crate) fn has_track(&self) -> bool {
        self.source_path.is_some()
    }

    pub(crate) fn is_now_playing(&self, path: &Path) -> bool {
        self.source_path.as_deref() == Some(path)
            && matches!(
                self.playback_state,
                PlaybackState::Loading | PlaybackState::Playing | PlaybackState::Paused
            )
    }

    pub(crate) fn play_library_tracks(
        &mut self,
        tracks: &[Track],
        start_index: usize,
        cx: &mut Context<Self>,
    ) {
        self.queue = QueueState::from_tracks(tracks, start_index);
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
        cx: &mut Context<Self>,
    ) {
        let queue = QueueState::from_tracks(tracks, start_index);
        let Some(track) = queue.current().cloned() else {
            return;
        };
        if self.apply_track_selection(&track) {
            self.queue = queue;
            cx.notify();
        }
    }

    pub(crate) fn handle_drop(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        if paths.paths().len() != 1 {
            self.error = Some("Drop one audio file at a time.".to_string());
            cx.notify();
            return;
        }

        let path = &paths.paths()[0];
        if !path.is_file() {
            self.error = Some("Drop a file, not a folder.".to_string());
            cx.notify();
            return;
        }
        if !is_supported_audio(path) {
            self.error = Some(
                "Unsupported file. Pulse accepts FLAC, ALAC (.m4a), AIFF, and WAV.".to_string(),
            );
            cx.notify();
            return;
        }

        self.error = None;
        self.notice = None;
        self.retry = None;
        self.pending_seek_ms = None;
        self.cover_art_path = None;
        self.queue = QueueState::default();
        self.play_file(path.clone(), cx);
    }

    fn play_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.controller.is_none() {
            self.initialize_output();
            if self.controller.is_none() {
                self.error = Some("No output device is available.".to_string());
                cx.notify();
                return;
            }
        }

        self.record_play_attempt(&path);
        self.send_command(PlaybackCommand::PlayFile { path }, cx);
    }

    /// Tracks what the controller is actually working on, so a device-failure
    /// retry replays the attempted file — `source_path`/`position_ms` still
    /// describe the previous track while a new one is loading.
    fn record_play_attempt(&mut self, path: &Path) {
        self.current_play = Some(PlayAttempt {
            target: RetryTarget {
                path: path.to_path_buf(),
                position_ms: self.pending_seek_ms.unwrap_or(0),
            },
            confirmed: false,
        });
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        if let Some(command) = self.prepare_toggle_command() {
            self.send_command(command, cx);
        }
    }

    /// Transport play from Idle/Ended/Error dispatches a `PlayFile` restart,
    /// which must be recorded as a fresh attempt like every other dispatch.
    fn prepare_toggle_command(&mut self) -> Option<PlaybackCommand> {
        let command = self.toggle_command()?;
        if let PlaybackCommand::PlayFile { path } = &command {
            self.pending_seek_ms = None;
            let path = path.clone();
            self.record_play_attempt(&path);
        }
        Some(command)
    }

    fn next_track(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        self.retry = None;
        if let Some(track) = self.queue.advance() {
            self.play_queue_track(track, cx);
        }
    }

    fn previous_track(&mut self, cx: &mut Context<Self>) {
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

    fn toggle_command(&self) -> Option<PlaybackCommand> {
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
    fn restart_path(&self) -> Option<PathBuf> {
        if self.playback_state == PlaybackState::Error
            && let Some(attempt) = &self.current_play
        {
            return Some(attempt.target.path.clone());
        }
        self.source_path.clone()
    }

    fn apply_track_selection(&mut self, track: &TrackRef) -> bool {
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

    fn apply_track_context(&mut self, track: &TrackRef) {
        self.title = track.title.clone();
        self.secondary = track.secondary();
        self.source_path = Some(track.path.clone());
        self.cover_art_path = track.cover_art_path.clone();
    }

    fn play_queue_track(&mut self, track: TrackRef, cx: &mut Context<Self>) {
        match self.prepare_queue_play(track) {
            Some(path) => self.play_file(path, cx),
            // The queue exhausted through unplayable entries. A user-initiated
            // jump or Next can land here while the superseded track is still
            // audible; stop it so the Stopped notice is truthful.
            None => {
                if self.active_playback_needs_stop() {
                    self.send_command(PlaybackCommand::Stop, cx);
                }
                cx.notify();
            }
        }
    }

    /// True when the engine is still producing (or about to produce) audio
    /// for a track the queue has already moved past.
    fn active_playback_needs_stop(&self) -> bool {
        matches!(
            self.playback_state,
            PlaybackState::Loading | PlaybackState::Playing | PlaybackState::Paused
        )
    }

    /// Cx-free half of a queue dispatch: resolves the skip-and-report path
    /// for missing files, applies the display context, and returns the path
    /// to hand to `play_file`.
    fn prepare_queue_play(&mut self, track: TrackRef) -> Option<PathBuf> {
        self.pending_seek_ms = None;
        let track = self.next_playable(track)?;
        self.apply_track_context(&track);
        self.error = None;
        Some(track.path)
    }

    /// Queue-popover row click: the index moves to the clicked entry and it
    /// plays; nothing is dropped. A jump to a missing entry follows the
    /// existing skip-and-report path inside `play_queue_track`.
    fn jump_to_queue_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        self.notice = None;
        self.retry = None;
        if let Some(track) = self.queue.jump_to(index) {
            self.play_queue_track(track, cx);
        }
        cx.notify();
    }

    fn remove_queue_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        self.queue.remove_at(index);
        self.hovered_upcoming = None;
        cx.notify();
    }

    fn clear_upcoming_queue(&mut self, cx: &mut Context<Self>) {
        self.queue.clear_upcoming();
        self.hovered_upcoming = None;
        cx.notify();
    }

    fn toggle_queue_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.queue_popover_open = !self.queue_popover_open;
        if self.queue_popover_open {
            self.hovered_upcoming = None;
            if let Some(focus) = &self.queue_popover_focus {
                window.focus(focus, cx);
            }
        }
        cx.notify();
    }

    fn toggle_volume_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.volume_popover_open = !self.volume_popover_open;
        if self.volume_popover_open
            && let Some(focus) = &self.volume_popover_focus
        {
            window.focus(focus, cx);
        }
        cx.notify();
    }

    /// Cheap existence check at play time: marks and skips entries whose file
    /// is gone, and reports when that empties the rest of the queue.
    fn next_playable(&mut self, first: TrackRef) -> Option<TrackRef> {
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
    fn handle_track_failure(&mut self) -> Option<TrackRef> {
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

    fn note_queue_stopped(&mut self, last: &TrackRef) {
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

    fn handle_device_failure(&mut self, message: &str, hog_pid: Option<i32>) {
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

    fn retry_playback(&mut self, cx: &mut Context<Self>) {
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
        cx.notify();
    }

    fn send_command(&mut self, command: PlaybackCommand, cx: &mut Context<Self>) {
        let Some(command_tx) = &self.command_tx else {
            return;
        };
        let is_play = matches!(command, PlaybackCommand::PlayFile { .. });
        if command_tx.send(command).is_err() {
            self.error = Some("Playback engine disconnected.".to_string());
            cx.notify();
        } else if is_play {
            self.dispatched_plays += 1;
        }
    }

    fn drain_events(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
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
            if let Some(track) = self.handle_event(event) {
                self.play_queue_track(track, cx);
            }
            if self.playback_state == PlaybackState::Playing
                && let Some(position_ms) = self.pending_seek_ms.take()
            {
                self.send_command(PlaybackCommand::Seek { position_ms }, cx);
            }
            changed = true;
        }

        if changed {
            cx.notify();
        }
    }

    /// Applies one controller event; returns the next queue entry to play
    /// when the event calls for an advance (track ended or failed).
    fn handle_event(&mut self, event: PlaybackEvent) -> Option<TrackRef> {
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
            PlaybackEvent::OutputDeviceChanged { device_id } => {
                self.complete_output_device_change(device_id);
            }
            PlaybackEvent::Ended { attempt } => {
                if attempt != self.dispatched_plays {
                    return None;
                }
                self.playback_state = PlaybackState::Ended;
                if let Some(duration_ms) = self.duration_ms {
                    self.position_ms = duration_ms;
                }
                return self.queue.advance();
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

    fn complete_output_device_change(&mut self, device_id: device::DeviceId) {
        let Some(pending) = self.pending_device_change.take() else {
            return;
        };
        if pending.device.id != device_id {
            return;
        }

        let persist = pending.persist;
        let output_device = self.apply_completed_output_device_change(pending);
        self.update_device_capabilities(&output_device);

        if persist && let Err(error) = preferences::save_output_device_uid(&output_device.uid) {
            self.device_message = Some(DeviceMessage {
                text: format!(
                    "Could not save {} as the output device: {error}",
                    output_device.name
                ),
                is_error: true,
            });
        }
    }

    fn apply_completed_output_device_change(
        &mut self,
        pending: PendingDeviceChange,
    ) -> device::Device {
        let output_device = pending.device;
        self.active_device = Some(output_device.clone());
        self.device_message = pending.success_message;
        output_device
    }

    fn check_active_device_presence(&mut self, cx: &mut Context<Self>) {
        if !matches!(
            self.playback_state,
            PlaybackState::Playing | PlaybackState::Loading
        ) {
            return;
        }
        let Ok(devices) = device::list_output_devices() else {
            return;
        };
        if self.note_device_loss(devices) {
            self.send_command(PlaybackCommand::Stop, cx);
            cx.notify();
        }
    }

    /// Returns true when the active output device is no longer attached; the
    /// engine has no mid-playback device-loss signal, so the row watches for
    /// it while playing.
    fn note_device_loss(&mut self, devices: Vec<device::Device>) -> bool {
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

    fn toggle_output_popover(&mut self, cx: &mut Context<Self>) {
        self.output_popover_open = !self.output_popover_open;
        if self.output_popover_open {
            self.refresh_output_devices(cx);
        }
        cx.notify();
    }

    fn refresh_output_devices(&mut self, cx: &mut Context<Self>) {
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

        let Some(active_device) = self.active_device.clone() else {
            self.initialize_output();
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

    fn select_output_device(&mut self, output_device: device::Device, cx: &mut Context<Self>) {
        self.request_output_device_change(output_device, true, None, cx);
    }

    fn request_output_device_change(
        &mut self,
        output_device: device::Device,
        persist: bool,
        success_message: Option<DeviceMessage>,
        cx: &mut Context<Self>,
    ) {
        if persist {
            self.output_popover_open = false;
            self.notice = None;
            self.retry = None;
        }
        self.error = None;
        self.device_message = None;
        self.device_capability_message = None;
        self.pending_device_change = Some(PendingDeviceChange {
            device: output_device.clone(),
            persist,
            success_message,
        });

        let Some(command_tx) = &self.command_tx else {
            self.install_controller(output_device.id);
            self.complete_output_device_change(output_device.id);
            cx.notify();
            return;
        };
        if command_tx
            .send(PlaybackCommand::SetOutputDevice {
                device_id: output_device.id,
            })
            .is_err()
        {
            self.pending_device_change = None;
            self.device_message = Some(DeviceMessage {
                text: "Playback engine disconnected while changing output devices.".to_string(),
                is_error: true,
            });
        }
        cx.notify();
    }

    fn begin_scrub(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if !matches!(
            self.playback_state,
            PlaybackState::Playing | PlaybackState::Paused
        ) || self.duration_ms.is_none_or(|duration_ms| duration_ms == 0)
        {
            return;
        }
        let Some(bounds) = self.track_bounds.get() else {
            return;
        };
        self.scrubbing = true;
        self.scrub_fraction = Some(fraction_at_x(bounds, event.position.x));
        cx.notify();
    }

    fn begin_volume_drag(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(bounds) = self.volume_bounds.get() else {
            return;
        };
        self.volume_dragging = true;
        self.set_volume_level(fraction_at_y(bounds, event.position.y), cx);
    }

    pub(crate) fn update_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            let was_dragging = self.scrubbing || self.volume_dragging;
            self.scrubbing = false;
            self.scrub_fraction = None;
            if self.volume_dragging {
                self.volume_dragging = false;
                self.persist_volume(cx);
            }
            if was_dragging {
                cx.notify();
            }
            return;
        }

        if self.scrubbing
            && let Some(bounds) = self.track_bounds.get()
        {
            self.scrub_fraction = Some(fraction_at_x(bounds, event.position.x));
            cx.notify();
        }
        if self.volume_dragging
            && let Some(bounds) = self.volume_bounds.get()
        {
            self.set_volume_level(fraction_at_y(bounds, event.position.y), cx);
        }
    }

    pub(crate) fn finish_drag(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if self.scrubbing {
            self.scrubbing = false;
            let fraction = self
                .track_bounds
                .get()
                .map(|bounds| fraction_at_x(bounds, event.position.x))
                .or(self.scrub_fraction);
            self.scrub_fraction = None;

            if let (Some(fraction), Some(duration_ms)) = (fraction, self.duration_ms) {
                let position_ms = scrub_position_ms(fraction, duration_ms);
                self.position_ms = position_ms;
                self.send_command(PlaybackCommand::Seek { position_ms }, cx);
            }
            cx.notify();
        }

        if self.volume_dragging {
            self.volume_dragging = false;
            if let Some(bounds) = self.volume_bounds.get() {
                self.set_volume_level(fraction_at_y(bounds, event.position.y), cx);
            }
            self.persist_volume(cx);
            cx.notify();
        }
    }

    fn displayed_fraction(&self) -> f32 {
        self.scrub_fraction.unwrap_or_else(|| {
            self.duration_ms
                .filter(|duration| *duration > 0)
                .map(|duration| (self.position_ms as f64 / duration as f64) as f32)
                .unwrap_or(0.0)
                .clamp(0.0, 1.0)
        })
    }

    fn displayed_position_ms(&self) -> u64 {
        match (self.scrub_fraction, self.duration_ms) {
            (Some(fraction), Some(duration_ms)) => scrub_position_ms(fraction, duration_ms),
            _ => self.position_ms,
        }
    }

    fn render_now_playing(&self) -> impl IntoElement {
        let cover = match &self.cover_art_path {
            Some(path) => img(path.clone())
                .size_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            None => svg()
                .path("icons/list-music.svg")
                .size(px(22.))
                .text_color(theme::text_muted())
                .into_any_element(),
        };
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .w(px(330.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(52.))
                    .flex_none()
                    .overflow_hidden()
                    .rounded(px(theme::RADIUS_SM))
                    .border_1()
                    .border_color(theme::border_strong())
                    .bg(theme::bg_elevated())
                    .child(cover),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .flex_col()
                    .min_w_0()
                    .gap(px(4.))
                    .child(
                        div()
                            .w_full()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(15.))
                            .text_color(theme::text_primary())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(12.))
                            .text_color(theme::text_secondary())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(self.secondary.clone()),
                    ),
            )
    }

    fn render_transport(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let progress = self.displayed_fraction();
        let track_bounds = Rc::clone(&self.track_bounds);
        let previous_enabled = self.source_path.is_some()
            && !matches!(
                self.playback_state,
                PlaybackState::Loading | PlaybackState::Stopping
            );
        let next_enabled = self.queue.can_advance()
            && !matches!(
                self.playback_state,
                PlaybackState::Loading | PlaybackState::Stopping
            );
        let play_icon = if self.playback_state == PlaybackState::Playing {
            "icons/pause.svg"
        } else {
            "icons/play.svg"
        };

        div()
            .flex()
            .flex_1()
            .min_w_0()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(12.))
                    .child(transport_icon("icons/shuffle.svg"))
                    .child(
                        div()
                            .id("playback-previous")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(17.))
                            .opacity(if previous_enabled { 1.0 } else { 0.35 })
                            .when(previous_enabled, |button| {
                                button
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.previous_track(cx);
                                    }))
                            })
                            .child(
                                svg()
                                    .path("icons/skip-back.svg")
                                    .size(px(17.))
                                    .text_color(theme::text_secondary()),
                            ),
                    )
                    .child(
                        div()
                            .id("playback-toggle")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.))
                            .rounded(px(theme::RADIUS_MD))
                            .bg(theme::accent())
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_playback(cx)))
                            .child(
                                svg()
                                    .path(play_icon)
                                    .size(px(16.))
                                    .text_color(theme::bg_inset()),
                            ),
                    )
                    .child(
                        div()
                            .id("playback-next")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(17.))
                            .opacity(if next_enabled { 1.0 } else { 0.35 })
                            .when(next_enabled, |button| {
                                button
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| this.next_track(cx)))
                            })
                            .child(
                                svg()
                                    .path("icons/skip-forward.svg")
                                    .size(px(17.))
                                    .text_color(theme::text_secondary()),
                            ),
                    )
                    .child(transport_icon("icons/repeat-2.svg")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .w_full()
                    .child(time_label(format_time(self.displayed_position_ms())))
                    .child(
                        div()
                            .id("progress-target")
                            .flex_1()
                            .flex()
                            .items_center()
                            .h(px(16.))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, event, _, cx| this.begin_scrub(event, cx)),
                            )
                            .child(
                                div()
                                    .relative()
                                    .w_full()
                                    .h(px(4.))
                                    .rounded(px(2.))
                                    .bg(theme::bg_inset())
                                    .child(
                                        div()
                                            .absolute()
                                            .left_0()
                                            .top_0()
                                            .bottom_0()
                                            .w(relative(progress))
                                            .rounded(px(2.))
                                            .bg(theme::accent()),
                                    )
                                    .child(
                                        canvas(
                                            move |bounds, _, _| track_bounds.set(Some(bounds)),
                                            |_, _, _, _| {},
                                        )
                                        .absolute()
                                        .top_0()
                                        .right_0()
                                        .bottom_0()
                                        .left_0(),
                                    ),
                            ),
                    )
                    .child(time_label(
                        self.duration_ms
                            .map(format_time)
                            .unwrap_or_else(|| "--:--".to_string()),
                    )),
            )
    }

    fn render_output(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let remaining = self.queue.remaining_count();
        let volume_icon = volume_icon_state(self.volume_level, self.volume_muted);
        let (quality, quality_color) = self
            .format
            .map(|format| {
                (
                    format_quality(self.source_path.as_deref(), format),
                    theme::quality(),
                )
            })
            .unwrap_or_else(|| ("—".to_string(), theme::text_muted()));
        let device = match (self.format, &self.active_device) {
            (Some(format), Some(device)) => {
                format!(
                    "{} · {}",
                    format_sample_rate(format.sample_rate),
                    device.name
                )
            }
            (_, Some(device)) => device.name.clone(),
            (_, None) => "No output selected".to_string(),
        };
        let mut output_details = div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .w(px(132.))
            .child(
                div()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(12.))
                    .text_color(quality_color)
                    .whitespace_nowrap()
                    .child(quality),
            )
            .child(
                div()
                    .w_full()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_secondary())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(device),
            );
        if let Some(message) = self.displayed_device_message() {
            output_details = output_details.child(
                div()
                    .w_full()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(10.))
                    .text_color(if message.is_error {
                        theme::danger()
                    } else {
                        theme::warning()
                    })
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(message.text),
            );
        }

        let mut volume = div()
            .id("volume-toggle")
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(px(17.))
            .flex_none()
            .cursor_pointer()
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Left {
                    this.volume_toggle_press_closed_popover = this.volume_popover_open;
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| {
                if std::mem::take(&mut this.volume_toggle_press_closed_popover) {
                    cx.notify();
                    return;
                }
                this.toggle_volume_popover(window, cx);
            }))
            .child(svg().path(volume_icon.path()).size(px(17.)).text_color(
                if volume_icon == VolumeIconState::Muted {
                    theme::text_muted()
                } else if self.volume_popover_open {
                    theme::accent()
                } else {
                    theme::text_secondary()
                },
            ));
        if self.volume_popover_open {
            volume = volume.child(self.render_volume_popover(cx));
        }

        let mut speaker = div()
            .id("output-device-toggle")
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(px(17.))
            .flex_none()
            .cursor_pointer()
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Left {
                    this.output_toggle_press_closed_popover = this.output_popover_open;
                }
            }))
            .on_click(cx.listener(|this, _, _, cx| {
                if std::mem::take(&mut this.output_toggle_press_closed_popover) {
                    cx.notify();
                    return;
                }
                this.toggle_output_popover(cx);
            }))
            .child(
                svg().path("icons/speaker.svg").size(px(17.)).text_color(
                    if self
                        .displayed_device_message()
                        .is_some_and(|message| message.is_error)
                    {
                        theme::danger()
                    } else if self.output_popover_open {
                        theme::accent()
                    } else {
                        theme::text_secondary()
                    },
                ),
            );
        if self.output_popover_open {
            speaker = speaker.child(self.render_output_popover(cx));
        }

        let mut queue_button = div()
            .id("queue-toggle")
            .relative()
            .w(px(38.))
            .h(px(34.))
            .cursor_pointer()
            // Same press-closed guard as the output picker and artist filter:
            // clicking the open trigger closes rather than closes-then-reopens.
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Left {
                    this.queue_toggle_press_closed_popover = this.queue_popover_open;
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| {
                if std::mem::take(&mut this.queue_toggle_press_closed_popover) {
                    cx.notify();
                    return;
                }
                this.toggle_queue_popover(window, cx);
            }))
            .child(
                svg()
                    .path("icons/list-music.svg")
                    .absolute()
                    .left_0()
                    .top(px(8.))
                    .size(px(17.))
                    .text_color(if self.queue_popover_open {
                        theme::accent()
                    } else {
                        theme::text_secondary()
                    }),
            );
        if self.queue_popover_open {
            queue_button = queue_button.child(self.render_queue_popover(cx));
        }

        div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(14.))
            .w(px(300.))
            .child(output_details)
            .child(div().w(px(1.)).h(px(24.)).flex_none().bg(theme::border()))
            .child(volume)
            .child(div().w(px(1.)).h(px(24.)).flex_none().bg(theme::border()))
            .child(speaker)
            .child(div().w(px(1.)).h(px(24.)).flex_none().bg(theme::border()))
            .child(queue_button.when(remaining > 0, |button| {
                button.child(
                    div()
                        .absolute()
                        .top(px(-2.))
                        .right(px(-2.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px(20.))
                        .min_w(px(20.))
                        .px(px(2.))
                        .rounded(px(10.))
                        .bg(theme::bg_surface())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .h(px(16.))
                                .min_w(px(16.))
                                .px(px(3.))
                                .rounded(px(8.))
                                .bg(theme::accent())
                                .font_family(theme::FONT_MONO)
                                .font_weight(FontWeight::BOLD)
                                .text_size(px(10.))
                                .text_color(theme::bg_inset())
                                .child(remaining.to_string()),
                        ),
                )
            }))
    }

    fn render_volume_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let volume_bounds = Rc::clone(&self.volume_bounds);
        let volume_fill = displayed_volume_level(self.volume_level, self.volume_muted);
        let volume_dragging = self.volume_dragging;
        let popover = div()
            .id("volume-popover")
            .absolute()
            .left(px(-19.5))
            .bottom(px(54.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(11.))
            .w(px(56.))
            .p(px(14.))
            .rounded(px(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_surface())
            .occlude()
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.update_drag(event, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.finish_drag(event, cx);
                }),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.volume_popover_open = false;
                cx.notify();
            }))
            .child(
                div()
                    .w_full()
                    .flex_none()
                    .font_family(theme::FONT_MONO)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(11.))
                    .text_center()
                    .text_color(theme::text_secondary())
                    .whitespace_nowrap()
                    .child(format_volume_percent(self.volume_level)),
            )
            .child(
                div()
                    .id("volume-slider-target")
                    .group("volume-slider")
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w_full()
                    .h(px(120.))
                    .flex_none()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event, _, cx| this.begin_volume_drag(event, cx)),
                    )
                    .child(
                        div()
                            .relative()
                            .w(px(4.))
                            .h_full()
                            .rounded(px(2.))
                            .bg(theme::bg_inset())
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .bottom_0()
                                    .h(relative(volume_fill))
                                    .rounded(px(2.))
                                    .bg(theme::accent())
                                    .child(
                                        div()
                                            .absolute()
                                            .top(px(-6.))
                                            .left(px(-4.))
                                            .size(px(12.))
                                            .rounded(px(6.))
                                            .bg(theme::accent())
                                            .opacity(if volume_dragging { 1.0 } else { 0.0 })
                                            .when(!volume_dragging, |thumb| {
                                                thumb.group_hover("volume-slider", |style| {
                                                    style.opacity(1.0)
                                                })
                                            }),
                                    ),
                            ),
                    )
                    .child(
                        canvas(
                            move |bounds, _, _| volume_bounds.set(Some(bounds)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0(),
                    ),
            )
            .child(
                div()
                    .id("volume-mute-toggle")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(17.))
                    .flex_none()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_volume_mute(cx)))
                    .child(
                        svg()
                            .path("icons/volume-x.svg")
                            .size(px(17.))
                            .text_color(theme::text_secondary()),
                    ),
            );

        match &self.volume_popover_focus {
            Some(focus) => popover
                .track_focus(focus)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        this.volume_popover_open = false;
                        cx.notify();
                    }
                }))
                .into_any_element(),
            None => popover.into_any_element(),
        }
    }

    fn render_output_popover(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_name = self
            .active_device
            .as_ref()
            .map(|device| device.name.clone())
            .unwrap_or_else(|| "No active output".to_string());
        let capability = self
            .device_capabilities
            .map(format_device_capabilities)
            .unwrap_or_else(|| "Capabilities unavailable".to_string());
        let mut direct_devices = div().flex().flex_col().gap(px(2.)).w_full();
        for (index, output_device) in self.devices.iter().cloned().enumerate() {
            direct_devices =
                direct_devices.child(self.render_output_device_row(output_device, index, cx));
        }
        if self.devices.is_empty() {
            direct_devices = direct_devices.child(
                div()
                    .px(px(10.))
                    .py(px(9.))
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child("No direct output devices found"),
            );
        }

        let mut popover = div()
            .id("output-device-popover")
            .absolute()
            .when(
                self.surface == PlaybackSurface::SettingsOutputPicker,
                |popover| popover.right_0().top(px(30.)),
            )
            .when(self.surface == PlaybackSurface::Transport, |popover| {
                popover.right(px(-52.)).bottom(px(54.))
            })
            .flex()
            .flex_col()
            .gap(px(11.))
            .w(px(360.))
            .p(px(14.))
            .rounded(px(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_surface())
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.output_popover_open = false;
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(17.))
                            .text_color(theme::text_primary())
                            .child("Choose audio output"),
                    )
                    .child(
                        svg()
                            .path("icons/settings.svg")
                            .size(px(16.))
                            .text_color(theme::text_secondary()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .w_full()
                    .p(px(12.))
                    .rounded(px(theme::RADIUS_MD))
                    .border_1()
                    .border_color(theme::accent())
                    .bg(theme::bg_inset())
                    .child(
                        svg()
                            .path("icons/speaker.svg")
                            .size(px(22.))
                            .flex_none()
                            .text_color(theme::accent()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .w_full()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(17.))
                                    .text_color(theme::text_primary())
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(active_name),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(px(12.))
                                    .text_color(theme::text_secondary())
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(if self.exclusive_mode {
                                        "CoreAudio · Exclusive during playback"
                                    } else {
                                        "CoreAudio · Shared playback"
                                    }),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .font_family(theme::FONT_MONO)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(11.))
                                    .text_color(if self.device_capabilities.is_some() {
                                        theme::quality()
                                    } else {
                                        theme::warning()
                                    })
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(capability),
                            ),
                    )
                    .when(self.active_device.is_some(), |card| {
                        card.child(
                            svg()
                                .path("icons/check.svg")
                                .size(px(18.))
                                .flex_none()
                                .text_color(theme::accent()),
                        )
                    }),
            );

        if let Some(message) = self.displayed_device_message() {
            popover = popover.child(
                div()
                    .w_full()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(11.))
                    .text_color(if message.is_error {
                        theme::danger()
                    } else {
                        theme::warning()
                    })
                    .child(message.text),
            );
        }

        popover
            .child(section_label("DIRECT DEVICES"))
            .child(direct_devices)
            .child(section_label("NETWORK DEVICES"))
            .child(
                div()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child("No network devices found"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .pt(px(12.))
                    .pr(px(2.))
                    .pb(px(2.))
                    .pl(px(2.))
                    .border_t_1()
                    .border_color(theme::border())
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(14.))
                            .text_color(theme::text_secondary())
                            .child("Can't find your device?"),
                    )
                    .child(
                        svg()
                            .path("icons/log-in.svg")
                            .size(px(16.))
                            .text_color(theme::text_muted()),
                    ),
            )
    }

    fn render_output_device_row(
        &self,
        output_device: device::Device,
        index: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self
            .active_device
            .as_ref()
            .is_some_and(|active| active.uid == output_device.uid);
        let selected_device = output_device.clone();

        div()
            .id(("output-device", index))
            .flex()
            .items_center()
            .gap(px(12.))
            .w_full()
            .px(px(10.))
            .py(px(9.))
            .rounded(px(theme::RADIUS_MD))
            .when(selected, |row| {
                row.border_1()
                    .border_color(theme::accent())
                    .bg(theme::accent_soft())
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_output_device(selected_device.clone(), cx);
            }))
            .child(
                svg()
                    .path("icons/speaker.svg")
                    .size(px(18.))
                    .flex_none()
                    .text_color(if selected {
                        theme::accent()
                    } else {
                        theme::text_muted()
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(theme::FONT_DISPLAY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.))
                    .text_color(if selected {
                        theme::text_primary()
                    } else {
                        theme::text_secondary()
                    })
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(output_device.name),
            )
            .when(selected, |row| {
                row.child(
                    svg()
                        .path("icons/check.svg")
                        .size(px(16.))
                        .flex_none()
                        .text_color(theme::accent()),
                )
            })
    }

    fn render_queue_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let upcoming_count = self.queue.remaining_count();

        let mut header = div().flex().items_center().gap(px(10.)).w_full().child(
            div()
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::BOLD)
                .text_size(px(17.))
                .text_color(theme::text_primary())
                .child("Queue"),
        );
        if upcoming_count > 0 {
            header = header
                .child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(10.))
                        .text_color(theme::text_muted())
                        .child(format_queue_meta(
                            upcoming_count,
                            self.queue.upcoming_duration_ms(),
                        )),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("queue-clear")
                        .flex()
                        .items_center()
                        .h(px(23.))
                        .px(px(8.))
                        .flex_none()
                        .rounded(px(theme::RADIUS_SM))
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::bg_muted())
                        .cursor_pointer()
                        .font_family(theme::FONT_DISPLAY)
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(12.))
                        .text_color(theme::text_secondary())
                        .child("Clear")
                        .on_click(cx.listener(|this, _, _, cx| this.clear_upcoming_queue(cx))),
                );
        }

        let mut popover = div()
            .id("queue-popover")
            .absolute()
            .right_0()
            .bottom(px(71.))
            .flex()
            .flex_col()
            .gap(px(11.))
            .w(px(376.))
            .max_h(px(541.))
            .p(px(14.))
            .rounded(px(theme::RADIUS_LG))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_surface())
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.queue_popover_open = false;
                cx.notify();
            }))
            .child(header);

        if let Some((title, secondary)) = self.now_playing_lines() {
            popover = popover.child(section_label("NOW PLAYING")).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .w_full()
                    .h(px(58.))
                    .flex_none()
                    .px(px(10.))
                    .relative()
                    .overflow_hidden()
                    .rounded(px(theme::RADIUS_MD))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg_inset())
                    .child(components::playing_row_glow())
                    .child(components::playing_row_bar())
                    .child(
                        svg()
                            .path("icons/audio-lines.svg")
                            .size(px(16.))
                            .flex_none()
                            .text_color(theme::accent()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(theme::FONT_DISPLAY)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(14.))
                                    .text_color(theme::text_primary())
                                    .child(title),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(theme::FONT_SANS)
                                    .text_size(px(11.))
                                    .text_color(theme::text_secondary())
                                    .child(secondary),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme::FONT_MONO)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(10.))
                            .text_color(theme::quality())
                            .child(format!(
                                "{} / {}",
                                format_queue_time(self.displayed_position_ms()),
                                self.duration_ms
                                    .map(format_queue_time)
                                    .unwrap_or_else(|| "--:--".to_string())
                            )),
                    ),
            );
        }

        popover = popover.child(section_label("UP NEXT"));
        if upcoming_count == 0 {
            popover = popover.child(
                div()
                    .px(px(10.))
                    .py(px(6.))
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child("Nothing up next"),
            );
        } else {
            // Virtualized: only the visible rows are built, so a full-library
            // queue stays cheap through the 100 ms position ticks.
            popover = popover.child(
                uniform_list(
                    "queue-upcoming-list",
                    upcoming_count,
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        let visible = this
                            .queue
                            .upcoming()
                            .skip(range.start)
                            .take(range.len())
                            .map(|(position, track)| (position, track.clone()))
                            .collect::<Vec<_>>();
                        visible
                            .into_iter()
                            .map(|(position, track)| this.render_upcoming_row(position, track, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .with_sizing_behavior(ListSizingBehavior::Infer)
                .min_h_0(),
            );
        }

        match &self.queue_popover_focus {
            Some(focus) => popover
                .track_focus(focus)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        this.queue_popover_open = false;
                        cx.notify();
                    }
                }))
                .into_any_element(),
            None => popover.into_any_element(),
        }
    }

    /// Title and `artist · album` for the popover's NOW PLAYING block, absent
    /// when nothing is playing.
    fn now_playing_lines(&self) -> Option<(String, String)> {
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

    fn render_upcoming_row(
        &self,
        position: usize,
        track: TrackRef,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let index = position - 1;
        let hovered = self.hovered_upcoming == Some(index);
        let duration = track
            .duration_ms
            .map(format_queue_time)
            .unwrap_or_else(|| "--:--".to_string());

        // Uniform 52px stride for the virtualized list: the 50px design row
        // plus its 2px gap as bottom padding.
        let row = div()
            .id(("queue-upcoming", index))
            .flex()
            .items_center()
            .gap(px(10.))
            .w_full()
            .h(px(50.))
            .flex_none()
            .px(px(10.))
            .rounded(px(theme::RADIUS_MD))
            .when(hovered, |row| row.bg(theme::bg_muted()))
            .cursor_pointer()
            .on_hover(cx.listener(move |this, &hovered, _, cx| {
                if hovered {
                    this.hovered_upcoming = Some(index);
                } else if this.hovered_upcoming == Some(index) {
                    this.hovered_upcoming = None;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.jump_to_queue_entry(index, cx)))
            .child(
                div()
                    .min_w(px(18.))
                    .flex_none()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(11.))
                    .text_color(theme::text_muted())
                    .child(position.to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(13.))
                            .text_color(theme::text_primary())
                            .child(track.title.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .font_family(theme::FONT_SANS)
                            .text_size(px(11.))
                            .text_color(theme::text_secondary())
                            .child(format!("{} · {}", track.artist, track.album)),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(11.))
                    .text_color(theme::text_muted())
                    .child(duration),
            )
            .when(hovered, |row| {
                row.child(
                    div()
                        .id(("queue-remove", index))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(14.))
                        .flex_none()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.remove_queue_entry(index, cx);
                        }))
                        .child(
                            svg()
                                .path("icons/x.svg")
                                .size(px(14.))
                                .text_color(theme::text_muted()),
                        ),
                )
            });

        div()
            .w_full()
            .h(px(52.))
            .pb(px(2.))
            .child(row)
            .into_any_element()
    }
}

impl Render for PlaybackRow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.surface == PlaybackSurface::SettingsOutputPicker {
            return div()
                .relative()
                .size(px(0.))
                .when(self.output_popover_open, |anchor| {
                    anchor.child(self.render_output_popover(cx))
                });
        }

        div()
            .flex()
            .flex_col()
            .w_full()
            .flex_none()
            .when_some(self.notice.clone(), |column, notice| {
                column.child(self.render_notice(notice, cx))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(22.))
                    .w_full()
                    .h(px(92.))
                    .flex_none()
                    .px(px(20.))
                    .py(px(12.))
                    .border_t_1()
                    .border_color(theme::border())
                    .bg(theme::bg_surface())
                    .child(self.render_now_playing())
                    .child(self.render_transport(cx))
                    .child(self.render_output(cx)),
            )
    }
}

impl PlaybackRow {
    fn render_notice(&self, notice: PlaybackNotice, cx: &mut Context<Self>) -> impl IntoElement {
        let (text, color, recovery, release_url) = match notice {
            PlaybackNotice::Skip { text } => (text, theme::warning(), false, None),
            PlaybackNotice::Stopped { text } => (text, theme::danger(), false, None),
            PlaybackNotice::DeviceFailure { text } => (text, theme::danger(), true, None),
            PlaybackNotice::UpdateAvailable { version, url } => (
                format!("Pulse {version} is available"),
                theme::quality(),
                false,
                Some(url),
            ),
        };
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .w_full()
            .px(px(20.))
            .py(px(7.))
            .border_t_1()
            .border_color(theme::border())
            .bg(theme::bg_surface())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme::FONT_SANS)
                    .text_size(px(12.))
                    .text_color(color)
                    .child(text),
            )
            .when_some(release_url, |banner, url| {
                banner.child(
                    crate::components::compact_secondary_button(
                        "playback-notice-view-release",
                        "View release",
                    )
                    .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
                )
            })
            .when(recovery, |banner| {
                banner
                    .child(
                        crate::components::compact_secondary_button(
                            "playback-notice-retry",
                            "Try again",
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.retry_playback(cx))),
                    )
                    .child(
                        crate::components::compact_secondary_button(
                            "playback-notice-outputs",
                            "Choose output",
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_output_popover(cx))),
                    )
            })
            .child(
                div()
                    .id("playback-notice-dismiss")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(20.))
                    .flex_none()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss_notice(cx)))
                    .child(
                        svg()
                            .path("icons/x.svg")
                            .size(px(13.))
                            .text_color(theme::text_muted()),
                    ),
            )
    }
}

fn section_label(label: &'static str) -> impl IntoElement {
    div()
        .font_family(theme::FONT_MONO)
        .font_weight(FontWeight::BOLD)
        .text_size(px(10.))
        .text_color(theme::text_muted())
        .child(label)
}

fn resolve_output_device(
    devices: &[device::Device],
    system_default: &device::Device,
    preferred_uid: Option<&str>,
) -> (device::Device, bool) {
    let Some(preferred_uid) = preferred_uid else {
        return (system_default.clone(), false);
    };
    match devices.iter().find(|device| device.uid == preferred_uid) {
        Some(device) => (device.clone(), false),
        None => (system_default.clone(), true),
    }
}

fn format_device_capabilities(capabilities: device::OutputDeviceCapabilities) -> String {
    let sample_rate = format_sample_rate(capabilities.max_sample_rate.round() as u32);
    match capabilities.max_bits_per_channel {
        Some(bits) => format!("Up to {bits}-bit / {sample_rate}"),
        None => format!("Up to {sample_rate}"),
    }
}

fn transport_icon(path: &'static str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(17.))
        .child(
            svg()
                .path(path)
                .size(px(17.))
                .text_color(theme::text_secondary()),
        )
}

fn time_label(value: String) -> impl IntoElement {
    div()
        .font_family(theme::FONT_MONO)
        .text_size(px(11.))
        .text_color(theme::text_muted())
        .child(value)
}

fn is_supported_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn track_title(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn track_secondary(path: &Path) -> String {
    path.parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn format_quality(path: Option<&Path>, format: PcmFormat) -> String {
    let container = path
        .and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_uppercase)
        .unwrap_or_else(|| "PCM".to_string());
    format!("{container} · {}-bit", format.bits_per_sample)
}

fn format_sample_rate(sample_rate: u32) -> String {
    if sample_rate.is_multiple_of(1_000) {
        format!("{} kHz", sample_rate / 1_000)
    } else {
        format!("{:.1} kHz", sample_rate as f64 / 1_000.0)
    }
}

fn format_time(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    if minutes >= 60 {
        format!("{}:{:02}:{seconds:02}", minutes / 60, minutes % 60)
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// Queue-popover times drop the leading zero on minutes ("1:24", "3:59").
fn format_queue_time(milliseconds: u64) -> String {
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
fn format_queue_meta(count: usize, total_ms: u64) -> String {
    if total_ms == 0 {
        return format!("{count} UP NEXT");
    }
    let minutes = ((total_ms + 30_000) / 60_000).max(1);
    format!("{count} UP NEXT · {minutes} MIN")
}

fn playback_state_label(state: PlaybackState) -> &'static str {
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

fn scrub_position_ms(fraction: f32, duration_ms: u64) -> u64 {
    (duration_ms as f64 * f64::from(fraction)).round() as u64
}

fn volume_gain_for_level(level: f32) -> f32 {
    let level = level.clamp(0.0, 1.0);
    if level == 0.0 {
        return 0.0;
    }
    (level * level * level).max(MIN_AUDIBLE_GAIN)
}

fn volume_icon_state(level: f32, muted: bool) -> VolumeIconState {
    if muted || level == 0.0 {
        VolumeIconState::Muted
    } else if level >= 0.5 {
        VolumeIconState::High
    } else {
        VolumeIconState::Low
    }
}

fn displayed_volume_level(level: f32, muted: bool) -> f32 {
    if muted { 0.0 } else { level }
}

fn format_volume_percent(level: f32) -> String {
    format!("{:.0}%", level.clamp(0.0, 1.0) * 100.0)
}

fn fraction_at_x(bounds: Bounds<Pixels>, x: Pixels) -> f32 {
    if bounds.size.width <= px(0.) {
        return 0.0;
    }
    ((x - bounds.origin.x) / bounds.size.width).clamp(0.0, 1.0)
}

fn fraction_at_y(bounds: Bounds<Pixels>, y: Pixels) -> f32 {
    if bounds.size.height <= px(0.) {
        return 0.0;
    }
    ((bounds.origin.y + bounds.size.height - y) / bounds.size.height).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use gpui::{bounds, point, px, size};

    use super::*;

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
    fn formats_time_for_tracks_and_long_programs() {
        assert_eq!(format_time(0), "00:00");
        assert_eq!(format_time(185_999), "03:05");
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
            PlaybackRow::initial().volume_command(),
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
    fn resolves_saved_output_by_uid_and_falls_back_visibly() {
        let system_default = output_device(1, "built-in", "Mac Speakers");
        let dac = output_device(9, "matrix", "mini-i Series");
        let devices = vec![system_default.clone(), dac.clone()];

        let (selected, missing) = resolve_output_device(&devices, &system_default, Some("matrix"));
        assert_eq!(selected.id, dac.id);
        assert!(!missing);

        let (selected, missing) =
            resolve_output_device(&devices, &system_default, Some("unplugged"));
        assert_eq!(selected.id, system_default.id);
        assert!(missing);
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
    fn repeated_capability_refresh_replaces_its_message() {
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
        let selected = output_device(9, "matrix", "mini-i Series");

        let applied = row.apply_completed_output_device_change(PendingDeviceChange {
            device: selected.clone(),
            persist: false,
            success_message: Some(DeviceMessage {
                text: "Using the system default.".to_string(),
                is_error: false,
            }),
        });

        assert_eq!(applied.id, selected.id);
        assert_eq!(row.active_device.as_ref().unwrap().uid, selected.uid);
        assert_eq!(
            row.device_message.as_ref().unwrap().text,
            "Using the system default."
        );
    }

    #[test]
    fn attributes_a_device_change_error_and_clears_the_pending_change() {
        let mut row = PlaybackRow::initial();
        row.pending_device_change = Some(PendingDeviceChange {
            device: output_device(9, "matrix", "mini-i Series"),
            persist: false,
            success_message: None,
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
        let mut row = PlaybackRow::initial();
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
        assert_eq!(row.displayed_fraction(), 0.5);
        assert!(row.error.is_none());

        row.handle_event(PlaybackEvent::Ended { attempt: 0 });
        assert_eq!(row.playback_state, PlaybackState::Ended);
        assert_eq!(row.position_ms, 268_000);
    }

    #[test]
    fn maps_real_transport_states_to_toggle_commands() {
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
            row.now_playing_lines(),
            Some(("playing".to_string(), "Artist - Album".to_string()))
        );
        let _ = row.handle_event(PlaybackEvent::StateChanged(PlaybackState::Idle));
        assert_eq!(row.now_playing_lines(), None);

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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
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
        let mut row = PlaybackRow::initial();
        assert_eq!(row.now_playing_lines(), None);

        let temp = tempfile::tempdir().unwrap();
        let tracks = wav_tracks(temp.path(), &["present"]);
        row.queue = QueueState::from_tracks(&tracks, 0);
        row.source_path = Some(tracks[0].path.clone());
        row.playback_state = PlaybackState::Playing;
        assert_eq!(
            row.now_playing_lines(),
            Some(("present".to_string(), "Artist · Album".to_string()))
        );

        row.playback_state = PlaybackState::Ended;
        assert_eq!(row.now_playing_lines(), None);

        // A dropped file plays without a queue; the row's display strings
        // stand in.
        row.queue = QueueState::default();
        row.playback_state = PlaybackState::Playing;
        row.title = "Dropped".to_string();
        row.secondary = "Folder".to_string();
        assert_eq!(
            row.now_playing_lines(),
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
