//! Safe wrapper over the AudioObject property API: hog mode, physical format,
//! nominal sample rate, property listeners.
//!
//! Core Audio is a property system: every entity (system, device, stream) is
//! an AudioObject, and nearly everything happens as a get/set on a
//! (selector, scope, element) address. Values are untyped bytes;
//! variable-sized properties take a get-size-then-get-data pair. All `unsafe`
//! FFI in the engine lives in this file — callers get typed,
//! `Result`-returning helpers with the failing C call named in the error.
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
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectHasProperty,
    AudioObjectID, AudioObjectIsPropertySettable, AudioObjectPropertyAddress,
    AudioObjectPropertyElement, AudioObjectPropertyScope, AudioObjectPropertySelector,
    AudioObjectSetPropertyData, AudioStreamRangedDescription,
    kAudioDevicePropertyAvailableNominalSampleRates, kAudioDevicePropertyHogMode,
    kAudioDevicePropertyMute, kAudioDevicePropertyNominalSampleRate, kAudioDevicePropertyStreams,
    kAudioDevicePropertySupportsMixing, kAudioDevicePropertyTransportType,
    kAudioDevicePropertyVolumeScalar, kAudioDeviceTransportTypeUnknown, kAudioHardwareNoError,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeOutput, kAudioStreamPropertyAvailablePhysicalFormats,
    kAudioStreamPropertyAvailableVirtualFormats, kAudioStreamPropertyPhysicalFormat,
    kAudioStreamPropertyVirtualFormat,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, AudioValueRange,
    kAudioFormatFlagIsBigEndian, kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved,
    kAudioFormatFlagIsNonMixable, kAudioFormatFlagIsSignedInteger, kAudioFormatLinearPCM,
    kAudioStreamAnyRate,
};
use objc2_core_foundation::{CFRetained, CFString};

use crate::{EngineError, PcmFormat};

const HOG_MODE_FREE: i32 = -1;
const FORMAT_SETTLE_TIMEOUT: Duration = Duration::from_secs(2);
const FORMAT_POLL_INTERVAL: Duration = Duration::from_millis(5);

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

/// Switches the device clock to the track's rate — the "native rate"
/// behavior. Skips the write when already there, because rate changes
/// reconfigure hardware and can audibly click. The change is asynchronous, so
/// the new rate is polled until it settles.
pub(crate) fn set_nominal_sample_rate(
    device_id: AudioObjectID,
    format: PcmFormat,
) -> Result<f64, EngineError> {
    let requested = f64::from(format.sample_rate);
    if !sample_rate_supported(device_id, requested)? {
        return Err(EngineError::UnsupportedNominalSampleRate(format));
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

/// Points the device's wire format at the best match for the source: the
/// first signed-integer linear-PCM physical format with enough channels and
/// bit depth on any output stream. The physical format is what actually
/// crosses to the DAC, which is why float candidates are rejected.
pub(crate) fn set_matching_physical_format(
    device_id: AudioObjectID,
    format: PcmFormat,
) -> Result<(), EngineError> {
    for stream_id in output_streams(device_id)? {
        for ranged_format in available_physical_formats(stream_id)? {
            let Some(candidate) = matching_physical_format(ranged_format, format) else {
                continue;
            };

            set_physical_format(stream_id, candidate)?;
            return Ok(());
        }
    }

    Err(EngineError::NoMatchingPhysicalFormat(format))
}

/// Maximum integer bit depth and sample rate across all output streams, plus
/// whether any format is safe for the integer engine and the transport type.
/// A missing bit depth means the device only offers mixable float, where bit
/// depth is meaningless.
pub(crate) struct ProbedOutputDeviceCapabilities {
    pub(crate) max_bits_per_channel: Option<u32>,
    pub(crate) max_sample_rate: f64,
    pub(crate) integer_wire_formats: bool,
    pub(crate) transport_type: u32,
}

pub(crate) fn output_device_capabilities(
    device_id: AudioObjectID,
) -> Result<Option<ProbedOutputDeviceCapabilities>, EngineError> {
    let mut formats = Vec::new();
    for stream_id in output_streams(device_id)? {
        formats.extend(available_physical_formats(stream_id)?);
    }
    let Some((max_bits_per_channel, max_sample_rate, integer_wire_formats)) =
        maximum_physical_format_capabilities(&formats)
    else {
        return Ok(None);
    };
    let transport_address = address(
        kAudioDevicePropertyTransportType,
        kAudioObjectPropertyScopeGlobal,
    );
    let transport_type = if has_property(device_id, transport_address) {
        get_value::<u32>(
            device_id,
            transport_address,
            "AudioObjectGetPropertyData(kAudioDevicePropertyTransportType)",
        )?
    } else {
        kAudioDeviceTransportTypeUnknown
    };
    Ok(Some(ProbedOutputDeviceCapabilities {
        max_bits_per_channel,
        max_sample_rate,
        integer_wire_formats,
        transport_type,
    }))
}

/// Sums output channels from raw `AudioBufferList` bytes. The C struct ends
/// in a flexible array member, so it cannot be read as one typed value —
/// fields are decoded manually with unaligned reads, tolerating truncation.
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

pub fn hog_owner(device_id: AudioObjectID) -> Result<i32, EngineError> {
    get_value::<i32>(
        device_id,
        address(kAudioDevicePropertyHogMode, kAudioObjectPropertyScopeGlobal),
        "AudioObjectGetPropertyData(kAudioDevicePropertyHogMode)",
    )
}

fn property_is_settable(object_id: AudioObjectID, mut address: AudioObjectPropertyAddress) -> bool {
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

pub fn output_streams(device_id: AudioObjectID) -> Result<Vec<AudioObjectID>, EngineError> {
    get_array::<AudioObjectID>(
        device_id,
        address(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeOutput),
        "AudioObjectGetPropertyData(kAudioDevicePropertyStreams)",
    )
}

pub fn available_physical_formats(
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

pub fn physical_format(
    stream_id: AudioObjectID,
) -> Result<AudioStreamBasicDescription, EngineError> {
    get_value::<AudioStreamBasicDescription>(
        stream_id,
        address(
            kAudioStreamPropertyPhysicalFormat,
            kAudioObjectPropertyScopeGlobal,
        ),
        "AudioObjectGetPropertyData(kAudioStreamPropertyPhysicalFormat)",
    )
}

pub fn available_virtual_formats(
    stream_id: AudioObjectID,
) -> Result<Vec<AudioStreamRangedDescription>, EngineError> {
    get_array::<AudioStreamRangedDescription>(
        stream_id,
        address(
            kAudioStreamPropertyAvailableVirtualFormats,
            kAudioObjectPropertyScopeGlobal,
        ),
        "AudioObjectGetPropertyData(kAudioStreamPropertyAvailableVirtualFormats)",
    )
}

pub fn virtual_format(
    stream_id: AudioObjectID,
) -> Result<AudioStreamBasicDescription, EngineError> {
    get_value::<AudioStreamBasicDescription>(
        stream_id,
        address(
            kAudioStreamPropertyVirtualFormat,
            kAudioObjectPropertyScopeGlobal,
        ),
        "AudioObjectGetPropertyData(kAudioStreamPropertyVirtualFormat)",
    )
}

pub fn set_virtual_format(
    stream_id: AudioObjectID,
    format: AudioStreamBasicDescription,
) -> Result<(), EngineError> {
    set_virtual_format_until(stream_id, format, Instant::now() + FORMAT_SETTLE_TIMEOUT)
}

fn set_virtual_format_until(
    stream_id: AudioObjectID,
    format: AudioStreamBasicDescription,
    deadline: Instant,
) -> Result<(), EngineError> {
    set_stream_format(
        stream_id,
        kAudioStreamPropertyVirtualFormat,
        format,
        "AudioObjectGetPropertyData(kAudioStreamPropertyVirtualFormat)",
        "AudioObjectSetPropertyData(kAudioStreamPropertyVirtualFormat)",
        "virtual stream format change",
        deadline,
    )
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

/// A candidate must be signed-integer linear PCM with at least the requested
/// channels and bits — a wider container (24-in-32) is fine, the extra bits
/// are padding. A `kAudioStreamAnyRate` wildcard resolves to the requested
/// rate.
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

pub(crate) fn is_integer_wire_format(format: &AudioStreamBasicDescription) -> bool {
    format.mFormatID == kAudioFormatLinearPCM
        && format.mFormatFlags & kAudioFormatFlagIsSignedInteger != 0
        && format.mFormatFlags & kAudioFormatFlagIsFloat == 0
        && format.mFormatFlags & kAudioFormatFlagIsNonMixable != 0
        && format.mFormatFlags & kAudioFormatFlagIsBigEndian == 0
        && format.mFormatFlags & kAudioFormatFlagIsNonInterleaved == 0
        && format.mBitsPerChannel > 0
        && format.mBitsPerChannel.is_multiple_of(8)
        && format.mChannelsPerFrame > 0
        && format.mBytesPerFrame > 0
        && format
            .mBytesPerFrame
            .is_multiple_of(format.mChannelsPerFrame)
        && format.mBytesPerFrame / format.mChannelsPerFrame <= 4
}

fn maximum_physical_format_capabilities(
    formats: &[AudioStreamRangedDescription],
) -> Option<(Option<u32>, f64, bool)> {
    let mut maximum: Option<(u32, f64)> = None;
    let mut maximum_mixable_float_rate: Option<f64> = None;
    let mut integer_wire_formats = false;

    for ranged_format in formats {
        let format = ranged_format.mFormat;
        if format.mFormatID != kAudioFormatLinearPCM {
            continue;
        }

        let sample_rate = format
            .mSampleRate
            .max(ranged_format.mSampleRateRange.mMaximum);
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            continue;
        }

        integer_wire_formats |= is_integer_wire_format(&format);

        if format.mFormatFlags & kAudioFormatFlagIsFloat != 0
            && format.mFormatFlags & kAudioFormatFlagIsNonMixable == 0
        {
            maximum_mixable_float_rate = Some(
                maximum_mixable_float_rate.map_or(sample_rate, |maximum| maximum.max(sample_rate)),
            );
            continue;
        }

        if format.mFormatFlags & kAudioFormatFlagIsSignedInteger == 0 || format.mBitsPerChannel == 0
        {
            continue;
        }

        maximum = Some(match maximum {
            Some((max_bits, max_rate)) => (
                max_bits.max(format.mBitsPerChannel),
                max_rate.max(sample_rate),
            ),
            None => (format.mBitsPerChannel, sample_rate),
        });
    }

    maximum
        .map(|(bits, rate)| (Some(bits), rate, integer_wire_formats))
        .or_else(|| maximum_mixable_float_rate.map(|rate| (None, rate, integer_wire_formats)))
}

pub fn set_physical_format(
    stream_id: AudioObjectID,
    format: AudioStreamBasicDescription,
) -> Result<(), EngineError> {
    set_physical_format_until(stream_id, format, Instant::now() + FORMAT_SETTLE_TIMEOUT)
}

fn set_physical_format_until(
    stream_id: AudioObjectID,
    format: AudioStreamBasicDescription,
    deadline: Instant,
) -> Result<(), EngineError> {
    set_stream_format(
        stream_id,
        kAudioStreamPropertyPhysicalFormat,
        format,
        "AudioObjectGetPropertyData(kAudioStreamPropertyPhysicalFormat)",
        "AudioObjectSetPropertyData(kAudioStreamPropertyPhysicalFormat)",
        "physical stream format change",
        deadline,
    )
}

fn set_stream_format(
    stream_id: AudioObjectID,
    selector: AudioObjectPropertySelector,
    requested: AudioStreamBasicDescription,
    get_call: &'static str,
    set_call: &'static str,
    timeout_name: &'static str,
    deadline: Instant,
) -> Result<(), EngineError> {
    let address = address(selector, kAudioObjectPropertyScopeGlobal);
    let current = get_value::<AudioStreamBasicDescription>(stream_id, address, get_call)?;
    if stream_formats_match(current, requested) {
        return Ok(());
    }

    set_value(stream_id, address, requested, set_call)?;

    loop {
        let current = get_value::<AudioStreamBasicDescription>(stream_id, address, get_call)?;
        if stream_formats_match(current, requested) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(EngineError::Timeout(timeout_name));
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

/// Rates arrive as `Float64` from the hardware, so equality is a ±0.5 Hz
/// test.
fn sample_rates_match(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.5
}

fn stream_formats_match(
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

fn has_property(object_id: AudioObjectID, mut address: AudioObjectPropertyAddress) -> bool {
    unsafe { AudioObjectHasProperty(object_id, (&mut address).into()) }
}

fn current_pid() -> i32 {
    i32::try_from(std::process::id()).expect("process id must fit in pid_t")
}

pub(crate) fn check_status(call: &'static str, status: i32) -> Result<(), EngineError> {
    if status == kAudioHardwareNoError {
        Ok(())
    } else {
        Err(EngineError::Os { call, status })
    }
}

fn non_null<T>(ptr: *mut T) -> NonNull<T> {
    NonNull::new(ptr).expect("Core Audio output buffer pointer must be non-null")
}

/// Property bytes carry no alignment guarantee, hence `ptr::read_unaligned`.
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

    #[test]
    fn hog_release_rechecks_ownership_before_toggling() {
        assert!(should_release_hog(true, Some(42), 42));
        assert!(!should_release_hog(false, Some(42), 42));
        assert!(!should_release_hog(true, Some(7), 42));
        assert!(!should_release_hog(true, None, 42));
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

    #[test]
    fn maximum_capabilities_pick_independent_pcm_bit_and_rate_maxima() {
        let formats = [
            ranged_format(
                44_100.0,
                44_100.0,
                96_000.0,
                32,
                kAudioFormatFlagIsSignedInteger,
            ),
            ranged_format(
                0.0,
                44_100.0,
                192_000.0,
                24,
                kAudioFormatFlagIsSignedInteger,
            ),
            ranged_format(384_000.0, 384_000.0, 384_000.0, 64, kAudioFormatFlagIsFloat),
        ];

        assert_eq!(
            maximum_physical_format_capabilities(&formats),
            Some((Some(32), 192_000.0, false))
        );
    }

    #[test]
    fn integer_wire_format_matches_stage_one_probe_flags_and_layout() {
        for flags in [0x54, 0x4c] {
            assert!(is_integer_wire_format(&stream_format(48_000.0, flags)));
        }
        for flags in [0x14, 0x0c, 0x04] {
            assert!(!is_integer_wire_format(&stream_format(48_000.0, flags)));
        }

        let mut format = stream_format(48_000.0, 0x54);
        format.mFormatFlags |= kAudioFormatFlagIsBigEndian;
        assert!(!is_integer_wire_format(&format));

        format = stream_format(48_000.0, 0x54);
        format.mFormatFlags |= kAudioFormatFlagIsNonInterleaved;
        assert!(!is_integer_wire_format(&format));

        format = stream_format(48_000.0, 0x54);
        format.mBytesPerFrame = 10;
        assert!(!is_integer_wire_format(&format));
    }

    #[test]
    fn maximum_capabilities_keep_mixable_integer_depth_without_an_integer_wire_format() {
        let formats = [ranged_format(
            0.0,
            44_100.0,
            192_000.0,
            24,
            kAudioFormatFlagIsSignedInteger,
        )];

        assert_eq!(
            maximum_physical_format_capabilities(&formats),
            Some((Some(24), 192_000.0, false))
        );
    }

    #[test]
    fn maximum_capabilities_fall_back_to_mixable_float_sample_rate() {
        let formats = [
            ranged_format(0.0, 44_100.0, 48_000.0, 32, kAudioFormatFlagIsFloat),
            ranged_format(
                0.0,
                44_100.0,
                192_000.0,
                32,
                kAudioFormatFlagIsFloat | kAudioFormatFlagIsNonMixable,
            ),
        ];

        assert_eq!(
            maximum_physical_format_capabilities(&formats),
            Some((None, 48_000.0, false))
        );
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

    #[test]
    fn stream_format_matching_checks_flags_and_layout_with_rate_tolerance() {
        let expected = stream_format(44_100.0, kAudioFormatFlagIsSignedInteger);

        let mut equivalent = expected;
        equivalent.mSampleRate += 0.49;
        equivalent.mReserved = 1;
        assert!(stream_formats_match(expected, equivalent));

        let mut different_flags = expected;
        different_flags.mFormatFlags |= kAudioFormatFlagIsNonMixable;
        assert!(!stream_formats_match(expected, different_flags));

        let mut different_layout = expected;
        different_layout.mBytesPerFrame += 1;
        assert!(!stream_formats_match(expected, different_layout));

        let mut different_rate = expected;
        different_rate.mSampleRate += 0.5;
        assert!(!stream_formats_match(expected, different_rate));
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

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + mem::size_of::<u32>()].copy_from_slice(&value.to_ne_bytes());
    }

    fn stream_format(sample_rate: f64, format_flags: u32) -> AudioStreamBasicDescription {
        AudioStreamBasicDescription {
            mSampleRate: sample_rate,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: format_flags,
            mBytesPerPacket: 8,
            mFramesPerPacket: 1,
            mBytesPerFrame: 8,
            mChannelsPerFrame: 2,
            mBitsPerChannel: 32,
            mReserved: 0,
        }
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
                mBytesPerPacket: bits_per_channel / 8 * 2,
                mFramesPerPacket: 1,
                mBytesPerFrame: bits_per_channel / 8 * 2,
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
