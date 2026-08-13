use std::path::PathBuf;

use crate::device::DeviceId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackCommand {
    PlayFile { path: PathBuf },
    Pause,
    Resume,
    Seek { position_ms: u64 },
    Stop,
    SetOutputDevice { device_id: DeviceId },
    SetExclusiveMode { enabled: bool },
}
