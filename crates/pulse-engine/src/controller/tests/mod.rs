use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Condvar, mpsc::RecvTimeoutError},
    time::{Duration, Instant},
};

use super::*;
use crate::VolumeDomain;
use crate::controller::{backend::*, *};
use std::sync::{Mutex, mpsc};

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
    fail_integer_start_device: Option<DeviceId>,
    fail_all_open_device: Option<DeviceId>,
    stop_error: bool,
    release_error: bool,
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
            fail_integer_start_device: None,
            fail_all_open_device: None,
            stop_error: false,
            release_error: false,
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
    engine_kind: EngineKind,
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
            && self.log.lock().unwrap().fail_integer_start_device == Some(self.device_id)
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

    fn volume_domain(&self) -> VolumeDomain {
        if self.hardware_volume_active {
            VolumeDomain::Device
        } else {
            match self.engine_kind {
                EngineKind::Universal { .. } => VolumeDomain::Software,
                EngineKind::Integer => VolumeDomain::Fixed,
            }
        }
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
        let mut log = self.log.lock().unwrap();
        log.releases += 1;
        if log.release_error {
            Err(EngineError::BackendRelease(
                "backend release failed".to_string(),
            ))
        } else {
            Ok(())
        }
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

fn fake_controller_with_kind(engine_kind: EngineKind) -> (PlaybackController, Arc<Mutex<FakeLog>>) {
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
                EngineKind::Integer => (true, true),
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
                engine_kind,
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

mod dropouts;
mod gapless;
mod output;
mod shutdown;
mod transport;
mod volume;
