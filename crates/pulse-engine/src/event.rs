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
    Error {
        message: String,
    },
}
