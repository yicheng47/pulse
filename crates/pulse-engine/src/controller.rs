use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread,
    time::Duration,
};

use crate::{
    Engine, EngineError, PcmFormat, PlayableSource, PlaybackCommand, PlaybackEvent, PlaybackState,
    decode::PcmDecoder, device::DeviceId,
};

const POSITION_EVENT_INTERVAL_MS: u64 = 100;
const FEED_RETRY_DELAY: Duration = Duration::from_millis(2);

type BackendFactory =
    Arc<dyn Fn(DeviceId) -> Result<Box<dyn PlaybackBackend>, EngineError> + Send + Sync>;
type DecoderFactory =
    Arc<dyn Fn(&Path) -> Result<Box<dyn SourceDecoder>, EngineError> + Send + Sync>;

pub struct PlaybackController {
    command_tx: Sender<PlaybackCommand>,
    subscribers: EventSubscribers,
}

impl PlaybackController {
    pub fn spawn(output_device: DeviceId) -> Self {
        Self::spawn_with_dependencies(
            output_device,
            Arc::new(|device_id| Ok(Box::new(EngineBackend::open(device_id)?))),
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
        backend_factory: BackendFactory,
        decoder_factory: DecoderFactory,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let worker_subscribers = Arc::clone(&subscribers);

        thread::Builder::new()
            .name("pulse-playback-controller".to_string())
            .spawn(move || {
                Worker::new(
                    output_device,
                    command_rx,
                    worker_subscribers,
                    backend_factory,
                    decoder_factory,
                )
                .run();
            })
            .expect("failed to spawn playback controller worker");

        Self {
            command_tx,
            subscribers,
        }
    }
}

type EventSubscribers = Arc<Mutex<Vec<Sender<PlaybackEvent>>>>;

trait PlaybackBackend {
    fn start(&mut self, format: PcmFormat) -> Result<(), EngineError>;
    fn feed(&mut self, pcm: &[u8]) -> usize;
    fn position(&self) -> u64;
    fn stop(&mut self) -> Result<(), EngineError>;
}

struct EngineBackend {
    engine: Engine,
}

impl EngineBackend {
    fn open(device_id: DeviceId) -> Result<Self, EngineError> {
        Ok(Self {
            engine: Engine::open(device_id)?,
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
}

struct ActivePlayback {
    decoder: Box<dyn SourceDecoder>,
    backend: Box<dyn PlaybackBackend>,
    base_position_ms: u64,
    pcm: Vec<u8>,
    pcm_offset: usize,
    fed_frames: u64,
    decoder_finished: bool,
    last_reported_position_ms: u64,
}

struct Worker {
    state: PlaybackState,
    output_device: DeviceId,
    current: Option<CurrentTrack>,
    active: Option<ActivePlayback>,
    command_rx: Receiver<PlaybackCommand>,
    subscribers: EventSubscribers,
    backend_factory: BackendFactory,
    decoder_factory: DecoderFactory,
}

impl Worker {
    fn new(
        output_device: DeviceId,
        command_rx: Receiver<PlaybackCommand>,
        subscribers: EventSubscribers,
        backend_factory: BackendFactory,
        decoder_factory: DecoderFactory,
    ) -> Self {
        Self {
            state: PlaybackState::Idle,
            output_device,
            current: None,
            active: None,
            command_rx,
            subscribers,
            backend_factory,
            decoder_factory,
        }
    }

    fn run(mut self) {
        loop {
            if self.active.is_some() {
                match self.command_rx.try_recv() {
                    Ok(command) => self.handle_command(command),
                    Err(TryRecvError::Empty) => {
                        if let Err(error) = self.pump() {
                            self.fail(error);
                        }
                        thread::sleep(FEED_RETRY_DELAY);
                    }
                    Err(TryRecvError::Disconnected) => break,
                }
            } else {
                match self.command_rx.recv() {
                    Ok(command) => self.handle_command(command),
                    Err(_) => break,
                }
            }
        }

        self.stop_active();
    }

    fn handle_command(&mut self, command: PlaybackCommand) {
        let result = match command {
            PlaybackCommand::PlayFile { path } => self.play_file(path),
            PlaybackCommand::Pause => self.pause(),
            PlaybackCommand::Resume => self.resume(),
            PlaybackCommand::Seek { position_ms } => self.seek(position_ms),
            PlaybackCommand::Stop => self.stop(),
            PlaybackCommand::SetOutputDevice { device_id } => self.set_output_device(device_id),
        };

        if let Err(error) = result {
            self.fail(error);
        }
    }

    fn play_file(&mut self, path: std::path::PathBuf) -> Result<(), EngineError> {
        self.stop_active();
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
        }
        self.stop_active();
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

        let position_ms = self.clamp_position(position_ms);
        if self.state == PlaybackState::Paused {
            let path = self.current_path()?;
            let mut decoder = (self.decoder_factory)(&path)?;
            let position_ms = if position_ms == 0 {
                0
            } else {
                decoder.seek(position_ms)?
            };
            if let Some(current) = &mut self.current {
                current.position_ms = position_ms;
            }
            self.emit_position(position_ms);
            return Ok(());
        }

        let path = self.current_path()?;
        self.set_state(PlaybackState::Loading);
        self.stop_active();
        self.start_path(&path, position_ms, false, true)
    }

    fn stop(&mut self) -> Result<(), EngineError> {
        if !matches!(
            self.state,
            PlaybackState::Playing
                | PlaybackState::Paused
                | PlaybackState::Ended
                | PlaybackState::Error
        ) {
            self.illegal_command("Stop");
            return Ok(());
        }

        self.set_state(PlaybackState::Stopping);
        self.stop_active();
        self.current = None;
        self.set_state(PlaybackState::Idle);
        Ok(())
    }

    fn set_output_device(&mut self, device_id: DeviceId) -> Result<(), EngineError> {
        if self.output_device == device_id {
            return Ok(());
        }

        if self.state == PlaybackState::Playing {
            let position_ms = self.logical_position_ms();
            let path = self.current_path()?;
            self.output_device = device_id;
            self.set_state(PlaybackState::Loading);
            self.stop_active();
            return self.start_path(&path, position_ms, false, true);
        }

        self.output_device = device_id;
        Ok(())
    }

    fn start_path(
        &mut self,
        path: &Path,
        requested_position_ms: u64,
        emit_now_playing: bool,
        emit_position: bool,
    ) -> Result<(), EngineError> {
        let mut decoder = (self.decoder_factory)(path)?;
        let format = decoder.format();
        let duration_ms = decoder.duration_ms();
        let requested_position_ms = duration_ms.map_or(requested_position_ms, |duration| {
            requested_position_ms.min(duration)
        });
        let actual_position_ms = if requested_position_ms == 0 {
            0
        } else {
            decoder.seek(requested_position_ms)?
        };

        let mut backend = (self.backend_factory)(self.output_device)?;
        backend.start(format)?;

        let source = PlayableSource {
            path: path.to_path_buf(),
            duration_ms,
        };
        self.current = Some(CurrentTrack {
            source: source.clone(),
            format,
            position_ms: actual_position_ms,
        });
        self.active = Some(ActivePlayback {
            decoder,
            backend,
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

    fn pump(&mut self) -> Result<(), EngineError> {
        let bytes_per_frame = self
            .current
            .as_ref()
            .expect("active playback must have a current track")
            .format
            .bytes_per_frame();
        let active = self.active.as_mut().expect("pump requires active playback");

        if active.pcm_offset == active.pcm.len() && !active.decoder_finished {
            match active.decoder.next_pcm(&mut active.pcm)? {
                Some(_) => active.pcm_offset = 0,
                None => active.decoder_finished = true,
            }
        }

        if active.pcm_offset < active.pcm.len() {
            let accepted_frames = active.backend.feed(&active.pcm[active.pcm_offset..]);
            active.pcm_offset += accepted_frames * bytes_per_frame;
            active.fed_frames += accepted_frames as u64;
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

        let finished = self.active.as_ref().is_some_and(|active| {
            active.decoder_finished
                && active.pcm_offset == active.pcm.len()
                && active.backend.position() >= active.fed_frames
        });
        if finished {
            self.finish_playback();
        }

        Ok(())
    }

    fn finish_playback(&mut self) {
        let position_ms = self.logical_position_ms();
        if let Some(current) = &mut self.current {
            current.position_ms = position_ms;
        }
        self.stop_active();
        self.emit_position(position_ms);
        self.set_state(PlaybackState::Ended);
        self.broadcast(PlaybackEvent::Ended);
    }

    fn fail(&mut self, error: EngineError) {
        self.stop_active();
        self.current = None;
        self.set_state(PlaybackState::Error);
        self.broadcast(PlaybackEvent::Error {
            message: error.to_string(),
        });
    }

    fn stop_active(&mut self) {
        if let Some(mut active) = self.active.take() {
            let _ = active.backend.stop();
        }
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

    fn illegal_command(&self, command: &str) {
        self.broadcast(PlaybackEvent::Error {
            message: format!("{command} is not valid while playback is {:?}", self.state),
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
        let position_ms = active
            .base_position_ms
            .saturating_add(frames_to_ms(active.backend.position(), format.sample_rate));
        self.clamp_position(position_ms)
    }

    fn clamp_position(&self, position_ms: u64) -> u64 {
        self.current
            .as_ref()
            .and_then(|current| current.source.duration_ms)
            .map_or(position_ms, |duration| position_ms.min(duration))
    }

    fn current_path(&self) -> Result<std::path::PathBuf, EngineError> {
        self.current
            .as_ref()
            .map(|current| current.source.path.clone())
            .ok_or_else(|| EngineError::Decode("no current source".to_string()))
    }

    fn current_path_and_position(&self) -> Result<(std::path::PathBuf, u64), EngineError> {
        self.current
            .as_ref()
            .map(|current| (current.source.path.clone(), current.position_ms))
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

    #[derive(Default)]
    struct FakeLog {
        opened_devices: Vec<DeviceId>,
        seek_positions: Vec<u64>,
        stops: usize,
    }

    struct FakeBackend {
        log: Arc<Mutex<FakeLog>>,
        fed_frames: u64,
    }

    impl PlaybackBackend for FakeBackend {
        fn start(&mut self, _format: PcmFormat) -> Result<(), EngineError> {
            Ok(())
        }

        fn feed(&mut self, pcm: &[u8]) -> usize {
            let frames = pcm.len() / TEST_FORMAT.bytes_per_frame();
            self.fed_frames += frames as u64;
            frames
        }

        fn position(&self) -> u64 {
            self.fed_frames.min(1_000)
        }

        fn stop(&mut self) -> Result<(), EngineError> {
            self.log.lock().unwrap().stops += 1;
            Ok(())
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
    fn fake_backend_pause_resume_and_seek_rebuild_from_logical_position() {
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
        assert_eq!(log.opened_devices, [7, 7, 7]);
        assert_eq!(log.seek_positions, [1_000, 5_000]);
        assert_eq!(log.stops, 3);
    }

    #[test]
    fn illegal_command_order_emits_errors_without_changing_idle_state() {
        let (controller, _) = fake_controller();
        let events = controller.subscribe();
        let commands = controller.command_sender();

        commands.send(PlaybackCommand::Pause).unwrap();
        commands.send(PlaybackCommand::Resume).unwrap();
        commands
            .send(PlaybackCommand::Seek { position_ms: 500 })
            .unwrap();

        let mut errors = Vec::new();
        while errors.len() < 3 {
            let event = events.recv_timeout(Duration::from_secs(1)).unwrap();
            match event {
                PlaybackEvent::Error { message } => errors.push(message),
                PlaybackEvent::StateChanged(state) => {
                    panic!("illegal command changed state to {state:?}")
                }
                _ => {}
            }
        }

        assert!(errors.iter().all(|message| message.contains("Idle")));
    }

    #[test]
    fn paused_seek_emits_and_records_the_decoder_actual_position() {
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
            *event == PlaybackEvent::StateChanged(PlaybackState::Playing)
        });

        assert_eq!(log.lock().unwrap().seek_positions, [5_000, 4_750]);
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

        let log = log.lock().unwrap();
        assert_eq!(log.opened_devices, [7, 9]);
        assert_eq!(log.seek_positions, [1_000]);
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
            Arc::new(move |device_id| {
                backend_log.lock().unwrap().opened_devices.push(device_id);
                Ok(Box::new(FakeBackend {
                    log: Arc::clone(&backend_log),
                    fed_frames: 0,
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
