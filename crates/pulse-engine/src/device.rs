use objc2_core_audio::{
    kAudioDevicePropertyDeviceUID, kAudioDevicePropertyStreamConfiguration,
    kAudioDeviceTransportTypeAVB, kAudioDeviceTransportTypeAggregate,
    kAudioDeviceTransportTypeAirPlay, kAudioDeviceTransportTypeBluetooth,
    kAudioDeviceTransportTypeBluetoothLE, kAudioDeviceTransportTypeBuiltIn,
    kAudioDeviceTransportTypeDisplayPort, kAudioDeviceTransportTypeFireWire,
    kAudioDeviceTransportTypeHDMI, kAudioDeviceTransportTypePCI,
    kAudioDeviceTransportTypeThunderbolt, kAudioDeviceTransportTypeUSB,
    kAudioDeviceTransportTypeUnknown, kAudioDeviceTransportTypeVirtual,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices,
    kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeOutput,
    kAudioObjectSystemObject, kAudioObjectUnknown,
};

use crate::{error::EngineError, hal};

/// Core Audio `AudioObjectID`.
pub type DeviceId = u32;

#[derive(Debug, Clone)]
pub struct Device {
    pub id: DeviceId,
    pub uid: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputDeviceCapabilities {
    pub max_bits_per_channel: Option<u32>,
    pub max_sample_rate: f64,
    pub transport: DeviceTransport,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DeviceTransport {
    Unknown,
    BuiltIn,
    Aggregate,
    Virtual,
    Pci,
    Usb,
    FireWire,
    Bluetooth,
    BluetoothLe,
    Hdmi,
    DisplayPort,
    AirPlay,
    Avb,
    Thunderbolt,
    Other,
}

impl DeviceTransport {
    pub fn supports_bit_perfect(self) -> bool {
        !matches!(
            self,
            Self::Bluetooth | Self::BluetoothLe | Self::Hdmi | Self::DisplayPort
        )
    }
}

impl From<u32> for DeviceTransport {
    fn from(value: u32) -> Self {
        if value == kAudioDeviceTransportTypeUnknown {
            Self::Unknown
        } else if value == kAudioDeviceTransportTypeBuiltIn {
            Self::BuiltIn
        } else if value == kAudioDeviceTransportTypeAggregate {
            Self::Aggregate
        } else if value == kAudioDeviceTransportTypeVirtual {
            Self::Virtual
        } else if value == kAudioDeviceTransportTypePCI {
            Self::Pci
        } else if value == kAudioDeviceTransportTypeUSB {
            Self::Usb
        } else if value == kAudioDeviceTransportTypeFireWire {
            Self::FireWire
        } else if value == kAudioDeviceTransportTypeBluetooth {
            Self::Bluetooth
        } else if value == kAudioDeviceTransportTypeBluetoothLE {
            Self::BluetoothLe
        } else if value == kAudioDeviceTransportTypeHDMI {
            Self::Hdmi
        } else if value == kAudioDeviceTransportTypeDisplayPort {
            Self::DisplayPort
        } else if value == kAudioDeviceTransportTypeAirPlay {
            Self::AirPlay
        } else if value == kAudioDeviceTransportTypeAVB {
            Self::Avb
        } else if value == kAudioDeviceTransportTypeThunderbolt {
            Self::Thunderbolt
        } else {
            Self::Other
        }
    }
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

pub fn output_device_capabilities(
    device_id: DeviceId,
) -> Result<OutputDeviceCapabilities, EngineError> {
    let Some((max_bits_per_channel, max_sample_rate, transport_type)) =
        hal::output_device_capabilities(device_id)?
    else {
        return Err(EngineError::NoOutputCapabilities(device_id));
    };

    Ok(OutputDeviceCapabilities {
        max_bits_per_channel,
        max_sample_rate,
        transport: transport_type.into(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_bluetooth_transports_gate_the_integer_engine() {
        for transport in [
            DeviceTransport::Bluetooth,
            DeviceTransport::BluetoothLe,
            DeviceTransport::Hdmi,
            DeviceTransport::DisplayPort,
        ] {
            assert!(!transport.supports_bit_perfect());
        }
        for transport in [
            DeviceTransport::Unknown,
            DeviceTransport::BuiltIn,
            DeviceTransport::Usb,
            DeviceTransport::Thunderbolt,
        ] {
            assert!(transport.supports_bit_perfect());
        }
    }
}
