use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::{
    Engine, EngineError, PcmFormat, PlayableSource, PlaybackCommand, PlaybackEvent, PlaybackState,
    decode::PcmDecoder, device::DeviceId,
};

const POSITION_EVENT_INTERVAL_MS: u64 = 100;
const FEED_RETRY_DELAY: Duration = Duration::from_millis(2);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(100);

type BackendFactory =
    Arc<dyn Fn(DeviceId, bool) -> Result<Box<dyn PlaybackBackend>, EngineError> + Send + Sync>;
type DecoderFactory =
    Arc<dyn Fn(&Path) -> Result<Box<dyn SourceDecoder>, EngineError> + Send + Sync>;

pub struct PlaybackController {
    command_tx: Sender<PlaybackCommand>,
    subscribers: EventSubscribers,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl PlaybackController {
    pub fn spawn(output_device: DeviceId, exclusive_mode: bool) -> Self {
        Self::spawn_with_dependencies(
            output_device,
            exclusive_mode,
            Arc::new(|device_id, exclusive_mode| {
                Ok(Box::new(EngineBackend::open(device_id, exclusive_mode)?))
            }),
            Arc::new(|path| Ok(Box::new(PcmDecoder::open(path)?))),
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
        exclusive_mode: bool,
        backend_factory: BackendFactory,
        decoder_factory: DecoderFactory,
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
                        output_device,
                        exclusive_mode,
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
    fn set_volume(&mut self, gain: f32, muted: bool);
    fn stop(&mut self) -> Result<(), EngineError>;
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

    fn set_volume(&mut self, gain: f32, muted: bool) {
        self.engine.set_volume(gain, muted);
    }

    fn stop(&mut self) -> Result<(), EngineError> {
        self.engine.pause()
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
}

struct ActivePlayback {
    decoder: Box<dyn SourceDecoder>,
    base_position_ms: u64,
    pcm: Vec<u8>,
    pcm_offset: usize,
    fed_frames: u64,
    decoder_finished: bool,
    last_reported_position_ms: u64,
}

struct PreparedDecoder {
    path: PathBuf,
    requested_position_ms: u64,
    actual_position_ms: u64,
    decoder: Box<dyn SourceDecoder>,
}

struct Worker {
    state: PlaybackState,
    /// Count of PlayFile commands processed; stamped on terminal events.
    attempt: u64,
    output_device: DeviceId,
    exclusive_mode: bool,
    volume_gain: f32,
    muted: bool,
    current: Option<CurrentTrack>,
    active: Option<ActivePlayback>,
    prepared_decoder: Option<PreparedDecoder>,
    backend: Option<(DeviceId, bool, Box<dyn PlaybackBackend>)>,
    command_rx: Receiver<PlaybackCommand>,
    subscribers: EventSubscribers,
    backend_factory: BackendFactory,
    decoder_factory: DecoderFactory,
    shutdown: Arc<AtomicBool>,
}

impl Worker {
    fn new(
        output_device: DeviceId,
        exclusive_mode: bool,
        command_rx: Receiver<PlaybackCommand>,
        subscribers: EventSubscribers,
        backend_factory: BackendFactory,
        decoder_factory: DecoderFactory,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            state: PlaybackState::Idle,
            attempt: 0,
            output_device,
            exclusive_mode,
            volume_gain: 1.0,
            muted: false,
            current: None,
            active: None,
            prepared_decoder: None,
            backend: None,
            command_rx,
            subscribers,
            backend_factory,
            decoder_factory,
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

        let _ = self.release_backend();
    }

    fn handle_command(&mut self, command: PlaybackCommand) {
        let result = match command {
            PlaybackCommand::PlayFile { path } => self.play_file(path),
            PlaybackCommand::Pause => self.pause(),
            PlaybackCommand::Resume => self.resume(),
            PlaybackCommand::Seek { position_ms } => self.seek(position_ms),
            PlaybackCommand::Stop => self.stop(),
            PlaybackCommand::SetOutputDevice { device_id } => self.set_output_device(device_id),
            PlaybackCommand::SetExclusiveMode { enabled } => self.set_exclusive_mode(enabled),
            PlaybackCommand::SetVolume { gain, muted } => {
                self.set_volume(gain, muted);
                Ok(())
            }
        };

        if let Err(error) = result {
            self.fail(error);
        }
    }

    fn play_file(&mut self, path: PathBuf) -> Result<(), EngineError> {
        self.attempt += 1;
        self.stop_active()?;
        self.prepared_decoder = None;
        self.current = None;
        self.set_state(PlaybackState::Loading);
        self.start_path(&path, 0, true, false)
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
        self.release_backend()?;
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
        self.stop_active()?;
        self.start_path(&path, requested_position_ms, false, true)
    }

    fn stop(&mut self) -> Result<(), EngineError> {
        if self.state == PlaybackState::Idle {
            return Ok(());
        }

        self.set_state(PlaybackState::Stopping);
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

    fn set_output_device(&mut self, device_id: DeviceId) -> Result<(), EngineError> {
        if self.output_device == device_id {
            self.broadcast(PlaybackEvent::OutputDeviceChanged { device_id });
            return Ok(());
        }

        if self.state == PlaybackState::Playing {
            let position_ms = self.logical_position_ms();
            let path = self.current_path()?;
            self.set_state(PlaybackState::Loading);
            self.release_backend()?;
            let previous_device = self.output_device;
            self.output_device = device_id;
            if let Err(error) = self.start_path(&path, position_ms, false, true) {
                self.output_device = previous_device;
                return Err(error);
            }
            self.broadcast(PlaybackEvent::OutputDeviceChanged { device_id });
            return Ok(());
        }

        if self.state == PlaybackState::Paused {
            self.release_backend()?;
        }

        self.output_device = device_id;
        self.broadcast(PlaybackEvent::OutputDeviceChanged { device_id });
        Ok(())
    }

    fn set_exclusive_mode(&mut self, enabled: bool) -> Result<(), EngineError> {
        if self.exclusive_mode == enabled {
            return Ok(());
        }

        if self.state == PlaybackState::Playing {
            let position_ms = self.logical_position_ms();
            let path = self.current_path()?;
            self.set_state(PlaybackState::Loading);
            self.exclusive_mode = enabled;
            self.release_backend()?;
            self.start_path(&path, position_ms, false, true)?;
            return Ok(());
        }

        self.exclusive_mode = enabled;
        if self.state == PlaybackState::Paused {
            self.release_backend()?;
        }
        Ok(())
    }

    fn set_volume(&mut self, gain: f32, muted: bool) {
        self.volume_gain = gain;
        self.muted = muted;
        if let Some((_, _, backend)) = &mut self.backend {
            backend.set_volume(gain, muted);
        }
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

        let mut backend = match self
            .backend
            .take()
            .filter(|(device_id, exclusive_mode, _)| {
                *device_id == self.output_device && *exclusive_mode == self.exclusive_mode
            }) {
            Some((_, _, backend)) => backend,
            None => (self.backend_factory)(self.output_device, self.exclusive_mode)?,
        };
        backend.set_volume(self.volume_gain, self.muted);
        backend.start(format)?;
        self.backend = Some((self.output_device, self.exclusive_mode, backend));

        let source = PlayableSource {
            path: path.to_path_buf(),
            duration_ms,
        };
        self.current = Some(CurrentTrack {
            source: source.clone(),
            format,
            position_ms: actual_position_ms,
            resume_position_ms: actual_position_ms,
        });
        self.active = Some(ActivePlayback {
            decoder,
            base_position_ms: actual_position_ms,
            pcm: Vec::new(),
            pcm_offset: 0,
            fed_frames: 0,
            decoder_finished: false,
            last_reported_position_ms: actual_position_ms,
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
                Some(_) => {
                    active.pcm_offset = 0;
                    made_progress = true;
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

        let position_ms = self.logical_position_ms();
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

        let backend_position = self
            .backend
            .as_ref()
            .expect("active playback must have a backend")
            .2
            .position();
        let finished = self.active.as_ref().is_some_and(|active| {
            active.decoder_finished
                && active.pcm_offset == active.pcm.len()
                && backend_position >= active.fed_frames
        });
        if finished {
            self.finish_playback();
            made_progress = true;
        }

        Ok(made_progress)
    }

    fn finish_playback(&mut self) {
        let position_ms = self.logical_position_ms();
        let should_emit_position = self
            .active
            .as_ref()
            .is_none_or(|active| active.last_reported_position_ms != position_ms);
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
        let _ = self.release_backend();
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
            let (device_id, exclusive_mode, mut backend) = self
                .backend
                .take()
                .expect("active playback must have a backend");
            backend.stop()?;
            self.backend = Some((device_id, exclusive_mode, backend));
        }
        Ok(())
    }

    fn release_backend(&mut self) -> Result<(), EngineError> {
        let was_active = self.active.take().is_some();
        let Some((_, _, mut backend)) = self.backend.take() else {
            return Ok(());
        };
        if was_active {
            backend.stop()?;
        }
        Ok(())
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

    fn logical_position_ms(&self) -> u64 {
        let Some(active) = &self.active else {
            return self
                .current
                .as_ref()
                .map_or(0, |current| current.position_ms);
        };
        let format = self
            .current
            .as_ref()
            .expect("active playback must have a current track")
            .format;
        let position_ms = active.base_position_ms.saturating_add(frames_to_ms(
            self.backend
                .as_ref()
                .expect("active playback must have a backend")
                .2
                .position(),
            format.sample_rate,
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

    fn emit_position(&self, position_ms: u64) {
        self.broadcast(PlaybackEvent::Position {
            position_ms,
            duration_ms: self
                .current
                .as_ref()
                .and_then(|current| current.source.duration_ms),
        });
    }

    fn broadcast(&self, event: PlaybackEvent) {
        self.subscribers
            .lock()
            .expect("playback event subscribers mutex poisoned")
            .retain(|subscriber| subscriber.send(event.clone()).is_ok());
    }
}

fn frames_to_ms(frames: u64, sample_rate: u32) -> u64 {
    frames.saturating_mul(1_000) / u64::from(sample_rate)
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::mpsc::RecvTimeoutError,
        time::{Duration, Instant},
    };

    use super::*;

    struct FakeLog {
        opened_devices: Vec<DeviceId>,
        exclusive_modes: Vec<bool>,
        seek_positions: Vec<u64>,
        started_volumes: Vec<(f32, bool)>,
        stops: usize,
        fail_open_device: Option<DeviceId>,
        stop_error: bool,
        position_limit: u64,
    }

    impl Default for FakeLog {
        fn default() -> Self {
            Self {
                opened_devices: Vec::new(),
                exclusive_modes: Vec::new(),
                seek_positions: Vec::new(),
                started_volumes: Vec::new(),
                stops: 0,
                fail_open_device: None,
                stop_error: false,
                position_limit: 1_000,
            }
        }
    }

    struct FakeBackend {
        log: Arc<Mutex<FakeLog>>,
        fed_frames: u64,
        volume: (f32, bool),
    }

    impl PlaybackBackend for FakeBackend {
        fn start(&mut self, _format: PcmFormat) -> Result<(), EngineError> {
            self.fed_frames = 0;
            self.log.lock().unwrap().started_volumes.push(self.volume);
            Ok(())
        }

        fn feed(&mut self, pcm: &[u8]) -> usize {
            let frames = pcm.len() / TEST_FORMAT.bytes_per_frame();
            self.fed_frames += frames as u64;
            frames
        }

        fn position(&self) -> u64 {
            self.fed_frames.min(self.log.lock().unwrap().position_limit)
        }

        fn set_volume(&mut self, gain: f32, muted: bool) {
            self.volume = (gain, muted);
        }

        fn stop(&mut self) -> Result<(), EngineError> {
            let mut log = self.log.lock().unwrap();
            log.stops += 1;
            if log.stop_error {
                Err(EngineError::Decode("backend stop failed".to_string()))
            } else {
                Ok(())
            }
        }
    }

    struct FakeDecoder {
        log: Arc<Mutex<FakeLog>>,
        emitted: bool,
        seek_offset_ms: u64,
    }

    impl SourceDecoder for FakeDecoder {
        fn format(&self) -> PcmFormat {
            TEST_FORMAT
        }

        fn duration_ms(&self) -> Option<u64> {
            Some(10_000)
        }

        fn seek(&mut self, position_ms: u64) -> Result<u64, EngineError> {
            self.log.lock().unwrap().seek_positions.push(position_ms);
            Ok(position_ms.saturating_sub(self.seek_offset_ms))
        }

        fn next_pcm(&mut self, pcm: &mut Vec<u8>) -> Result<Option<u64>, EngineError> {
            if self.emitted {
                return Ok(None);
            }
            pcm.resize(2_000 * TEST_FORMAT.bytes_per_frame(), 0);
            self.emitted = true;
            Ok(Some(2_000))
        }
    }

    const TEST_FORMAT: PcmFormat = PcmFormat {
        sample_rate: 1_000,
        bits_per_sample: 16,
        channels: 2,
    };

    #[test]
    fn pause_releases_backend_while_seek_reuses_resumed_backend() {
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
            .send(PlaybackCommand::SetOutputDevice { device_id: 9 })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });
        assert_eq!(
            events.recv_timeout(Duration::from_secs(1)).unwrap(),
            PlaybackEvent::OutputDeviceChanged { device_id: 9 }
        );

        let log = log.lock().unwrap();
        assert_eq!(log.opened_devices, [7, 9]);
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
    fn volume_survives_pause_resume_seek_track_changes_and_device_switches() {
        let (controller, log) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();
        commands
            .send(PlaybackCommand::SetVolume {
                gain: 0.25,
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
            .send(PlaybackCommand::SetOutputDevice { device_id: 9 })
            .unwrap();
        wait_for(&events, |event| {
            *event == PlaybackEvent::OutputDeviceChanged { device_id: 9 }
        });

        assert_eq!(log.lock().unwrap().started_volumes, [(0.25, true); 5]);
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
        log.lock().unwrap().fail_open_device = Some(9);

        commands
            .send(PlaybackCommand::SetOutputDevice { device_id: 9 })
            .unwrap();
        let error = wait_for(&events, |event| {
            matches!(event, PlaybackEvent::Error { .. })
        });

        assert_eq!(
            error,
            PlaybackEvent::Error {
                attempt: 1,
                kind: crate::PlaybackErrorKind::Device { hog_pid: Some(42) },
                message: "device hogged by pid 42".to_string()
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
        assert_eq!(log.opened_devices, [7, 9, 7]);
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
    }

    #[test]
    fn a_panicking_worker_disconnects_event_subscribers() {
        let log = Arc::new(Mutex::new(FakeLog::default()));
        let backend_log = Arc::clone(&log);
        let controller = PlaybackController::spawn_with_dependencies(
            7,
            true,
            Arc::new(move |_, _| {
                Ok(Box::new(FakeBackend {
                    log: Arc::clone(&backend_log),
                    fed_frames: 0,
                    volume: (f32::NAN, false),
                }))
            }),
            Arc::new(|_| panic!("decoder factory exploded")),
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
        fake_controller_with_seek_offset(0)
    }

    fn fake_controller_with_seek_offset(
        seek_offset_ms: u64,
    ) -> (PlaybackController, Arc<Mutex<FakeLog>>) {
        let log = Arc::new(Mutex::new(FakeLog::default()));
        let backend_log = Arc::clone(&log);
        let decoder_log = Arc::clone(&log);
        let controller = PlaybackController::spawn_with_dependencies(
            7,
            true,
            Arc::new(move |device_id, exclusive_mode| {
                let fail_open = {
                    let mut log = backend_log.lock().unwrap();
                    log.opened_devices.push(device_id);
                    log.exclusive_modes.push(exclusive_mode);
                    log.fail_open_device == Some(device_id)
                };
                if fail_open {
                    return Err(EngineError::Hogged(42));
                }
                Ok(Box::new(FakeBackend {
                    log: Arc::clone(&backend_log),
                    fed_frames: 0,
                    volume: (f32::NAN, false),
                }))
            }),
            Arc::new(move |_| {
                Ok(Box::new(FakeDecoder {
                    log: Arc::clone(&decoder_log),
                    emitted: false,
                    seek_offset_ms,
                }))
            }),
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
}
