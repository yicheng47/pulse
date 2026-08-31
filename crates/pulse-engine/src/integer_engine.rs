use objc2_core_audio::AudioStreamRangedDescription;
use objc2_core_audio_types::{
    AudioStreamBasicDescription, kAudioFormatFlagIsAlignedHigh, kAudioFormatFlagIsBigEndian,
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved, kAudioFormatFlagIsNonMixable,
    kAudioFormatFlagIsSignedInteger, kAudioFormatLinearPCM,
};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::{EngineError, PcmFormat, device, hal, raw_sink};

#[derive(Debug, Clone, Copy)]
struct IntPacker {
    source_bytes_per_sample: usize,
    source_bytes_per_frame: usize,
    output_bytes_per_frame: usize,
    channels: usize,
    low_zero_bytes: usize,
    high_sign_bytes: usize,
}

impl IntPacker {
    fn new(
        source: PcmFormat,
        device_format: AudioStreamBasicDescription,
    ) -> Result<Self, EngineError> {
        if !matches!(source.bits_per_sample, 16 | 24 | 32) {
            return Err(EngineError::UnsupportedFormat(format!(
                "{}-bit PCM is not supported by the integer packer",
                source.bits_per_sample
            )));
        }
        let channels = usize::from(source.channels);
        if channels == 0 {
            return Err(EngineError::UnsupportedFormat(
                "zero-channel playback is not supported".to_string(),
            ));
        }
        if device_format.mFormatID != kAudioFormatLinearPCM
            || device_format.mFormatFlags & kAudioFormatFlagIsFloat != 0
            || device_format.mFormatFlags & kAudioFormatFlagIsSignedInteger == 0
            || device_format.mFormatFlags & kAudioFormatFlagIsBigEndian != 0
            || device_format.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0
            || device_format.mChannelsPerFrame != u32::from(source.channels)
            || device_format.mBitsPerChannel < u32::from(source.bits_per_sample)
            || !device_format.mBitsPerChannel.is_multiple_of(8)
        {
            return Err(EngineError::UnsupportedFormat(
                "selected device format is not compatible interleaved native-endian integer PCM"
                    .to_string(),
            ));
        }

        let output_bytes_per_frame =
            usize::try_from(device_format.mBytesPerFrame).map_err(|_| {
                EngineError::UnsupportedFormat(
                    "device bytes per frame do not fit usize".to_string(),
                )
            })?;
        if output_bytes_per_frame % channels != 0 {
            return Err(EngineError::UnsupportedFormat(
                "device bytes per frame are not channel-aligned".to_string(),
            ));
        }
        let source_bytes_per_sample = usize::from(source.bits_per_sample).div_ceil(8);
        let output_bytes_per_sample = output_bytes_per_frame / channels;
        let device_bytes_per_sample = device_format.mBitsPerChannel as usize / 8;
        if !(source_bytes_per_sample..=4).contains(&device_bytes_per_sample)
            || !(device_bytes_per_sample..=4).contains(&output_bytes_per_sample)
        {
            return Err(EngineError::UnsupportedFormat(
                "integer device containers wider than 32 bits are not supported".to_string(),
            ));
        }
        let low_zero_bytes = if device_format.mFormatFlags & kAudioFormatFlagIsAlignedHigh != 0 {
            output_bytes_per_sample
        } else {
            device_bytes_per_sample
        } - source_bytes_per_sample;
        let high_sign_bytes = output_bytes_per_sample - source_bytes_per_sample - low_zero_bytes;

        Ok(Self {
            source_bytes_per_sample,
            source_bytes_per_frame: source.bytes_per_frame(),
            output_bytes_per_frame,
            channels,
            low_zero_bytes,
            high_sign_bytes,
        })
    }

    fn pack(&self, pcm: &[u8], output: &mut Vec<u8>) {
        for frame in pcm.chunks_exact(self.source_bytes_per_frame) {
            for channel in 0..self.channels {
                let offset = channel * self.source_bytes_per_sample;
                let sample = &frame[offset..offset + self.source_bytes_per_sample];
                output.extend(std::iter::repeat_n(0, self.low_zero_bytes));
                output.extend_from_slice(sample);
                let sign = if sample[self.source_bytes_per_sample - 1] & 0x80 != 0 {
                    0xff
                } else {
                    0x00
                };
                output.extend(std::iter::repeat_n(sign, self.high_sign_bytes));
            }
        }
    }
}

pub(crate) struct IntegerEngine {
    device: device::DeviceId,
    format_restore: Option<hal::FormatRestoreGuard>,
    _hog: hal::HogGuard,
    hardware_volume: Option<hal::HardwareVolume>,
    hardware_volume_event_pending: bool,
    producer: Option<Producer<u8>>,
    consumer: Option<Consumer<u8>>,
    sink: Option<raw_sink::RawSink>,
    format: Option<PcmFormat>,
    device_format: Option<AudioStreamBasicDescription>,
    packer: Option<IntPacker>,
    pack_buffer: Vec<u8>,
}

impl IntegerEngine {
    pub(crate) fn open(device: device::DeviceId) -> Result<Self, EngineError> {
        let hog = hal::HogGuard::acquire(device)?;
        if !hog.owns() {
            return Err(EngineError::Hogged(
                i32::try_from(std::process::id()).expect("process id must fit in pid_t"),
            ));
        }
        let format_restore = hal::FormatRestoreGuard::capture(device)?;
        hal::set_mixing_enabled(device, false)?;
        let hardware_volume = hal::hardware_volume_control(device);
        let hardware_volume_event_pending = hardware_volume.is_some();

        Ok(Self {
            device,
            format_restore: Some(format_restore),
            _hog: hog,
            hardware_volume,
            hardware_volume_event_pending,
            producer: None,
            consumer: None,
            sink: None,
            format: None,
            device_format: None,
            packer: None,
            pack_buffer: Vec::new(),
        })
    }

    pub(crate) fn set_format(&mut self, format: PcmFormat) -> Result<(), EngineError> {
        if self.format == Some(format) {
            return Ok(());
        }

        let (stream_id, device_format) = select_integer_format(self.device, format)?;
        let packer = IntPacker::new(format, device_format)?;
        self.pause()?;
        self.sink = None;
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
        if let Some(sink) = &mut self.sink {
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
                self.sink = Some(sink);
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
        if let Some(sink) = &mut self.sink {
            sink.stop()?;
        }
        Ok(())
    }

    pub(crate) fn feed(&mut self, pcm: &[u8]) -> usize {
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
        self.sink
            .as_ref()
            .map_or(0, raw_sink::RawSink::position_frames)
    }

    pub(crate) fn underrun_frames(&self) -> u64 {
        self.sink
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

    pub(crate) fn set_volume(&mut self, level: f32, muted: bool) -> Result<(), EngineError> {
        if let Some(hardware_volume) = &mut self.hardware_volume {
            hardware_volume.set_volume(level, muted)?;
        }
        Ok(())
    }

    pub(crate) fn release(mut self) -> Result<(), EngineError> {
        let mut errors = Vec::new();
        if let Err(error) = self.pause() {
            errors.push(error);
        }
        self.sink = None;
        if let Some(guard) = self.format_restore.take() {
            errors.extend(guard.restore());
        }
        collected_release_result(errors)
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
        let _ = self.pause();
        self.sink = None;
    }
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
    if format.mFormatID != kAudioFormatLinearPCM
        || format.mFormatFlags & kAudioFormatFlagIsFloat != 0
        || format.mFormatFlags & kAudioFormatFlagIsSignedInteger == 0
        || format.mFormatFlags & kAudioFormatFlagIsNonMixable == 0
        || format.mFormatFlags & kAudioFormatFlagIsBigEndian != 0
        || format.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0
        || format.mChannelsPerFrame != u32::from(source.channels)
        || format.mBitsPerChannel < u32::from(source.bits_per_sample)
        || !format.mBitsPerChannel.is_multiple_of(8)
        || format.mBytesPerFrame == 0
        || format.mChannelsPerFrame == 0
        || !format
            .mBytesPerFrame
            .is_multiple_of(format.mChannelsPerFrame)
        || format.mBytesPerFrame / format.mChannelsPerFrame
            < u32::from(source.bits_per_sample).div_ceil(8)
        || format.mBytesPerFrame / format.mChannelsPerFrame > 4
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

fn collected_release_result(mut errors: Vec<EngineError>) -> Result<(), EngineError> {
    match errors.len() {
        0 => Ok(()),
        1 => Err(errors.pop().expect("one collected error must exist")),
        _ => Err(EngineError::BackendRelease(
            errors
                .into_iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        )),
    }
}

#[cfg(test)]
mod tests {
    use objc2_core_audio_types::{
        AudioValueRange, kAudioFormatFlagIsAlignedHigh, kAudioFormatFlagIsNonMixable,
        kAudioFormatFlagIsPacked, kAudioFormatFlagIsSignedInteger,
    };

    use super::*;

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
    const SOURCE_32: PcmFormat = PcmFormat {
        sample_rate: 48_000,
        bits_per_sample: 32,
        channels: 2,
    };
    const PACKED_NON_MIXABLE: u32 =
        kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked | kAudioFormatFlagIsNonMixable;
    const ALIGNED_HIGH_NON_MIXABLE: u32 = kAudioFormatFlagIsSignedInteger
        | kAudioFormatFlagIsAlignedHigh
        | kAudioFormatFlagIsNonMixable;

    #[test]
    fn packer_copies_16_bit_into_16_bit_packed_0x4c() {
        assert_eq!(PACKED_NON_MIXABLE, 0x4c);
        assert_pack(
            SOURCE_16,
            format(16, 4, PACKED_NON_MIXABLE),
            &[0x00, 0x80, 0xff, 0xff, 0x00, 0x00, 0xff, 0x7f],
            &[0x00, 0x80, 0xff, 0xff, 0x00, 0x00, 0xff, 0x7f],
        );
    }

    #[test]
    fn packer_copies_24_bit_into_24_bit_packed_0x4c() {
        assert_pack(
            SOURCE_24,
            format(24, 6, PACKED_NON_MIXABLE),
            &[
                0x00, 0x00, 0x80, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0xff, 0xff, 0x7f,
            ],
            &[
                0x00, 0x00, 0x80, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0xff, 0xff, 0x7f,
            ],
        );
    }

    #[test]
    fn packer_shifts_24_bit_into_aligned_high_32_bit_0x54() {
        assert_eq!(ALIGNED_HIGH_NON_MIXABLE, 0x54);
        assert_pack(
            SOURCE_24,
            format(24, 8, ALIGNED_HIGH_NON_MIXABLE),
            &[
                0x00, 0x00, 0x80, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0xff, 0xff, 0x7f,
            ],
            &[
                0x00, 0x00, 0x00, 0x80, 0x00, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff,
                0xff, 0x7f,
            ],
        );
    }

    #[test]
    fn packer_shifts_24_bit_into_32_bit_packed_0x4c() {
        assert_pack(
            SOURCE_24,
            format(32, 8, PACKED_NON_MIXABLE),
            &[
                0x00, 0x00, 0x80, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0xff, 0xff, 0x7f,
            ],
            &[
                0x00, 0x00, 0x00, 0x80, 0x00, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff,
                0xff, 0x7f,
            ],
        );
    }

    #[test]
    fn packer_sign_extends_24_bit_in_a_low_aligned_32_bit_container() {
        assert_pack(
            SOURCE_24,
            format(24, 8, kAudioFormatFlagIsSignedInteger),
            &[
                0x00, 0x00, 0x80, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0xff, 0xff, 0x7f,
            ],
            &[
                0x00, 0x00, 0x80, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff,
                0x7f, 0x00,
            ],
        );
    }

    #[test]
    fn packer_copies_32_bit_packed_0x4c() {
        let input = [
            0x00, 0x00, 0x00, 0x80, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff,
            0xff, 0x7f,
        ];
        assert_pack(SOURCE_32, format(32, 8, PACKED_NON_MIXABLE), &input, &input);
    }

    #[test]
    fn packer_shifts_16_bit_into_32_bit_aligned_high_and_packed() {
        let input = [0x00, 0x80, 0xff, 0x7f];
        assert_pack(
            SOURCE_16,
            format(32, 8, ALIGNED_HIGH_NON_MIXABLE),
            &input,
            &[0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0xff, 0x7f],
        );
        assert_pack(
            SOURCE_16,
            format(32, 8, PACKED_NON_MIXABLE),
            &input,
            &[0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0xff, 0x7f],
        );
    }

    #[test]
    fn packer_shifts_16_bit_into_24_bit_packed() {
        assert_pack(
            SOURCE_16,
            format(24, 6, PACKED_NON_MIXABLE),
            &[0x00, 0x80, 0xff, 0x7f],
            &[0x00, 0x00, 0x80, 0x00, 0xff, 0x7f],
        );
    }

    #[test]
    fn packer_places_16_bit_in_a_low_aligned_24_bit_field() {
        assert_pack(
            SOURCE_16,
            format(24, 8, kAudioFormatFlagIsSignedInteger),
            &[0x00, 0x80, 0xff, 0x7f],
            &[0x00, 0x00, 0x80, 0xff, 0x00, 0xff, 0x7f, 0x00],
        );
    }

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

    fn assert_pack(
        source: PcmFormat,
        device_format: AudioStreamBasicDescription,
        input: &[u8],
        expected: &[u8],
    ) {
        let packer = IntPacker::new(source, device_format).unwrap();
        let mut output = Vec::new();
        packer.pack(input, &mut output);
        assert_eq!(output, expected);
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
