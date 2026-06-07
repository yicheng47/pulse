//! Bit-perfect Core Audio playback engine. UI-agnostic — no Tauri, no web.
//! Driven by `pulse-cli` to validate hog/integer mode against a real DAC
//! before any shell exists.

pub mod decode;
pub mod device;
mod error;
mod hal;
mod ioproc;
mod levels;

pub use error::EngineError;
pub use levels::Levels;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmFormat {
    pub sample_rate: u32,
    pub bits_per_sample: u8,
    pub channels: u8,
}

/// Exclusive-mode playback engine for one output device.
pub struct Engine {
    _device: device::DeviceId,
}

impl Engine {
    /// Takes hog mode on the device.
    pub fn open(_device: device::DeviceId) -> Result<Self, EngineError> {
        todo!()
    }

    /// Switches the device's physical format to `fmt` (integer mode, native
    /// rate). Blocks until the async rate switch completes.
    pub fn set_format(&mut self, _fmt: PcmFormat) -> Result<(), EngineError> {
        todo!()
    }

    pub fn play(&mut self) -> Result<(), EngineError> {
        todo!()
    }

    pub fn pause(&mut self) -> Result<(), EngineError> {
        todo!()
    }

    /// Pushes interleaved PCM (in the format from `set_format`) into the ring
    /// buffer. Returns frames accepted — backpressure, never blocks.
    pub fn feed(&mut self, _pcm: &[u8]) -> usize {
        todo!()
    }

    /// Playback position in frames since `play`.
    pub fn position(&self) -> u64 {
        todo!()
    }

    /// Latest RMS/peak from the realtime tap.
    pub fn levels(&self) -> Levels {
        todo!()
    }
}
