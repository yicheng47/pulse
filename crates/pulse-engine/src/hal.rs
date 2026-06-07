//! Safe wrapper over the AudioObject property API: hog mode, physical format,
//! nominal sample rate, property listeners.
//!
//! Crib sheet: coreaudio-rs `macos_helpers` (post-PR #128 it uses these same
//! objc2 bindings). Rate/format switches are async — always wait on a property
//! listener before trusting the new state.

use std::{ffi::c_void, mem, ptr, ptr::NonNull, slice};

use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectPropertyElement, AudioObjectPropertyScope,
    AudioObjectPropertySelector, kAudioHardwareNoError, kAudioObjectPropertyElementMain,
};
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
    if bytes.len() < mem::size_of::<u32>() {
        return 0;
    }

    let list = bytes
        .as_ptr()
        .cast::<objc2_core_audio_types::AudioBufferList>();
    let buffer_count = unsafe { (*list).mNumberBuffers as usize };
    if buffer_count == 0 {
        return 0;
    }

    let buffers = unsafe {
        slice::from_raw_parts(
            std::ptr::addr_of!((*list).mBuffers).cast::<objc2_core_audio_types::AudioBuffer>(),
            buffer_count,
        )
    };
    buffers.iter().map(|buffer| buffer.mNumberChannels).sum()
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
