//! Core Audio playback engine. UI-agnostic — no GPUI or app types.
//! Driven in-process by `pulse-app`.

mod auhal;
pub mod command;
pub mod controller;
pub mod decode;
pub mod device;
mod engine;
mod error;
pub mod event;
mod gain;
#[doc(hidden)]
pub mod hal;
mod integer_engine;
mod levels;
mod raw_sink;
pub mod source;
pub mod state;

pub use command::EngineKind;
pub use command::PlaybackCommand;
pub use controller::PlaybackController;
pub use engine::Engine;
pub use error::EngineError;
pub use event::{PlaybackErrorKind, PlaybackEvent, VolumeDomain, VolumeState};
pub use gain::volume_gain_for_level;
pub use levels::Levels;
pub use source::PlayableSource;
pub use state::PlaybackState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmFormat {
    pub sample_rate: u32,
    pub bits_per_sample: u8,
    pub channels: u8,
}

impl PcmFormat {
    pub fn bytes_per_frame(self) -> usize {
        usize::from(self.channels) * usize::from(self.bits_per_sample).div_ceil(8)
    }
}
