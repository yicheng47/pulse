use super::{
    FORMAT_POLL_INTERVAL, FORMAT_SETTLE_TIMEOUT,
    formats::{
        output_streams, physical_format, set_physical_format_until, set_virtual_format_until,
        virtual_format,
    },
    property::{
        address, check_status, get_value, has_property, non_null, property_is_settable, set_value,
    },
};
use crate::EngineError;
use objc2_core_audio::{
    AudioObjectID, AudioObjectSetPropertyData, kAudioDevicePropertyHogMode,
    kAudioDevicePropertySupportsMixing, kAudioObjectPropertyScopeGlobal,
};
use objc2_core_audio_types::AudioStreamBasicDescription;
use std::{ffi::c_void, mem, ptr, thread, time::Instant};
const HOG_MODE_FREE: i32 = -1;

/// Exclusive device ownership (`kAudioDevicePropertyHogMode`), released on
/// drop. `owns: false` means this process already held the hog before
/// `acquire` — the guard then must not release it on drop.
pub struct HogGuard {
    device_id: AudioObjectID,
    owns: bool,
}

pub struct FormatRestoreGuard {
    state: Option<SavedFormatState>,
}

impl FormatRestoreGuard {
    pub fn capture(device_id: AudioObjectID) -> Result<Self, EngineError> {
        Ok(Self {
            state: Some(capture_format_state(&CoreAudioFormatProperties, device_id)?),
        })
    }

    pub fn restore(mut self) -> Vec<EngineError> {
        restore_format_state(
            &CoreAudioFormatProperties,
            self.state.take().expect("format state must be armed"),
        )
    }
}

impl Drop for FormatRestoreGuard {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let _ = restore_format_state(&CoreAudioFormatProperties, state);
        }
    }
}

impl HogGuard {
    /// Takes the hog if the device is free, succeeds idempotently if this
    /// process already owns it, and reports the owning pid otherwise. The HAL
    /// arbitrates races, so the outcome is read back rather than assumed.
    pub fn acquire(device_id: AudioObjectID) -> Result<Self, EngineError> {
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
                    Err(EngineError::HogModeNotAcquired)
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

    pub fn owns(&self) -> bool {
        self.owns
    }
}

impl Drop for HogGuard {
    fn drop(&mut self) {
        if should_release_hog(self.owns, hog_owner(self.device_id).ok(), current_pid()) {
            let _ = toggle_hog_mode(self.device_id);
        }
    }
}

fn should_release_hog(owns: bool, owner: Option<i32>, process_id: i32) -> bool {
    owns && owner == Some(process_id)
}

struct SavedStreamFormats {
    stream_id: AudioObjectID,
    physical: AudioStreamBasicDescription,
    virtual_format: AudioStreamBasicDescription,
}

struct SavedFormatState {
    device_id: AudioObjectID,
    streams: Vec<SavedStreamFormats>,
    mixing: Option<bool>,
}

trait FormatPropertyAccess {
    fn output_streams(&self, device_id: AudioObjectID) -> Result<Vec<AudioObjectID>, EngineError>;
    fn physical_format(
        &self,
        stream_id: AudioObjectID,
    ) -> Result<AudioStreamBasicDescription, EngineError>;
    fn virtual_format(
        &self,
        stream_id: AudioObjectID,
    ) -> Result<AudioStreamBasicDescription, EngineError>;
    fn mixing_enabled(&self, device_id: AudioObjectID) -> Result<Option<bool>, EngineError>;
    fn set_physical_format(
        &self,
        stream_id: AudioObjectID,
        format: AudioStreamBasicDescription,
        deadline: Instant,
    ) -> Result<(), EngineError>;
    fn set_virtual_format(
        &self,
        stream_id: AudioObjectID,
        format: AudioStreamBasicDescription,
        deadline: Instant,
    ) -> Result<(), EngineError>;
    fn set_mixing_enabled(
        &self,
        device_id: AudioObjectID,
        enabled: bool,
        deadline: Instant,
    ) -> Result<(), EngineError>;
}

struct CoreAudioFormatProperties;

impl FormatPropertyAccess for CoreAudioFormatProperties {
    fn output_streams(&self, device_id: AudioObjectID) -> Result<Vec<AudioObjectID>, EngineError> {
        output_streams(device_id)
    }

    fn physical_format(
        &self,
        stream_id: AudioObjectID,
    ) -> Result<AudioStreamBasicDescription, EngineError> {
        physical_format(stream_id)
    }

    fn virtual_format(
        &self,
        stream_id: AudioObjectID,
    ) -> Result<AudioStreamBasicDescription, EngineError> {
        virtual_format(stream_id)
    }

    fn mixing_enabled(&self, device_id: AudioObjectID) -> Result<Option<bool>, EngineError> {
        mixing_enabled(device_id)
    }

    fn set_physical_format(
        &self,
        stream_id: AudioObjectID,
        format: AudioStreamBasicDescription,
        deadline: Instant,
    ) -> Result<(), EngineError> {
        set_physical_format_until(stream_id, format, deadline)
    }

    fn set_virtual_format(
        &self,
        stream_id: AudioObjectID,
        format: AudioStreamBasicDescription,
        deadline: Instant,
    ) -> Result<(), EngineError> {
        set_virtual_format_until(stream_id, format, deadline)
    }

    fn set_mixing_enabled(
        &self,
        device_id: AudioObjectID,
        enabled: bool,
        deadline: Instant,
    ) -> Result<(), EngineError> {
        set_mixing_enabled_until(device_id, enabled, deadline)
    }
}

fn capture_format_state(
    properties: &impl FormatPropertyAccess,
    device_id: AudioObjectID,
) -> Result<SavedFormatState, EngineError> {
    let stream_ids = properties.output_streams(device_id)?;
    let mut streams = Vec::with_capacity(stream_ids.len());
    for stream_id in stream_ids {
        streams.push(SavedStreamFormats {
            stream_id,
            physical: properties.physical_format(stream_id)?,
            virtual_format: properties.virtual_format(stream_id)?,
        });
    }

    Ok(SavedFormatState {
        device_id,
        streams,
        mixing: properties.mixing_enabled(device_id)?,
    })
}

fn restore_format_state(
    properties: &impl FormatPropertyAccess,
    state: SavedFormatState,
) -> Vec<EngineError> {
    let deadline = Instant::now() + FORMAT_SETTLE_TIMEOUT;
    let mut errors = Vec::new();
    for stream in state.streams {
        if let Err(error) =
            properties.set_physical_format(stream.stream_id, stream.physical, deadline)
        {
            errors.push(error);
        }
        if let Err(error) =
            properties.set_virtual_format(stream.stream_id, stream.virtual_format, deadline)
        {
            errors.push(error);
        }
    }
    if let Some(mixing) = state.mixing
        && let Err(error) = properties.set_mixing_enabled(state.device_id, mixing, deadline)
    {
        errors.push(error);
    }
    errors
}

pub fn hog_owner(device_id: AudioObjectID) -> Result<i32, EngineError> {
    get_value::<i32>(
        device_id,
        address(kAudioDevicePropertyHogMode, kAudioObjectPropertyScopeGlobal),
        "AudioObjectGetPropertyData(kAudioDevicePropertyHogMode)",
    )
}

/// Setting `HogMode` toggles: if the device is free the HAL assigns the hog
/// to this process; if this process owns it, the write releases it. The
/// written value is ignored — the read-back owner is the actual outcome.
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

pub fn mixing_enabled(device_id: AudioObjectID) -> Result<Option<bool>, EngineError> {
    let address = address(
        kAudioDevicePropertySupportsMixing,
        kAudioObjectPropertyScopeGlobal,
    );
    if !has_property(device_id, address) {
        return Ok(None);
    }

    get_value::<u32>(
        device_id,
        address,
        "AudioObjectGetPropertyData(kAudioDevicePropertySupportsMixing)",
    )
    .map(|value| Some(value != 0))
}

pub fn set_mixing_enabled(device_id: AudioObjectID, enabled: bool) -> Result<(), EngineError> {
    set_mixing_enabled_until(device_id, enabled, Instant::now() + FORMAT_SETTLE_TIMEOUT)
}

fn set_mixing_enabled_until(
    device_id: AudioObjectID,
    enabled: bool,
    deadline: Instant,
) -> Result<(), EngineError> {
    let address = address(
        kAudioDevicePropertySupportsMixing,
        kAudioObjectPropertyScopeGlobal,
    );
    if !property_is_settable(device_id, address) {
        return Ok(());
    }

    let requested = u32::from(enabled);
    let current = get_value::<u32>(
        device_id,
        address,
        "AudioObjectGetPropertyData(kAudioDevicePropertySupportsMixing)",
    )?;
    if current == requested {
        return Ok(());
    }

    set_value(
        device_id,
        address,
        requested,
        "AudioObjectSetPropertyData(kAudioDevicePropertySupportsMixing)",
    )?;

    loop {
        let current = get_value::<u32>(
            device_id,
            address,
            "AudioObjectGetPropertyData(kAudioDevicePropertySupportsMixing)",
        )?;
        if current == requested {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(EngineError::Timeout("device mixing state change"));
        }
        thread::sleep(FORMAT_POLL_INTERVAL);
    }
}

fn current_pid() -> i32 {
    i32::try_from(std::process::id()).expect("process id must fit in pid_t")
}

#[cfg(test)]
mod tests {
    use super::super::test_support::stream_format;
    use super::*;
    use objc2_core_audio_types::kAudioFormatFlagIsFloat;
    use objc2_core_audio_types::kAudioFormatFlagIsSignedInteger;

    #[test]
    fn hog_release_rechecks_ownership_before_toggling() {
        assert!(should_release_hog(true, Some(42), 42));
        assert!(!should_release_hog(false, Some(42), 42));
        assert!(!should_release_hog(true, Some(7), 42));
        assert!(!should_release_hog(true, None, 42));
    }
    #[test]
    fn format_restore_captures_and_restores_every_property_in_order() {
        let properties = FakeFormatProperties::new(Some(true));
        let state = capture_format_state(&properties, 7).expect("format state should be captured");

        let errors = restore_format_state(&properties, state);

        assert!(errors.is_empty());
        assert_eq!(
            properties.calls.borrow().as_slice(),
            [
                FormatCall::OutputStreams(7),
                FormatCall::PhysicalFormat(11),
                FormatCall::VirtualFormat(11),
                FormatCall::PhysicalFormat(22),
                FormatCall::VirtualFormat(22),
                FormatCall::MixingEnabled(7),
                FormatCall::SetPhysicalFormat(11, 44_111),
                FormatCall::SetVirtualFormat(11, 48_011),
                FormatCall::SetPhysicalFormat(22, 44_122),
                FormatCall::SetVirtualFormat(22, 48_022),
                FormatCall::SetMixingEnabled(7, true),
            ]
        );
        let deadlines = properties.deadlines.borrow();
        assert_eq!(deadlines.len(), 5);
        assert!(deadlines.iter().all(|deadline| *deadline == deadlines[0]));
    }
    #[test]
    fn format_restore_continues_after_a_failed_property_write() {
        let properties = FakeFormatProperties::new(Some(true)).with_failing_physical_stream(11);
        let state = capture_format_state(&properties, 7).expect("format state should be captured");
        properties.calls.borrow_mut().clear();

        let errors = restore_format_state(&properties, state);

        assert_eq!(errors.len(), 1);
        assert_eq!(
            properties.calls.borrow().as_slice(),
            [
                FormatCall::SetPhysicalFormat(11, 44_111),
                FormatCall::SetVirtualFormat(11, 48_011),
                FormatCall::SetPhysicalFormat(22, 44_122),
                FormatCall::SetVirtualFormat(22, 48_022),
                FormatCall::SetMixingEnabled(7, true),
            ]
        );
    }
    #[test]
    fn format_restore_skips_absent_mixing_property() {
        let properties = FakeFormatProperties::new(None);
        let state = capture_format_state(&properties, 7).expect("format state should be captured");
        properties.calls.borrow_mut().clear();

        let errors = restore_format_state(&properties, state);

        assert!(errors.is_empty());
        assert_eq!(
            properties.calls.borrow().as_slice(),
            [
                FormatCall::SetPhysicalFormat(11, 44_111),
                FormatCall::SetVirtualFormat(11, 48_011),
                FormatCall::SetPhysicalFormat(22, 44_122),
                FormatCall::SetVirtualFormat(22, 48_022),
            ]
        );
    }
    #[derive(Debug, PartialEq)]
    enum FormatCall {
        OutputStreams(AudioObjectID),
        PhysicalFormat(AudioObjectID),
        VirtualFormat(AudioObjectID),
        MixingEnabled(AudioObjectID),
        SetPhysicalFormat(AudioObjectID, u32),
        SetVirtualFormat(AudioObjectID, u32),
        SetMixingEnabled(AudioObjectID, bool),
    }
    struct FakeFormatProperties {
        calls: std::cell::RefCell<Vec<FormatCall>>,
        deadlines: std::cell::RefCell<Vec<Instant>>,
        mixing: Option<bool>,
        failing_physical_stream: Option<AudioObjectID>,
    }
    impl FakeFormatProperties {
        fn new(mixing: Option<bool>) -> Self {
            Self {
                calls: std::cell::RefCell::new(Vec::new()),
                deadlines: std::cell::RefCell::new(Vec::new()),
                mixing,
                failing_physical_stream: None,
            }
        }

        fn with_failing_physical_stream(mut self, stream_id: AudioObjectID) -> Self {
            self.failing_physical_stream = Some(stream_id);
            self
        }
    }
    impl FormatPropertyAccess for FakeFormatProperties {
        fn output_streams(
            &self,
            device_id: AudioObjectID,
        ) -> Result<Vec<AudioObjectID>, EngineError> {
            self.calls
                .borrow_mut()
                .push(FormatCall::OutputStreams(device_id));
            Ok(vec![11, 22])
        }

        fn physical_format(
            &self,
            stream_id: AudioObjectID,
        ) -> Result<AudioStreamBasicDescription, EngineError> {
            self.calls
                .borrow_mut()
                .push(FormatCall::PhysicalFormat(stream_id));
            Ok(stream_format(
                44_100.0 + f64::from(stream_id),
                kAudioFormatFlagIsSignedInteger,
            ))
        }

        fn virtual_format(
            &self,
            stream_id: AudioObjectID,
        ) -> Result<AudioStreamBasicDescription, EngineError> {
            self.calls
                .borrow_mut()
                .push(FormatCall::VirtualFormat(stream_id));
            Ok(stream_format(
                48_000.0 + f64::from(stream_id),
                kAudioFormatFlagIsFloat,
            ))
        }

        fn mixing_enabled(&self, device_id: AudioObjectID) -> Result<Option<bool>, EngineError> {
            self.calls
                .borrow_mut()
                .push(FormatCall::MixingEnabled(device_id));
            Ok(self.mixing)
        }

        fn set_physical_format(
            &self,
            stream_id: AudioObjectID,
            format: AudioStreamBasicDescription,
            deadline: Instant,
        ) -> Result<(), EngineError> {
            self.deadlines.borrow_mut().push(deadline);
            self.calls.borrow_mut().push(FormatCall::SetPhysicalFormat(
                stream_id,
                format.mSampleRate as u32,
            ));
            if self.failing_physical_stream == Some(stream_id) {
                return Err(EngineError::Os {
                    call: "fake physical format restore",
                    status: -1,
                });
            }
            Ok(())
        }

        fn set_virtual_format(
            &self,
            stream_id: AudioObjectID,
            format: AudioStreamBasicDescription,
            deadline: Instant,
        ) -> Result<(), EngineError> {
            self.deadlines.borrow_mut().push(deadline);
            self.calls.borrow_mut().push(FormatCall::SetVirtualFormat(
                stream_id,
                format.mSampleRate as u32,
            ));
            Ok(())
        }

        fn set_mixing_enabled(
            &self,
            device_id: AudioObjectID,
            enabled: bool,
            deadline: Instant,
        ) -> Result<(), EngineError> {
            self.deadlines.borrow_mut().push(deadline);
            self.calls
                .borrow_mut()
                .push(FormatCall::SetMixingEnabled(device_id, enabled));
            Ok(())
        }
    }
}
