use objc2_core_audio_types::{
    AudioStreamBasicDescription, kAudioFormatFlagIsAlignedHigh, kAudioFormatFlagIsBigEndian,
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved, kAudioFormatFlagIsSignedInteger,
    kAudioFormatLinearPCM,
};

use crate::{EngineError, PcmFormat};

#[derive(Debug, Clone, Copy)]
pub(super) struct IntPacker {
    source_bytes_per_sample: usize,
    pub(super) source_bytes_per_frame: usize,
    pub(super) output_bytes_per_frame: usize,
    channels: usize,
    low_zero_bytes: usize,
    high_sign_bytes: usize,
}

impl IntPacker {
    pub(super) fn new(
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

    pub(super) fn pack(&self, pcm: &[u8], output: &mut Vec<u8>) {
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

#[cfg(test)]
mod tests {
    use objc2_core_audio_types::{
        kAudioFormatFlagIsAlignedHigh, kAudioFormatFlagIsNonMixable, kAudioFormatFlagIsPacked,
        kAudioFormatFlagIsSignedInteger,
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
