//! The playback controller: one worker thread that owns a decoder and an output backend and turns
//! `PlaybackCommand`s into `PlaybackEvent`s.
//!
//! Three layers live here. `PlaybackController` is the public handle (command sender, event
//! subscriptions, shutdown). `Worker` is the state machine on its own thread. `PlaybackBackend` is
//! the one trait both engines sit behind — `AuhalBackend` (AUHAL, universal) and `IntegerBackend`
//! (raw HAL, integer) — so the worker never knows which one it is driving.
//!
//! The worker moves bytes: `SourceDecoder::next_pcm` fills a chunk, `PlaybackBackend::feed` takes as
//! much as the engine's ring has room for, and `pump` repeats until the decoder is dry. No sample
//! value is inspected or changed on this thread; the bit-perfect claim depends on that.
//!
//! Reading order: the `Worker` fields and their invariants, then `run`, `start_path` →
//! `start_backend`, `pump`, and the ways a track ends (`begin_seamless_transition` /
//! `rebuild_for_preloaded`, or `finish_playback`). `stop_active` versus `release_backend` is the
//! distinction most of the device behaviour hangs on.

mod backend;
mod decoder;
mod worker;

use crate::{
    EngineError, EngineKind, PlaybackCommand, PlaybackEvent, decode::PcmDecoder,
    decode_dsd::DsdDopDecoder, device::DeviceId,
};
use backend::{
    ActiveBackendRelease, AuhalBackend, BackendFactory, BackendReleaseOutcome, IntegerBackend,
    PlaybackBackend, combine_backend_errors,
};
use decoder::{DecoderFactory, SourceDecoder};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use worker::{Clock, OUTPUT_STALL_TIMEOUT, Worker, WorkerSettings};

// These bound the two waits independently; HAL teardown uses its own deadlines once begun.
const SHUTDOWN_RELEASE_LOCK_TIMEOUT: Duration = Duration::from_secs(1);
const SHUTDOWN_WORKER_JOIN_TIMEOUT: Duration = Duration::from_secs(1);
const SHUTDOWN_JOIN_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Public handle to the playback worker. Cloneable command senders and per-subscriber event
/// channels are the whole API; the worker thread owns every engine object.
pub struct PlaybackController {
    command_tx: Sender<PlaybackCommand>,
    subscribers: EventSubscribers,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    backend_release: ActiveBackendRelease,
}

impl PlaybackController {
    pub fn spawn(output_device: DeviceId, engine_kind: EngineKind) -> Self {
        Self::spawn_with_dependencies(
            output_device,
            engine_kind,
            Arc::new(|device_id, kind| match kind {
                EngineKind::Universal { exclusive_mode } => {
                    Ok(Box::new(AuhalBackend::open(device_id, exclusive_mode)?)
                        as Box<dyn PlaybackBackend>)
                }
                EngineKind::Integer => {
                    Ok(Box::new(IntegerBackend::open(device_id)?) as Box<dyn PlaybackBackend>)
                }
            }),
            Arc::new(|path| {
                if path.extension().is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("dsf") || extension.eq_ignore_ascii_case("dff")
                }) {
                    Ok(Box::new(DsdDopDecoder::open(path)?) as Box<dyn SourceDecoder>)
                } else {
                    Ok(Box::new(PcmDecoder::open(path)?) as Box<dyn SourceDecoder>)
                }
            }),
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
        let backend_release = ActiveBackendRelease::default();
        let worker_backend_release = backend_release.clone();

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
                        worker_backend_release,
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
            backend_release,
        }
    }

    pub fn shutdown(&mut self) -> Result<(), EngineError> {
        self.shutdown_with_timeouts(SHUTDOWN_RELEASE_LOCK_TIMEOUT, SHUTDOWN_WORKER_JOIN_TIMEOUT)
    }

    /// Quit path. Flags the worker to stop, releases the device from this thread if a handle
    /// exists, then waits a bounded time for the worker to exit. The two waits are bounded
    /// independently so a hung HAL call cannot block the app's exit; the error text says which
    /// half did not finish.
    fn shutdown_with_timeouts(
        &mut self,
        release_lock_timeout: Duration,
        worker_join_timeout: Duration,
    ) -> Result<(), EngineError> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };

        self.shutdown.store(true, Ordering::Release);
        let release_deadline = Instant::now() + release_lock_timeout;
        let mut release_outcome = self.backend_release.release_before(release_deadline);
        let join_deadline = Instant::now() + worker_join_timeout;
        while !worker.is_finished() && Instant::now() < join_deadline {
            thread::sleep(
                SHUTDOWN_JOIN_POLL_INTERVAL
                    .min(join_deadline.saturating_duration_since(Instant::now())),
            );
        }

        let worker_error = if worker.is_finished() {
            worker.join().err().map(|_| {
                EngineError::BackendRelease("playback worker panicked during shutdown".to_string())
            })
        } else {
            if matches!(release_outcome, BackendReleaseOutcome::NoHandle) {
                release_outcome = self.backend_release.release_before(Instant::now());
            }
            drop(worker);
            let release_status = match &release_outcome {
                BackendReleaseOutcome::NoHandle => {
                    "no independent device release handle was active"
                }
                BackendReleaseOutcome::Completed => {
                    "device release completed on the shutdown thread"
                }
                BackendReleaseOutcome::Failed(_) => {
                    "device release did not complete on the shutdown thread"
                }
            };
            Some(EngineError::BackendRelease(format!(
                "playback worker did not exit within {} ms after the device release attempt; {release_status}",
                worker_join_timeout.as_millis()
            )))
        };

        let release_error = match release_outcome {
            BackendReleaseOutcome::Failed(error) => Some(error),
            BackendReleaseOutcome::NoHandle | BackendReleaseOutcome::Completed => None,
        };

        match (release_error, worker_error) {
            (None, None) => Ok(()),
            (Some(error), None) | (None, Some(error)) => Err(error),
            (Some(first), Some(second)) => Err(combine_backend_errors(first, second)),
        }
    }
}

impl Drop for PlaybackController {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

type EventSubscribers = Arc<Mutex<Vec<Sender<PlaybackEvent>>>>;
