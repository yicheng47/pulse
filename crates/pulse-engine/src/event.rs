use crate::{PcmFormat, PlayableSource, PlaybackState};

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
    Ended,
    CommandRejected {
        command: &'static str,
        state: PlaybackState,
    },
    /// A runtime failure. It is fatal when paired with `StateChanged(Error)` and advisory when
    /// emitted after teardown has already reached `Idle` or `Ended`.
    Error {
        message: String,
    },
}
