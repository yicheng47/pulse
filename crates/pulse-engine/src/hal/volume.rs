use super::property::{address, get_value, property_is_settable, set_value};
use crate::EngineError;
use objc2_core_audio::{
    AudioObjectID, kAudioDevicePropertyMute, kAudioDevicePropertyVolumeScalar,
    kAudioObjectPropertyScopeOutput,
};

#[derive(Debug)]
/// A device's own output volume/mute controls, probed once and then written
/// through. Only exists for devices whose driver exposes a settable volume
/// scalar (e.g. DACs with hardware volume); everything else stays on the
/// engine's software gain.
pub(crate) struct HardwareVolume {
    device_id: AudioObjectID,
    pub level: f32,
    pub muted: bool,
    mute_settable: bool,
}

impl HardwareVolume {
    /// A device without a settable mute control emulates mute by writing a
    /// zero volume scalar instead.
    pub(crate) fn set_volume(&mut self, level: f32, muted: bool) -> Result<(), EngineError> {
        let scalar = hardware_volume_scalar(level, muted, self.mute_settable);
        set_value(
            self.device_id,
            address(
                kAudioDevicePropertyVolumeScalar,
                kAudioObjectPropertyScopeOutput,
            ),
            scalar,
            "AudioObjectSetPropertyData(kAudioDevicePropertyVolumeScalar)",
        )?;
        if self.mute_settable && muted != self.muted {
            set_value(
                self.device_id,
                address(kAudioDevicePropertyMute, kAudioObjectPropertyScopeOutput),
                u32::from(muted),
                "AudioObjectSetPropertyData(kAudioDevicePropertyMute)",
            )?;
        }
        self.level = level;
        self.muted = muted;
        Ok(())
    }
}

/// Probes for usable hardware volume; `None` sends the caller down the
/// software (float gain) volume path instead.
pub(crate) fn hardware_volume_control(device_id: AudioObjectID) -> Option<HardwareVolume> {
    let volume_address = address(
        kAudioDevicePropertyVolumeScalar,
        kAudioObjectPropertyScopeOutput,
    );
    if !property_is_settable(device_id, volume_address) {
        return None;
    }
    let level = get_value::<f32>(
        device_id,
        volume_address,
        "AudioObjectGetPropertyData(kAudioDevicePropertyVolumeScalar)",
    )
    .ok()?;
    if !hardware_volume_level_is_valid(level) {
        return None;
    }

    let mute_address = address(kAudioDevicePropertyMute, kAudioObjectPropertyScopeOutput);
    let mute_settable = property_is_settable(device_id, mute_address);
    let muted = if mute_settable {
        get_value::<u32>(
            device_id,
            mute_address,
            "AudioObjectGetPropertyData(kAudioDevicePropertyMute)",
        )
        .map(|muted| muted != 0)
        .unwrap_or(level == 0.0)
    } else {
        level == 0.0
    };

    Some(HardwareVolume {
        device_id,
        level,
        muted,
        mute_settable,
    })
}

fn hardware_volume_scalar(level: f32, muted: bool, mute_settable: bool) -> f32 {
    if muted && !mute_settable { 0.0 } else { level }
}

fn hardware_volume_level_is_valid(level: f32) -> bool {
    level.is_finite() && (0.0..=1.0).contains(&level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_volume_without_mute_control_uses_zero_scalar_for_mute() {
        assert_eq!(hardware_volume_scalar(0.6, true, false), 0.0);
        assert_eq!(hardware_volume_scalar(0.6, false, false), 0.6);
        assert_eq!(hardware_volume_scalar(0.6, true, true), 0.6);
    }
    #[test]
    fn hardware_volume_rejects_invalid_driver_scalars() {
        assert!(hardware_volume_level_is_valid(0.0));
        assert!(hardware_volume_level_is_valid(1.0));
        assert!(!hardware_volume_level_is_valid(f32::NAN));
        assert!(!hardware_volume_level_is_valid(-0.1));
        assert!(!hardware_volume_level_is_valid(1.1));
    }
}
