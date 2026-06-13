use objc2_core_audio::{
    kAudioDevicePropertyStreamConfiguration, kAudioHardwarePropertyDefaultOutputDevice,
    kAudioHardwarePropertyDevices, kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
};

use crate::{error::EngineError, hal};

/// Core Audio `AudioObjectID`.
pub type DeviceId = u32;

#[derive(Debug, Clone)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
}

pub fn list_output_devices() -> Result<Vec<Device>, EngineError> {
    let devices = hal::get_array::<DeviceId>(
        kAudioObjectSystemObject as DeviceId,
        hal::address(
            kAudioHardwarePropertyDevices,
            kAudioObjectPropertyScopeGlobal,
        ),
        "AudioObjectGetPropertyData(kAudioHardwarePropertyDevices)",
    )?;

    devices
        .into_iter()
        .filter_map(|id| match has_output_channels(id) {
            Ok(true) => Some(device_from_id(id)),
            Ok(false) => None,
            Err(err) => Some(Err(err)),
        })
        .collect()
}

pub fn default_output_device() -> Result<Device, EngineError> {
    let id = hal::get_value::<DeviceId>(
        kAudioObjectSystemObject as DeviceId,
        hal::address(
            kAudioHardwarePropertyDefaultOutputDevice,
            kAudioObjectPropertyScopeGlobal,
        ),
        "AudioObjectGetPropertyData(kAudioHardwarePropertyDefaultOutputDevice)",
    )?;
    device_from_id(id)
}

fn device_from_id(id: DeviceId) -> Result<Device, EngineError> {
    Ok(Device {
        id,
        name: device_name(id)?,
    })
}

fn device_name(id: DeviceId) -> Result<String, EngineError> {
    hal::get_cf_string(
        id,
        hal::address(kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal),
        "AudioObjectGetPropertyData(kAudioObjectPropertyName)",
    )
}

fn has_output_channels(id: DeviceId) -> Result<bool, EngineError> {
    let bytes = hal::get_bytes(
        id,
        hal::address(
            kAudioDevicePropertyStreamConfiguration,
            kAudioObjectPropertyScopeOutput,
        ),
        "AudioObjectGetPropertyData(kAudioDevicePropertyStreamConfiguration)",
    )?;
    Ok(hal::audio_buffer_list_channel_count(&bytes) > 0)
}
