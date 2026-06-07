use crate::error::EngineError;

/// Core Audio `AudioObjectID`.
pub type DeviceId = u32;

#[derive(Debug, Clone)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
}

pub fn list_output_devices() -> Result<Vec<Device>, EngineError> {
    todo!()
}

pub fn default_output_device() -> Result<Device, EngineError> {
    todo!()
}
