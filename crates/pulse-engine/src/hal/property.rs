//! Core Audio is a property system: each system, device, or stream is an
//! AudioObject addressed by a (selector, scope, element) triple. Values are
//! untyped bytes; variable-sized properties need a get-size-then-get-data pair.
//!
//! These typed, Result-returning helpers name the failing C call in errors.
//! They contain the shared property FFI; ownership.rs also uses FFI for the hog
//! toggle, and the audio sinks have their own callback/lifecycle FFI.
//!
//! Reference: coreaudio-rs `macos_helpers` uses the same objc2 bindings after PR #128.

use crate::EngineError;
use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectHasProperty,
    AudioObjectID, AudioObjectIsPropertySettable, AudioObjectPropertyAddress,
    AudioObjectPropertyElement, AudioObjectPropertyScope, AudioObjectPropertySelector,
    AudioObjectSetPropertyData, kAudioHardwareNoError, kAudioObjectPropertyElementMain,
};
use objc2_core_foundation::{CFRetained, CFString};
use std::{ffi::c_void, mem, ptr, ptr::NonNull};

/// Builds the property address triple with element Main — the whole object,
/// as opposed to one channel.
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

/// First half of Core Audio's two-step read: a variable-sized property must
/// be asked for its byte size before the data itself.
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

/// Reads a fixed-size property straight into a `T`. Core Audio just writes
/// bytes, so `T` must match the property's documented layout.
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

/// Two-step read of a variable-length property as a `Vec<T>`. The final
/// element count is whatever the HAL reports at read time — it can shrink
/// between the size query and the read.
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

/// Raw-bytes read for properties without a fixed layout (e.g. the
/// flexible-array `AudioBufferList`); the caller does the parsing.
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

/// Reads a `CFString` property. The HAL hands back a +1-retained reference;
/// `CFRetained::from_raw` adopts it so it is released exactly once. A null
/// string falls back to a synthetic object name.
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

pub(super) fn property_is_settable(
    object_id: AudioObjectID,
    mut address: AudioObjectPropertyAddress,
) -> bool {
    if !has_property(object_id, address) {
        return false;
    }
    let mut settable = 0_u8;
    let status = unsafe {
        AudioObjectIsPropertySettable(
            object_id,
            (&mut address).into(),
            NonNull::from(&mut settable),
        )
    };
    status == kAudioHardwareNoError && settable != 0
}

pub(super) fn has_property(
    object_id: AudioObjectID,
    mut address: AudioObjectPropertyAddress,
) -> bool {
    unsafe { AudioObjectHasProperty(object_id, (&mut address).into()) }
}
pub(crate) fn check_status(call: &'static str, status: i32) -> Result<(), EngineError> {
    if status == kAudioHardwareNoError {
        Ok(())
    } else {
        Err(EngineError::Os { call, status })
    }
}

pub(super) fn non_null<T>(ptr: *mut T) -> NonNull<T> {
    NonNull::new(ptr).expect("Core Audio output buffer pointer must be non-null")
}
