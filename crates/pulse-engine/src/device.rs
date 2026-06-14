use objc2_core_audio::{
    kAudioDevicePropertyDeviceUID, kAudioDevicePropertyStreamConfiguration,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices,
    kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeOutput,
    kAudioObjectSystemObject, kAudioObjectUnknown,
};

use crate::{PcmFormat, error::EngineError, hal};

/// Core Audio `AudioObjectID`.
pub type DeviceId = u32;

#[derive(Debug, Clone)]
pub struct Device {
    pub id: DeviceId,
    pub uid: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct FormatValidation {
    pub device: Device,
    pub requested: PcmFormat,
    pub nominal_sample_rate: f64,
    pub physical_format: PhysicalFormat,
}

#[derive(Debug, Clone)]
pub struct PhysicalFormat {
    pub stream_id: DeviceId,
    pub sample_rate: f64,
    pub format_flags: u32,
    pub bytes_per_packet: u32,
    pub frames_per_packet: u32,
    pub bytes_per_frame: u32,
    pub channels_per_frame: u32,
    pub bits_per_channel: u32,
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
    if id == kAudioObjectUnknown {
        return Err(EngineError::NoOutputDevice);
    }
    device_from_id(id)
}

pub fn validate_output_format(
    device_id: DeviceId,
    requested: PcmFormat,
) -> Result<FormatValidation, EngineError> {
    let device = device_from_id(device_id)?;
    let _hog = hal::HogGuard::acquire(device_id)?;
    let nominal_sample_rate = hal::set_nominal_sample_rate(device_id, requested)?;
    let selected = hal::set_matching_physical_format(device_id, requested)?;
    let format = selected.format;

    Ok(FormatValidation {
        device,
        requested,
        nominal_sample_rate,
        physical_format: PhysicalFormat {
            stream_id: selected.stream_id,
            sample_rate: format.mSampleRate,
            format_flags: format.mFormatFlags,
            bytes_per_packet: format.mBytesPerPacket,
            frames_per_packet: format.mFramesPerPacket,
            bytes_per_frame: format.mBytesPerFrame,
            channels_per_frame: format.mChannelsPerFrame,
            bits_per_channel: format.mBitsPerChannel,
        },
    })
}

fn device_from_id(id: DeviceId) -> Result<Device, EngineError> {
    if id == kAudioObjectUnknown {
        return Err(EngineError::NoOutputDevice);
    }

    Ok(Device {
        id,
        uid: device_uid(id)?,
        name: device_name(id)?,
    })
}

fn device_uid(id: DeviceId) -> Result<String, EngineError> {
    hal::get_cf_string(
        id,
        hal::address(
            kAudioDevicePropertyDeviceUID,
            kAudioObjectPropertyScopeGlobal,
        ),
        "AudioObjectGetPropertyData(kAudioDevicePropertyDeviceUID)",
    )
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
