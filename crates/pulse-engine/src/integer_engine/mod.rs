mod packing;
mod release;

use crate::{EngineError, PcmFormat, device, event::VolumeDomain, hal, raw_sink};
use objc2_core_audio::AudioStreamRangedDescription;
use objc2_core_audio_types::AudioStreamBasicDescription;
use packing::IntPacker;
pub(crate) use release::IntegerReleaseHandle;
use rtrb::{Consumer, Producer, RingBuffer};

pub(crate) struct IntegerEngine {
    device: device::DeviceId,
    release_handle: IntegerReleaseHandle,
    hardware_volume: Option<hal::HardwareVolume>,
    hardware_volume_event_pending: bool,
    producer: Option<Producer<u8>>,
    consumer: Option<Consumer<u8>>,
    format: Option<PcmFormat>,
    device_format: Option<AudioStreamBasicDescription>,
    packer: Option<IntPacker>,
    pack_buffer: Vec<u8>,
}

impl IntegerEngine {
    pub(crate) fn open(device: device::DeviceId) -> Result<Self, EngineError> {
        let hog = hal::HogGuard::acquire(device)?;
        if !hog.owns() {
            return Err(EngineError::HoggedByCurrentProcess);
        }
        let format_restore = hal::FormatRestoreGuard::capture(device)?;
        hal::set_mixing_enabled(device, false)?;
        let hardware_volume = hal::hardware_volume_control(device);
        let hardware_volume_event_pending = hardware_volume.is_some();

        Ok(Self {
            device,
            release_handle: IntegerReleaseHandle::new(format_restore, hog),
            hardware_volume,
            hardware_volume_event_pending,
            producer: None,
            consumer: None,
            format: None,
            device_format: None,
            packer: None,
            pack_buffer: Vec::new(),
        })
    }

    pub(crate) fn set_format(&mut self, format: PcmFormat) -> Result<(), EngineError> {
        let release_handle = self.release_handle.clone();
        if self.format == Some(format) {
            return if release_handle.lock().released {
                Err(integer_engine_released())
            } else {
                Ok(())
            };
        }
        let (stream_id, device_format) = select_integer_format(self.device, format)?;
        let packer = IntPacker::new(format, device_format)?;
        let mut resources = release_handle.lock();
        if resources.released {
            return Err(integer_engine_released());
        }
        if let Some(sink) = &mut resources.sink {
            sink.stop()?;
        }
        resources.sink = None;
        self.format = None;
        self.device_format = None;
        self.packer = None;
        hal::set_nominal_sample_rate(self.device, format)?;
        hal::set_physical_format(stream_id, device_format)?;
        hal::set_virtual_format(stream_id, device_format)?;

        self.reset_ring(format, packer)?;
        self.format = Some(format);
        self.device_format = Some(device_format);
        self.packer = Some(packer);
        Ok(())
    }

    pub(crate) fn play(&mut self) -> Result<(), EngineError> {
        let release_handle = self.release_handle.clone();
        let mut resources = release_handle.lock();
        if resources.released {
            return Err(integer_engine_released());
        }
        if let Some(sink) = &mut resources.sink {
            return sink.restart();
        }
        let format = self.format.ok_or_else(|| {
            EngineError::UnsupportedFormat("integer engine format is not set".to_string())
        })?;
        let device_format = self.device_format.ok_or_else(|| {
            EngineError::UnsupportedFormat("integer device format is not set".to_string())
        })?;
        let consumer = self.consumer.take().ok_or_else(|| {
            EngineError::UnsupportedFormat(
                "raw sink is unavailable; call set_format before playing again".to_string(),
            )
        })?;
        match raw_sink::RawSink::start(self.device, consumer, device_format) {
            Ok(sink) => {
                resources.sink = Some(sink);
                Ok(())
            }
            Err(error) => {
                let packer = self.packer.expect("configured engine must have a packer");
                self.reset_ring(format, packer)?;
                Err(error)
            }
        }
    }

    pub(crate) fn pause(&mut self) -> Result<(), EngineError> {
        let release_handle = self.release_handle.clone();
        let mut resources = release_handle.lock();
        if let Some(sink) = &mut resources.sink {
            sink.stop()?;
        }
        Ok(())
    }

    pub(crate) fn feed(&mut self, pcm: &[u8]) -> usize {
        let release_handle = self.release_handle.clone();
        let resources = release_handle.lock();
        if resources.released {
            return 0;
        }
        let Some(packer) = self.packer else {
            return 0;
        };
        let Some(producer) = &mut self.producer else {
            return 0;
        };

        let source_frames = pcm.len() / packer.source_bytes_per_frame;
        let writable_frames = producer.slots() / packer.output_bytes_per_frame;
        let frames = source_frames.min(writable_frames);
        if frames == 0 {
            return 0;
        }

        self.pack_buffer.clear();
        self.pack_buffer
            .reserve(frames * packer.output_bytes_per_frame);
        packer.pack(
            &pcm[..frames * packer.source_bytes_per_frame],
            &mut self.pack_buffer,
        );
        let (pushed, _) = producer.push_partial_slice(&self.pack_buffer);
        pushed.len() / packer.output_bytes_per_frame
    }

    pub(crate) fn position(&self) -> u64 {
        self.release_handle
            .lock()
            .sink
            .as_ref()
            .map_or(0, raw_sink::RawSink::position_frames)
    }

    pub(crate) fn underrun_frames(&self) -> u64 {
        self.release_handle
            .lock()
            .sink
            .as_ref()
            .map_or(0, raw_sink::RawSink::underrun_frames)
    }

    pub(crate) fn take_hardware_volume(&mut self) -> Option<(f32, bool)> {
        if !self.hardware_volume_event_pending {
            return None;
        }
        self.hardware_volume_event_pending = false;
        self.hardware_volume
            .as_ref()
            .map(|volume| (volume.level, volume.muted))
    }

    pub(crate) fn volume_domain(&self) -> VolumeDomain {
        if self.hardware_volume.is_some() {
            VolumeDomain::Device
        } else {
            VolumeDomain::Fixed
        }
    }

    pub(crate) fn set_volume(&mut self, level: f32, muted: bool) -> Result<(), EngineError> {
        let release_handle = self.release_handle.clone();
        let resources = release_handle.lock();
        if resources.released {
            return Err(integer_engine_released());
        }
        if let Some(hardware_volume) = &mut self.hardware_volume {
            hardware_volume.set_volume(level, muted)?;
        }
        Ok(())
    }

    pub(crate) fn release_handle(&self) -> IntegerReleaseHandle {
        self.release_handle.clone()
    }

    pub(crate) fn release(self) -> Result<(), EngineError> {
        self.release_handle.release()
    }

    fn reset_ring(&mut self, format: PcmFormat, packer: IntPacker) -> Result<(), EngineError> {
        let ring_capacity = usize::try_from(format.sample_rate)
            .ok()
            .and_then(|sample_rate| sample_rate.checked_mul(packer.output_bytes_per_frame))
            .and_then(|bytes_per_second| bytes_per_second.checked_mul(4))
            .ok_or_else(|| {
                EngineError::UnsupportedFormat("ring buffer size overflow".to_string())
            })?;
        let (producer, consumer) = RingBuffer::<u8>::new(ring_capacity);
        self.producer = Some(producer);
        self.consumer = Some(consumer);
        self.pack_buffer.clear();
        Ok(())
    }
}

impl Drop for IntegerEngine {
    fn drop(&mut self) {
        let _ = self.release_handle.release();
    }
}

fn integer_engine_released() -> EngineError {
    EngineError::BackendRelease("integer engine is already released".to_string())
}

fn select_integer_format(
    device_id: device::DeviceId,
    source: PcmFormat,
) -> Result<(u32, AudioStreamBasicDescription), EngineError> {
    let mut best = None;
    for stream_id in hal::output_streams(device_id)? {
        for ranged in hal::available_physical_formats(stream_id)? {
            let Some(candidate) = integer_candidate(ranged, source) else {
                continue;
            };
            let rank = integer_candidate_rank(candidate, source);
            if best
                .as_ref()
                .is_none_or(|(_, _, best_rank)| rank < *best_rank)
            {
                best = Some((stream_id, candidate, rank));
            }
        }
    }

    best.map(|(stream_id, format, _)| (stream_id, format))
        .ok_or(EngineError::NoMatchingPhysicalFormat(source))
}

fn integer_candidate(
    ranged: AudioStreamRangedDescription,
    source: PcmFormat,
) -> Option<AudioStreamBasicDescription> {
    let mut format = ranged.mFormat;
    let requested_rate = f64::from(source.sample_rate);
    let rate_supported = (format.mSampleRate - requested_rate).abs() < 0.5
        || (requested_rate >= ranged.mSampleRateRange.mMinimum
            && requested_rate <= ranged.mSampleRateRange.mMaximum);
    if !hal::is_integer_wire_format(&format)
        || format.mChannelsPerFrame != u32::from(source.channels)
        || format.mBitsPerChannel < u32::from(source.bits_per_sample)
        || format.mBytesPerFrame / format.mChannelsPerFrame
            < u32::from(source.bits_per_sample).div_ceil(8)
        || !rate_supported
    {
        return None;
    }
    format.mSampleRate = requested_rate;
    Some(format)
}

fn integer_candidate_rank(
    candidate: AudioStreamBasicDescription,
    source: PcmFormat,
) -> (bool, u32, u32) {
    (
        candidate.mBitsPerChannel != u32::from(source.bits_per_sample),
        candidate.mBytesPerFrame / candidate.mChannelsPerFrame,
        candidate.mBitsPerChannel,
    )
}

#[cfg(test)]
mod tests {
    use objc2_core_audio_types::{
        AudioValueRange, kAudioFormatFlagIsAlignedHigh, kAudioFormatFlagIsNonMixable,
        kAudioFormatFlagIsPacked, kAudioFormatFlagIsSignedInteger,
    };

    use super::*;
    use objc2_core_audio_types::kAudioFormatLinearPCM;

    const SOURCE_16: PcmFormat = PcmFormat {
        sample_rate: 48_000,
        bits_per_sample: 16,
        channels: 2,
    };
    const SOURCE_24: PcmFormat = PcmFormat {
        sample_rate: 48_000,
        bits_per_sample: 24,
        channels: 2,
    };
    const PACKED_NON_MIXABLE: u32 =
        kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked | kAudioFormatFlagIsNonMixable;
    const ALIGNED_HIGH_NON_MIXABLE: u32 = kAudioFormatFlagIsSignedInteger
        | kAudioFormatFlagIsAlignedHigh
        | kAudioFormatFlagIsNonMixable;

    #[test]
    fn integer_candidate_preserves_probed_flags_and_sets_explicit_rate() {
        let ranged = AudioStreamRangedDescription {
            mFormat: format(24, 8, ALIGNED_HIGH_NON_MIXABLE),
            mSampleRateRange: AudioValueRange {
                mMinimum: 44_100.0,
                mMaximum: 192_000.0,
            },
        };

        let candidate = integer_candidate(ranged, SOURCE_24).unwrap();

        assert_eq!(candidate.mSampleRate, 48_000.0);
        assert_eq!(candidate.mFormatFlags, 0x54);
    }
    #[test]
    fn integer_candidate_rejects_mixable_and_fractional_width_formats() {
        let range = AudioValueRange {
            mMinimum: 44_100.0,
            mMaximum: 192_000.0,
        };

        assert!(
            integer_candidate(
                AudioStreamRangedDescription {
                    mFormat: format(24, 6, kAudioFormatFlagIsSignedInteger),
                    mSampleRateRange: range,
                },
                SOURCE_16,
            )
            .is_none()
        );
        assert!(
            integer_candidate(
                AudioStreamRangedDescription {
                    mFormat: format(
                        20,
                        8,
                        kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsNonMixable,
                    ),
                    mSampleRateRange: range,
                },
                SOURCE_16,
            )
            .is_none()
        );
    }
    #[test]
    fn integer_candidate_rank_prefers_exact_width() {
        let non_mixable_wide = format(32, 8, PACKED_NON_MIXABLE);
        let non_mixable_exact = format(24, 8, ALIGNED_HIGH_NON_MIXABLE);

        assert!(
            integer_candidate_rank(non_mixable_exact, SOURCE_24)
                < integer_candidate_rank(non_mixable_wide, SOURCE_24)
        );
    }
    fn format(
        bits_per_channel: u32,
        bytes_per_frame: u32,
        flags: u32,
    ) -> AudioStreamBasicDescription {
        AudioStreamBasicDescription {
            mSampleRate: 0.0,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: flags,
            mBytesPerPacket: bytes_per_frame,
            mFramesPerPacket: 1,
            mBytesPerFrame: bytes_per_frame,
            mChannelsPerFrame: 2,
            mBitsPerChannel: bits_per_channel,
            mReserved: 0,
        }
    }
}
