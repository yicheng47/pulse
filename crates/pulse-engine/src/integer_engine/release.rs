use std::{
    sync::{Arc, Mutex, MutexGuard, TryLockError},
    thread,
    time::{Duration, Instant},
};

use crate::{EngineError, hal, raw_sink};

const RELEASE_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Clone)]
pub(crate) struct IntegerReleaseHandle {
    resources: Arc<Mutex<IntegerDeviceResources>>,
}

pub(super) struct IntegerDeviceResources {
    pub(super) sink: Option<raw_sink::RawSink>,
    format_restore: Option<hal::FormatRestoreGuard>,
    hog: Option<hal::HogGuard>,
    pub(super) released: bool,
}

impl IntegerDeviceResources {
    fn begin_release(&mut self) -> bool {
        if self.released {
            return false;
        }
        self.released = true;
        true
    }
}

impl IntegerReleaseHandle {
    pub(super) fn new(format_restore: hal::FormatRestoreGuard, hog: hal::HogGuard) -> Self {
        Self {
            resources: Arc::new(Mutex::new(IntegerDeviceResources {
                sink: None,
                format_restore: Some(format_restore),
                hog: Some(hog),
                released: false,
            })),
        }
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, IntegerDeviceResources> {
        self.resources
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn release(&self) -> Result<(), EngineError> {
        let mut resources = self.lock();
        Self::release_resources(&mut resources)
    }

    pub(crate) fn release_before(&self, deadline: Instant) -> Result<(), EngineError> {
        loop {
            match self.resources.try_lock() {
                Ok(mut resources) => return Self::release_resources(&mut resources),
                Err(TryLockError::Poisoned(error)) => {
                    return Self::release_resources(&mut error.into_inner());
                }
                Err(TryLockError::WouldBlock) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(EngineError::BackendRelease(
                            "timed out waiting for the integer device release lock".to_string(),
                        ));
                    }
                    thread::sleep(
                        RELEASE_LOCK_POLL_INTERVAL.min(deadline.saturating_duration_since(now)),
                    );
                }
            }
        }
    }

    fn release_resources(resources: &mut IntegerDeviceResources) -> Result<(), EngineError> {
        if !resources.begin_release() {
            return Ok(());
        }

        let mut errors = Vec::new();
        if let Some(sink) = &mut resources.sink
            && let Err(error) = sink.stop()
        {
            errors.push(error);
        }
        resources.sink = None;
        if let Some(guard) = resources.format_restore.take() {
            errors.extend(guard.restore());
        }
        resources.hog = None;
        collected_release_result(errors)
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self {
            resources: Arc::new(Mutex::new(IntegerDeviceResources {
                sink: None,
                format_restore: None,
                hog: None,
                released: false,
            })),
        }
    }

    #[cfg(test)]
    pub(crate) fn hold_resources_for_test(
        &self,
        locked: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    ) {
        let _resources = self.lock();
        locked.send(()).expect("lock observer must remain alive");
        release
            .recv()
            .expect("lock release sender must remain alive");
    }
}

fn collected_release_result(mut errors: Vec<EngineError>) -> Result<(), EngineError> {
    match errors.len() {
        0 => Ok(()),
        1 => Err(errors.pop().expect("one collected error must exist")),
        _ => Err(EngineError::BackendRelease(
            errors
                .into_iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_release_gate_arms_guard_teardown_once() {
        let mut resources = IntegerDeviceResources {
            sink: None,
            format_restore: None,
            hog: None,
            released: false,
        };

        assert!(resources.begin_release());
        assert!(!resources.begin_release());
    }
}
