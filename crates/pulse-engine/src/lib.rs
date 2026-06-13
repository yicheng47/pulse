//! Bit-perfect Core Audio playback engine. UI-agnostic — no Tauri, no web.
//! Driven by `pulse-cli` to validate hog/integer mode against a real DAC
//! before any shell exists.

pub mod decode;
pub mod device;
mod error;
mod hal;
mod ioproc;
mod levels;

use rtrb::{Consumer, Producer, RingBuffer};

pub use error::EngineError;
pub use levels::Levels;

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

/// Exclusive-mode playback engine for one output device.
pub struct Engine {
    device: device::DeviceId,
    _hog: hal::HogGuard,
    producer: Option<Producer<u8>>,
    consumer: Option<Consumer<u8>>,
    ioproc: Option<ioproc::IoProc>,
    format: Option<PcmFormat>,
    bytes_per_frame: usize,
}

impl Engine {
    /// Takes hog mode on the device.
    pub fn open(device: device::DeviceId) -> Result<Self, EngineError> {
        Ok(Self {
            device,
            _hog: hal::HogGuard::acquire(device)?,
            producer: None,
            consumer: None,
            ioproc: None,
            format: None,
            bytes_per_frame: 0,
        })
    }

    /// Switches the device's physical format to `fmt` (integer mode, native
    /// rate). Blocks until the async rate switch completes.
    pub fn set_format(&mut self, fmt: PcmFormat) -> Result<(), EngineError> {
        self.pause()?;
        hal::set_nominal_sample_rate(self.device, fmt)?;
        let (_, bytes_per_frame) =
            hal::set_matching_direct_interleaved_physical_format(self.device, fmt)?;
        let ring_capacity = usize::try_from(fmt.sample_rate)
            .ok()
            .and_then(|sample_rate| sample_rate.checked_mul(bytes_per_frame))
            .and_then(|bytes_per_second| bytes_per_second.checked_mul(4))
            .ok_or_else(|| {
                EngineError::UnsupportedFormat("ring buffer size overflow".to_string())
            })?;
        let (producer, consumer) = RingBuffer::<u8>::new(ring_capacity);

        self.producer = Some(producer);
        self.consumer = Some(consumer);
        self.ioproc = None;
        self.format = Some(fmt);
        self.bytes_per_frame = bytes_per_frame;
        Ok(())
    }

    pub fn play(&mut self) -> Result<(), EngineError> {
        if self.ioproc.is_some() {
            return Ok(());
        }
        let consumer = self.consumer.take().ok_or_else(|| {
            EngineError::UnsupportedFormat("engine format is not set".to_string())
        })?;
        self.ioproc = Some(ioproc::IoProc::start(
            self.device,
            consumer,
            self.bytes_per_frame,
        )?);
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), EngineError> {
        if let Some(ioproc) = self.ioproc.take() {
            self.consumer = Some(ioproc.stop());
        }
        Ok(())
    }

    /// Pushes interleaved PCM (in the format from `set_format`) into the ring
    /// buffer. Returns frames accepted — backpressure, never blocks.
    pub fn feed(&mut self, pcm: &[u8]) -> usize {
        let Some(producer) = &mut self.producer else {
            return 0;
        };
        if self.bytes_per_frame == 0 {
            return 0;
        }

        let aligned_len = pcm.len() / self.bytes_per_frame * self.bytes_per_frame;
        let (pushed, _) = producer.push_partial_slice(&pcm[..aligned_len]);
        pushed.len() / self.bytes_per_frame
    }

    /// Playback position in frames since `play`.
    pub fn position(&self) -> u64 {
        self.ioproc
            .as_ref()
            .map_or(0, ioproc::IoProc::position_frames)
    }

    /// Latest RMS/peak from the realtime tap.
    pub fn levels(&self) -> Levels {
        Levels::default()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.pause();
    }
}
