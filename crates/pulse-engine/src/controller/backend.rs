use crate::{
    AuhalEngine, EngineError, EngineKind, PcmFormat, VolumeDomain, device::DeviceId,
    integer_engine::IntegerEngine,
};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

pub(super) type BackendFactory = Arc<
    dyn Fn(DeviceId, EngineKind) -> Result<Box<dyn PlaybackBackend>, EngineError> + Send + Sync,
>;

/// A handle that can give the output device back from a thread other than the worker's — the app's
/// quit path uses it with a deadline so a stuck worker cannot keep the hog or the device format.
/// Only the integer engine provides one; the universal engine has nothing to release out of band.
pub(super) trait BackendRelease: Send + Sync {
    fn release_before(&self, deadline: Instant) -> Result<(), EngineError>;
}

pub(super) type BackendReleaseHandle = Arc<dyn BackendRelease>;

pub(super) enum BackendReleaseOutcome {
    NoHandle,
    Completed,
    Failed(EngineError),
}

/// The release handle of whichever backend is currently open, shared between the worker (which
/// replaces it on open and clears it on release) and `PlaybackController::shutdown`.
#[derive(Clone, Default)]
pub(super) struct ActiveBackendRelease {
    handle: Arc<Mutex<Option<BackendReleaseHandle>>>,
}

impl ActiveBackendRelease {
    pub(super) fn replace(&self, handle: Option<BackendReleaseHandle>) {
        *self
            .handle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = handle;
    }

    pub(super) fn clear(&self) {
        self.replace(None);
    }

    pub(super) fn release_before(&self, deadline: Instant) -> BackendReleaseOutcome {
        let handle = self
            .handle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        match handle {
            Some(handle) => match handle.release_before(deadline) {
                Ok(()) => BackendReleaseOutcome::Completed,
                Err(error) => BackendReleaseOutcome::Failed(error),
            },
            None => BackendReleaseOutcome::NoHandle,
        }
    }
}

/// What the worker needs from an output engine. Both engines implement the same lifecycle:
///
/// - `start(format)`: negotiate the device for `format`, then play. On an already-open backend
///   this renegotiates in place (rate, physical/virtual format, fresh ring) — the basis of the
///   format-change boundary reuse in `rebuild_for_preloaded`.
/// - `feed`: push interleaved PCM in `format`; returns whole frames accepted, never blocks.
/// - `position`: frames the device has consumed since `start`. Every position and gapless
///   decision in the worker is derived from it, never from what was fed.
/// - `stop`: stop the sink and drain the ring; the device stays open.
/// - `release`: give the device back (hog, restored formats). The box is consumed.
/// - `retains_device_when_paused`: whether Pause should `stop` (keep the device: integer engine)
///   or `release` (universal engine, which holds nothing worth keeping).
pub(super) trait PlaybackBackend {
    fn start(&mut self, format: PcmFormat) -> Result<(), EngineError>;
    fn feed(&mut self, pcm: &[u8]) -> usize;
    fn position(&self) -> u64;
    fn underrun_frames(&self) -> u64;
    fn take_hardware_volume(&mut self) -> Option<(f32, bool)>;
    fn volume_domain(&self) -> VolumeDomain;
    fn set_volume(&mut self, level: f32, muted: bool) -> Result<(), EngineError>;
    fn stop(&mut self) -> Result<(), EngineError>;
    fn retains_device_when_paused(&self) -> bool {
        false
    }
    fn release(self: Box<Self>) -> Result<(), EngineError> {
        Ok(())
    }
    fn shutdown_handle(&self) -> Option<BackendReleaseHandle> {
        None
    }
}

/// The AUHAL engine behind the trait — Universal mode: float32 client format, shared or exclusive.
pub(super) struct AuhalBackend {
    engine: AuhalEngine,
}

impl AuhalBackend {
    pub(super) fn open(device_id: DeviceId, exclusive_mode: bool) -> Result<Self, EngineError> {
        Ok(Self {
            engine: AuhalEngine::open(device_id, exclusive_mode)?,
        })
    }
}

impl PlaybackBackend for AuhalBackend {
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

    fn volume_domain(&self) -> VolumeDomain {
        self.engine.volume_domain()
    }

    fn set_volume(&mut self, level: f32, muted: bool) -> Result<(), EngineError> {
        self.engine.set_volume(level, muted)
    }

    fn stop(&mut self) -> Result<(), EngineError> {
        self.engine.pause()
    }

    fn shutdown_handle(&self) -> Option<BackendReleaseHandle> {
        // Universal mode has no device lease shared outside its worker-owned AuhalEngine.
        None
    }
}

/// The integer engine behind the trait: raw HAL IOProc, hog held, integer device formats.
pub(super) struct IntegerBackend {
    engine: IntegerEngine,
}

impl IntegerBackend {
    pub(super) fn open(device_id: DeviceId) -> Result<Self, EngineError> {
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

    fn volume_domain(&self) -> VolumeDomain {
        self.engine.volume_domain()
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

    fn shutdown_handle(&self) -> Option<BackendReleaseHandle> {
        Some(Arc::new(self.engine.release_handle()))
    }
}

impl BackendRelease for crate::integer_engine::IntegerReleaseHandle {
    fn release_before(&self, deadline: Instant) -> Result<(), EngineError> {
        crate::integer_engine::IntegerReleaseHandle::release_before(self, deadline)
    }
}

pub(super) fn combine_backend_errors(first: EngineError, second: EngineError) -> EngineError {
    EngineError::BackendRelease(format!("{first}; {second}"))
}
