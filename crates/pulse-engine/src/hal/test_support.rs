use objc2_core_audio::AudioStreamRangedDescription;
use objc2_core_audio_types::{AudioStreamBasicDescription, AudioValueRange, kAudioFormatLinearPCM};
use std::mem;

pub(super) fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + mem::size_of::<u32>()].copy_from_slice(&value.to_ne_bytes());
}
pub(super) fn stream_format(sample_rate: f64, format_flags: u32) -> AudioStreamBasicDescription {
    AudioStreamBasicDescription {
        mSampleRate: sample_rate,
        mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: format_flags,
        mBytesPerPacket: 8,
        mFramesPerPacket: 1,
        mBytesPerFrame: 8,
        mChannelsPerFrame: 2,
        mBitsPerChannel: 32,
        mReserved: 0,
    }
}
pub(super) fn ranged_format(
    sample_rate: f64,
    minimum_rate: f64,
    maximum_rate: f64,
    bits_per_channel: u32,
    format_flags: u32,
) -> AudioStreamRangedDescription {
    AudioStreamRangedDescription {
        mFormat: AudioStreamBasicDescription {
            mSampleRate: sample_rate,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: format_flags,
            mBytesPerPacket: bits_per_channel / 8 * 2,
            mFramesPerPacket: 1,
            mBytesPerFrame: bits_per_channel / 8 * 2,
            mChannelsPerFrame: 2,
            mBitsPerChannel: bits_per_channel,
            mReserved: 0,
        },
        mSampleRateRange: AudioValueRange {
            mMinimum: minimum_rate,
            mMaximum: maximum_rate,
        },
    }
}
