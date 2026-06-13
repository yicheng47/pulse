//! Safe wrapper over the AudioObject property API: hog mode, physical format,
//! nominal sample rate, property listeners.
//!
//! Crib sheet: coreaudio-rs `macos_helpers` (post-PR #128 it uses these same
//! objc2 bindings). Rate/format switches are async — always wait on a property
//! listener before trusting the new state.

use std::{
    ffi::c_void,
    mem, ptr,
    ptr::NonNull,
    thread,
    time::{Duration, Instant},
};

use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectPropertyElement, AudioObjectPropertyScope,
    AudioObjectPropertySelector, AudioObjectSetPropertyData, AudioStreamRangedDescription,
    kAudioDevicePropertyAvailableNominalSampleRates, kAudioDevicePropertyHogMode,
    kAudioDevicePropertyNominalSampleRate, kAudioDevicePropertyStreams, kAudioHardwareNoError,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeOutput, kAudioStreamPropertyAvailablePhysicalFormats,
    kAudioStreamPropertyPhysicalFormat,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, AudioValueRange,
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsSignedInteger, kAudioFormatLinearPCM,
    kAudioStreamAnyRate,
};
use objc2_core_foundation::{CFRetained, CFString};

use crate::{EngineError, PcmFormat};

const HOG_MODE_FREE: i32 = -1;
const FORMAT_SETTLE_TIMEOUT: Duration = Duration::from_secs(2);
const FORMAT_POLL_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Debug, Clone, Copy)]
pub(crate) struct SelectedPhysicalFormat {
    pub stream_id: AudioObjectID,
    pub format: AudioStreamBasicDescription,
}

pub(crate) struct HogGuard {
    device_id: AudioObjectID,
    owns: bool,
}

impl HogGuard {
    pub(crate) fn acquire(device_id: AudioObjectID) -> Result<Self, EngineError> {
        let current_pid = current_pid();
        match hog_owner(device_id)? {
            HOG_MODE_FREE => {
                let owner = toggle_hog_mode(device_id)?;
                if owner == current_pid {
                    Ok(Self {
                        device_id,
                        owns: true,
                    })
                } else if owner == HOG_MODE_FREE {
                    Err(EngineError::Hogged(HOG_MODE_FREE))
                } else {
                    Err(EngineError::Hogged(owner))
                }
            }
            owner if owner == current_pid => Ok(Self {
                device_id,
                owns: false,
            }),
            owner => Err(EngineError::Hogged(owner)),
        }
    }
}

impl Drop for HogGuard {
    fn drop(&mut self) {
        if self.owns && hog_owner(self.device_id).ok() == Some(current_pid()) {
            let _ = toggle_hog_mode(self.device_id);
        }
    }
}

pub(crate) fn address(
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain as AudioObjectPropertyElement,
    }
}

pub(crate) fn get_data_size(
    object_id: AudioObjectID,
    mut address: AudioObjectPropertyAddress,
    call: &'static str,
) -> Result<u32, EngineError> {
    let mut size = 0_u32;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            object_id,
            (&mut address).into(),
            0,
            ptr::null(),
            (&mut size).into(),
        )
    };
    check_status(call, status)?;
    Ok(size)
}

pub(crate) fn get_value<T: Copy>(
    object_id: AudioObjectID,
    mut address: AudioObjectPropertyAddress,
    call: &'static str,
) -> Result<T, EngineError> {
    let mut value = mem::MaybeUninit::<T>::uninit();
    let mut size = mem::size_of::<T>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            object_id,
            (&mut address).into(),
            0,
            ptr::null(),
            (&mut size).into(),
            non_null(value.as_mut_ptr().cast::<c_void>()),
        )
    };
    check_status(call, status)?;
    Ok(unsafe { value.assume_init() })
}

pub(crate) fn set_value<T: Copy>(
    object_id: AudioObjectID,
    mut address: AudioObjectPropertyAddress,
    mut value: T,
    call: &'static str,
) -> Result<(), EngineError> {
    let status = unsafe {
        AudioObjectSetPropertyData(
            object_id,
            (&mut address).into(),
            0,
            ptr::null(),
            mem::size_of::<T>() as u32,
            non_null((&mut value as *mut T).cast::<c_void>()),
        )
    };
    check_status(call, status)
}

pub(crate) fn get_array<T: Copy>(
    object_id: AudioObjectID,
    address: AudioObjectPropertyAddress,
    call: &'static str,
) -> Result<Vec<T>, EngineError> {
    let size = get_data_size(object_id, address, call)?;
    let len = size as usize / mem::size_of::<T>();
    let mut values = Vec::<T>::with_capacity(len);
    let mut read_size = size;
    let mut read_address = address;
    let status = unsafe {
        AudioObjectGetPropertyData(
            object_id,
            (&mut read_address).into(),
            0,
            ptr::null(),
            (&mut read_size).into(),
            non_null(values.as_mut_ptr().cast::<c_void>()),
        )
    };
    check_status(call, status)?;
    unsafe {
        values.set_len(read_size as usize / mem::size_of::<T>());
    }
    Ok(values)
}

pub(crate) fn get_bytes(
    object_id: AudioObjectID,
    address: AudioObjectPropertyAddress,
    call: &'static str,
) -> Result<Vec<u8>, EngineError> {
    let size = get_data_size(object_id, address, call)?;
    let mut bytes = vec![0_u8; size as usize];
    let mut read_size = size;
    let mut read_address = address;
    let status = unsafe {
        AudioObjectGetPropertyData(
            object_id,
            (&mut read_address).into(),
            0,
            ptr::null(),
            (&mut read_size).into(),
            non_null(bytes.as_mut_ptr().cast::<c_void>()),
        )
    };
    check_status(call, status)?;
    bytes.truncate(read_size as usize);
    Ok(bytes)
}

pub(crate) fn get_cf_string(
    object_id: AudioObjectID,
    mut address: AudioObjectPropertyAddress,
    call: &'static str,
) -> Result<String, EngineError> {
    let mut value: Option<NonNull<CFString>> = None;
    let mut size = mem::size_of_val(&value) as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            object_id,
            (&mut address).into(),
            0,
            ptr::null(),
            (&mut size).into(),
            non_null((&mut value as *mut Option<NonNull<CFString>>).cast::<c_void>()),
        )
    };
    check_status(call, status)?;

    let Some(value) = value else {
        return Ok(format!("AudioObject {object_id}"));
    };

    let value = unsafe { CFRetained::from_raw(value) };
    Ok(value.to_string())
}

pub(crate) fn set_nominal_sample_rate(
    device_id: AudioObjectID,
    format: PcmFormat,
) -> Result<f64, EngineError> {
    let requested = f64::from(format.sample_rate);
    if !sample_rate_supported(device_id, requested)? {
        return Err(EngineError::NoMatchingFormat(format));
    }

    let address = address(
        kAudioDevicePropertyNominalSampleRate,
        kAudioObjectPropertyScopeGlobal,
    );
    let current = get_value::<f64>(
        device_id,
        address,
        "AudioObjectGetPropertyData(kAudioDevicePropertyNominalSampleRate)",
    )?;
    if sample_rates_match(current, requested) {
        return Ok(current);
    }

    set_value(
        device_id,
        address,
        requested,
        "AudioObjectSetPropertyData(kAudioDevicePropertyNominalSampleRate)",
    )?;
    wait_for_nominal_sample_rate(device_id, requested)?;
    Ok(requested)
}

pub(crate) fn set_matching_physical_format(
    device_id: AudioObjectID,
    format: PcmFormat,
) -> Result<SelectedPhysicalFormat, EngineError> {
    for stream_id in output_streams(device_id)? {
        for ranged_format in available_physical_formats(stream_id)? {
            let Some(candidate) = matching_physical_format(ranged_format, format) else {
                continue;
            };

            set_physical_format(stream_id, candidate)?;
            return Ok(SelectedPhysicalFormat {
                stream_id,
                format: candidate,
            });
        }
    }

    Err(EngineError::NoMatchingFormat(format))
}

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

fn hog_owner(device_id: AudioObjectID) -> Result<i32, EngineError> {
    get_value::<i32>(
        device_id,
        address(kAudioDevicePropertyHogMode, kAudioObjectPropertyScopeGlobal),
        "AudioObjectGetPropertyData(kAudioDevicePropertyHogMode)",
    )
}

fn toggle_hog_mode(device_id: AudioObjectID) -> Result<i32, EngineError> {
    let mut address = address(kAudioDevicePropertyHogMode, kAudioObjectPropertyScopeGlobal);
    let mut pid = HOG_MODE_FREE;
    let status = unsafe {
        AudioObjectSetPropertyData(
            device_id,
            (&mut address).into(),
            0,
            ptr::null(),
            mem::size_of::<i32>() as u32,
            non_null((&mut pid as *mut i32).cast::<c_void>()),
        )
    };
    check_status(
        "AudioObjectSetPropertyData(kAudioDevicePropertyHogMode)",
        status,
    )?;
    hog_owner(device_id)
}

fn sample_rate_supported(device_id: AudioObjectID, sample_rate: f64) -> Result<bool, EngineError> {
    let ranges = get_array::<AudioValueRange>(
        device_id,
        address(
            kAudioDevicePropertyAvailableNominalSampleRates,
            kAudioObjectPropertyScopeGlobal,
        ),
        "AudioObjectGetPropertyData(kAudioDevicePropertyAvailableNominalSampleRates)",
    )?;
    Ok(ranges
        .iter()
        .any(|range| sample_rate >= range.mMinimum && sample_rate <= range.mMaximum))
}

fn wait_for_nominal_sample_rate(
    device_id: AudioObjectID,
    requested: f64,
) -> Result<(), EngineError> {
    let deadline = Instant::now() + FORMAT_SETTLE_TIMEOUT;
    loop {
        let current = get_value::<f64>(
            device_id,
            address(
                kAudioDevicePropertyNominalSampleRate,
                kAudioObjectPropertyScopeGlobal,
            ),
            "AudioObjectGetPropertyData(kAudioDevicePropertyNominalSampleRate)",
        )?;
        if sample_rates_match(current, requested) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(EngineError::Timeout("nominal sample rate change"));
        }
        thread::sleep(FORMAT_POLL_INTERVAL);
    }
}

fn output_streams(device_id: AudioObjectID) -> Result<Vec<AudioObjectID>, EngineError> {
    get_array::<AudioObjectID>(
        device_id,
        address(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeOutput),
        "AudioObjectGetPropertyData(kAudioDevicePropertyStreams)",
    )
}

fn available_physical_formats(
    stream_id: AudioObjectID,
) -> Result<Vec<AudioStreamRangedDescription>, EngineError> {
    get_array::<AudioStreamRangedDescription>(
        stream_id,
        address(
            kAudioStreamPropertyAvailablePhysicalFormats,
            kAudioObjectPropertyScopeGlobal,
        ),
        "AudioObjectGetPropertyData(kAudioStreamPropertyAvailablePhysicalFormats)",
    )
}

fn matching_physical_format(
    ranged_format: AudioStreamRangedDescription,
    requested: PcmFormat,
) -> Option<AudioStreamBasicDescription> {
    let mut format = ranged_format.mFormat;
    if format.mFormatID != kAudioFormatLinearPCM {
        return None;
    }
    if format.mFormatFlags & kAudioFormatFlagIsFloat != 0 {
        return None;
    }
    if format.mFormatFlags & kAudioFormatFlagIsSignedInteger == 0 {
        return None;
    }
    if format.mChannelsPerFrame < u32::from(requested.channels) {
        return None;
    }
    if format.mBitsPerChannel < u32::from(requested.bits_per_sample) {
        return None;
    }

    let requested_rate = f64::from(requested.sample_rate);
    if !ranged_format_supports_rate(ranged_format, requested_rate) {
        return None;
    }
    if sample_rates_match(format.mSampleRate, kAudioStreamAnyRate) {
        format.mSampleRate = requested_rate;
    }

    Some(format)
}

fn set_physical_format(
    stream_id: AudioObjectID,
    format: AudioStreamBasicDescription,
) -> Result<(), EngineError> {
    let address = address(
        kAudioStreamPropertyPhysicalFormat,
        kAudioObjectPropertyScopeGlobal,
    );
    let current = get_value::<AudioStreamBasicDescription>(
        stream_id,
        address,
        "AudioObjectGetPropertyData(kAudioStreamPropertyPhysicalFormat)",
    )?;
    if physical_formats_match(current, format) {
        return Ok(());
    }

    set_value(
        stream_id,
        address,
        format,
        "AudioObjectSetPropertyData(kAudioStreamPropertyPhysicalFormat)",
    )?;
    wait_for_physical_format(stream_id, format)
}

fn wait_for_physical_format(
    stream_id: AudioObjectID,
    requested: AudioStreamBasicDescription,
) -> Result<(), EngineError> {
    let deadline = Instant::now() + FORMAT_SETTLE_TIMEOUT;
    loop {
        let current = get_value::<AudioStreamBasicDescription>(
            stream_id,
            address(
                kAudioStreamPropertyPhysicalFormat,
                kAudioObjectPropertyScopeGlobal,
            ),
            "AudioObjectGetPropertyData(kAudioStreamPropertyPhysicalFormat)",
        )?;
        if physical_formats_match(current, requested) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(EngineError::Timeout("physical stream format change"));
        }
        thread::sleep(FORMAT_POLL_INTERVAL);
    }
}

fn ranged_format_supports_rate(
    ranged_format: AudioStreamRangedDescription,
    requested_rate: f64,
) -> bool {
    sample_rates_match(ranged_format.mFormat.mSampleRate, requested_rate)
        || (requested_rate >= ranged_format.mSampleRateRange.mMinimum
            && requested_rate <= ranged_format.mSampleRateRange.mMaximum)
}

fn sample_rates_match(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.5
}

fn physical_formats_match(
    left: AudioStreamBasicDescription,
    right: AudioStreamBasicDescription,
) -> bool {
    sample_rates_match(left.mSampleRate, right.mSampleRate)
        && left.mFormatID == right.mFormatID
        && left.mFormatFlags == right.mFormatFlags
        && left.mBytesPerPacket == right.mBytesPerPacket
        && left.mFramesPerPacket == right.mFramesPerPacket
        && left.mBytesPerFrame == right.mBytesPerFrame
        && left.mChannelsPerFrame == right.mChannelsPerFrame
        && left.mBitsPerChannel == right.mBitsPerChannel
}

fn current_pid() -> i32 {
    i32::try_from(std::process::id()).expect("process id must fit in pid_t")
}

fn check_status(call: &'static str, status: i32) -> Result<(), EngineError> {
    if status == kAudioHardwareNoError {
        Ok(())
    } else {
        Err(EngineError::Os { call, status })
    }
}

fn non_null<T>(ptr: *mut T) -> NonNull<T> {
    NonNull::new(ptr).expect("Core Audio output buffer pointer must be non-null")
}

fn read_unaligned<T: Copy>(bytes: &[u8], offset: usize) -> Option<T> {
    let end = offset.checked_add(mem::size_of::<T>())?;
    if end > bytes.len() {
        return None;
    }

    Some(unsafe { ptr::read_unaligned(bytes.as_ptr().add(offset).cast::<T>()) })
}

#[cfg(test)]
mod tests {
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
    fn matching_physical_format_accepts_larger_integer_container() {
        let matched = matching_physical_format(
            ranged_format(0.0, 44_100.0, 44_100.0, 32, kAudioFormatFlagIsSignedInteger),
            PcmFormat {
                sample_rate: 44_100,
                bits_per_sample: 24,
                channels: 2,
            },
        )
        .expect("24-bit source can fit in 32-bit integer physical format");

        assert_eq!(matched.mSampleRate as u32, 44_100);
        assert_eq!(matched.mBitsPerChannel, 32);
    }

    #[test]
    fn matching_physical_format_rejects_float_output() {
        let matched = matching_physical_format(
            ranged_format(44_100.0, 44_100.0, 44_100.0, 32, kAudioFormatFlagIsFloat),
            PcmFormat {
                sample_rate: 44_100,
                bits_per_sample: 24,
                channels: 2,
            },
        );

        assert!(matched.is_none());
    }

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + mem::size_of::<u32>()].copy_from_slice(&value.to_ne_bytes());
    }

    fn ranged_format(
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
                mBytesPerPacket: 8,
                mFramesPerPacket: 1,
                mBytesPerFrame: 8,
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
}
