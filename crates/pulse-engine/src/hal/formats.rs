//! Rate and format changes are asynchronous. A successful property write alone
//! does not establish the new state; these helpers poll readback until it matches
//! or the settle deadline expires.

use super::{
    FORMAT_POLL_INTERVAL, FORMAT_SETTLE_TIMEOUT,
    property::{address, get_array, get_value, set_value},
};
use crate::{EngineError, PcmFormat};
use objc2_core_audio::{
    AudioObjectID, AudioObjectPropertySelector, AudioStreamRangedDescription,
    kAudioDevicePropertyAvailableNominalSampleRates, kAudioDevicePropertyNominalSampleRate,
    kAudioDevicePropertyStreams, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeOutput,
    kAudioStreamPropertyAvailablePhysicalFormats, kAudioStreamPropertyAvailableVirtualFormats,
    kAudioStreamPropertyPhysicalFormat, kAudioStreamPropertyVirtualFormat,
};
use objc2_core_audio_types::{
    AudioStreamBasicDescription, AudioValueRange, kAudioFormatFlagIsFloat,
    kAudioFormatFlagIsSignedInteger, kAudioFormatLinearPCM, kAudioStreamAnyRate,
};
use std::{thread, time::Instant};

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

pub(super) fn set_virtual_format_until(
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

pub fn set_physical_format(
    stream_id: AudioObjectID,
    format: AudioStreamBasicDescription,
) -> Result<(), EngineError> {
    set_physical_format_until(stream_id, format, Instant::now() + FORMAT_SETTLE_TIMEOUT)
}

pub(super) fn set_physical_format_until(
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

#[cfg(test)]
mod tests {
    use super::super::test_support::{ranged_format, stream_format};
    use super::*;
    use objc2_core_audio_types::kAudioFormatFlagIsNonMixable;

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
}
