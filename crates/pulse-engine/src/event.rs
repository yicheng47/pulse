use crate::{EngineError, PcmFormat, PlayableSource, PlaybackState};

/// What failed, independent of the display text. Track-scoped failures let a
/// queue skip to the next entry; device-scoped ones need output recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackErrorKind {
    Track,
    Device { hog_pid: Option<i32> },
}

impl From<&EngineError> for PlaybackErrorKind {
    fn from(error: &EngineError) -> Self {
        match error {
            EngineError::Decode(_) | EngineError::UnsupportedFormat(_) | EngineError::Io(_) => {
                Self::Track
            }
            EngineError::Hogged(pid) => Self::Device {
                hog_pid: Some(*pid),
            },
            EngineError::Os { .. }
            | EngineError::NoOutputDevice
            | EngineError::NoOutputCapabilities(_)
            | EngineError::UnsupportedNominalSampleRate(_)
            | EngineError::NoMatchingPhysicalFormat(_)
            | EngineError::Timeout(_)
            | EngineError::AudioUnit(_) => Self::Device { hog_pid: None },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackEvent {
    StateChanged(PlaybackState),
    NowPlaying {
        source: PlayableSource,
        format: PcmFormat,
    },
    Position {
        position_ms: u64,
        duration_ms: Option<u64>,
    },
    OutputDeviceChanged {
        device_id: crate::device::DeviceId,
        exclusive_mode: bool,
    },
    ExclusiveModeFallback {
        device_id: crate::device::DeviceId,
    },
    Ended {
        /// See [`PlaybackEvent::Error::attempt`].
        attempt: u64,
    },
    CommandRejected {
        command: &'static str,
        state: PlaybackState,
    },
    /// A runtime failure. It is fatal when paired with `StateChanged(Error)` and advisory when
    /// emitted after teardown has already reached `Idle` or `Ended`.
    Error {
        /// Ordinal of the `PlayFile` command this terminal event belongs to
        /// (the nth `PlayFile` the worker processed; 0 before any). Commands
        /// and events are FIFO, so a consumer that counts its own dispatched
        /// `PlayFile`s can discard terminal events from superseded plays.
        attempt: u64,
        kind: PlaybackErrorKind,
        message: String,
    },
}
