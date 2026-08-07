//! Core Audio playback engine. UI-agnostic — no Tauri, no web.
//! Driven by `pulse-cli` to validate device control and playback against a real
//! DAC before any shell exists.

mod auhal;
pub mod command;
pub mod controller;
pub mod decode;
pub mod device;
mod engine;
mod error;
pub mod event;
mod hal;
mod levels;
pub mod source;
pub mod state;

pub use command::PlaybackCommand;
pub use controller::PlaybackController;
pub use engine::Engine;
pub use error::EngineError;
pub use event::{PlaybackErrorKind, PlaybackEvent};
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
