use std::path::PathBuf;

use crate::device::DeviceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    Universal { exclusive_mode: bool },
    BitPerfect,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackCommand {
    PlayFile {
        path: PathBuf,
    },
    Load {
        path: PathBuf,
        position_ms: u64,
    },
    /// Preloads one source. Once same-format PCM has already been buffered for an audible
    /// transition, this replaces that incoming track's successor instead of the buffered audio.
    SetNext {
        path: PathBuf,
    },
    /// Clears the preloaded source or, during a buffered transition, the incoming track's
    /// successor. Already-buffered PCM remains scheduled for playback.
    ClearNext,
    Pause,
    Resume,
    Seek {
        position_ms: u64,
    },
    Stop,
    SetOutputDevice {
        device_id: DeviceId,
        kind: EngineKind,
    },
    SetExclusiveMode {
        enabled: bool,
    },
    SetVolume {
        level: f32,
        muted: bool,
    },
}
