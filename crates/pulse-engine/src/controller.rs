use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{
    Engine, EngineError, EngineKind, PcmFormat, PlayableSource, PlaybackCommand, PlaybackEvent,
    PlaybackState, decode::PcmDecoder, device::DeviceId, integer_engine::IntegerEngine,
};

const POSITION_EVENT_INTERVAL_MS: u64 = 100;
const FEED_RETRY_DELAY: Duration = Duration::from_millis(2);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(100);
const OUTPUT_STALL_TIMEOUT: Duration = Duration::from_secs(2);

type BackendFactory = Arc<
    dyn Fn(DeviceId, EngineKind) -> Result<Box<dyn PlaybackBackend>, EngineError> + Send + Sync,
>;
type DecoderFactory =
    Arc<dyn Fn(&Path) -> Result<Box<dyn SourceDecoder>, EngineError> + Send + Sync>;
type Clock = Box<dyn Fn() -> Instant + Send>;

pub struct PlaybackController {
    command_tx: Sender<PlaybackCommand>,
    subscribers: EventSubscribers,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl PlaybackController {
    pub fn spawn(output_device: DeviceId, engine_kind: EngineKind) -> Self {
        Self::spawn_with_dependencies(
            output_device,
            engine_kind,
            Arc::new(|device_id, kind| match kind {
                EngineKind::Universal { exclusive_mode } => {
                    Ok(Box::new(EngineBackend::open(device_id, exclusive_mode)?)
                        as Box<dyn PlaybackBackend>)
                }
                EngineKind::BitPerfect => {
                    Ok(Box::new(IntegerBackend::open(device_id)?) as Box<dyn PlaybackBackend>)
                }
            }),
            Arc::new(|path| Ok(Box::new(PcmDecoder::open(path)?))),
            OUTPUT_STALL_TIMEOUT,
            Box::new(Instant::now),
        )
    }

    pub fn command_sender(&self) -> Sender<PlaybackCommand> {
        self.command_tx.clone()
    }

    pub fn subscribe(&self) -> Receiver<PlaybackEvent> {
        let (event_tx, event_rx) = mpsc::channel();
        self.subscribers
            .lock()
            .expect("playback event subscribers mutex poisoned")
            .push(event_tx);
        event_rx
    }

    fn spawn_with_dependencies(
        output_device: DeviceId,
        engine_kind: EngineKind,
        backend_factory: BackendFactory,
        decoder_factory: DecoderFactory,
        output_stall_timeout: Duration,
        now: Clock,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let worker_subscribers = Arc::clone(&subscribers);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);

        let worker = thread::Builder::new()
            .name("pulse-playback-controller".to_string())
            .spawn(move || {
                let subscribers = Arc::clone(&worker_subscribers);
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    Worker::new(
                        WorkerSettings {
                            output_device,
                            engine_kind,
                            output_stall_timeout,
                            now,
                        },
                        command_rx,
                        worker_subscribers,
                        backend_factory,
                        decoder_factory,
                        worker_shutdown,
                    )
                    .run();
                }));
                // On exit — including a panic — drop every subscriber sender
                // so receivers observe Disconnected instead of waiting on a
                // dead worker forever.
                subscribers
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clear();
            })
            .expect("failed to spawn playback controller worker");

        Self {
            command_tx,
            subscribers,
            shutdown,
            worker: Some(worker),
        }
    }
}

impl Drop for PlaybackController {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

type EventSubscribers = Arc<Mutex<Vec<Sender<PlaybackEvent>>>>;

trait PlaybackBackend {
    fn start(&mut self, format: PcmFormat) -> Result<(), EngineError>;
    fn feed(&mut self, pcm: &[u8]) -> usize;
    fn position(&self) -> u64;
    fn underrun_frames(&self) -> u64;
    fn take_hardware_volume(&mut self) -> Option<(f32, bool)>;
    fn set_volume(&mut self, level: f32, muted: bool) -> Result<(), EngineError>;
    fn stop(&mut self) -> Result<(), EngineError>;
    fn retains_device_when_paused(&self) -> bool {
        false
    }
    fn release(self: Box<Self>) -> Result<(), EngineError> {
        Ok(())
    }
}

struct EngineBackend {
    engine: Engine,
}

impl EngineBackend {
    fn open(device_id: DeviceId, exclusive_mode: bool) -> Result<Self, EngineError> {
        Ok(Self {
            engine: Engine::open(device_id, exclusive_mode)?,
        })
    }
}

impl PlaybackBackend for EngineBackend {
    fn start(&mut self, format: PcmFormat) -> Result<(), EngineError> {
        self.engine.set_format(format)?;
        self.engine.play()
    }

    fn feed(&mut self, pcm: &[u8]) -> usize {
        self.engine.feed(pcm)
    }

    fn position(&self) -> u64 {
        self.engine.position()
    }

    fn underrun_frames(&self) -> u64 {
        self.engine.underrun_frames()
    }

    fn take_hardware_volume(&mut self) -> Option<(f32, bool)> {
        self.engine.take_hardware_volume()
    }

    fn set_volume(&mut self, level: f32, muted: bool) -> Result<(), EngineError> {
        self.engine.set_volume(level, muted)
    }

    fn stop(&mut self) -> Result<(), EngineError> {
        self.engine.pause()
    }
}

struct IntegerBackend {
    engine: IntegerEngine,
}

impl IntegerBackend {
    fn open(device_id: DeviceId) -> Result<Self, EngineError> {
        Ok(Self {
            engine: IntegerEngine::open(device_id)?,
        })
    }
}

impl PlaybackBackend for IntegerBackend {
    fn start(&mut self, format: PcmFormat) -> Result<(), EngineError> {
        self.engine.set_format(format)?;
        self.engine.play()
    }

    fn feed(&mut self, pcm: &[u8]) -> usize {
        self.engine.feed(pcm)
    }

    fn position(&self) -> u64 {
        self.engine.position()
    }

    fn underrun_frames(&self) -> u64 {
        self.engine.underrun_frames()
    }

    fn take_hardware_volume(&mut self) -> Option<(f32, bool)> {
        self.engine.take_hardware_volume()
    }

    fn set_volume(&mut self, level: f32, muted: bool) -> Result<(), EngineError> {
        self.engine.set_volume(level, muted)
    }

    fn stop(&mut self) -> Result<(), EngineError> {
        self.engine.pause()
    }

    fn retains_device_when_paused(&self) -> bool {
        true
    }

    fn release(self: Box<Self>) -> Result<(), EngineError> {
        let Self { engine } = *self;
        engine.release()
    }
}

trait SourceDecoder {
    fn format(&self) -> PcmFormat;
    fn duration_ms(&self) -> Option<u64>;
    fn seek(&mut self, position_ms: u64) -> Result<u64, EngineError>;
    fn next_pcm(&mut self, pcm: &mut Vec<u8>) -> Result<Option<u64>, EngineError>;
}

impl SourceDecoder for PcmDecoder {
    fn format(&self) -> PcmFormat {
        self.format()
    }

    fn duration_ms(&self) -> Option<u64> {
        self.duration_ms()
    }

    fn seek(&mut self, position_ms: u64) -> Result<u64, EngineError> {
        self.seek(position_ms)
    }

    fn next_pcm(&mut self, pcm: &mut Vec<u8>) -> Result<Option<u64>, EngineError> {
        self.next_pcm(pcm)
    }
}

struct CurrentTrack {
    source: PlayableSource,
    format: PcmFormat,
    position_ms: u64,
    resume_position_ms: u64,
    dropout_frames: u64,
    last_reported_dropout_frames: u64,
}

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

struct PreloadedSource {
    source: PlayableSource,
    format: PcmFormat,
    decoder: Box<dyn SourceDecoder>,
}

struct PendingTransition {
    boundary_frames: u64,
    incoming: CurrentTrack,
}

struct PreparedDecoder {
    path: PathBuf,
    requested_position_ms: u64,
    actual_position_ms: u64,
    decoder: Box<dyn SourceDecoder>,
}

struct WorkerSettings {
    output_device: DeviceId,
    engine_kind: EngineKind,
    output_stall_timeout: Duration,
    now: Clock,
}

struct Worker {
    state: PlaybackState,
    /// Count of PlayFile and Load commands processed. Gapless advances keep the current count.
    attempt: u64,
    output_device: DeviceId,
    engine_kind: EngineKind,
    shared_mode_fallback: bool,
    bit_perfect_active: bool,
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
    backend: Option<(DeviceId, EngineKind, Box<dyn PlaybackBackend>)>,
    command_rx: Receiver<PlaybackCommand>,
    subscribers: EventSubscribers,
    backend_factory: BackendFactory,
    decoder_factory: DecoderFactory,
    output_stall_timeout: Duration,
    now: Clock,
    shutdown: Arc<AtomicBool>,
}

impl Worker {
    fn new(
        settings: WorkerSettings,
        command_rx: Receiver<PlaybackCommand>,
        subscribers: EventSubscribers,
        backend_factory: BackendFactory,
        decoder_factory: DecoderFactory,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            state: PlaybackState::Idle,
            attempt: 0,
            output_device: settings.output_device,
            engine_kind: settings.engine_kind,
            shared_mode_fallback: false,
            bit_perfect_active: false,
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
        }
    }

    fn run(mut self) {
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

    fn resume(&mut self) -> Result<(), EngineError> {
        if self.state != PlaybackState::Paused {
            self.illegal_command("Resume");
            return Ok(());
        }

        let (path, position_ms) = self.current_path_and_position()?;
        self.set_state(PlaybackState::Loading);
        self.start_path(&path, position_ms, false, true)
    }

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
            EngineKind::BitPerfect => true,
        }
    }

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
                backend.set_volume(self.volume_level, self.muted)?;
                None
            }
            Some((level, _)) => {
                backend.set_volume(level, self.muted)?;
                Some((level, self.muted))
            }
            None => {
                backend.set_volume(self.volume_level, self.muted)?;
                None
            }
        };
        match backend.start(format) {
            Ok(()) => {
                self.backend = Some((self.output_device, engine_kind, backend));
                self.set_bit_perfect_active(engine_kind == EngineKind::BitPerfect);
                if let Some((level, muted)) = hardware_volume_event {
                    self.volume_level = level;
                    self.adopted_hardware_volume.insert(self.output_device);
                    self.broadcast(PlaybackEvent::HardwareVolume { level, muted });
                }
                Ok(())
            }
            Err(error) if exclusive_mode && exclusive_start_can_fallback(&error) => {
                backend.release()?;
                self.start_shared_fallback(format)
            }
            Err(error) => match backend.release() {
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
            EngineKind::BitPerfect => EngineKind::BitPerfect,
        }
    }

    fn take_or_open_backend(
        &mut self,
        engine_kind: EngineKind,
    ) -> Result<Box<dyn PlaybackBackend>, EngineError> {
        match self
            .backend
            .take()
            .filter(|(device_id, backend_engine_kind, _)| {
                *device_id == self.output_device && *backend_engine_kind == engine_kind
            }) {
            Some((_, _, backend)) => Ok(backend),
            None => (self.backend_factory)(self.output_device, engine_kind),
        }
    }

    fn start_shared_fallback(&mut self, format: PcmFormat) -> Result<(), EngineError> {
        let shared = EngineKind::Universal {
            exclusive_mode: false,
        };
        let mut backend = self.take_or_open_backend(shared)?;
        backend.set_volume(self.volume_level, self.muted)?;
        backend.start(format)?;
        self.shared_mode_fallback = true;
        self.backend = Some((self.output_device, shared, backend));
        self.broadcast(PlaybackEvent::ExclusiveModeFallback {
            device_id: self.output_device,
        });
        Ok(())
    }

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
                Some(true) => {
                    let next = self.next_source.take().expect("next source was checked");
                    self.begin_seamless_transition(next);
                    made_progress = true;
                    if self.advance_transition_if_audible(backend_position) {
                        made_progress = true;
                    }
                }
                Some(false) if backend_position >= fed_frames => {
                    let next = self.next_source.take().expect("next source was checked");
                    self.rebuild_for_preloaded(next)?;
                    made_progress = true;
                }
                None if backend_position >= fed_frames => {
                    self.finish_playback();
                    made_progress = true;
                }
                Some(false) | None => {}
            }
        }

        Ok(made_progress)
    }

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

    fn rebuild_for_preloaded(&mut self, next: PreloadedSource) -> Result<(), EngineError> {
        self.release_backend()?;
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

    fn release_backend(&mut self) -> Result<(), EngineError> {
        let was_active = self.active.take().is_some();
        self.set_bit_perfect_active(false);
        let Some((_, _, mut backend)) = self.backend.take() else {
            return Ok(());
        };
        let stop_error = if was_active {
            backend.stop().err()
        } else {
            None
        };
        let release_error = backend.release().err();
        match (stop_error, release_error) {
            (None, None) => Ok(()),
            (Some(error), None) | (None, Some(error)) => Err(error),
            (Some(stop_error), Some(release_error)) => {
                Err(combine_backend_errors(stop_error, release_error))
            }
        }
    }

    fn set_bit_perfect_active(&mut self, active: bool) {
        if self.bit_perfect_active == active {
            return;
        }
        self.bit_perfect_active = active;
        self.broadcast(PlaybackEvent::BitPerfectStateChanged { active });
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

fn exclusive_start_can_fallback(error: &EngineError) -> bool {
    matches!(
        error,
        EngineError::UnsupportedNominalSampleRate(_)
            | EngineError::Os { .. }
            | EngineError::Timeout(_)
    )
}

fn combine_backend_errors(first: EngineError, second: EngineError) -> EngineError {
    EngineError::BackendRelease(format!("{first}; {second}"))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        path::PathBuf,
        sync::{Condvar, mpsc::RecvTimeoutError},
        time::{Duration, Instant},
    };

    use super::*;

    struct FakeLog {
        opened_devices: Vec<DeviceId>,
        engine_kinds: Vec<EngineKind>,
        exclusive_modes: Vec<bool>,
        seek_positions: Vec<u64>,
        started_volumes: Vec<(f32, bool)>,
        volume_writes: Vec<(f32, bool)>,
        software_volume_writes: Vec<(f32, bool)>,
        hardware_volume: Option<(f32, bool)>,
        hardware_volume_on_release: Option<(f32, bool)>,
        hardware_volume_settable: bool,
        hardware_volume_writes: Vec<(f32, bool)>,
        backend_starts: Vec<PcmFormat>,
        backend_fed_frames: u64,
        stops: usize,
        releases: usize,
        fail_exclusive_open_device: Option<DeviceId>,
        fail_exclusive_start_device: Option<DeviceId>,
        fail_bitperfect_start_device: Option<DeviceId>,
        fail_all_open_device: Option<DeviceId>,
        stop_error: bool,
        position_limit: u64,
        decoder_starved_after_first_chunk: bool,
        underrun_frames: u64,
        decoder_specs: HashMap<PathBuf, FakeDecoderSpec>,
        unreadable_paths: HashSet<PathBuf>,
        decoder_opens: Vec<PathBuf>,
        decoder_eofs: Vec<PathBuf>,
        prepared_pcm_drains: Vec<PathBuf>,
    }

    impl Default for FakeLog {
        fn default() -> Self {
            Self {
                opened_devices: Vec::new(),
                engine_kinds: Vec::new(),
                exclusive_modes: Vec::new(),
                seek_positions: Vec::new(),
                started_volumes: Vec::new(),
                volume_writes: Vec::new(),
                software_volume_writes: Vec::new(),
                hardware_volume: None,
                hardware_volume_on_release: None,
                hardware_volume_settable: false,
                hardware_volume_writes: Vec::new(),
                backend_starts: Vec::new(),
                backend_fed_frames: 0,
                stops: 0,
                releases: 0,
                fail_exclusive_open_device: None,
                fail_exclusive_start_device: None,
                fail_bitperfect_start_device: None,
                fail_all_open_device: None,
                stop_error: false,
                position_limit: 1_000,
                decoder_starved_after_first_chunk: false,
                underrun_frames: 0,
                decoder_specs: HashMap::new(),
                unreadable_paths: HashSet::new(),
                decoder_opens: Vec::new(),
                decoder_eofs: Vec::new(),
                prepared_pcm_drains: Vec::new(),
            }
        }
    }

    #[derive(Clone, Copy)]
    struct FakeDecoderSpec {
        format: PcmFormat,
        duration_ms: Option<u64>,
        frames: u64,
        seek_pending_frames: u64,
    }

    impl Default for FakeDecoderSpec {
        fn default() -> Self {
            Self {
                format: TEST_FORMAT,
                duration_ms: Some(10_000),
                frames: 2_000,
                seek_pending_frames: 0,
            }
        }
    }

    struct FakeBackend {
        log: Arc<Mutex<FakeLog>>,
        device_id: DeviceId,
        exclusive_mode: bool,
        retains_device: bool,
        fed_frames: u64,
        format: Option<PcmFormat>,
        volume: (f32, bool),
        hardware_volume: Option<(f32, bool)>,
        hardware_volume_active: bool,
        hardware_volume_event_pending: bool,
    }

    impl PlaybackBackend for FakeBackend {
        fn start(&mut self, format: PcmFormat) -> Result<(), EngineError> {
            if self.exclusive_mode
                && self.log.lock().unwrap().fail_exclusive_start_device == Some(self.device_id)
            {
                return Err(EngineError::UnsupportedNominalSampleRate(TEST_FORMAT));
            }
            if self.retains_device
                && self.log.lock().unwrap().fail_bitperfect_start_device == Some(self.device_id)
            {
                return Err(EngineError::Os {
                    call: "AudioDeviceStart",
                    status: -1,
                });
            }
            self.fed_frames = 0;
            self.format = Some(format);
            let mut log = self.log.lock().unwrap();
            log.started_volumes.push(self.volume);
            log.backend_starts.push(format);
            log.backend_fed_frames = 0;
            Ok(())
        }

        fn feed(&mut self, pcm: &[u8]) -> usize {
            let frames = pcm.len()
                / self
                    .format
                    .expect("fake backend must be started before feed")
                    .bytes_per_frame();
            self.fed_frames += frames as u64;
            self.log.lock().unwrap().backend_fed_frames = self.fed_frames;
            frames
        }

        fn position(&self) -> u64 {
            self.fed_frames.min(self.log.lock().unwrap().position_limit)
        }

        fn underrun_frames(&self) -> u64 {
            self.log.lock().unwrap().underrun_frames
        }

        fn take_hardware_volume(&mut self) -> Option<(f32, bool)> {
            if !self.hardware_volume_event_pending {
                return None;
            }
            self.hardware_volume_event_pending = false;
            self.hardware_volume
        }

        fn set_volume(&mut self, level: f32, muted: bool) -> Result<(), EngineError> {
            let mut log = self.log.lock().unwrap();
            log.volume_writes.push((level, muted));
            if self.hardware_volume_active {
                self.volume = (1.0, false);
                self.hardware_volume = Some((level, muted));
                log.hardware_volume = self.hardware_volume;
                log.hardware_volume_writes.push((level, muted));
            } else {
                self.volume = (crate::volume_gain_for_level(level), muted);
                log.software_volume_writes.push(self.volume);
            }
            Ok(())
        }

        fn stop(&mut self) -> Result<(), EngineError> {
            let mut log = self.log.lock().unwrap();
            log.stops += 1;
            if self.hardware_volume_active
                && let Some(hardware_volume) = log.hardware_volume_on_release
            {
                log.hardware_volume = Some(hardware_volume);
            }
            if log.stop_error {
                Err(EngineError::Decode("backend stop failed".to_string()))
            } else {
                Ok(())
            }
        }

        fn retains_device_when_paused(&self) -> bool {
            self.retains_device
        }

        fn release(self: Box<Self>) -> Result<(), EngineError> {
            self.log.lock().unwrap().releases += 1;
            Ok(())
        }
    }

    struct FakeDecoder {
        log: Arc<Mutex<FakeLog>>,
        path: PathBuf,
        format: PcmFormat,
        duration_ms: Option<u64>,
        frames: u64,
        emitted: bool,
        seek_offset_ms: u64,
        seek_pending_frames: u64,
        pending_frames: u64,
    }

    impl SourceDecoder for FakeDecoder {
        fn format(&self) -> PcmFormat {
            self.format
        }

        fn duration_ms(&self) -> Option<u64> {
            self.duration_ms
        }

        fn seek(&mut self, position_ms: u64) -> Result<u64, EngineError> {
            self.log.lock().unwrap().seek_positions.push(position_ms);
            self.pending_frames = self.seek_pending_frames;
            Ok(position_ms.saturating_sub(self.seek_offset_ms))
        }

        fn next_pcm(&mut self, pcm: &mut Vec<u8>) -> Result<Option<u64>, EngineError> {
            if self.pending_frames > 0 {
                let frames = self.pending_frames;
                self.pending_frames = 0;
                pcm.resize(frames as usize * self.format.bytes_per_frame(), 1);
                self.log
                    .lock()
                    .unwrap()
                    .prepared_pcm_drains
                    .push(self.path.clone());
                return Ok(Some(frames));
            }
            if self.emitted {
                if self.log.lock().unwrap().decoder_starved_after_first_chunk {
                    pcm.clear();
                    return Ok(Some(0));
                }
                self.log
                    .lock()
                    .unwrap()
                    .decoder_eofs
                    .push(self.path.clone());
                return Ok(None);
            }
            pcm.resize(self.frames as usize * self.format.bytes_per_frame(), 0);
            self.emitted = true;
            Ok(Some(self.frames))
        }
    }

    const TEST_FORMAT: PcmFormat = PcmFormat {
        sample_rate: 1_000,
        bits_per_sample: 16,
        channels: 2,
    };
    const ALT_FORMAT: PcmFormat = PcmFormat {
        sample_rate: 2_000,
        bits_per_sample: 16,
        channels: 2,
    };
    const TEST_STALL_TIMEOUT: Duration = Duration::from_millis(100);

    #[derive(Clone)]
    struct FakeClock {
        state: Arc<(Mutex<FakeClockState>, Condvar)>,
    }

    struct FakeClockState {
        now: Instant,
        reads: u64,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                state: Arc::new((
                    Mutex::new(FakeClockState {
                        now: Instant::now(),
                        reads: 0,
                    }),
                    Condvar::new(),
                )),
            }
        }

        fn now(&self) -> Instant {
            let (state, read) = &*self.state;
            let mut state = state.lock().unwrap();
            state.reads += 1;
            read.notify_all();
            state.now
        }

        fn advance(&self, duration: Duration) {
            self.state.0.lock().unwrap().now += duration;
        }

        fn reads(&self) -> u64 {
            self.state.0.lock().unwrap().reads
        }

        fn wait_for_read_after(&self, reads: u64) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let (state, read) = &*self.state;
            let mut state = state.lock().unwrap();
            while state.reads <= reads {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let (next_state, timeout) = read.wait_timeout(state, remaining).unwrap();
                state = next_state;
                assert!(
                    !timeout.timed_out(),
                    "timed out waiting for worker clock read"
                );
            }
        }
    }

    #[test]
    fn priming_and_drain_tail_underruns_are_not_counted() {
        let (controller, log) = fake_controller();
        {
            let mut log = log.lock().unwrap();
            log.position_limit = 0;
            log.underrun_frames = 100;
        }
        let events = controller.subscribe();
        controller
            .command_sender()
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.backend_fed_frames == 2_000
                && log.decoder_eofs.contains(&PathBuf::from("track.flac"))
        });

        log.lock().unwrap().position_limit = 100;
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 100,
                    ..
                }
            )),
            PlaybackEvent::Position {
                position_ms: 100,
                duration_ms: Some(10_000),
                dropout_frames: 0,
            }
        );

        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 107;
            log.position_limit = 200;
        }
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Dropout { .. }
            )),
            PlaybackEvent::Dropout {
                attempt: 1,
                frames: 7,
                cumulative_frames: 7,
            }
        );
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::Position {
                position_ms: 200,
                duration_ms: Some(10_000),
                dropout_frames: 7,
            }
        );

        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 111;
            log.position_limit = 2_000;
        }
        wait_for(&events, |event| {
            matches!(event, PlaybackEvent::Ended { .. })
        });
        assert_no_matching_event(&events, Duration::from_millis(20), |event| {
            matches!(event, PlaybackEvent::Dropout { .. })
        });
    }

    #[test]
    fn steady_state_underruns_report_deltas_and_track_cumulative_frames() {
        let (controller, log) = fake_controller();
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        controller
            .command_sender()
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| log.backend_fed_frames == 2_000);
        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 5;
            log.position_limit = 100;
        }
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 100,
                    ..
                }
            )
        });

        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 12;
            log.position_limit = 200;
        }
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Dropout { .. }
            )),
            PlaybackEvent::Dropout {
                attempt: 1,
                frames: 7,
                cumulative_frames: 7,
            }
        );
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::Position {
                position_ms: 200,
                duration_ms: Some(10_000),
                dropout_frames: 7,
            }
        );

        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 15;
            log.position_limit = 300;
        }
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Dropout { .. }
            )),
            PlaybackEvent::Dropout {
                attempt: 1,
                frames: 3,
                cumulative_frames: 10,
            }
        );
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::Position {
                position_ms: 300,
                duration_ms: Some(10_000),
                dropout_frames: 10,
            }
        );
    }

    #[test]
    fn underrun_free_playback_emits_no_dropout_event() {
        let (controller, log) = fake_controller();
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        controller
            .command_sender()
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| log.backend_fed_frames == 2_000);
        log.lock().unwrap().position_limit = 200;

        loop {
            match events.recv_timeout(Duration::from_secs(1)).unwrap() {
                PlaybackEvent::Dropout { .. } => {
                    panic!("underrun-free playback reported a dropout")
                }
                PlaybackEvent::Position {
                    position_ms: 200,
                    dropout_frames,
                    ..
                } => {
                    assert_eq!(dropout_frames, 0);
                    break;
                }
                _ => {}
            }
        }
    }

    #[test]
    fn seamless_transition_keeps_the_underrun_baseline_and_resets_track_total() {
        let (controller, log) = fake_controller();
        configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
        configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| log.backend_fed_frames == 4_000);
        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 5;
            log.position_limit = 100;
        }
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 100,
                    ..
                }
            )
        });
        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 7;
            log.position_limit = 200;
        }
        wait_for(&events, |event| {
            matches!(event, PlaybackEvent::Dropout { .. })
        });
        let _ = events.recv_timeout(Duration::from_secs(1)).unwrap();

        log.lock().unwrap().position_limit = 2_000;
        loop {
            match events.recv_timeout(Duration::from_secs(1)).unwrap() {
                PlaybackEvent::Dropout { .. } => {
                    panic!("gapless boundary emitted a spurious dropout")
                }
                PlaybackEvent::Advanced { .. } => break,
                _ => {}
            }
        }
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::Position {
                position_ms: 0,
                duration_ms: Some(2_000),
                dropout_frames: 0,
            }
        );

        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 10;
            log.position_limit = 2_100;
        }
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Dropout { .. }
            )),
            PlaybackEvent::Dropout {
                attempt: 1,
                frames: 3,
                cumulative_frames: 3,
            }
        );
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::Position {
                position_ms: 100,
                duration_ms: Some(2_000),
                dropout_frames: 3,
            }
        );
    }

    #[test]
    fn pause_resume_preserves_the_track_tally_and_rebaselines_the_new_sink() {
        let (controller, log) = fake_controller();
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| log.backend_fed_frames == 2_000);
        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 5;
            log.position_limit = 100;
        }
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 100,
                    ..
                }
            )
        });
        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 12;
            log.position_limit = 200;
        }
        wait_for(&events, |event| {
            matches!(event, PlaybackEvent::Dropout { .. })
        });
        let _ = events.recv_timeout(Duration::from_secs(1)).unwrap();

        commands.send(PlaybackCommand::Pause).unwrap();
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 200,
                    ..
                }
            )),
            PlaybackEvent::Position {
                position_ms: 200,
                duration_ms: Some(10_000),
                dropout_frames: 7,
            }
        );
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });

        commands.send(PlaybackCommand::Resume).unwrap();
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 200,
                    ..
                }
            )),
            PlaybackEvent::Position {
                position_ms: 200,
                duration_ms: Some(10_000),
                dropout_frames: 7,
            }
        );
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 400,
                    ..
                }
            )
        });

        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 15;
            log.position_limit = 300;
        }
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Dropout { .. }
            )),
            PlaybackEvent::Dropout {
                attempt: 1,
                frames: 3,
                cumulative_frames: 10,
            }
        );
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::Position {
                position_ms: 500,
                duration_ms: Some(10_000),
                dropout_frames: 10,
            }
        );
    }

    #[test]
    fn gapless_boundary_flushes_pending_dropout_before_resetting_the_incoming_track() {
        let (controller, log, clock) = fake_controller_with_stall_timeout(Duration::MAX);
        configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
        configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| log.backend_fed_frames == 4_000);
        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 5;
            log.position_limit = 100;
        }
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 100,
                    ..
                }
            )
        });

        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 8;
            log.position_limit = 150;
        }
        wait_for_worker_pumps(&clock, 2);
        log.lock().unwrap().position_limit = 2_000;

        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Dropout { .. }
            )),
            PlaybackEvent::Dropout {
                attempt: 1,
                frames: 3,
                cumulative_frames: 3,
            }
        );
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::Position {
                position_ms: 2_000,
                duration_ms: Some(2_000),
                dropout_frames: 3,
            }
        );
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::Advanced { .. }
        ));
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::Position {
                position_ms: 0,
                duration_ms: Some(2_000),
                dropout_frames: 0,
            }
        );
    }

    #[test]
    fn finish_flushes_pending_dropout_when_the_position_did_not_move() {
        let (controller, log, clock) = fake_controller_with_stall_timeout(Duration::MAX);
        {
            let mut log = log.lock().unwrap();
            log.position_limit = 0;
            log.decoder_starved_after_first_chunk = true;
        }
        let events = controller.subscribe();
        controller
            .command_sender()
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| log.backend_fed_frames == 2_000);
        {
            let mut log = log.lock().unwrap();
            log.underrun_frames = 5;
            log.position_limit = 100;
        }
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 100,
                    ..
                }
            )
        });
        log.lock().unwrap().position_limit = 2_000;
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 2_000,
                    ..
                }
            )
        });

        log.lock().unwrap().underrun_frames = 12;
        wait_for_worker_pumps(&clock, 2);
        log.lock().unwrap().decoder_starved_after_first_chunk = false;

        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Dropout { .. }
            )),
            PlaybackEvent::Dropout {
                attempt: 1,
                frames: 7,
                cumulative_frames: 7,
            }
        );
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::Position {
                position_ms: 2_000,
                duration_ms: Some(10_000),
                dropout_frames: 7,
            }
        );
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Ended { .. }
            )),
            PlaybackEvent::Ended { attempt: 1 }
        );
    }

    #[test]
    fn stalled_output_emits_device_error_and_engine_remains_reusable() {
        let (controller, log, clock) = fake_controller_with_stall_timeout(TEST_STALL_TIMEOUT);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("stalled.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        clock.advance(TEST_STALL_TIMEOUT * 2);
        wait_for_worker_pumps(&clock, 2);
        assert_no_error_pending(&events);

        log.lock().unwrap().position_limit = 1_000;
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });
        wait_for_worker_pumps(&clock, 2);
        clock.advance(TEST_STALL_TIMEOUT);

        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Error { .. }
            )),
            PlaybackEvent::Error {
                attempt: 1,
                kind: crate::PlaybackErrorKind::Device { hog_pid: None },
                message: "timed out waiting for audio output progress".to_string(),
            }
        );

        commands.send(PlaybackCommand::Stop).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
        });

        log.lock().unwrap().position_limit = u64::MAX;
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("recovered.flac"),
            })
            .unwrap();
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Ended { .. }
            )),
            PlaybackEvent::Ended { attempt: 2 }
        );
    }

    #[test]
    fn paused_playback_does_not_trigger_stall_watchdog() {
        let (controller, _, clock) = fake_controller_with_stall_timeout(TEST_STALL_TIMEOUT);
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });

        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });
        clock.advance(TEST_STALL_TIMEOUT * 2);
        assert_no_matching_event(&events, TEST_STALL_TIMEOUT * 2, |event| {
            matches!(event, PlaybackEvent::Error { .. })
        });
    }

    #[test]
    fn load_prepares_a_paused_source_without_opening_or_starting_the_backend() {
        let (controller, log) = fake_controller_with_seek_offset(250);
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::Load {
                path: PathBuf::from("track.flac"),
                position_ms: 5_000,
            })
            .unwrap();

        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::NowPlaying { .. }
            )),
            PlaybackEvent::NowPlaying {
                source: PlayableSource {
                    path: PathBuf::from("track.flac"),
                    duration_ms: Some(10_000),
                },
                format: TEST_FORMAT,
            }
        );
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Position { .. }
            )),
            PlaybackEvent::Position {
                position_ms: 4_750,
                duration_ms: Some(10_000),
                dropout_frames: 0,
            }
        );
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });
        {
            let log = log.lock().unwrap();
            assert!(log.opened_devices.is_empty());
            assert!(log.backend_starts.is_empty());
            assert_eq!(log.seek_positions, [5_000]);
        }

        commands.send(PlaybackCommand::Resume).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        let log = log.lock().unwrap();
        assert_eq!(log.opened_devices, [7]);
        assert_eq!(log.backend_starts, [TEST_FORMAT]);
        assert_eq!(log.seek_positions, [5_000]);
    }

    #[test]
    fn load_is_rejected_while_a_source_is_paused() {
        let (controller, log) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::Load {
                path: PathBuf::from("first.flac"),
                position_ms: 0,
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });

        commands
            .send(PlaybackCommand::Load {
                path: PathBuf::from("second.flac"),
                position_ms: 0,
            })
            .unwrap();

        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::CommandRejected { .. }
            )),
            PlaybackEvent::CommandRejected {
                command: "Load",
                state: PlaybackState::Paused,
            }
        );
        assert_eq!(
            log.lock().unwrap().decoder_opens,
            [PathBuf::from("first.flac")]
        );
    }

    #[test]
    fn unreadable_loads_return_to_idle_and_increment_attempts() {
        let (controller, log) = fake_controller();
        log.lock()
            .unwrap()
            .unreadable_paths
            .extend([PathBuf::from("first.flac"), PathBuf::from("second.flac")]);
        let events = controller.subscribe();
        let commands = controller.command_sender();

        for (path, attempt) in [("first.flac", 1), ("second.flac", 2)] {
            commands
                .send(PlaybackCommand::Load {
                    path: PathBuf::from(path),
                    position_ms: 1_000,
                })
                .unwrap();
            assert_eq!(
                wait_for(&events, |event| matches!(
                    event,
                    PlaybackEvent::Error { .. }
                )),
                PlaybackEvent::Error {
                    attempt,
                    kind: crate::PlaybackErrorKind::Track,
                    message: "decode: unreadable source".to_string(),
                }
            );
        }

        let log = log.lock().unwrap();
        assert!(log.opened_devices.is_empty());
        assert!(log.backend_starts.is_empty());
    }

    #[test]
    fn decoder_underrun_does_not_trigger_stall_watchdog() {
        let (controller, log, clock) = fake_controller_with_stall_timeout(TEST_STALL_TIMEOUT);
        {
            let mut log = log.lock().unwrap();
            log.position_limit = u64::MAX;
            log.decoder_starved_after_first_chunk = true;
        }
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 2_000,
                    ..
                }
            )
        });

        clock.advance(TEST_STALL_TIMEOUT * 2);
        wait_for_worker_pumps(&clock, 2);
        assert_no_error_pending(&events);

        commands.send(PlaybackCommand::Stop).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
        });
    }

    #[test]
    fn universal_pause_releases_backend_while_seek_reuses_resumed_backend() {
        let (controller, log) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });

        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });

        commands.send(PlaybackCommand::Resume).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        commands
            .send(PlaybackCommand::Seek { position_ms: 5_000 })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 5_000,
                    ..
                }
            )
        });
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        commands.send(PlaybackCommand::Stop).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
        });

        let log = log.lock().unwrap();
        assert_eq!(log.opened_devices, [7, 7]);
        assert_eq!(log.seek_positions, [1_000, 5_000]);
        assert_eq!(log.stops, 3);
        assert_eq!(log.releases, 2);
    }

    #[test]
    fn bitperfect_pause_retains_backend_and_resume_reuses_it() {
        let (controller, log) = fake_controller_with_kind(EngineKind::BitPerfect);
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::BitPerfectStateChanged { active: true }
        });
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });

        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });
        {
            let log = log.lock().unwrap();
            assert_eq!(log.opened_devices, [7]);
            assert_eq!(log.stops, 1);
            assert_eq!(log.releases, 0);
        }

        commands.send(PlaybackCommand::Resume).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands.send(PlaybackCommand::Stop).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::BitPerfectStateChanged { active: false }
        });
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
        });

        let log = log.lock().unwrap();
        assert_eq!(log.opened_devices, [7]);
        assert_eq!(log.backend_starts, [TEST_FORMAT, TEST_FORMAT]);
        assert_eq!(log.stops, 2);
        assert_eq!(log.releases, 1);
    }

    #[test]
    fn stop_releases_a_backend_retained_by_pause() {
        let (controller, log) = fake_controller_with_kind(EngineKind::BitPerfect);
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });
        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });

        commands.send(PlaybackCommand::Stop).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
        });

        let log = log.lock().unwrap();
        assert_eq!(log.stops, 1);
        assert_eq!(log.releases, 1);
    }

    #[test]
    fn device_switch_releases_a_backend_retained_by_pause() {
        let (controller, log) = fake_controller_with_kind(EngineKind::BitPerfect);
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });
        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });

        commands
            .send(PlaybackCommand::SetOutputDevice {
                device_id: 9,
                kind: EngineKind::BitPerfect,
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::OutputDeviceChanged { device_id: 9, .. }
            )
        });

        let log = log.lock().unwrap();
        assert_eq!(log.opened_devices, [7]);
        assert_eq!(log.releases, 1);
    }

    #[test]
    fn engine_switch_releases_a_backend_retained_by_pause() {
        let (controller, log) = fake_controller_with_kind(EngineKind::BitPerfect);
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });
        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });

        commands
            .send(PlaybackCommand::SetOutputDevice {
                device_id: 7,
                kind: EngineKind::Universal {
                    exclusive_mode: false,
                },
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::OutputDeviceChanged { device_id: 7, .. }
            )
        });

        let log = log.lock().unwrap();
        assert_eq!(log.engine_kinds, [EngineKind::BitPerfect]);
        assert_eq!(log.releases, 1);
    }

    #[test]
    fn exclusive_mode_command_does_not_change_bitperfect_backend() {
        let (controller, log) = fake_controller_with_kind(EngineKind::BitPerfect);
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });
        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });
        commands
            .send(PlaybackCommand::SetExclusiveMode { enabled: false })
            .unwrap();
        commands.send(PlaybackCommand::Resume).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        let log = log.lock().unwrap();
        assert_eq!(log.engine_kinds, [EngineKind::BitPerfect]);
        assert_eq!(log.releases, 0);
    }

    #[test]
    fn illegal_command_order_is_rejected_without_changing_idle_state() {
        let (controller, _) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();

        commands.send(PlaybackCommand::Pause).unwrap();
        commands.send(PlaybackCommand::Resume).unwrap();
        commands
            .send(PlaybackCommand::Seek { position_ms: 500 })
            .unwrap();

        let mut rejections = Vec::new();
        while rejections.len() < 3 {
            let event = events.recv_timeout(Duration::from_secs(1)).unwrap();
            match event {
                PlaybackEvent::CommandRejected { command, state } => {
                    rejections.push((command, state))
                }
                PlaybackEvent::StateChanged(state) => {
                    panic!("illegal command changed state to {state:?}")
                }
                _ => {}
            }
        }

        assert_eq!(
            rejections,
            [
                ("Pause", PlaybackState::Idle),
                ("Resume", PlaybackState::Idle),
                ("Seek", PlaybackState::Idle),
            ]
        );
    }

    #[test]
    fn paused_seek_resumes_from_the_original_target_without_compounding_seek_error() {
        let (controller, log) = fake_controller_with_seek_offset(250);
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });
        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });

        commands
            .send(PlaybackCommand::Seek { position_ms: 5_000 })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 4_750,
                    ..
                }
            )
        });
        commands.send(PlaybackCommand::Resume).unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 4_750,
                    ..
                }
            )
        });
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        assert_eq!(log.lock().unwrap().seek_positions, [5_000]);
    }

    #[test]
    fn paused_seek_resume_drains_pending_pcm_from_the_prepared_decoder() {
        let (controller, log) = fake_controller();
        configure_decoder(&log, "track.flac", TEST_FORMAT, 10_000, 2_000);
        log.lock()
            .unwrap()
            .decoder_specs
            .get_mut(Path::new("track.flac"))
            .unwrap()
            .seek_pending_frames = 37;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });
        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });

        commands
            .send(PlaybackCommand::Seek { position_ms: 5_000 })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 5_000,
                    ..
                }
            )
        });
        commands.send(PlaybackCommand::Resume).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        wait_for_log(&log, |log| {
            log.prepared_pcm_drains == [PathBuf::from("track.flac")]
        });

        let log = log.lock().unwrap();
        assert_eq!(
            log.decoder_opens
                .iter()
                .filter(|path| path.as_path() == Path::new("track.flac"))
                .count(),
            2
        );
        assert_eq!(log.seek_positions, [5_000]);
    }

    #[test]
    fn changing_output_device_restarts_active_playback() {
        let (controller, log) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });

        commands
            .send(PlaybackCommand::SetOutputDevice {
                device_id: 9,
                kind: EngineKind::Universal {
                    exclusive_mode: false,
                },
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::OutputDeviceChanged {
                device_id: 9,
                kind: EngineKind::Universal {
                    exclusive_mode: false,
                },
            }
        );

        let log = log.lock().unwrap();
        assert_eq!(log.opened_devices, [7, 9]);
        assert_eq!(log.exclusive_modes, [true, false]);
        assert_eq!(log.seek_positions, [1_000]);
    }

    #[test]
    fn changing_exclusive_mode_reopens_the_backend_without_losing_position() {
        let (controller, log) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });

        commands
            .send(PlaybackCommand::SetExclusiveMode { enabled: false })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        let log = log.lock().unwrap();
        assert_eq!(log.opened_devices, [7, 7]);
        assert_eq!(log.exclusive_modes, [true, false]);
        assert_eq!(log.seek_positions, [1_000]);
    }

    #[test]
    fn shared_mode_start_uses_a_shared_backend() {
        let (controller, log) = fake_controller_with_exclusive_mode(false);
        let events = controller.subscribe();
        controller
            .command_sender()
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        assert_eq!(log.lock().unwrap().exclusive_modes, [false]);
    }

    #[test]
    fn bitperfect_start_failure_surfaces_without_float_fallback() {
        let (controller, log) = fake_controller_with_kind(EngineKind::BitPerfect);
        log.lock().unwrap().fail_bitperfect_start_device = Some(7);
        let events = controller.subscribe();
        controller
            .command_sender()
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();

        let mut saw_fallback = false;
        let error = loop {
            match events.recv_timeout(Duration::from_secs(1)).unwrap() {
                PlaybackEvent::ExclusiveModeFallback { .. } => saw_fallback = true,
                event @ PlaybackEvent::Error { .. } => break event,
                _ => {}
            }
        };

        assert!(!saw_fallback);
        assert_eq!(
            error,
            PlaybackEvent::Error {
                attempt: 1,
                kind: crate::PlaybackErrorKind::Device { hog_pid: None },
                message: "AudioDeviceStart failed (OSStatus -1)".to_string(),
            }
        );
        let log = log.lock().unwrap();
        assert_eq!(log.opened_devices, [7]);
        assert_eq!(log.engine_kinds, [EngineKind::BitPerfect]);
        assert_eq!(log.releases, 1);
    }

    #[test]
    fn bitperfect_restart_failure_clears_the_confirmed_state() {
        let (controller, log) = fake_controller_with_kind(EngineKind::BitPerfect);
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("first.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::BitPerfectStateChanged { active: true }
        });
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        log.lock().unwrap().fail_bitperfect_start_device = Some(7);
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("second.flac"),
            })
            .unwrap();

        assert_eq!(
            wait_for(&events, |event| {
                *event == PlaybackEvent::BitPerfectStateChanged { active: false }
            }),
            PlaybackEvent::BitPerfectStateChanged { active: false }
        );
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Error { .. }
            )),
            PlaybackEvent::Error {
                attempt: 2,
                kind: crate::PlaybackErrorKind::Device { hog_pid: None },
                message: "AudioDeviceStart failed (OSStatus -1)".to_string(),
            }
        );
        assert_eq!(log.lock().unwrap().releases, 1);
    }

    #[test]
    fn exclusive_open_failure_retries_shared_once_for_the_device_session() {
        let (controller, log) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("first.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });
        log.lock().unwrap().fail_exclusive_open_device = Some(9);

        commands
            .send(PlaybackCommand::SetOutputDevice {
                device_id: 9,
                kind: EngineKind::Universal {
                    exclusive_mode: true,
                },
            })
            .unwrap();
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::ExclusiveModeFallback { .. }
            )),
            PlaybackEvent::ExclusiveModeFallback { device_id: 9 }
        );
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::OutputDeviceChanged { .. }
            )),
            PlaybackEvent::OutputDeviceChanged {
                device_id: 9,
                kind: EngineKind::Universal {
                    exclusive_mode: false,
                },
            }
        );

        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("second.flac"),
            })
            .unwrap();
        wait_for(
            &events,
            |event| matches!(event, PlaybackEvent::NowPlaying { source, .. } if source.path == Path::new("second.flac")),
        );
        while let Ok(event) = events.recv_timeout(Duration::from_millis(50)) {
            assert!(!matches!(
                event,
                PlaybackEvent::ExclusiveModeFallback { .. }
            ));
        }

        let log = log.lock().unwrap();
        assert_eq!(log.opened_devices, [7, 9, 9]);
        assert_eq!(log.exclusive_modes, [true, true, false]);
        assert_eq!(log.seek_positions, [1_000]);
    }

    #[test]
    fn unsupported_exclusive_nominal_rate_retries_shared() {
        let (controller, log) = fake_controller();
        log.lock().unwrap().fail_exclusive_start_device = Some(7);
        let events = controller.subscribe();
        controller
            .command_sender()
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();

        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::ExclusiveModeFallback { .. }
            )),
            PlaybackEvent::ExclusiveModeFallback { device_id: 7 }
        );
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        assert_eq!(log.lock().unwrap().exclusive_modes, [true, false]);
    }

    #[test]
    fn reselecting_exclusive_after_fallback_retries_exclusive() {
        let (controller, log) = fake_controller();
        log.lock().unwrap().fail_exclusive_start_device = Some(7);
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(event, PlaybackEvent::ExclusiveModeFallback { device_id: 7 })
        });
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        log.lock().unwrap().fail_exclusive_start_device = None;
        commands
            .send(PlaybackCommand::SetOutputDevice {
                device_id: 7,
                kind: EngineKind::Universal {
                    exclusive_mode: true,
                },
            })
            .unwrap();

        assert_eq!(
            wait_for(&events, |event| {
                matches!(
                    event,
                    PlaybackEvent::OutputDeviceChanged {
                        device_id: 7,
                        kind: EngineKind::Universal {
                            exclusive_mode: true,
                        },
                    }
                )
            }),
            PlaybackEvent::OutputDeviceChanged {
                device_id: 7,
                kind: EngineKind::Universal {
                    exclusive_mode: true,
                },
            }
        );
        assert_eq!(log.lock().unwrap().exclusive_modes, [true, false, true]);
    }

    #[test]
    fn volume_survives_pause_resume_seek_track_changes_and_device_switches() {
        let (controller, log) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::SetVolume {
                level: 0.25,
                muted: true,
            })
            .unwrap();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("first.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });
        commands.send(PlaybackCommand::Resume).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        commands
            .send(PlaybackCommand::Seek { position_ms: 5_000 })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("second.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::NowPlaying { source, .. }
                    if source.path == Path::new("second.flac")
            )
        });
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        commands
            .send(PlaybackCommand::SetOutputDevice {
                device_id: 9,
                kind: EngineKind::Universal {
                    exclusive_mode: false,
                },
            })
            .unwrap();
        wait_for(&events, |event| {
            *event
                == PlaybackEvent::OutputDeviceChanged {
                    device_id: 9,
                    kind: EngineKind::Universal {
                        exclusive_mode: false,
                    },
                }
        });

        assert_eq!(
            log.lock().unwrap().started_volumes,
            [(crate::volume_gain_for_level(0.25), true); 5]
        );
    }

    #[test]
    fn hogged_controllable_device_writes_hardware_and_keeps_software_gain_at_unity() {
        let (controller, log) = fake_controller();
        {
            let mut log = log.lock().unwrap();
            log.hardware_volume = Some((0.4, false));
            log.hardware_volume_settable = true;
        }
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event
                == PlaybackEvent::HardwareVolume {
                    level: 0.4,
                    muted: false,
                }
        });
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        {
            let log = log.lock().unwrap();
            assert_eq!(log.volume_writes, [(0.4, false)]);
            assert!(log.software_volume_writes.is_empty());
            assert_eq!(log.hardware_volume_writes, [(0.4, false)]);
            assert_eq!(log.started_volumes, [(1.0, false)]);
        }

        commands
            .send(PlaybackCommand::SetVolume {
                level: 0.8,
                muted: true,
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.hardware_volume_writes == [(0.4, false), (0.8, true)]
        });

        let log = log.lock().unwrap();
        assert!(log.software_volume_writes.is_empty());
        assert_eq!(log.started_volumes, [(1.0, false)]);
    }

    #[test]
    fn first_hardware_adoption_preserves_the_apps_mute() {
        let (controller, log) = fake_controller();
        {
            let mut log = log.lock().unwrap();
            log.hardware_volume = Some((0.4, false));
            log.hardware_volume_settable = true;
        }
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::SetVolume {
                level: 0.7,
                muted: true,
            })
            .unwrap();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();

        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::HardwareVolume { .. }
            )),
            PlaybackEvent::HardwareVolume {
                level: 0.4,
                muted: true,
            }
        );
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        let log = log.lock().unwrap();
        assert_eq!(log.hardware_volume_writes, [(0.4, true)]);
        assert_eq!(log.started_volumes, [(1.0, false)]);
    }

    #[test]
    fn later_hog_reapplies_the_app_level_without_emitting_another_hardware_event() {
        let (controller, log) = fake_controller();
        {
            let mut log = log.lock().unwrap();
            log.hardware_volume = Some((0.5, false));
            log.hardware_volume_on_release = Some((0.5, false));
            log.hardware_volume_settable = true;
        }
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(event, PlaybackEvent::HardwareVolume { .. })
        });
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetVolume {
                level: 0.2,
                muted: false,
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.hardware_volume_writes.last() == Some(&(0.2, false))
        });

        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });
        assert_eq!(log.lock().unwrap().hardware_volume, Some((0.5, false)));
        commands.send(PlaybackCommand::Resume).unwrap();
        let mut emitted_hardware_volume = false;
        loop {
            match events.recv_timeout(Duration::from_secs(1)).unwrap() {
                PlaybackEvent::HardwareVolume { .. } => emitted_hardware_volume = true,
                PlaybackEvent::StateChanged(PlaybackState::Playing) => break,
                _ => {}
            }
        }

        assert!(!emitted_hardware_volume);
        let log = log.lock().unwrap();
        assert_eq!(log.hardware_volume, Some((0.2, false)));
        assert_eq!(
            log.hardware_volume_writes,
            [(0.5, false), (0.2, false), (0.2, false)]
        );
        assert_eq!(log.started_volumes, [(1.0, false), (1.0, false)]);
    }

    #[test]
    fn hogged_device_without_hardware_control_uses_software_gain() {
        let (controller, log) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::SetVolume {
                level: 0.5,
                muted: false,
            })
            .unwrap();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        let log = log.lock().unwrap();
        assert_eq!(log.started_volumes, [(0.125, false)]);
        assert_eq!(log.volume_writes, [(0.5, false)]);
        assert_eq!(log.software_volume_writes, [(0.125, false)]);
        assert!(log.hardware_volume_writes.is_empty());
    }

    #[test]
    fn shared_mode_uses_software_gain_even_when_the_device_has_hardware_volume() {
        let (controller, log) = fake_controller_with_exclusive_mode(false);
        {
            let mut log = log.lock().unwrap();
            log.hardware_volume = Some((0.4, false));
            log.hardware_volume_settable = true;
        }
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::SetVolume {
                level: 0.5,
                muted: false,
            })
            .unwrap();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        let log = log.lock().unwrap();
        assert_eq!(log.started_volumes, [(0.125, false)]);
        assert_eq!(log.volume_writes, [(0.5, false)]);
        assert_eq!(log.software_volume_writes, [(0.125, false)]);
        assert!(log.hardware_volume_writes.is_empty());
    }

    #[test]
    fn controllable_hog_emits_hardware_volume_once() {
        let (controller, log) = fake_controller();
        {
            let mut log = log.lock().unwrap();
            log.hardware_volume = Some((0.4, true));
            log.hardware_volume_settable = true;
        }
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("first.flac"),
            })
            .unwrap();
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::HardwareVolume { .. }
            )),
            PlaybackEvent::HardwareVolume {
                level: 0.4,
                muted: false,
            }
        );
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("second.flac"),
            })
            .unwrap();
        wait_for(
            &events,
            |event| matches!(event, PlaybackEvent::NowPlaying { source, .. } if source.path == Path::new("second.flac")),
        );
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        assert_no_matching_event(&events, Duration::from_millis(20), |event| {
            matches!(event, PlaybackEvent::HardwareVolume { .. })
        });
    }

    #[test]
    fn switching_from_controllable_hog_to_shared_reapplies_last_level_as_software_gain() {
        let (controller, log) = fake_controller();
        {
            let mut log = log.lock().unwrap();
            log.hardware_volume = Some((0.5, false));
            log.hardware_volume_settable = true;
        }
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(event, PlaybackEvent::HardwareVolume { .. })
        });
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        commands
            .send(PlaybackCommand::SetOutputDevice {
                device_id: 9,
                kind: EngineKind::Universal {
                    exclusive_mode: false,
                },
            })
            .unwrap();
        wait_for(&events, |event| {
            *event
                == PlaybackEvent::OutputDeviceChanged {
                    device_id: 9,
                    kind: EngineKind::Universal {
                        exclusive_mode: false,
                    },
                }
        });

        assert_eq!(
            log.lock().unwrap().started_volumes,
            [(1.0, false), (0.125, false)]
        );
    }

    #[test]
    fn device_round_trip_reapplies_the_adopted_level_without_readopting() {
        let (controller, log) = fake_controller();
        {
            let mut log = log.lock().unwrap();
            log.hardware_volume = Some((0.5, false));
            log.hardware_volume_settable = true;
        }
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(event, PlaybackEvent::HardwareVolume { .. })
        });
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetVolume {
                level: 0.2,
                muted: false,
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.hardware_volume_writes.last() == Some(&(0.2, false))
        });

        commands
            .send(PlaybackCommand::SetOutputDevice {
                device_id: 9,
                kind: EngineKind::Universal {
                    exclusive_mode: false,
                },
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::OutputDeviceChanged { device_id: 9, .. }
            )
        });
        commands
            .send(PlaybackCommand::SetOutputDevice {
                device_id: 7,
                kind: EngineKind::Universal {
                    exclusive_mode: true,
                },
            })
            .unwrap();
        let mut emitted_hardware_volume = false;
        loop {
            match events.recv_timeout(Duration::from_secs(1)).unwrap() {
                PlaybackEvent::HardwareVolume { .. } => emitted_hardware_volume = true,
                PlaybackEvent::OutputDeviceChanged { device_id: 7, .. } => break,
                _ => {}
            }
        }

        assert!(!emitted_hardware_volume);
        assert_eq!(log.lock().unwrap().hardware_volume, Some((0.2, false)));
    }

    #[test]
    fn coalesces_queued_volume_commands_before_transport() {
        let (command_tx, command_rx) = mpsc::channel();
        command_tx
            .send(PlaybackCommand::SetVolume {
                level: 0.2,
                muted: false,
            })
            .unwrap();
        command_tx
            .send(PlaybackCommand::SetVolume {
                level: 0.7,
                muted: true,
            })
            .unwrap();
        command_tx.send(PlaybackCommand::Pause).unwrap();

        let PlaybackCommand::SetVolume { level, muted } = command_rx.recv().unwrap() else {
            panic!("first queued command must set volume");
        };
        assert_eq!(
            coalesce_volume_commands(&command_rx, level, muted),
            (0.7, true, Some(PlaybackCommand::Pause))
        );
    }

    #[test]
    fn end_of_track_emits_ended_and_supports_stop_from_ended() {
        let (controller, log) = fake_controller();
        log.lock().unwrap().position_limit = u64::MAX;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        let mut ending_events = Vec::new();
        loop {
            let event = events.recv_timeout(Duration::from_secs(1)).unwrap();
            let ended = matches!(event, PlaybackEvent::Ended { .. });
            ending_events.push(event);
            if ended {
                break;
            }
        }
        assert_eq!(
            ending_events,
            [
                PlaybackEvent::Position {
                    position_ms: 2_000,
                    duration_ms: Some(10_000),
                    dropout_frames: 0,
                },
                PlaybackEvent::StateChanged(PlaybackState::Ended),
                PlaybackEvent::Ended { attempt: 1 },
            ]
        );

        commands.send(PlaybackCommand::Resume).unwrap();
        assert_eq!(
            wait_for(&events, |event| {
                matches!(event, PlaybackEvent::CommandRejected { .. })
            }),
            PlaybackEvent::CommandRejected {
                command: "Resume",
                state: PlaybackState::Ended,
            }
        );

        commands.send(PlaybackCommand::Stop).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
        });
        assert_eq!(log.lock().unwrap().stops, 1);
    }

    #[test]
    fn end_of_track_stop_failure_preserves_ended_before_error() {
        let (controller, log) = fake_controller();
        {
            let mut log = log.lock().unwrap();
            log.position_limit = u64::MAX;
            log.stop_error = true;
        }
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();

        wait_for(&events, |event| {
            matches!(event, PlaybackEvent::Ended { .. })
        });
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::Error {
                attempt: 1,
                kind: crate::PlaybackErrorKind::Track,
                message: "decode: backend stop failed".to_string(),
            }
        );
        assert_eq!(log.lock().unwrap().stops, 1);
    }

    #[test]
    fn play_file_while_playing_reuses_backend_for_new_track() {
        let (controller, log) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("first.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });

        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("second.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::NowPlaying { source, .. }
                    if source.path == Path::new("second.flac")
            )
        });
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        let log = log.lock().unwrap();
        assert_eq!(log.opened_devices, [7]);
        assert_eq!(log.stops, 1);
    }

    #[test]
    fn output_device_failure_stops_playback_and_emits_error() {
        let (controller, log) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });
        log.lock().unwrap().fail_all_open_device = Some(9);

        commands
            .send(PlaybackCommand::SetOutputDevice {
                device_id: 9,
                kind: EngineKind::Universal {
                    exclusive_mode: true,
                },
            })
            .unwrap();
        let error = wait_for(&events, |event| {
            matches!(event, PlaybackEvent::Error { .. })
        });

        assert_eq!(
            error,
            PlaybackEvent::Error {
                attempt: 1,
                kind: crate::PlaybackErrorKind::Device { hog_pid: None },
                message: "audio unit: output unavailable".to_string()
            }
        );

        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        let log = log.lock().unwrap();
        assert_eq!(log.opened_devices, [7, 9, 9, 7]);
        assert_eq!(log.stops, 1);
    }

    #[test]
    fn backend_stop_failure_emits_error_instead_of_paused() {
        let (controller, log) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });
        log.lock().unwrap().stop_error = true;

        commands.send(PlaybackCommand::Pause).unwrap();
        let error = wait_for(&events, |event| {
            matches!(event, PlaybackEvent::Error { .. })
        });

        assert_eq!(
            error,
            PlaybackEvent::Error {
                attempt: 1,
                kind: crate::PlaybackErrorKind::Track,
                message: "decode: backend stop failed".to_string()
            }
        );
        assert_eq!(log.lock().unwrap().stops, 1);
    }

    #[test]
    fn dropping_controller_stops_active_playback_with_sender_clone_alive() {
        let (controller, log) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });

        drop(controller);
        assert!(commands.send(PlaybackCommand::Stop).is_err());
        loop {
            match events.recv_timeout(Duration::from_secs(1)) {
                Ok(_) => {}
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    panic!("playback worker did not shut down after command disconnect")
                }
            }
        }

        assert_eq!(log.lock().unwrap().stops, 1);
        assert_eq!(log.lock().unwrap().releases, 1);
    }

    #[test]
    fn dropping_controller_releases_a_backend_retained_by_pause() {
        let (controller, log) = fake_controller_with_kind(EngineKind::BitPerfect);
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_000,
                    ..
                }
            )
        });
        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });

        drop(controller);
        while let Ok(_event) = events.recv_timeout(Duration::from_secs(1)) {}

        let log = log.lock().unwrap();
        assert_eq!(log.stops, 1);
        assert_eq!(log.releases, 1);
    }

    #[test]
    fn same_format_preload_advances_only_at_the_fed_frame_boundary() {
        let (controller, log) = fake_controller();
        configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
        configure_decoder(&log, "b.flac", TEST_FORMAT, 4_000, 2_000);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.backend_fed_frames == 4_000 && log.decoder_eofs.contains(&PathBuf::from("b.flac"))
        });

        log.lock().unwrap().position_limit = 1_999;
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 1_999,
                    ..
                }
            )),
            PlaybackEvent::Position {
                position_ms: 1_999,
                duration_ms: Some(2_000),
                dropout_frames: 0,
            }
        );
        assert_no_matching_event(&events, Duration::from_millis(20), |event| {
            matches!(event, PlaybackEvent::Advanced { .. })
        });

        log.lock().unwrap().position_limit = 2_000;
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Advanced { .. }
            )),
            PlaybackEvent::Advanced {
                attempt: 1,
                source: PlayableSource {
                    path: PathBuf::from("b.flac"),
                    duration_ms: Some(4_000),
                },
                format: TEST_FORMAT,
            }
        );
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::Position {
                position_ms: 0,
                duration_ms: Some(4_000),
                dropout_frames: 0,
            }
        );

        log.lock().unwrap().position_limit = 2_500;
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 500,
                    ..
                }
            )),
            PlaybackEvent::Position {
                position_ms: 500,
                duration_ms: Some(4_000),
                dropout_frames: 0,
            }
        );
        let log = log.lock().unwrap();
        assert_eq!(log.backend_starts, [TEST_FORMAT]);
        assert_eq!(log.stops, 0);
    }

    #[test]
    fn continuous_progress_across_seamless_boundary_does_not_stall() {
        let (controller, log, clock) = fake_controller_with_stall_timeout(TEST_STALL_TIMEOUT);
        configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
        configure_decoder(&log, "b.flac", TEST_FORMAT, 10_000, 10_000);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| log.backend_fed_frames == 12_000);

        let mut position = 0;
        let mut advanced = false;
        for _ in 0..8 {
            position += 500;
            log.lock().unwrap().position_limit = position;
            wait_for_worker_pumps(&clock, 2);
            clock.advance(TEST_STALL_TIMEOUT / 4);
            while let Ok(event) = events.try_recv() {
                match event {
                    PlaybackEvent::Advanced { .. } => advanced = true,
                    PlaybackEvent::Error { message, .. } => {
                        panic!("continuous backend progress stalled: {message}")
                    }
                    _ => {}
                }
            }
        }
        wait_for_worker_pumps(&clock, 2);
        assert_no_error_pending(&events);
        assert!(advanced, "test progress never crossed the audible boundary");

        commands.send(PlaybackCommand::Stop).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
        });
    }

    #[test]
    fn stalled_output_at_seamless_boundary_still_times_out() {
        let (controller, log, clock) = fake_controller_with_stall_timeout(TEST_STALL_TIMEOUT);
        configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
        configure_decoder(&log, "b.flac", TEST_FORMAT, 10_000, 10_000);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| log.backend_fed_frames == 12_000);

        log.lock().unwrap().position_limit = 2_000;
        wait_for(&events, |event| {
            matches!(event, PlaybackEvent::Advanced { .. })
        });
        wait_for_worker_pumps(&clock, 2);
        clock.advance(TEST_STALL_TIMEOUT);
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Error { .. }
            )),
            PlaybackEvent::Error {
                attempt: 1,
                kind: crate::PlaybackErrorKind::Device { hog_pid: None },
                message: "timed out waiting for audio output progress".to_string(),
            }
        );
    }

    #[test]
    fn format_mismatch_rebuilds_backend_and_still_advances() {
        let (controller, log) = fake_controller();
        configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
        configure_decoder(&log, "b.flac", ALT_FORMAT, 1_000, 2_000);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| log.backend_fed_frames == 2_000);

        log.lock().unwrap().position_limit = u64::MAX;
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Advanced { .. }
            )),
            PlaybackEvent::Advanced {
                attempt: 1,
                source: PlayableSource {
                    path: PathBuf::from("b.flac"),
                    duration_ms: Some(1_000),
                },
                format: ALT_FORMAT,
            }
        );
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::Position {
                position_ms: 0,
                duration_ms: Some(1_000),
                dropout_frames: 0,
            }
        );
        let log = log.lock().unwrap();
        assert_eq!(log.backend_starts, [TEST_FORMAT, ALT_FORMAT]);
        assert!(log.stops >= 1);
    }

    #[test]
    fn unreadable_set_next_is_rejected_and_current_playback_continues() {
        let (controller, log) = fake_controller();
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        log.lock()
            .unwrap()
            .unreadable_paths
            .insert(PathBuf::from("bad.flac"));
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("bad.flac"),
            })
            .unwrap();

        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::NextRejected { .. }
            )),
            PlaybackEvent::NextRejected {
                attempt: 1,
                path: PathBuf::from("bad.flac"),
                message: "decode: unreadable source".to_string(),
            }
        );
        assert_no_matching_event(&events, Duration::from_millis(20), |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Error)
        });
        log.lock().unwrap().position_limit = u64::MAX;
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Ended { .. }
            )),
            PlaybackEvent::Ended { attempt: 1 }
        );
    }

    #[test]
    fn redundant_same_path_set_next_reuses_the_existing_preload() {
        let (controller, log) = fake_controller();
        {
            let mut log = log.lock().unwrap();
            log.position_limit = 0;
            log.decoder_starved_after_first_chunk = true;
        }
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.decoder_opens.contains(&PathBuf::from("b.flac"))
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));

        assert_eq!(
            log.lock()
                .unwrap()
                .decoder_opens
                .iter()
                .filter(|path| path.as_path() == Path::new("b.flac"))
                .count(),
            1
        );
    }

    #[test]
    fn clear_next_before_eof_preserves_existing_ended_behavior() {
        let (controller, log) = fake_controller();
        {
            let mut log = log.lock().unwrap();
            log.position_limit = 0;
            log.decoder_starved_after_first_chunk = true;
        }
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        commands.send(PlaybackCommand::ClearNext).unwrap();
        wait_for_log(&log, |log| {
            log.decoder_opens.contains(&PathBuf::from("b.flac"))
        });
        {
            let mut log = log.lock().unwrap();
            log.decoder_starved_after_first_chunk = false;
            log.position_limit = u64::MAX;
        }

        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Ended { .. }
            )),
            PlaybackEvent::Ended { attempt: 1 }
        );
        let log = log.lock().unwrap();
        assert_eq!(log.backend_starts, [TEST_FORMAT]);
        assert_eq!(log.stops, 1);
    }

    #[test]
    fn set_next_during_transition_replaces_the_incoming_tracks_successor() {
        let (controller, log) = fake_controller();
        for path in ["a.flac", "b.flac", "c.flac", "d.flac"] {
            configure_decoder(&log, path, TEST_FORMAT, 2_000, 2_000);
        }
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.decoder_eofs.contains(&PathBuf::from("b.flac"))
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("c.flac"),
            })
            .unwrap();
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("d.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.decoder_opens.contains(&PathBuf::from("d.flac"))
        });

        log.lock().unwrap().position_limit = 2_000;
        assert!(matches!(
            wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac"))),
            PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")
        ));
        log.lock().unwrap().position_limit = 4_000;
        assert!(matches!(
            wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("d.flac"))),
            PlaybackEvent::Advanced { source, .. } if source.path == Path::new("d.flac")
        ));
        assert_no_matching_event(
            &events,
            Duration::from_millis(20),
            |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("c.flac")),
        );
        let log = log.lock().unwrap();
        assert_eq!(log.backend_starts, [TEST_FORMAT]);
        assert_eq!(log.stops, 0);
    }

    #[test]
    fn clear_next_during_transition_keeps_incoming_audio_but_clears_its_successor() {
        let (controller, log) = fake_controller();
        for path in ["a.flac", "b.flac", "c.flac"] {
            configure_decoder(&log, path, TEST_FORMAT, 2_000, 2_000);
        }
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.decoder_eofs.contains(&PathBuf::from("b.flac"))
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("c.flac"),
            })
            .unwrap();
        commands.send(PlaybackCommand::ClearNext).unwrap();

        log.lock().unwrap().position_limit = 2_000;
        assert!(matches!(
            wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac"))),
            PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")
        ));
        log.lock().unwrap().position_limit = 4_000;
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Ended { .. }
            )),
            PlaybackEvent::Ended { attempt: 1 }
        );
        assert_no_matching_event(
            &events,
            Duration::from_millis(20),
            |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("c.flac")),
        );
    }

    #[test]
    fn pause_during_transition_repreloads_incoming_from_zero() {
        let (controller, log) = fake_controller();
        configure_decoder(&log, "a.flac", TEST_FORMAT, 3_000, 2_000);
        configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.decoder_eofs.contains(&PathBuf::from("b.flac"))
        });
        log.lock().unwrap().position_limit = 500;
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 500,
                    ..
                }
            )
        });

        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });
        {
            let log = log.lock().unwrap();
            assert_eq!(
                log.decoder_opens
                    .iter()
                    .filter(|path| path.as_path() == Path::new("b.flac"))
                    .count(),
                2
            );
            assert!(log.seek_positions.is_empty());
        }

        log.lock().unwrap().position_limit = 0;
        commands.send(PlaybackCommand::Resume).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        wait_for_log(&log, |log| log.backend_fed_frames == 4_000);
        log.lock().unwrap().position_limit = 2_000;
        assert!(matches!(
            wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac"))),
            PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")
        ));
        assert_eq!(log.lock().unwrap().seek_positions, [500]);
    }

    #[test]
    fn pause_with_unreadable_buffered_source_is_advisory_and_resume_works() {
        let (controller, log) = fake_controller();
        configure_decoder(&log, "a.flac", TEST_FORMAT, 3_000, 2_000);
        configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.decoder_eofs.contains(&PathBuf::from("b.flac"))
        });
        log.lock().unwrap().position_limit = 500;
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 500,
                    ..
                }
            )
        });
        log.lock()
            .unwrap()
            .unreadable_paths
            .insert(PathBuf::from("b.flac"));

        commands.send(PlaybackCommand::Pause).unwrap();
        let mut next_rejected = false;
        loop {
            match events.recv_timeout(Duration::from_secs(1)).unwrap() {
                PlaybackEvent::NextRejected {
                    attempt,
                    path,
                    message,
                } => {
                    assert_eq!(attempt, 1);
                    assert_eq!(path, PathBuf::from("b.flac"));
                    assert_eq!(message, "decode: unreadable source");
                    next_rejected = true;
                }
                PlaybackEvent::StateChanged(PlaybackState::Paused) => break,
                PlaybackEvent::StateChanged(PlaybackState::Error) => {
                    panic!("unreadable lookahead made pause fatal")
                }
                _ => {}
            }
        }
        assert!(next_rejected);

        log.lock().unwrap().position_limit = 0;
        commands.send(PlaybackCommand::Resume).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        log.lock().unwrap().position_limit = u64::MAX;
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Ended { .. }
            )),
            PlaybackEvent::Ended { attempt: 1 }
        );
    }

    #[test]
    fn pause_after_boundary_crossing_uses_the_incoming_tracks_position() {
        let (controller, log) = fake_controller();
        configure_decoder(&log, "a.flac", TEST_FORMAT, 2_000, 2_000);
        configure_decoder(&log, "b.flac", TEST_FORMAT, 3_000, 2_000);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| log.backend_fed_frames == 4_000);

        log.lock().unwrap().position_limit = 2_500;
        commands.send(PlaybackCommand::Pause).unwrap();
        wait_for(
            &events,
            |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")),
        );
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 500,
                    ..
                }
            )),
            PlaybackEvent::Position {
                position_ms: 500,
                duration_ms: Some(3_000),
                dropout_frames: 0,
            }
        );
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
        });
    }

    #[test]
    fn seek_during_transition_repreloads_incoming_from_zero() {
        let (controller, log) = fake_controller();
        configure_decoder(&log, "a.flac", TEST_FORMAT, 4_000, 2_000);
        configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.decoder_eofs.contains(&PathBuf::from("b.flac"))
        });

        commands
            .send(PlaybackCommand::Seek { position_ms: 1_000 })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        wait_for_log(&log, |log| log.backend_fed_frames == 4_000);
        {
            let log = log.lock().unwrap();
            assert_eq!(log.seek_positions, [1_000]);
            assert_eq!(
                log.decoder_opens
                    .iter()
                    .filter(|path| path.as_path() == Path::new("b.flac"))
                    .count(),
                2
            );
        }
        log.lock().unwrap().position_limit = 2_000;
        assert!(matches!(
            wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac"))),
            PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")
        ));
        let log = log.lock().unwrap();
        assert_eq!(log.backend_starts, [TEST_FORMAT, TEST_FORMAT]);
        assert_eq!(log.stops, 1);
    }

    #[test]
    fn output_device_change_drops_buffered_transition_and_successor() {
        let (controller, log) = fake_controller();
        for path in ["a.flac", "b.flac", "c.flac"] {
            configure_decoder(&log, path, TEST_FORMAT, 3_000, 2_000);
        }
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.decoder_eofs.contains(&PathBuf::from("b.flac"))
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("c.flac"),
            })
            .unwrap();
        log.lock().unwrap().position_limit = 500;
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 500,
                    ..
                }
            )
        });

        commands
            .send(PlaybackCommand::SetOutputDevice {
                device_id: 9,
                kind: EngineKind::Universal {
                    exclusive_mode: false,
                },
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::OutputDeviceChanged { device_id: 9, .. }
            )
        });
        {
            let log = log.lock().unwrap();
            assert_eq!(log.backend_starts, [TEST_FORMAT, TEST_FORMAT]);
            assert_eq!(log.stops, 1);
            assert_eq!(log.seek_positions, [500]);
            assert_eq!(
                log.decoder_opens
                    .iter()
                    .filter(|path| path.as_path() == Path::new("b.flac"))
                    .count(),
                1
            );
        }

        log.lock().unwrap().position_limit = u64::MAX;
        loop {
            match events.recv_timeout(Duration::from_secs(1)).unwrap() {
                PlaybackEvent::Advanced { source, .. } => {
                    panic!("unexpected advance to {}", source.path.display())
                }
                PlaybackEvent::Ended { attempt } => {
                    assert_eq!(attempt, 1);
                    break;
                }
                _ => {}
            }
        }
    }

    #[test]
    fn unchanged_output_device_keeps_buffered_transition_without_restart() {
        let (controller, log) = fake_controller();
        configure_decoder(&log, "a.flac", TEST_FORMAT, 3_000, 2_000);
        configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.decoder_eofs.contains(&PathBuf::from("b.flac"))
        });

        commands
            .send(PlaybackCommand::SetOutputDevice {
                device_id: 7,
                kind: EngineKind::Universal {
                    exclusive_mode: true,
                },
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::OutputDeviceChanged { device_id: 7, .. }
            )
        });
        {
            let log = log.lock().unwrap();
            assert_eq!(log.backend_starts, [TEST_FORMAT]);
            assert_eq!(log.stops, 0);
            assert!(log.seek_positions.is_empty());
        }

        log.lock().unwrap().position_limit = 2_000;
        assert!(matches!(
            wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac"))),
            PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")
        ));
    }

    #[test]
    fn exclusive_mode_change_drops_buffered_transition() {
        let (controller, log) = fake_controller();
        configure_decoder(&log, "a.flac", TEST_FORMAT, 3_000, 2_000);
        configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.decoder_eofs.contains(&PathBuf::from("b.flac"))
        });
        log.lock().unwrap().position_limit = 500;
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::Position {
                    position_ms: 500,
                    ..
                }
            )
        });

        commands
            .send(PlaybackCommand::SetExclusiveMode { enabled: false })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        {
            let log = log.lock().unwrap();
            assert_eq!(log.backend_starts, [TEST_FORMAT, TEST_FORMAT]);
            assert_eq!(log.stops, 1);
            assert_eq!(log.seek_positions, [500]);
            assert_eq!(
                log.decoder_opens
                    .iter()
                    .filter(|path| path.as_path() == Path::new("b.flac"))
                    .count(),
                1
            );
        }

        log.lock().unwrap().position_limit = u64::MAX;
        loop {
            match events.recv_timeout(Duration::from_secs(1)).unwrap() {
                PlaybackEvent::Advanced { source, .. } => {
                    panic!("unexpected advance to {}", source.path.display())
                }
                PlaybackEvent::Ended { attempt } => {
                    assert_eq!(attempt, 1);
                    break;
                }
                _ => {}
            }
        }
    }

    #[test]
    fn unchanged_exclusive_mode_keeps_buffered_transition_without_restart() {
        let (controller, log) = fake_controller();
        configure_decoder(&log, "a.flac", TEST_FORMAT, 3_000, 2_000);
        configure_decoder(&log, "b.flac", TEST_FORMAT, 2_000, 2_000);
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.decoder_eofs.contains(&PathBuf::from("b.flac"))
        });

        commands
            .send(PlaybackCommand::SetExclusiveMode { enabled: true })
            .unwrap();
        commands
            .send(PlaybackCommand::SetOutputDevice {
                device_id: 7,
                kind: EngineKind::Universal {
                    exclusive_mode: true,
                },
            })
            .unwrap();
        wait_for(&events, |event| {
            matches!(
                event,
                PlaybackEvent::OutputDeviceChanged { device_id: 7, .. }
            )
        });
        {
            let log = log.lock().unwrap();
            assert_eq!(log.backend_starts, [TEST_FORMAT]);
            assert_eq!(log.stops, 0);
            assert!(log.seek_positions.is_empty());
        }

        log.lock().unwrap().position_limit = 2_000;
        assert!(matches!(
            wait_for(&events, |event| matches!(event, PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac"))),
            PlaybackEvent::Advanced { source, .. } if source.path == Path::new("b.flac")
        ));
    }

    #[test]
    fn stop_drops_buffered_transition_before_a_later_play() {
        let (controller, log) = fake_controller();
        for path in ["a.flac", "b.flac", "c.flac"] {
            configure_decoder(&log, path, TEST_FORMAT, 2_000, 2_000);
        }
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.decoder_eofs.contains(&PathBuf::from("b.flac"))
        });

        commands.send(PlaybackCommand::Stop).unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Idle)
        });
        log.lock().unwrap().position_limit = u64::MAX;
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("c.flac"),
            })
            .unwrap();
        loop {
            match events.recv_timeout(Duration::from_secs(1)).unwrap() {
                PlaybackEvent::Advanced { source, .. } => {
                    panic!("unexpected advance to {}", source.path.display())
                }
                PlaybackEvent::Ended { attempt } => {
                    assert_eq!(attempt, 2);
                    break;
                }
                _ => {}
            }
        }
    }

    #[test]
    fn play_file_drops_buffered_transition() {
        let (controller, log) = fake_controller();
        for path in ["a.flac", "b.flac", "c.flac"] {
            configure_decoder(&log, path, TEST_FORMAT, 2_000, 2_000);
        }
        log.lock().unwrap().position_limit = 0;
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("a.flac"),
            })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        wait_for_log(&log, |log| {
            log.decoder_eofs.contains(&PathBuf::from("b.flac"))
        });

        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("c.flac"),
            })
            .unwrap();
        wait_for(
            &events,
            |event| matches!(event, PlaybackEvent::NowPlaying { source, .. } if source.path == Path::new("c.flac")),
        );
        log.lock().unwrap().position_limit = u64::MAX;
        loop {
            match events.recv_timeout(Duration::from_secs(1)).unwrap() {
                PlaybackEvent::Advanced { source, .. } => {
                    panic!("unexpected advance to {}", source.path.display())
                }
                PlaybackEvent::Ended { attempt } => {
                    assert_eq!(attempt, 2);
                    break;
                }
                _ => {}
            }
        }
        let log = log.lock().unwrap();
        assert_eq!(log.backend_starts, [TEST_FORMAT, TEST_FORMAT]);
        assert_eq!(log.stops, 2);
    }

    #[test]
    fn next_source_commands_are_rejected_when_nothing_can_follow() {
        let (controller, _) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::SetNext {
                path: PathBuf::from("b.flac"),
            })
            .unwrap();
        commands.send(PlaybackCommand::ClearNext).unwrap();

        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::CommandRejected {
                    command: "SetNext",
                    ..
                }
            )),
            PlaybackEvent::CommandRejected {
                command: "SetNext",
                state: PlaybackState::Idle,
            }
        );
        assert_eq!(
            wait_for(&events, |event| matches!(
                event,
                PlaybackEvent::CommandRejected {
                    command: "ClearNext",
                    ..
                }
            )),
            PlaybackEvent::CommandRejected {
                command: "ClearNext",
                state: PlaybackState::Idle,
            }
        );
    }

    #[test]
    fn a_panicking_worker_disconnects_event_subscribers() {
        let log = Arc::new(Mutex::new(FakeLog::default()));
        let backend_log = Arc::clone(&log);
        let controller = PlaybackController::spawn_with_dependencies(
            7,
            EngineKind::Universal {
                exclusive_mode: true,
            },
            Arc::new(move |device_id, engine_kind| {
                let EngineKind::Universal { exclusive_mode } = engine_kind else {
                    panic!("test factory expected universal engine")
                };
                Ok(Box::new(FakeBackend {
                    log: Arc::clone(&backend_log),
                    device_id,
                    exclusive_mode,
                    retains_device: false,
                    fed_frames: 0,
                    format: None,
                    volume: (f32::NAN, false),
                    hardware_volume: None,
                    hardware_volume_active: false,
                    hardware_volume_event_pending: false,
                }))
            }),
            Arc::new(|_| panic!("decoder factory exploded")),
            Duration::MAX,
            Box::new(Instant::now),
        );
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::PlayFile {
                path: PathBuf::from("track.flac"),
            })
            .unwrap();

        loop {
            match events.recv_timeout(Duration::from_secs(2)) {
                Ok(_) => {}
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    panic!("subscribers were not disconnected after a worker panic")
                }
            }
        }
    }

    fn fake_controller() -> (PlaybackController, Arc<Mutex<FakeLog>>) {
        fake_controller_with_exclusive_mode_and_seek_offset(true, 0)
    }

    fn fake_controller_with_exclusive_mode(
        exclusive_mode: bool,
    ) -> (PlaybackController, Arc<Mutex<FakeLog>>) {
        fake_controller_with_exclusive_mode_and_seek_offset(exclusive_mode, 0)
    }

    fn fake_controller_with_seek_offset(
        seek_offset_ms: u64,
    ) -> (PlaybackController, Arc<Mutex<FakeLog>>) {
        fake_controller_with_exclusive_mode_and_seek_offset(true, seek_offset_ms)
    }

    fn fake_controller_with_stall_timeout(
        output_stall_timeout: Duration,
    ) -> (PlaybackController, Arc<Mutex<FakeLog>>, FakeClock) {
        let clock = FakeClock::new();
        let worker_clock = clock.clone();
        let (controller, log) = fake_controller_with_options_and_clock(
            true,
            0,
            output_stall_timeout,
            Box::new(move || worker_clock.now()),
        );
        (controller, log, clock)
    }

    fn fake_controller_with_exclusive_mode_and_seek_offset(
        exclusive_mode: bool,
        seek_offset_ms: u64,
    ) -> (PlaybackController, Arc<Mutex<FakeLog>>) {
        fake_controller_with_options(exclusive_mode, seek_offset_ms, Duration::MAX)
    }

    fn fake_controller_with_options(
        exclusive_mode: bool,
        seek_offset_ms: u64,
        output_stall_timeout: Duration,
    ) -> (PlaybackController, Arc<Mutex<FakeLog>>) {
        fake_controller_with_options_and_clock(
            exclusive_mode,
            seek_offset_ms,
            output_stall_timeout,
            Box::new(Instant::now),
        )
    }

    fn fake_controller_with_options_and_clock(
        exclusive_mode: bool,
        seek_offset_ms: u64,
        output_stall_timeout: Duration,
        now: Clock,
    ) -> (PlaybackController, Arc<Mutex<FakeLog>>) {
        fake_controller_with_kind_and_options_clock(
            EngineKind::Universal { exclusive_mode },
            seek_offset_ms,
            output_stall_timeout,
            now,
        )
    }

    fn fake_controller_with_kind(
        engine_kind: EngineKind,
    ) -> (PlaybackController, Arc<Mutex<FakeLog>>) {
        fake_controller_with_kind_and_options_clock(
            engine_kind,
            0,
            Duration::MAX,
            Box::new(Instant::now),
        )
    }

    fn fake_controller_with_kind_and_options_clock(
        engine_kind: EngineKind,
        seek_offset_ms: u64,
        output_stall_timeout: Duration,
        now: Clock,
    ) -> (PlaybackController, Arc<Mutex<FakeLog>>) {
        let log = Arc::new(Mutex::new(FakeLog::default()));
        let backend_log = Arc::clone(&log);
        let decoder_log = Arc::clone(&log);
        let controller = PlaybackController::spawn_with_dependencies(
            7,
            engine_kind,
            Arc::new(move |device_id, engine_kind| {
                let (exclusive_mode, retains_device) = match engine_kind {
                    EngineKind::Universal { exclusive_mode } => (exclusive_mode, false),
                    EngineKind::BitPerfect => (true, true),
                };
                let (fail_all_open, fail_exclusive_open, hardware_volume, hardware_volume_settable) = {
                    let mut log = backend_log.lock().unwrap();
                    log.opened_devices.push(device_id);
                    log.engine_kinds.push(engine_kind);
                    log.exclusive_modes.push(exclusive_mode);
                    (
                        log.fail_all_open_device == Some(device_id),
                        exclusive_mode && log.fail_exclusive_open_device == Some(device_id),
                        log.hardware_volume,
                        log.hardware_volume_settable,
                    )
                };
                if fail_all_open {
                    return Err(EngineError::AudioUnit("output unavailable".to_string()));
                }
                if fail_exclusive_open {
                    return Err(EngineError::Hogged(42));
                }
                let hardware_volume_active =
                    exclusive_mode && hardware_volume_settable && hardware_volume.is_some();
                Ok(Box::new(FakeBackend {
                    log: Arc::clone(&backend_log),
                    device_id,
                    exclusive_mode,
                    retains_device,
                    fed_frames: 0,
                    format: None,
                    volume: (f32::NAN, false),
                    hardware_volume,
                    hardware_volume_active,
                    hardware_volume_event_pending: hardware_volume_active,
                }))
            }),
            Arc::new(move |path| {
                let spec = {
                    let mut log = decoder_log.lock().unwrap();
                    log.decoder_opens.push(path.to_path_buf());
                    if log.unreadable_paths.contains(path) {
                        return Err(EngineError::Decode("unreadable source".to_string()));
                    }
                    log.decoder_specs.get(path).copied().unwrap_or_default()
                };
                Ok(Box::new(FakeDecoder {
                    log: Arc::clone(&decoder_log),
                    path: path.to_path_buf(),
                    format: spec.format,
                    duration_ms: spec.duration_ms,
                    frames: spec.frames,
                    emitted: false,
                    seek_offset_ms,
                    seek_pending_frames: spec.seek_pending_frames,
                    pending_frames: 0,
                }))
            }),
            output_stall_timeout,
            now,
        );
        (controller, log)
    }

    fn wait_for(
        events: &Receiver<PlaybackEvent>,
        predicate: impl Fn(&PlaybackEvent) -> bool,
    ) -> PlaybackEvent {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match events.recv_timeout(remaining) {
                Ok(event) if predicate(&event) => return event,
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for playback event"),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("playback event channel disconnected")
                }
            }
        }
    }

    fn configure_decoder(
        log: &Arc<Mutex<FakeLog>>,
        path: &str,
        format: PcmFormat,
        duration_ms: u64,
        frames: u64,
    ) {
        log.lock().unwrap().decoder_specs.insert(
            PathBuf::from(path),
            FakeDecoderSpec {
                format,
                duration_ms: Some(duration_ms),
                frames,
                seek_pending_frames: 0,
            },
        );
    }

    fn wait_for_log(log: &Arc<Mutex<FakeLog>>, predicate: impl Fn(&FakeLog) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if predicate(&log.lock().unwrap()) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for fake log");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn wait_for_worker_pumps(clock: &FakeClock, count: usize) {
        for _ in 0..count {
            let reads = clock.reads();
            clock.wait_for_read_after(reads);
        }
    }

    fn assert_no_matching_event(
        events: &Receiver<PlaybackEvent>,
        duration: Duration,
        predicate: impl Fn(&PlaybackEvent) -> bool,
    ) {
        let deadline = Instant::now() + duration;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match events.recv_timeout(remaining) {
                Ok(event) => assert!(!predicate(&event), "unexpected playback event: {event:?}"),
                Err(RecvTimeoutError::Timeout) => return,
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("playback event channel disconnected")
                }
            }
        }
    }

    fn assert_no_error_pending(events: &Receiver<PlaybackEvent>) {
        while let Ok(event) = events.try_recv() {
            if let PlaybackEvent::Error { message, .. } = event {
                panic!("unexpected playback error: {message}");
            }
        }
    }
}
