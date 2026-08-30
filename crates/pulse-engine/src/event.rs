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

#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackEvent {
    StateChanged(PlaybackState),
    NowPlaying {
        source: PlayableSource,
        format: PcmFormat,
    },
    /// A preloaded source has reached the audible boundary. Consumers apply its source and format
    /// as the new now-playing track, then advance their queue without dispatching another
    /// `PlayFile`. This event replaces the outgoing track's `Ended` event and is immediately
    /// followed by `Position { position_ms: 0, .. }`.
    Advanced {
        /// The current `PlayFile` ordinal. Gapless advancement does not increment it because the
        /// engine starts the preloaded source without processing another `PlayFile` command.
        attempt: u64,
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
    /// Output-main level captured the first time this app session takes a controllable device's
    /// hog. Later hogs for the same device reapply the app's current level without another event.
    /// `muted` is the app's mute as applied before playback, not the probed hardware mute.
    /// Consumers should persist this state without sending a `SetVolume` command back.
    HardwareVolume {
        level: f32,
        muted: bool,
    },
    Ended {
        /// See [`PlaybackEvent::Error::attempt`].
        attempt: u64,
    },
    CommandRejected {
        command: &'static str,
        state: PlaybackState,
    },
    /// A lookahead source could not be opened. Current playback and state are unchanged.
    NextRejected {
        /// The current `PlayFile` ordinal, matching [`PlaybackEvent::Advanced::attempt`].
        attempt: u64,
        path: std::path::PathBuf,
        message: String,
    },
    /// A runtime failure. It is fatal when paired with `StateChanged(Error)` and advisory when a
    /// teardown has already reached `Idle` or `Ended`.
    Error {
        /// Ordinal of the `PlayFile` command this event belongs to (the nth `PlayFile` the worker
        /// processed; 0 before any). Commands and events are FIFO, so a consumer that counts its
        /// own dispatched `PlayFile`s can discard events from superseded plays. Gapless
        /// advancement keeps the same attempt because it does not process another `PlayFile`.
        attempt: u64,
        kind: PlaybackErrorKind,
        message: String,
    },
}
