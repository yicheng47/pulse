use super::{
    EventSubscribers,
    backend::{ActiveBackendRelease, BackendFactory, PlaybackBackend, combine_backend_errors},
    decoder::{DecoderFactory, SourceDecoder},
};
use crate::{
    EngineError, EngineKind, PcmFormat, PlayableSource, PlaybackCommand, PlaybackEvent,
    PlaybackState, VolumeState, device::DeviceId,
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

const POSITION_EVENT_INTERVAL_MS: u64 = 100;
const FEED_RETRY_DELAY: Duration = Duration::from_millis(10);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const OUTPUT_STALL_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) type Clock = Box<dyn Fn() -> Instant + Send>;

/// The track the listener hears — what `NowPlaying`, `Position`, and dropout events describe.
/// `position_ms` is the last reported position (or the resting point while paused);
/// `resume_position_ms` is where Resume restarts from. Dropout tallies are per track.
struct CurrentTrack {
    source: PlayableSource,
    format: PcmFormat,
    position_ms: u64,
    resume_position_ms: u64,
    dropout_frames: u64,
    last_reported_dropout_frames: u64,
}

/// The pump's working state; present exactly while audio is flowing (`Playing`), absent while
/// paused, ended, or idle. `pcm` / `pcm_offset` stage the decoder's current chunk between feeds;
/// `fed_frames` counts frames handed to the backend this run; `track_start_frames` and
/// `base_position_ms` map the backend's frame counter back to a position in the current track
/// (they move at a gapless boundary, see `advance_transition_if_audible`). The remaining fields
/// drive dropout accounting and the output-stall watchdog.
struct ActivePlayback {
    decoder: Box<dyn SourceDecoder>,
    base_position_ms: u64,
    track_start_frames: u64,
    pcm: Vec<u8>,
    pcm_offset: usize,
    fed_frames: u64,
    decoder_finished: bool,
    last_reported_position_ms: u64,
    last_underrun_frames: Option<u64>,
    last_backend_position: u64,
    backend_has_progressed: bool,
    stalled_since: Option<Instant>,
}

/// A `SetNext` track, opened and format-checked ahead of time so the boundary needs no I/O.
struct PreloadedSource {
    source: PlayableSource,
    format: PcmFormat,
    decoder: Box<dyn SourceDecoder>,
}

/// A gapless boundary that has been fed into the ring but is not audible yet. Until the backend's
/// frame counter reaches `boundary_frames`, positions still belong to the outgoing track; then
/// `advance_transition_if_audible` makes `incoming` the current track and announces `Advanced`.
struct PendingTransition {
    boundary_frames: u64,
    incoming: CurrentTrack,
}

/// A decoder already opened and seeked while paused (`Load`, or `Seek` in `Paused`), so Resume
/// starts from the exact position without repeating the seek.
struct PreparedDecoder {
    path: PathBuf,
    requested_position_ms: u64,
    actual_position_ms: u64,
    decoder: Box<dyn SourceDecoder>,
}

pub(super) struct WorkerSettings {
    pub(super) output_device: DeviceId,
    pub(super) engine_kind: EngineKind,
    pub(super) output_stall_timeout: Duration,
    pub(super) now: Clock,
}

/// The state machine, single-threaded on the worker thread. Three optional slots carry the
/// playback lifecycle, with these invariants:
///
/// - `active.is_some()` implies `current` and `backend` are set (audio is flowing).
/// - `backend` can outlive `active`: paused on the integer engine (device kept), or between the
///   stop and the restart at a format-change boundary.
/// - `next_source` holds a `SetNext` track until its boundary; `transition` holds a boundary that
///   is fed but not yet audible; a track is never in both.
///
/// `state` is what the UI sees; `set_state` guards the transitions. `bit_perfect_active` and
/// `volume_state` feed the UI indicators and change only in `start_backend` / `release_backend`,
/// which is why a boundary that skips `release_backend` shows no flicker.
pub(super) struct Worker {
    state: PlaybackState,
    /// Count of PlayFile and Load commands processed. Gapless advances keep the current count.
    attempt: u64,
    output_device: DeviceId,
    engine_kind: EngineKind,
    /// True after an exclusive start fell back to shared; cleared by any device or mode change.
    shared_mode_fallback: bool,
    bit_perfect_active: bool,
    volume_state: VolumeState,
    /// Devices whose hardware volume was read once and adopted as the app level; later starts on
    /// them push the app level down instead of reading again.
    adopted_hardware_volume: HashSet<DeviceId>,
    volume_level: f32,
    muted: bool,
    current: Option<CurrentTrack>,
    active: Option<ActivePlayback>,
    // Once a transition is buffered, it cannot be replaced without changing audible output;
    // lookahead commands then update the buffered incoming track's successor instead.
    next_source: Option<PreloadedSource>,
    transition: Option<PendingTransition>,
    prepared_decoder: Option<PreparedDecoder>,
    /// The open engine, tagged with the device and kind it was opened for so
    /// `take_or_open_backend` knows whether it can be reused.
    backend: Option<(DeviceId, EngineKind, Box<dyn PlaybackBackend>)>,
    command_rx: Receiver<PlaybackCommand>,
    subscribers: EventSubscribers,
    backend_factory: BackendFactory,
    decoder_factory: DecoderFactory,
    output_stall_timeout: Duration,
    now: Clock,
    shutdown: Arc<AtomicBool>,
    backend_release: ActiveBackendRelease,
}

impl Worker {
    pub(super) fn new(
        settings: WorkerSettings,
        command_rx: Receiver<PlaybackCommand>,
        subscribers: EventSubscribers,
        backend_factory: BackendFactory,
        decoder_factory: DecoderFactory,
        shutdown: Arc<AtomicBool>,
        backend_release: ActiveBackendRelease,
    ) -> Self {
        Self {
            state: PlaybackState::Idle,
            attempt: 0,
            output_device: settings.output_device,
            engine_kind: settings.engine_kind,
            shared_mode_fallback: false,
            bit_perfect_active: false,
            volume_state: VolumeState::default(),
            adopted_hardware_volume: HashSet::new(),
            volume_level: 1.0,
            muted: false,
            current: None,
            active: None,
            next_source: None,
            transition: None,
            prepared_decoder: None,
            backend: None,
            command_rx,
            subscribers,
            backend_factory,
            decoder_factory,
            output_stall_timeout: settings.output_stall_timeout,
            now: settings.now,
            shutdown,
            backend_release,
        }
    }

    /// The loop. While audio flows, commands are polled without blocking and `pump` does the
    /// work, sleeping `FEED_RETRY_DELAY` only when nothing moved (ring full, decoder waiting).
    /// Idle, it blocks on the command channel. However the loop ends, the device is released.
    pub(super) fn run(mut self) {
        while !self.shutdown.load(Ordering::Acquire) {
            if self.active.is_some() {
                match self.command_rx.try_recv() {
                    Ok(command) => self.handle_command(command),
                    Err(TryRecvError::Empty) => match self.pump() {
                        Ok(true) => {}
                        Ok(false) => thread::sleep(FEED_RETRY_DELAY),
                        Err(error) => self.fail(error),
                    },
                    Err(TryRecvError::Disconnected) => break,
                }
            } else {
                match self.command_rx.recv_timeout(SHUTDOWN_POLL_INTERVAL) {
                    Ok(command) => self.handle_command(command),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        }

        if let Err(error) = self.release_backend() {
            self.broadcast(PlaybackEvent::Error {
                attempt: self.attempt,
                kind: (&error).into(),
                message: error.to_string(),
            });
        }
    }

    fn handle_command(&mut self, command: PlaybackCommand) {
        let next_command = match command {
            PlaybackCommand::SetVolume { level, muted } => {
                let (level, muted, next_command) =
                    coalesce_volume_commands(&self.command_rx, level, muted);
                self.handle_command_once(PlaybackCommand::SetVolume { level, muted });
                next_command
            }
            command => {
                self.handle_command_once(command);
                None
            }
        };
        if let Some(command) = next_command {
            self.handle_command(command);
        }
    }

    /// One command. A boundary that has become audible is settled first, so the command applies
    /// to the track the listener is actually hearing.
    fn handle_command_once(&mut self, command: PlaybackCommand) {
        if self.transition.is_some() {
            let backend_position = self
                .backend
                .as_ref()
                .expect("buffered transition must have a backend")
                .2
                .position();
            self.advance_transition_if_audible(backend_position);
        }
        let result = match command {
            PlaybackCommand::PlayFile { path } => self.play_file(path),
            PlaybackCommand::Load { path, position_ms } => self.load(path, position_ms),
            PlaybackCommand::SetNext { path } => self.set_next(path),
            PlaybackCommand::ClearNext => {
                self.clear_next();
                Ok(())
            }
            PlaybackCommand::Pause => self.pause(),
            PlaybackCommand::Resume => self.resume(),
            PlaybackCommand::Seek { position_ms } => self.seek(position_ms),
            PlaybackCommand::Stop => self.stop(),
            PlaybackCommand::SetOutputDevice { device_id, kind } => {
                self.set_output_device(device_id, kind)
            }
            PlaybackCommand::SetExclusiveMode { enabled } => self.set_exclusive_mode(enabled),
            PlaybackCommand::SetVolume { level, muted } => self.set_volume(level, muted),
        };

        if let Err(error) = result {
            self.fail(error);
        }
    }

    /// Start a track from the top. Everything queued is dropped; the open backend is kept for
    /// `start_backend` to reuse.
    fn play_file(&mut self, path: PathBuf) -> Result<(), EngineError> {
        self.attempt += 1;
        self.next_source = None;
        self.transition = None;
        self.stop_active()?;
        self.prepared_decoder = None;
        self.current = None;
        self.set_state(PlaybackState::Loading);
        self.start_path(&path, 0, true, false)
    }

    /// Restore a track paused at `position_ms` without touching the device (launch-state
    /// restore): the decoder is opened and seeked, and the worker parks in `Paused`.
    fn load(&mut self, path: PathBuf, position_ms: u64) -> Result<(), EngineError> {
        if !matches!(
            self.state,
            PlaybackState::Idle | PlaybackState::Ended | PlaybackState::Error
        ) {
            self.illegal_command("Load");
            return Ok(());
        }

        self.attempt += 1;
        self.next_source = None;
        self.transition = None;
        self.release_backend()?;
        self.prepared_decoder = None;
        self.current = None;
        self.set_state(PlaybackState::Loading);

        let result: Result<(), EngineError> = (|| {
            let mut decoder = (self.decoder_factory)(&path)?;
            let format = decoder.format();
            let duration_ms = decoder.duration_ms();
            let requested_position_ms =
                duration_ms.map_or(position_ms, |duration| position_ms.min(duration));
            let actual_position_ms = if requested_position_ms == 0 {
                0
            } else {
                decoder.seek(requested_position_ms)?
            };
            let source = PlayableSource {
                path: path.clone(),
                duration_ms,
            };
            self.prepared_decoder = Some(PreparedDecoder {
                path,
                requested_position_ms,
                actual_position_ms,
                decoder,
            });
            self.current = Some(CurrentTrack {
                source: source.clone(),
                format,
                position_ms: actual_position_ms,
                resume_position_ms: requested_position_ms,
                dropout_frames: 0,
                last_reported_dropout_frames: 0,
            });
            self.broadcast(PlaybackEvent::NowPlaying { source, format });
            self.emit_position(actual_position_ms);
            self.set_state(PlaybackState::Paused);
            Ok(())
        })();

        if let Err(error) = result {
            self.prepared_decoder = None;
            self.current = None;
            self.set_state(PlaybackState::Idle);
            self.broadcast(PlaybackEvent::Error {
                attempt: self.attempt,
                kind: (&error).into(),
                message: error.to_string(),
            });
        }
        Ok(())
    }

    fn set_next(&mut self, path: PathBuf) -> Result<(), EngineError> {
        if !matches!(
            self.state,
            PlaybackState::Loading | PlaybackState::Playing | PlaybackState::Paused
        ) {
            self.illegal_command("SetNext");
            return Ok(());
        }
        if self
            .next_source
            .as_ref()
            .is_some_and(|next| next.source.path == path)
        {
            return Ok(());
        }

        match self.open_preloaded_source(&path) {
            Ok(source) => self.next_source = Some(source),
            Err(error) => self.broadcast(PlaybackEvent::NextRejected {
                attempt: self.attempt,
                path,
                message: error.to_string(),
            }),
        }
        Ok(())
    }

    fn clear_next(&mut self) {
        if !matches!(
            self.state,
            PlaybackState::Loading | PlaybackState::Playing | PlaybackState::Paused
        ) {
            self.illegal_command("ClearNext");
            return;
        }
        self.next_source = None;
    }

    /// Pause. The integer engine keeps the device (hog, negotiated format) and only stops the
    /// sink; the universal engine releases it. A buffered gapless boundary is undone back into
    /// `next_source`, because its frames were discarded with the ring.
    fn pause(&mut self) -> Result<(), EngineError> {
        if self.state != PlaybackState::Playing {
            self.illegal_command("Pause");
            return Ok(());
        }

        let position_ms = self.logical_position_ms();
        if let Some(current) = &mut self.current {
            current.position_ms = position_ms;
            current.resume_position_ms = position_ms;
        }
        let transition = self.transition.take();
        if self
            .backend
            .as_ref()
            .is_some_and(|(_, _, backend)| backend.retains_device_when_paused())
        {
            self.stop_active()?;
        } else {
            self.release_backend()?;
        }
        self.restore_transition_as_next(transition);
        self.emit_position(position_ms);
        self.set_state(PlaybackState::Paused);
        Ok(())
    }

    /// Resume is a fresh start of the current path at its resume position; the backend is
    /// reused if it is still open.
    fn resume(&mut self) -> Result<(), EngineError> {
        if self.state != PlaybackState::Paused {
            self.illegal_command("Resume");
            return Ok(());
        }

        let (path, position_ms) = self.current_path_and_position()?;
        self.set_state(PlaybackState::Loading);
        self.start_path(&path, position_ms, false, true)
    }

    /// Seek. Paused: open a decoder, seek it, and park it as `prepared_decoder` for Resume.
    /// Playing: stop the sink and restart the same path at the new position (the backend stays
    /// open).
    fn seek(&mut self, position_ms: u64) -> Result<(), EngineError> {
        if !matches!(self.state, PlaybackState::Playing | PlaybackState::Paused) {
            self.illegal_command("Seek");
            return Ok(());
        }

        let requested_position_ms = self.clamp_position(position_ms);
        if self.state == PlaybackState::Paused {
            let path = self.current_path()?;
            let actual_position_ms = if requested_position_ms == 0 {
                self.prepared_decoder = None;
                0
            } else {
                let mut decoder = (self.decoder_factory)(&path)?;
                let actual_position_ms = decoder.seek(requested_position_ms)?;
                self.prepared_decoder = Some(PreparedDecoder {
                    path,
                    requested_position_ms,
                    actual_position_ms,
                    decoder,
                });
                actual_position_ms
            };
            if let Some(current) = &mut self.current {
                current.position_ms = actual_position_ms;
                current.resume_position_ms = requested_position_ms;
            }
            self.emit_position(actual_position_ms);
            return Ok(());
        }

        let path = self.current_path()?;
        self.set_state(PlaybackState::Loading);
        self.prepared_decoder = None;
        let transition = self.transition.take();
        self.stop_active()?;
        self.restore_transition_as_next(transition);
        self.start_path(&path, requested_position_ms, false, true)
    }

    /// Stop: release the device and forget the track. Idle is reported even if the stop call
    /// fails, because dropping the backend releases the device regardless.
    fn stop(&mut self) -> Result<(), EngineError> {
        if self.state == PlaybackState::Idle {
            return Ok(());
        }

        self.set_state(PlaybackState::Stopping);
        self.next_source = None;
        self.transition = None;
        // Dropping the backend releases the device even if its stop call fails, so Idle is factual.
        let stop_error = self.release_backend().err();
        self.prepared_decoder = None;
        self.current = None;
        self.set_state(PlaybackState::Idle);
        if let Some(error) = stop_error {
            self.broadcast(PlaybackEvent::Error {
                attempt: self.attempt,
                kind: (&error).into(),
                message: error.to_string(),
            });
        }
        Ok(())
    }

    /// Switch device (and engine kind). While playing, restart the current track on the new
    /// device from the audible position, rolling the selection back if it fails to start.
    fn set_output_device(
        &mut self,
        device_id: DeviceId,
        engine_kind: EngineKind,
    ) -> Result<(), EngineError> {
        if self.output_device == device_id
            && self.engine_kind == engine_kind
            && !self.shared_mode_fallback
        {
            self.broadcast(PlaybackEvent::OutputDeviceChanged {
                device_id,
                kind: self.actual_engine_kind(),
            });
            return Ok(());
        }
        let restart = if self.state == PlaybackState::Playing {
            Some((self.logical_position_ms(), self.current_path()?))
        } else {
            None
        };
        self.next_source = None;
        self.transition = None;

        if self.state == PlaybackState::Playing {
            let (position_ms, path) = restart.expect("playing output change needs restart state");
            self.set_state(PlaybackState::Loading);
            self.release_backend()?;
            let previous_device = self.output_device;
            let previous_engine_kind = self.engine_kind;
            let previous_shared_mode_fallback = self.shared_mode_fallback;
            self.output_device = device_id;
            self.engine_kind = engine_kind;
            self.shared_mode_fallback = false;
            if let Err(error) = self.start_path(&path, position_ms, false, true) {
                self.output_device = previous_device;
                self.engine_kind = previous_engine_kind;
                self.shared_mode_fallback = previous_shared_mode_fallback;
                return Err(error);
            }
            self.broadcast(PlaybackEvent::OutputDeviceChanged {
                device_id,
                kind: self.actual_engine_kind(),
            });
            return Ok(());
        }

        if self.state == PlaybackState::Paused {
            self.release_backend()?;
        }

        self.output_device = device_id;
        self.engine_kind = engine_kind;
        self.shared_mode_fallback = false;
        self.broadcast(PlaybackEvent::OutputDeviceChanged {
            device_id,
            kind: self.actual_engine_kind(),
        });
        Ok(())
    }

    /// Universal engine only: flip exclusive mode, restarting the current track if one is
    /// playing.
    fn set_exclusive_mode(&mut self, enabled: bool) -> Result<(), EngineError> {
        let EngineKind::Universal { exclusive_mode } = self.engine_kind else {
            return Ok(());
        };
        if exclusive_mode == enabled && !self.shared_mode_fallback {
            return Ok(());
        }
        let restart = if self.state == PlaybackState::Playing {
            Some((self.logical_position_ms(), self.current_path()?))
        } else {
            None
        };
        self.next_source = None;
        self.transition = None;

        if self.state == PlaybackState::Playing {
            let (position_ms, path) = restart.expect("playing mode change needs restart state");
            self.set_state(PlaybackState::Loading);
            self.release_backend()?;
            self.engine_kind = EngineKind::Universal {
                exclusive_mode: enabled,
            };
            self.shared_mode_fallback = false;
            self.start_path(&path, position_ms, false, true)?;
            return Ok(());
        }

        if self.state == PlaybackState::Paused {
            self.release_backend()?;
        }
        self.engine_kind = EngineKind::Universal {
            exclusive_mode: enabled,
        };
        self.shared_mode_fallback = false;
        Ok(())
    }

    fn set_volume(&mut self, level: f32, muted: bool) -> Result<(), EngineError> {
        self.volume_level = level;
        self.muted = muted;
        if let Some((_, _, backend)) = &mut self.backend {
            backend.set_volume(level, muted)?;
        }
        Ok(())
    }

    fn actual_exclusive_mode(&self) -> bool {
        match self.engine_kind {
            EngineKind::Universal { exclusive_mode } => {
                exclusive_mode && !self.shared_mode_fallback
            }
            EngineKind::Integer => true,
        }
    }

    /// The one place a backend is obtained and started. Reuses the open backend when device and
    /// kind match, otherwise opens one; applies volume (adopting the device's hardware level the
    /// first time it is seen); then `start(format)`. An exclusive universal start that fails for
    /// a recoverable reason falls back to a fresh shared backend and reports
    /// `ExclusiveModeFallback`. Success is the only path that sets `bit_perfect_active` and the
    /// volume state.
    fn start_backend(&mut self, format: PcmFormat) -> Result<(), EngineError> {
        let engine_kind = self.actual_engine_kind();
        let exclusive_mode = matches!(
            engine_kind,
            EngineKind::Universal {
                exclusive_mode: true
            }
        );
        let mut backend = match self.take_or_open_backend(engine_kind) {
            Ok(backend) => backend,
            Err(_) if exclusive_mode => return self.start_shared_fallback(format),
            Err(error) => return Err(error),
        };
        let hardware_volume_event = match backend.take_hardware_volume() {
            Some(_) if self.adopted_hardware_volume.contains(&self.output_device) => {
                if let Err(error) = backend.set_volume(self.volume_level, self.muted) {
                    return Err(self.release_backend_after_start_error(backend, error));
                }
                None
            }
            Some((level, _)) => {
                if let Err(error) = backend.set_volume(level, self.muted) {
                    return Err(self.release_backend_after_start_error(backend, error));
                }
                Some((level, self.muted))
            }
            None => {
                if let Err(error) = backend.set_volume(self.volume_level, self.muted) {
                    return Err(self.release_backend_after_start_error(backend, error));
                }
                None
            }
        };
        match backend.start(format) {
            Ok(()) => {
                let volume_state = VolumeState::new(backend.volume_domain());
                self.backend = Some((self.output_device, engine_kind, backend));
                self.set_bit_perfect_active(engine_kind == EngineKind::Integer);
                self.set_volume_state(volume_state);
                if let Some((level, muted)) = hardware_volume_event {
                    self.volume_level = level;
                    self.adopted_hardware_volume.insert(self.output_device);
                    self.broadcast(PlaybackEvent::HardwareVolume { level, muted });
                }
                Ok(())
            }
            Err(error) if exclusive_mode && exclusive_start_can_fallback(&error) => {
                self.release_backend_value(backend)?;
                self.start_shared_fallback(format)
            }
            Err(error) => match self.release_backend_value(backend) {
                Ok(()) => Err(error),
                Err(release_error) => Err(combine_backend_errors(error, release_error)),
            },
        }
    }

    fn actual_engine_kind(&self) -> EngineKind {
        match self.engine_kind {
            EngineKind::Universal { .. } => EngineKind::Universal {
                exclusive_mode: self.actual_exclusive_mode(),
            },
            EngineKind::Integer => EngineKind::Integer,
        }
    }

    /// Hand back the open backend if it is for the same device and kind; otherwise release it (a
    /// failure there is reported but does not block the new device) and open a fresh one.
    fn take_or_open_backend(
        &mut self,
        engine_kind: EngineKind,
    ) -> Result<Box<dyn PlaybackBackend>, EngineError> {
        let mut release_error = None;
        if let Some((device_id, backend_engine_kind, backend)) = self.backend.take() {
            if device_id == self.output_device && backend_engine_kind == engine_kind {
                return Ok(backend);
            }
            // A stale device teardown error is advisory; it must not block the selected device.
            release_error = self.release_backend_value(backend).err();
        }
        let backend = match (self.backend_factory)(self.output_device, engine_kind) {
            Ok(backend) => backend,
            Err(error) => {
                return Err(match release_error {
                    Some(release_error) => combine_backend_errors(error, release_error),
                    None => error,
                });
            }
        };
        if let Some(error) = release_error {
            self.broadcast(PlaybackEvent::Error {
                attempt: self.attempt,
                kind: (&error).into(),
                message: error.to_string(),
            });
        }
        self.backend_release.replace(backend.shutdown_handle());
        Ok(backend)
    }

    fn start_shared_fallback(&mut self, format: PcmFormat) -> Result<(), EngineError> {
        let shared = EngineKind::Universal {
            exclusive_mode: false,
        };
        let mut backend = self.take_or_open_backend(shared)?;
        if let Err(error) = backend.set_volume(self.volume_level, self.muted) {
            return Err(self.release_backend_after_start_error(backend, error));
        }
        if let Err(error) = backend.start(format) {
            return Err(self.release_backend_after_start_error(backend, error));
        }
        let volume_state = VolumeState::new(backend.volume_domain());
        self.shared_mode_fallback = true;
        self.backend = Some((self.output_device, shared, backend));
        self.set_volume_state(volume_state);
        self.broadcast(PlaybackEvent::ExclusiveModeFallback {
            device_id: self.output_device,
        });
        Ok(())
    }

    /// Open (or take the prepared) decoder, seek it, start the backend for its format, and
    /// install the track as `current` + `active`. Used by play, resume, seek-while-playing, and
    /// device or mode switches; the flags say which events the caller has already announced.
    fn start_path(
        &mut self,
        path: &Path,
        requested_position_ms: u64,
        emit_now_playing: bool,
        emit_position: bool,
    ) -> Result<(), EngineError> {
        let prepared = self.prepared_decoder.take().filter(|prepared| {
            prepared.path == path && prepared.requested_position_ms == requested_position_ms
        });
        let (mut decoder, prepared_position_ms) = match prepared {
            Some(prepared) => (prepared.decoder, Some(prepared.actual_position_ms)),
            None => ((self.decoder_factory)(path)?, None),
        };
        let format = decoder.format();
        let duration_ms = decoder.duration_ms();
        let requested_position_ms = duration_ms.map_or(requested_position_ms, |duration| {
            requested_position_ms.min(duration)
        });
        let actual_position_ms = match prepared_position_ms {
            Some(position_ms) => position_ms,
            None if requested_position_ms == 0 => 0,
            None => decoder.seek(requested_position_ms)?,
        };

        self.start_backend(format)?;

        let source = PlayableSource {
            path: path.to_path_buf(),
            duration_ms,
        };
        let (dropout_frames, last_reported_dropout_frames) = if emit_now_playing {
            (0, 0)
        } else {
            self.current.as_ref().map_or((0, 0), |current| {
                (current.dropout_frames, current.last_reported_dropout_frames)
            })
        };
        self.current = Some(CurrentTrack {
            source: source.clone(),
            format,
            position_ms: actual_position_ms,
            resume_position_ms: actual_position_ms,
            dropout_frames,
            last_reported_dropout_frames,
        });
        self.active = Some(ActivePlayback {
            decoder,
            base_position_ms: actual_position_ms,
            track_start_frames: 0,
            pcm: Vec::new(),
            pcm_offset: 0,
            fed_frames: 0,
            decoder_finished: false,
            last_reported_position_ms: actual_position_ms,
            last_underrun_frames: None,
            last_backend_position: 0,
            backend_has_progressed: false,
            stalled_since: None,
        });

        if emit_now_playing {
            self.broadcast(PlaybackEvent::NowPlaying { source, format });
        }
        if emit_position {
            self.emit_position(actual_position_ms);
        }
        self.set_state(PlaybackState::Playing);
        Ok(())
    }

    /// One iteration of the audio loop; returns whether anything moved. In order: decode the next
    /// chunk once the staged one is fully fed; feed as much as the ring takes; read the backend's
    /// frame counter and derive dropouts, gapless advancement, and position events from it; trip
    /// the stall watchdog if the counter stops while frames are outstanding; and, once the
    /// decoder is dry and everything fed has been heard, decide how the track ends — splice the
    /// next track into the same ring (same format), rebuild for a new format, or finish.
    fn pump(&mut self) -> Result<bool, EngineError> {
        let mut made_progress = false;
        let bytes_per_frame = self
            .current
            .as_ref()
            .expect("active playback must have a current track")
            .format
            .bytes_per_frame();
        let active = self.active.as_mut().expect("pump requires active playback");

        if active.pcm_offset == active.pcm.len() && !active.decoder_finished {
            match active.decoder.next_pcm(&mut active.pcm)? {
                Some(decoded_frames) => {
                    active.pcm_offset = 0;
                    made_progress |= decoded_frames > 0;
                }
                None => {
                    active.decoder_finished = true;
                    made_progress = true;
                }
            }
        }

        if active.pcm_offset < active.pcm.len() {
            let accepted_frames = self
                .backend
                .as_mut()
                .expect("active playback must have a backend")
                .2
                .feed(&active.pcm[active.pcm_offset..]);
            active.pcm_offset += accepted_frames * bytes_per_frame;
            active.fed_frames += accepted_frames as u64;
            made_progress |= accepted_frames > 0;
        }

        let backend_position = self
            .backend
            .as_ref()
            .expect("active playback must have a backend")
            .2
            .position();
        self.update_dropout_accounting(backend_position);
        let advanced = self.advance_transition_if_audible(backend_position);
        if advanced {
            made_progress = true;
        } else {
            let position_ms = self.logical_position_ms_at(backend_position);
            let should_report = self.active.as_ref().is_some_and(|active| {
                position_ms.saturating_sub(active.last_reported_position_ms)
                    >= POSITION_EVENT_INTERVAL_MS
            });
            if should_report {
                if let Some(active) = &mut self.active {
                    active.last_reported_position_ms = position_ms;
                }
                if let Some(current) = &mut self.current {
                    current.position_ms = position_ms;
                }
                self.emit_position(position_ms);
            }
        }

        let now = (self.now)();
        let output_stalled = self.active.as_mut().is_some_and(|active| {
            if backend_position != active.last_backend_position {
                active.last_backend_position = backend_position;
                active.backend_has_progressed = true;
                active.stalled_since = None;
                return false;
            }

            if !active.backend_has_progressed || backend_position >= active.fed_frames {
                active.stalled_since = None;
                return false;
            }

            match active.stalled_since {
                Some(stalled_since) => {
                    now.duration_since(stalled_since) >= self.output_stall_timeout
                }
                None => {
                    active.stalled_since = Some(now);
                    false
                }
            }
        });
        if output_stalled {
            return Err(EngineError::Timeout("audio output progress"));
        }

        let decoder_drained = self
            .active
            .as_ref()
            .is_some_and(|active| active.decoder_finished && active.pcm_offset == active.pcm.len());
        if decoder_drained && self.transition.is_none() {
            let fed_frames = self
                .active
                .as_ref()
                .expect("decoder drain requires active playback")
                .fed_frames;
            match self
                .next_source
                .as_ref()
                .map(|next| next.format == self.current_format())
            {
                // Same format: keep feeding the same ring, mark where the new track starts.
                Some(true) => {
                    let next = self.next_source.take().expect("next source was checked");
                    self.begin_seamless_transition(next);
                    made_progress = true;
                    if self.advance_transition_if_audible(backend_position) {
                        made_progress = true;
                    }
                }
                // Different format: once the ring has been heard out, renegotiate the device.
                Some(false) if backend_position >= fed_frames => {
                    let next = self.next_source.take().expect("next source was checked");
                    self.rebuild_for_preloaded(next)?;
                    made_progress = true;
                }
                // Nothing queued and the last fed frame has been heard.
                None if backend_position >= fed_frames => {
                    self.finish_playback();
                    made_progress = true;
                }
                Some(false) | None => {}
            }
        }

        Ok(made_progress)
    }

    /// Gapless splice: swap the decoder under the live ring and remember the frame count at
    /// which the incoming track begins. Nothing on the device changes.
    fn begin_seamless_transition(&mut self, next: PreloadedSource) {
        let active = self
            .active
            .as_mut()
            .expect("seamless transition requires active playback");
        let boundary_frames = active.fed_frames;
        active.decoder = next.decoder;
        active.pcm.clear();
        active.pcm_offset = 0;
        active.decoder_finished = false;
        self.transition = Some(PendingTransition {
            boundary_frames,
            incoming: CurrentTrack {
                source: next.source,
                format: next.format,
                position_ms: 0,
                resume_position_ms: 0,
                dropout_frames: 0,
                last_reported_dropout_frames: 0,
            },
        });
    }

    /// Once the backend has consumed up to the boundary, make the incoming track `current`,
    /// rebase the position math on `boundary_frames`, and announce `Advanced`. Returns whether
    /// it did.
    fn advance_transition_if_audible(&mut self, backend_position: u64) -> bool {
        let Some(transition) = self
            .transition
            .take_if(|transition| backend_position >= transition.boundary_frames)
        else {
            return false;
        };
        let outgoing_position_ms = self.logical_position_ms_at(backend_position);
        let has_pending_dropout = self
            .current
            .as_ref()
            .is_some_and(|current| current.last_reported_dropout_frames != current.dropout_frames);
        if has_pending_dropout {
            if let Some(current) = &mut self.current {
                current.position_ms = outgoing_position_ms;
            }
            self.emit_position(outgoing_position_ms);
        }
        let source = transition.incoming.source.clone();
        let format = transition.incoming.format;
        self.current = Some(transition.incoming);
        if let Some(active) = &mut self.active {
            active.base_position_ms = 0;
            active.track_start_frames = transition.boundary_frames;
            active.last_reported_position_ms = 0;
        }
        self.broadcast(PlaybackEvent::Advanced {
            attempt: self.attempt,
            source,
            format,
        });
        self.emit_position(0);
        true
    }

    /// Format-change boundary. The ring is already heard out (the caller waited for it), so the
    /// sink is stopped and the same backend is restarted for the new format: the engine
    /// renegotiates rate and formats on the device it still holds. The backend is deliberately
    /// not released — no hog gap, no restore-then-reapply, no indicator flicker (feature 78,
    /// stage 1).
    fn rebuild_for_preloaded(&mut self, next: PreloadedSource) -> Result<(), EngineError> {
        self.stop_active()?;
        self.start_backend(next.format)?;
        let source = next.source.clone();
        self.current = Some(CurrentTrack {
            source: next.source,
            format: next.format,
            position_ms: 0,
            resume_position_ms: 0,
            dropout_frames: 0,
            last_reported_dropout_frames: 0,
        });
        self.active = Some(ActivePlayback {
            decoder: next.decoder,
            base_position_ms: 0,
            track_start_frames: 0,
            pcm: Vec::new(),
            pcm_offset: 0,
            fed_frames: 0,
            decoder_finished: false,
            last_reported_position_ms: 0,
            last_underrun_frames: None,
            last_backend_position: 0,
            backend_has_progressed: false,
            stalled_since: None,
        });
        self.broadcast(PlaybackEvent::Advanced {
            attempt: self.attempt,
            source,
            format: next.format,
        });
        self.emit_position(0);
        Ok(())
    }

    /// Natural end: the last fed frame has been heard. Releases the device and reports `Ended`.
    fn finish_playback(&mut self) {
        let position_ms = self.logical_position_ms();
        let should_emit_position = self
            .active
            .as_ref()
            .is_none_or(|active| active.last_reported_position_ms != position_ms)
            || self.current.as_ref().is_some_and(|current| {
                current.last_reported_dropout_frames != current.dropout_frames
            });
        if let Some(current) = &mut self.current {
            current.position_ms = position_ms;
            current.resume_position_ms = position_ms;
        }
        // All fed frames were consumed, so Ended remains factual even if AUHAL stop reports failure.
        let stop_error = self.release_backend().err();
        self.prepared_decoder = None;
        if should_emit_position {
            self.emit_position(position_ms);
        }
        self.set_state(PlaybackState::Ended);
        self.broadcast(PlaybackEvent::Ended {
            attempt: self.attempt,
        });
        if let Some(error) = stop_error {
            self.broadcast(PlaybackEvent::Error {
                attempt: self.attempt,
                kind: (&error).into(),
                message: error.to_string(),
            });
        }
    }

    /// Any error: release the device, drop every queued track, report `Error`.
    fn fail(&mut self, error: EngineError) {
        let error = match self.release_backend() {
            Ok(()) => error,
            Err(release_error) => combine_backend_errors(error, release_error),
        };
        self.next_source = None;
        self.transition = None;
        self.prepared_decoder = None;
        self.current = None;
        self.set_state(PlaybackState::Error);
        self.broadcast(PlaybackEvent::Error {
            attempt: self.attempt,
            kind: (&error).into(),
            message: error.to_string(),
        });
    }

    /// Stop the audio without giving up the device: the sink stops and drains, the backend stays
    /// in its slot for `start_backend` to reuse. Pairs with `release_backend`, which gives it up.
    fn stop_active(&mut self) -> Result<(), EngineError> {
        if self.active.take().is_some() {
            let (device_id, engine_kind, mut backend) = self
                .backend
                .take()
                .expect("active playback must have a backend");
            let result = backend.stop();
            self.backend = Some((device_id, engine_kind, backend));
            result?;
        }
        Ok(())
    }

    /// Give the device back: stop if audio was flowing, then `release` (hog, formats). This is
    /// the only place the bit-perfect indicator and the volume state are cleared.
    fn release_backend(&mut self) -> Result<(), EngineError> {
        let was_active = self.active.take().is_some();
        self.set_bit_perfect_active(false);
        self.set_volume_state(VolumeState::default());
        let Some((_, _, mut backend)) = self.backend.take() else {
            return Ok(());
        };
        let stop_error = if was_active {
            backend.stop().err()
        } else {
            None
        };
        let release_error = self.release_backend_value(backend).err();
        match (stop_error, release_error) {
            (None, None) => Ok(()),
            (Some(error), None) | (None, Some(error)) => Err(error),
            (Some(stop_error), Some(release_error)) => {
                Err(combine_backend_errors(stop_error, release_error))
            }
        }
    }

    fn release_backend_value(&self, backend: Box<dyn PlaybackBackend>) -> Result<(), EngineError> {
        let result = backend.release();
        self.backend_release.clear();
        result
    }

    fn release_backend_after_start_error(
        &self,
        backend: Box<dyn PlaybackBackend>,
        error: EngineError,
    ) -> EngineError {
        match self.release_backend_value(backend) {
            Ok(()) => error,
            Err(release_error) => combine_backend_errors(error, release_error),
        }
    }

    fn set_bit_perfect_active(&mut self, active: bool) {
        if self.bit_perfect_active == active {
            return;
        }
        self.bit_perfect_active = active;
        self.broadcast(PlaybackEvent::BitPerfectStateChanged { active });
    }

    fn set_volume_state(&mut self, state: VolumeState) {
        if self.volume_state == state {
            return;
        }
        self.volume_state = state;
        self.broadcast(PlaybackEvent::VolumeStateChanged(state));
    }

    fn set_state(&mut self, next: PlaybackState) {
        if self.state == next {
            return;
        }
        debug_assert!(
            self.state.can_transition_to(next),
            "invalid playback state transition {:?} -> {:?}",
            self.state,
            next
        );
        self.state = next;
        self.broadcast(PlaybackEvent::StateChanged(next));
    }

    fn illegal_command(&self, command: &'static str) {
        self.broadcast(PlaybackEvent::CommandRejected {
            command,
            state: self.state,
        });
    }

    fn open_preloaded_source(&self, path: &Path) -> Result<PreloadedSource, EngineError> {
        let decoder = (self.decoder_factory)(path)?;
        Ok(PreloadedSource {
            source: PlayableSource {
                path: path.to_path_buf(),
                duration_ms: decoder.duration_ms(),
            },
            format: decoder.format(),
            decoder,
        })
    }

    fn restore_transition_as_next(&mut self, transition: Option<PendingTransition>) {
        let Some(transition) = transition else {
            return;
        };
        let path = transition.incoming.source.path;
        match self.open_preloaded_source(&path) {
            Ok(next) => self.next_source = Some(next),
            Err(error) => {
                self.next_source = None;
                self.broadcast(PlaybackEvent::NextRejected {
                    attempt: self.attempt,
                    path,
                    message: error.to_string(),
                });
            }
        }
    }

    fn current_format(&self) -> PcmFormat {
        self.current
            .as_ref()
            .expect("active playback must have a current track")
            .format
    }

    /// Position of the audible track in ms, from the backend's frame counter (settling a
    /// boundary that has become audible first).
    fn logical_position_ms(&mut self) -> u64 {
        if self.active.is_none() {
            return self
                .current
                .as_ref()
                .map_or(0, |current| current.position_ms);
        }
        let backend_position = self
            .backend
            .as_ref()
            .expect("active playback must have a backend")
            .2
            .position();
        self.advance_transition_if_audible(backend_position);
        self.logical_position_ms_at(backend_position)
    }

    /// Base position plus the frames consumed since the current track started, clamped to its
    /// duration.
    fn logical_position_ms_at(&self, backend_position: u64) -> u64 {
        let active = self
            .active
            .as_ref()
            .expect("logical active position requires active playback");
        let position_ms = active.base_position_ms.saturating_add(frames_to_ms(
            backend_position.saturating_sub(active.track_start_frames),
            self.current_format().sample_rate,
        ));
        self.clamp_position(position_ms)
    }

    fn clamp_position(&self, position_ms: u64) -> u64 {
        self.current
            .as_ref()
            .and_then(|current| current.source.duration_ms)
            .map_or(position_ms, |duration| position_ms.min(duration))
    }

    fn current_path(&self) -> Result<PathBuf, EngineError> {
        self.current
            .as_ref()
            .map(|current| current.source.path.clone())
            .ok_or_else(|| EngineError::Decode("no current source".to_string()))
    }

    fn current_path_and_position(&self) -> Result<(PathBuf, u64), EngineError> {
        self.current
            .as_ref()
            .map(|current| (current.source.path.clone(), current.resume_position_ms))
            .ok_or_else(|| EngineError::Decode("no current source".to_string()))
    }

    /// Turn the backend's cumulative underrun counter into per-track dropout frames. The first
    /// reading after a start is a baseline, not a dropout (priming zeros), and underruns after
    /// the decoder has drained with nothing queued are the drain tail, not dropouts either.
    fn update_dropout_accounting(&mut self, backend_position: u64) {
        let has_following_source = self.transition.is_some() || self.next_source.is_some();
        let underrun_frames = self
            .backend
            .as_ref()
            .expect("active playback must have a backend")
            .2
            .underrun_frames();
        let active = self
            .active
            .as_mut()
            .expect("dropout accounting requires active playback");

        let Some(previous_underrun_frames) = active.last_underrun_frames else {
            if active.fed_frames > 0 && backend_position > 0 {
                active.last_underrun_frames = Some(underrun_frames);
            }
            return;
        };

        let decoder_drained = active.decoder_finished
            && active.pcm_offset == active.pcm.len()
            && backend_position >= active.fed_frames
            && !has_following_source;
        active.last_underrun_frames = Some(underrun_frames);
        if decoder_drained {
            return;
        }
        let new_dropout_frames = underrun_frames.saturating_sub(previous_underrun_frames);
        let current = self
            .current
            .as_mut()
            .expect("dropout accounting requires a current track");
        current.dropout_frames = current.dropout_frames.saturating_add(new_dropout_frames);
    }

    /// Report position, preceded by a `Dropout` event when the tally moved since the last report.
    fn emit_position(&mut self, position_ms: u64) {
        let dropout_report = self.current.as_mut().and_then(|current| {
            let frames = current
                .dropout_frames
                .saturating_sub(current.last_reported_dropout_frames);
            if frames == 0 {
                return None;
            }
            current.last_reported_dropout_frames = current.dropout_frames;
            Some((frames, current.dropout_frames))
        });
        if let Some((frames, cumulative_frames)) = dropout_report {
            self.broadcast(PlaybackEvent::Dropout {
                attempt: self.attempt,
                frames,
                cumulative_frames,
            });
        }
        let dropout_frames = self
            .current
            .as_ref()
            .map_or(0, |current| current.dropout_frames);
        self.broadcast(PlaybackEvent::Position {
            position_ms,
            duration_ms: self
                .current
                .as_ref()
                .and_then(|current| current.source.duration_ms),
            dropout_frames,
        });
    }

    fn broadcast(&self, event: PlaybackEvent) {
        self.subscribers
            .lock()
            .expect("playback event subscribers mutex poisoned")
            .retain(|subscriber| subscriber.send(event.clone()).is_ok());
    }
}

/// Collapse a burst of queued volume commands into the last one. The first non-volume command
/// met on the way is returned so it is not lost.
fn coalesce_volume_commands(
    command_rx: &Receiver<PlaybackCommand>,
    mut level: f32,
    mut muted: bool,
) -> (f32, bool, Option<PlaybackCommand>) {
    loop {
        match command_rx.try_recv() {
            Ok(PlaybackCommand::SetVolume {
                level: next_level,
                muted: next_muted,
            }) => {
                level = next_level;
                muted = next_muted;
            }
            Ok(command) => return (level, muted, Some(command)),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                return (level, muted, None);
            }
        }
    }
}

fn frames_to_ms(frames: u64, sample_rate: u32) -> u64 {
    frames.saturating_mul(1_000) / u64::from(sample_rate)
}

/// Exclusive-start failures that mean "this device will not do exclusive", where shared mode is
/// the right recovery; anything else is a real error.
fn exclusive_start_can_fallback(error: &EngineError) -> bool {
    matches!(
        error,
        EngineError::UnsupportedNominalSampleRate(_)
            | EngineError::Os { .. }
            | EngineError::Timeout(_)
    )
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
