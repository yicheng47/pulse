//! Safe wrapper over the AudioObject property API: hog mode, physical format,
//! nominal sample rate, property listeners.
//!
//! Crib sheet: coreaudio-rs `macos_helpers` (post-PR #128 it uses these same
//! objc2 bindings). Rate/format switches are async — always wait on a property
//! listener before trusting the new state.

use std::{ffi::c_void, mem, ptr, ptr::NonNull};

use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectPropertyElement, AudioObjectPropertyScope,
    AudioObjectPropertySelector, kAudioHardwareNoError, kAudioObjectPropertyElementMain,
};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList};
use objc2_core_foundation::{CFRetained, CFString};

use crate::EngineError;

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

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + mem::size_of::<u32>()].copy_from_slice(&value.to_ne_bytes());
    }
}
