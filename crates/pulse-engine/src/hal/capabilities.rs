use super::{
    formats::{available_physical_formats, output_streams},
    property::{address, get_value, has_property},
};
use crate::EngineError;
use objc2_core_audio::{
    AudioObjectID, AudioStreamRangedDescription, kAudioDevicePropertyTransportType,
    kAudioDeviceTransportTypeUnknown, kAudioObjectPropertyScopeGlobal,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, kAudioFormatFlagIsBigEndian,
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved, kAudioFormatFlagIsNonMixable,
    kAudioFormatFlagIsSignedInteger, kAudioFormatLinearPCM,
};
use std::{mem, ptr};

/// Maximum integer bit depth and sample rate across all output streams, plus
/// whether any format is safe for the integer engine and the transport type.
/// A missing bit depth means the device only offers mixable float, where bit
/// depth is meaningless.
pub(crate) struct ProbedOutputDeviceCapabilities {
    pub(crate) max_bits_per_channel: Option<u32>,
    pub(crate) max_sample_rate: f64,
    pub(crate) integer_wire_formats: bool,
    pub(crate) transport_type: u32,
}

pub(crate) fn output_device_capabilities(
    device_id: AudioObjectID,
) -> Result<Option<ProbedOutputDeviceCapabilities>, EngineError> {
    let mut formats = Vec::new();
    for stream_id in output_streams(device_id)? {
        formats.extend(available_physical_formats(stream_id)?);
    }
    let Some((max_bits_per_channel, max_sample_rate, integer_wire_formats)) =
        maximum_physical_format_capabilities(&formats)
    else {
        return Ok(None);
    };
    let transport_address = address(
        kAudioDevicePropertyTransportType,
        kAudioObjectPropertyScopeGlobal,
    );
    let transport_type = if has_property(device_id, transport_address) {
        get_value::<u32>(
            device_id,
            transport_address,
            "AudioObjectGetPropertyData(kAudioDevicePropertyTransportType)",
        )?
    } else {
        kAudioDeviceTransportTypeUnknown
    };
    Ok(Some(ProbedOutputDeviceCapabilities {
        max_bits_per_channel,
        max_sample_rate,
        integer_wire_formats,
        transport_type,
    }))
}

/// Sums output channels from raw `AudioBufferList` bytes. The C struct ends
/// in a flexible array member, so it cannot be read as one typed value —
/// fields are decoded manually with unaligned reads, tolerating truncation.
pub(crate) fn audio_buffer_list_channel_count(bytes: &[u8]) -> u32 {
    let Some(buffer_count) =
        read_unaligned::<u32>(bytes, mem::offset_of!(AudioBufferList, mNumberBuffers))
    else {
        return 0;
    };
    let Ok(buffer_count) = usize::try_from(buffer_count) else {
        return 0;
    };

    let buffers_offset = mem::offset_of!(AudioBufferList, mBuffers);
    let buffer_size = mem::size_of::<AudioBuffer>();
    let Some(required_len) = buffer_count
        .checked_mul(buffer_size)
        .and_then(|buffer_bytes| buffers_offset.checked_add(buffer_bytes))
    else {
        return 0;
    };
    if bytes.len() < required_len {
        return 0;
    }

    (0..buffer_count)
        .filter_map(|index| {
            let buffer_offset = buffers_offset + index * buffer_size;
            read_unaligned::<u32>(
                bytes,
                buffer_offset + mem::offset_of!(AudioBuffer, mNumberChannels),
            )
        })
        .sum()
}

pub(crate) fn is_integer_wire_format(format: &AudioStreamBasicDescription) -> bool {
    format.mFormatID == kAudioFormatLinearPCM
        && format.mFormatFlags & kAudioFormatFlagIsSignedInteger != 0
        && format.mFormatFlags & kAudioFormatFlagIsFloat == 0
        && format.mFormatFlags & kAudioFormatFlagIsNonMixable != 0
        && format.mFormatFlags & kAudioFormatFlagIsBigEndian == 0
        && format.mFormatFlags & kAudioFormatFlagIsNonInterleaved == 0
        && format.mBitsPerChannel > 0
        && format.mBitsPerChannel.is_multiple_of(8)
        && format.mChannelsPerFrame > 0
        && format.mBytesPerFrame > 0
        && format
            .mBytesPerFrame
            .is_multiple_of(format.mChannelsPerFrame)
        && format.mBytesPerFrame / format.mChannelsPerFrame <= 4
}

fn maximum_physical_format_capabilities(
    formats: &[AudioStreamRangedDescription],
) -> Option<(Option<u32>, f64, bool)> {
    let mut maximum: Option<(u32, f64)> = None;
    let mut maximum_mixable_float_rate: Option<f64> = None;
    let mut integer_wire_formats = false;

    for ranged_format in formats {
        let format = ranged_format.mFormat;
        if format.mFormatID != kAudioFormatLinearPCM {
            continue;
        }

        let sample_rate = format
            .mSampleRate
            .max(ranged_format.mSampleRateRange.mMaximum);
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            continue;
        }

        integer_wire_formats |= is_integer_wire_format(&format);

        if format.mFormatFlags & kAudioFormatFlagIsFloat != 0
            && format.mFormatFlags & kAudioFormatFlagIsNonMixable == 0
        {
            maximum_mixable_float_rate = Some(
                maximum_mixable_float_rate.map_or(sample_rate, |maximum| maximum.max(sample_rate)),
            );
            continue;
        }

        if format.mFormatFlags & kAudioFormatFlagIsSignedInteger == 0 || format.mBitsPerChannel == 0
        {
            continue;
        }

        maximum = Some(match maximum {
            Some((max_bits, max_rate)) => (
                max_bits.max(format.mBitsPerChannel),
                max_rate.max(sample_rate),
            ),
            None => (format.mBitsPerChannel, sample_rate),
        });
    }

    maximum
        .map(|(bits, rate)| (Some(bits), rate, integer_wire_formats))
        .or_else(|| maximum_mixable_float_rate.map(|rate| (None, rate, integer_wire_formats)))
}

/// Property bytes carry no alignment guarantee, hence `ptr::read_unaligned`.
fn read_unaligned<T: Copy>(bytes: &[u8], offset: usize) -> Option<T> {
    let end = offset.checked_add(mem::size_of::<T>())?;
    if end > bytes.len() {
        return None;
    }

    Some(unsafe { ptr::read_unaligned(bytes.as_ptr().add(offset).cast::<T>()) })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{ranged_format, stream_format, write_u32};
    use super::*;

    #[test]
    fn audio_buffer_list_channel_count_handles_truncated_header() {
        assert_eq!(audio_buffer_list_channel_count(&[1, 0, 0]), 0);
    }
    #[test]
    fn audio_buffer_list_channel_count_handles_truncated_buffers() {
        let mut bytes =
            vec![
                0_u8;
                mem::offset_of!(AudioBufferList, mBuffers) + mem::size_of::<AudioBuffer>() - 1
            ];
        write_u32(
            &mut bytes,
            mem::offset_of!(AudioBufferList, mNumberBuffers),
            1,
        );

        assert_eq!(audio_buffer_list_channel_count(&bytes), 0);
    }
    #[test]
    fn audio_buffer_list_channel_count_sums_channels() {
        let buffers_offset = mem::offset_of!(AudioBufferList, mBuffers);
        let buffer_size = mem::size_of::<AudioBuffer>();
        let mut bytes = vec![0_u8; buffers_offset + buffer_size * 2];

        write_u32(
            &mut bytes,
            mem::offset_of!(AudioBufferList, mNumberBuffers),
            2,
        );
        write_u32(
            &mut bytes,
            buffers_offset + mem::offset_of!(AudioBuffer, mNumberChannels),
            2,
        );
        write_u32(
            &mut bytes,
            buffers_offset + buffer_size + mem::offset_of!(AudioBuffer, mNumberChannels),
            6,
        );

        assert_eq!(audio_buffer_list_channel_count(&bytes), 8);
    }
    #[test]
    fn maximum_capabilities_pick_independent_pcm_bit_and_rate_maxima() {
        let formats = [
            ranged_format(
                44_100.0,
                44_100.0,
                96_000.0,
                32,
                kAudioFormatFlagIsSignedInteger,
            ),
            ranged_format(
                0.0,
                44_100.0,
                192_000.0,
                24,
                kAudioFormatFlagIsSignedInteger,
            ),
            ranged_format(384_000.0, 384_000.0, 384_000.0, 64, kAudioFormatFlagIsFloat),
        ];

        assert_eq!(
            maximum_physical_format_capabilities(&formats),
            Some((Some(32), 192_000.0, false))
        );
    }
    #[test]
    fn integer_wire_format_matches_stage_one_probe_flags_and_layout() {
        for flags in [0x54, 0x4c] {
            assert!(is_integer_wire_format(&stream_format(48_000.0, flags)));
        }
        for flags in [0x14, 0x0c, 0x04] {
            assert!(!is_integer_wire_format(&stream_format(48_000.0, flags)));
        }

        let mut format = stream_format(48_000.0, 0x54);
        format.mFormatFlags |= kAudioFormatFlagIsBigEndian;
        assert!(!is_integer_wire_format(&format));

        format = stream_format(48_000.0, 0x54);
        format.mFormatFlags |= kAudioFormatFlagIsNonInterleaved;
        assert!(!is_integer_wire_format(&format));

        format = stream_format(48_000.0, 0x54);
        format.mBytesPerFrame = 10;
        assert!(!is_integer_wire_format(&format));
    }
    #[test]
    fn maximum_capabilities_keep_mixable_integer_depth_without_an_integer_wire_format() {
        let formats = [ranged_format(
            0.0,
            44_100.0,
            192_000.0,
            24,
            kAudioFormatFlagIsSignedInteger,
        )];

        assert_eq!(
            maximum_physical_format_capabilities(&formats),
            Some((Some(24), 192_000.0, false))
        );
    }
    #[test]
    fn maximum_capabilities_fall_back_to_mixable_float_sample_rate() {
        let formats = [
            ranged_format(0.0, 44_100.0, 48_000.0, 32, kAudioFormatFlagIsFloat),
            ranged_format(
                0.0,
                44_100.0,
                192_000.0,
                32,
                kAudioFormatFlagIsFloat | kAudioFormatFlagIsNonMixable,
            ),
        ];

        assert_eq!(
            maximum_physical_format_capabilities(&formats),
            Some((None, 48_000.0, false))
        );
    }
}
