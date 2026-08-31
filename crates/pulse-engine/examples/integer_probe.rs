use objc2_core_audio::AudioStreamRangedDescription;
use objc2_core_audio_types::{
    AudioStreamBasicDescription, kAudioFormatFlagIsAlignedHigh, kAudioFormatFlagIsBigEndian,
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved, kAudioFormatFlagIsNonMixable,
    kAudioFormatFlagIsPacked, kAudioFormatFlagIsSignedInteger, kAudioFormatLinearPCM,
};
use pulse_engine::{
    EngineError,
    device::{self, Device},
    hal::{self, HogGuard},
};

const PROBE_SAMPLE_RATES: [f64; 4] = [44_100.0, 48_000.0, 96_000.0, 192_000.0];

fn main() {
    if let Err(error) = run() {
        eprintln!("integer probe failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let devices = device::list_output_devices().map_err(|error| error.to_string())?;
    if devices.is_empty() {
        return Err("no Core Audio output devices found".to_string());
    }
    let reports = devices.iter().map(probe_device).collect::<Vec<_>>();

    print_inventory(&reports);
    print_findings(&reports);

    let errors = reports
        .iter()
        .flat_map(|report| {
            report
                .errors
                .iter()
                .map(|error| format!("{}: {error}", report.inventory.device))
        })
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

struct StreamProbe {
    id: u32,
    physical_format_count: usize,
    virtual_format_count: usize,
    candidates: Vec<AudioStreamBasicDescription>,
}

struct SavedStreamState {
    id: u32,
    physical: AudioStreamBasicDescription,
    virtual_format: AudioStreamBasicDescription,
}

struct SavedDeviceState {
    device_id: u32,
    streams: Vec<SavedStreamState>,
    mixing: Option<bool>,
}

impl SavedDeviceState {
    fn capture(device_id: u32, streams: &[StreamProbe]) -> Result<Self, EngineError> {
        let streams = streams
            .iter()
            .map(|stream| {
                Ok(SavedStreamState {
                    id: stream.id,
                    physical: hal::physical_format(stream.id)?,
                    virtual_format: hal::virtual_format(stream.id)?,
                })
            })
            .collect::<Result<Vec<_>, EngineError>>()?;

        Ok(Self {
            device_id,
            streams,
            mixing: hal::mixing_enabled(device_id)?,
        })
    }

    fn restore(&self) -> Vec<String> {
        let mut errors = Vec::new();

        for stream in &self.streams {
            if let Err(error) = hal::set_physical_format(stream.id, stream.physical) {
                errors.push(format!(
                    "stream {} physical format restore failed: {error}",
                    stream.id
                ));
            }
            if let Err(error) = hal::set_virtual_format(stream.id, stream.virtual_format) {
                errors.push(format!(
                    "stream {} virtual format restore failed: {error}",
                    stream.id
                ));
            }
        }

        if let Some(mixing) = self.mixing
            && let Err(error) = hal::set_mixing_enabled(self.device_id, mixing)
        {
            errors.push(format!("mixing state restore failed: {error}"));
        }

        for stream in &self.streams {
            match hal::physical_format(stream.id) {
                Ok(actual) if formats_match(actual, stream.physical) => {}
                Ok(actual) => errors.push(format!(
                    "stream {} physical format was not restored (actual {})",
                    stream.id,
                    describe_asbd(actual)
                )),
                Err(error) => errors.push(format!(
                    "stream {} physical format restore verification failed: {error}",
                    stream.id
                )),
            }
            match hal::virtual_format(stream.id) {
                Ok(actual) if formats_match(actual, stream.virtual_format) => {}
                Ok(actual) => errors.push(format!(
                    "stream {} virtual format was not restored (actual {})",
                    stream.id,
                    describe_asbd(actual)
                )),
                Err(error) => errors.push(format!(
                    "stream {} virtual format restore verification failed: {error}",
                    stream.id
                )),
            }
        }

        if let Some(expected) = self.mixing {
            match hal::mixing_enabled(self.device_id) {
                Ok(Some(actual)) if actual == expected => {}
                Ok(Some(actual)) => errors.push(format!(
                    "mixing state was not restored (expected {}, actual {})",
                    on_off(expected),
                    on_off(actual)
                )),
                Ok(None) => errors.push("mixing property disappeared during probe".to_string()),
                Err(error) => {
                    errors.push(format!("mixing state restore verification failed: {error}"))
                }
            }
        }

        errors
    }
}

struct InventoryRow {
    device: String,
    uid: String,
    capabilities: String,
    stream_count: usize,
    physical_format_count: usize,
    virtual_format_count: usize,
    mixing: String,
    restored: String,
}

struct FindingRow {
    device: String,
    uid: String,
    stream: String,
    sample_rate: String,
    bits_and_bytes: String,
    channels: String,
    flags: String,
    physical: String,
    virtual_format: String,
    physical_after_virtual: String,
    result: String,
}

struct DeviceReport {
    inventory: InventoryRow,
    findings: Vec<FindingRow>,
    errors: Vec<String>,
}

fn probe_device(device: &Device) -> DeviceReport {
    let capabilities = device::output_device_capabilities(device.id)
        .map(describe_capabilities)
        .unwrap_or_else(|error| format!("error: {error}"));
    let mut report = DeviceReport {
        inventory: InventoryRow {
            device: device.name.clone(),
            uid: device.uid.clone(),
            capabilities,
            stream_count: 0,
            physical_format_count: 0,
            virtual_format_count: 0,
            mixing: "not read".to_string(),
            restored: "not run".to_string(),
        },
        findings: Vec::new(),
        errors: Vec::new(),
    };

    let stream_ids = match hal::output_streams(device.id) {
        Ok(streams) => streams,
        Err(error) => {
            report
                .errors
                .push(format!("output stream query failed: {error}"));
            return report;
        }
    };

    let mut streams = Vec::with_capacity(stream_ids.len());
    for stream_id in stream_ids {
        let physical_formats = match hal::available_physical_formats(stream_id) {
            Ok(formats) => formats,
            Err(error) => {
                report.errors.push(format!(
                    "stream {stream_id} physical format query failed: {error}"
                ));
                continue;
            }
        };
        let virtual_formats = match hal::available_virtual_formats(stream_id) {
            Ok(formats) => formats,
            Err(error) => {
                report.errors.push(format!(
                    "stream {stream_id} virtual format query failed: {error}"
                ));
                continue;
            }
        };
        streams.push(StreamProbe {
            id: stream_id,
            physical_format_count: physical_formats.len(),
            virtual_format_count: virtual_formats.len(),
            candidates: integer_candidates(&physical_formats),
        });
    }

    report.inventory.stream_count = streams.len();
    report.inventory.physical_format_count = streams
        .iter()
        .map(|stream| stream.physical_format_count)
        .sum();
    report.inventory.virtual_format_count = streams
        .iter()
        .map(|stream| stream.virtual_format_count)
        .sum();

    if streams.is_empty() {
        report
            .errors
            .push("device has no probeable output streams".to_string());
        return report;
    }

    let hog = match HogGuard::acquire(device.id) {
        Ok(hog) => hog,
        Err(error) => {
            report
                .errors
                .push(format!("hog acquisition failed: {error}"));
            return report;
        }
    };

    let saved = match SavedDeviceState::capture(device.id, &streams) {
        Ok(saved) => saved,
        Err(error) => {
            report.errors.push(format!("state capture failed: {error}"));
            drop(hog);
            verify_hog_released(device.id, &mut report);
            return report;
        }
    };

    report.inventory.mixing = disable_mixing_for_probe(device.id, saved.mixing);

    for stream in &streams {
        if stream.candidates.is_empty() {
            report.findings.push(FindingRow {
                device: device.name.clone(),
                uid: device.uid.clone(),
                stream: stream.id.to_string(),
                sample_rate: "—".to_string(),
                bits_and_bytes: "—".to_string(),
                channels: "—".to_string(),
                flags: "—".to_string(),
                physical: "—".to_string(),
                virtual_format: "—".to_string(),
                physical_after_virtual: "—".to_string(),
                result: "no signed-integer physical format at probe rates".to_string(),
            });
            continue;
        }

        report.findings.extend(
            stream
                .candidates
                .iter()
                .copied()
                .map(|candidate| probe_candidate(device, stream.id, candidate)),
        );
    }

    let restore_errors = saved.restore();
    if restore_errors.is_empty() {
        report.inventory.restored = "verified".to_string();
    } else {
        report.inventory.restored = "FAILED".to_string();
        report.errors.extend(restore_errors);
    }

    drop(hog);
    verify_hog_released(device.id, &mut report);
    report
}

fn disable_mixing_for_probe(device_id: u32, saved: Option<bool>) -> String {
    match saved {
        None => "property unavailable".to_string(),
        Some(false) => "off (unchanged)".to_string(),
        Some(true) => match hal::set_mixing_enabled(device_id, false) {
            Ok(()) => match hal::mixing_enabled(device_id) {
                Ok(Some(false)) => "on → off".to_string(),
                Ok(Some(true)) => "on; disable refused".to_string(),
                Ok(None) => "property disappeared".to_string(),
                Err(error) => format!("disable verification error: {error}"),
            },
            Err(error) => format!("on; disable refused: {error}"),
        },
    }
}

fn verify_hog_released(device_id: u32, report: &mut DeviceReport) {
    match hal::hog_owner(device_id) {
        Ok(-1) => {}
        Ok(owner) => report
            .errors
            .push(format!("hog remained owned by pid {owner}")),
        Err(error) => report
            .errors
            .push(format!("hog release verification failed: {error}")),
    }
}

fn probe_candidate(
    device: &Device,
    stream_id: u32,
    candidate: AudioStreamBasicDescription,
) -> FindingRow {
    let physical_set_error = hal::set_physical_format(stream_id, candidate)
        .err()
        .map(|error| error.to_string());
    let physical_readback = hal::physical_format(stream_id);
    let physical_accepted = physical_readback
        .as_ref()
        .is_ok_and(|actual| formats_match(*actual, candidate));

    let (virtual_attempted, virtual_set_error) = if physical_accepted {
        (
            true,
            hal::set_virtual_format(stream_id, candidate)
                .err()
                .map(|error| error.to_string()),
        )
    } else {
        (false, None)
    };

    let physical_after_virtual = hal::physical_format(stream_id);
    let virtual_readback = hal::virtual_format(stream_id);
    let physical_still_matches = physical_after_virtual
        .as_ref()
        .is_ok_and(|actual| formats_match(*actual, candidate));
    let virtual_accepted = virtual_readback
        .as_ref()
        .is_ok_and(|actual| formats_match(*actual, candidate));

    FindingRow {
        device: device.name.clone(),
        uid: device.uid.clone(),
        stream: stream_id.to_string(),
        sample_rate: format!("{:.1} kHz", candidate.mSampleRate / 1_000.0),
        bits_and_bytes: format!(
            "{} bit, {} B/frame",
            candidate.mBitsPerChannel, candidate.mBytesPerFrame
        ),
        channels: candidate.mChannelsPerFrame.to_string(),
        flags: describe_flags(candidate.mFormatFlags),
        physical: describe_attempt(
            candidate,
            true,
            physical_set_error.as_deref(),
            &physical_readback,
        ),
        virtual_format: describe_attempt(
            candidate,
            virtual_attempted,
            virtual_set_error.as_deref(),
            &virtual_readback,
        ),
        physical_after_virtual: describe_readback(candidate, &physical_after_virtual),
        result: if physical_accepted && virtual_accepted && physical_still_matches {
            "accepted".to_string()
        } else {
            "refused".to_string()
        },
    }
}

fn integer_candidates(
    formats: &[AudioStreamRangedDescription],
) -> Vec<AudioStreamBasicDescription> {
    let mut candidates = Vec::new();
    for ranged in formats {
        if ranged.mFormat.mFormatID != kAudioFormatLinearPCM
            || ranged.mFormat.mFormatFlags & kAudioFormatFlagIsFloat != 0
            || ranged.mFormat.mFormatFlags & kAudioFormatFlagIsSignedInteger == 0
        {
            continue;
        }

        for sample_rate in PROBE_SAMPLE_RATES {
            if !ranged_format_supports_rate(*ranged, sample_rate) {
                continue;
            }
            let mut candidate = ranged.mFormat;
            candidate.mSampleRate = sample_rate;
            if !candidates
                .iter()
                .any(|existing| formats_match(*existing, candidate))
            {
                candidates.push(candidate);
            }
        }
    }

    candidates.sort_by(|left, right| {
        left.mSampleRate
            .total_cmp(&right.mSampleRate)
            .then(left.mBitsPerChannel.cmp(&right.mBitsPerChannel))
            .then(left.mBytesPerFrame.cmp(&right.mBytesPerFrame))
            .then(left.mChannelsPerFrame.cmp(&right.mChannelsPerFrame))
            .then(left.mFormatFlags.cmp(&right.mFormatFlags))
    });
    candidates
}

fn ranged_format_supports_rate(ranged: AudioStreamRangedDescription, sample_rate: f64) -> bool {
    sample_rates_match(ranged.mFormat.mSampleRate, sample_rate)
        || (sample_rate >= ranged.mSampleRateRange.mMinimum
            && sample_rate <= ranged.mSampleRateRange.mMaximum)
}

fn formats_match(left: AudioStreamBasicDescription, right: AudioStreamBasicDescription) -> bool {
    sample_rates_match(left.mSampleRate, right.mSampleRate)
        && left.mFormatID == right.mFormatID
        && left.mFormatFlags == right.mFormatFlags
        && left.mBytesPerPacket == right.mBytesPerPacket
        && left.mFramesPerPacket == right.mFramesPerPacket
        && left.mBytesPerFrame == right.mBytesPerFrame
        && left.mChannelsPerFrame == right.mChannelsPerFrame
        && left.mBitsPerChannel == right.mBitsPerChannel
}

fn sample_rates_match(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.5
}

fn describe_capabilities(capabilities: device::OutputDeviceCapabilities) -> String {
    match capabilities.max_bits_per_channel {
        Some(bits) => format!(
            "{bits}-bit integer / {:.1} kHz",
            capabilities.max_sample_rate / 1_000.0
        ),
        None => format!(
            "mixable float / {:.1} kHz",
            capabilities.max_sample_rate / 1_000.0
        ),
    }
}

fn describe_attempt(
    requested: AudioStreamBasicDescription,
    attempted: bool,
    set_error: Option<&str>,
    readback: &Result<AudioStreamBasicDescription, EngineError>,
) -> String {
    let mut description = match readback {
        Ok(actual) if attempted && formats_match(*actual, requested) => {
            format!("accepted ({})", describe_asbd(*actual))
        }
        Ok(actual) if attempted => format!("refused ({})", describe_asbd(*actual)),
        Ok(actual) => format!("not attempted ({})", describe_asbd(*actual)),
        Err(error) => format!("readback error: {error}"),
    };
    if let Some(error) = set_error {
        description.push_str(&format!("; set error: {error}"));
    }
    description
}

fn describe_readback(
    requested: AudioStreamBasicDescription,
    readback: &Result<AudioStreamBasicDescription, EngineError>,
) -> String {
    match readback {
        Ok(actual) if formats_match(*actual, requested) => {
            format!("unchanged ({})", describe_asbd(*actual))
        }
        Ok(actual) => format!("changed ({})", describe_asbd(*actual)),
        Err(error) => format!("readback error: {error}"),
    }
}

fn describe_asbd(format: AudioStreamBasicDescription) -> String {
    format!(
        "{:.1} kHz, {} bit, {} B/frame, {} ch, {}",
        format.mSampleRate / 1_000.0,
        format.mBitsPerChannel,
        format.mBytesPerFrame,
        format.mChannelsPerFrame,
        describe_flags(format.mFormatFlags)
    )
}

fn describe_flags(flags: u32) -> String {
    let mut names = Vec::new();
    for (flag, name) in [
        (kAudioFormatFlagIsFloat, "float"),
        (kAudioFormatFlagIsBigEndian, "big-endian"),
        (kAudioFormatFlagIsSignedInteger, "signed-integer"),
        (kAudioFormatFlagIsPacked, "packed"),
        (kAudioFormatFlagIsAlignedHigh, "aligned-high"),
        (kAudioFormatFlagIsNonInterleaved, "non-interleaved"),
        (kAudioFormatFlagIsNonMixable, "non-mixable"),
    ] {
        if flags & flag != 0 {
            names.push(name);
        }
    }
    let names = if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    };
    format!("0x{flags:08x} ({names})")
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

fn print_inventory(reports: &[DeviceReport]) {
    println!("## Device inventory");
    println!();
    println!(
        "| Device | UID | Capabilities | Output streams | Available physical formats | Available virtual formats | Mixing during probe | Restore |"
    );
    println!("|---|---|---:|---:|---:|---:|---|---|");
    for report in reports {
        let row = &report.inventory;
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            markdown(&row.device),
            markdown(&row.uid),
            markdown(&row.capabilities),
            row.stream_count,
            row.physical_format_count,
            row.virtual_format_count,
            markdown(&row.mixing),
            markdown(&row.restored),
        );
    }
    println!();
}

fn print_findings(reports: &[DeviceReport]) {
    println!("## Integer virtual-format findings");
    println!();
    println!(
        "| Device | UID | Stream | Rate | Bits / frame bytes | Channels | Candidate flags | Physical set readback | Virtual readback | Physical after virtual | Result |"
    );
    println!("|---|---|---:|---:|---:|---:|---|---|---|---|---|");
    for finding in reports.iter().flat_map(|report| &report.findings) {
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            markdown(&finding.device),
            markdown(&finding.uid),
            markdown(&finding.stream),
            markdown(&finding.sample_rate),
            markdown(&finding.bits_and_bytes),
            markdown(&finding.channels),
            markdown(&finding.flags),
            markdown(&finding.physical),
            markdown(&finding.virtual_format),
            markdown(&finding.physical_after_virtual),
            markdown(&finding.result),
        );
    }
}

fn markdown(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use objc2_core_audio_types::AudioValueRange;

    use super::*;

    #[test]
    fn integer_candidates_expand_supported_probe_rates_and_keep_full_flags() {
        let flags = kAudioFormatFlagIsSignedInteger
            | kAudioFormatFlagIsPacked
            | kAudioFormatFlagIsAlignedHigh;
        let formats = [ranged_format(0.0, 44_100.0, 96_000.0, flags)];

        let candidates = integer_candidates(&formats);

        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].mSampleRate, 44_100.0);
        assert_eq!(candidates[1].mSampleRate, 48_000.0);
        assert_eq!(candidates[2].mSampleRate, 96_000.0);
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.mFormatFlags == flags)
        );
    }

    #[test]
    fn integer_candidates_reject_float_and_out_of_range_rates() {
        let formats = [
            ranged_format(0.0, 44_100.0, 192_000.0, kAudioFormatFlagIsFloat),
            ranged_format(0.0, 96_000.0, 96_000.0, kAudioFormatFlagIsSignedInteger),
        ];

        let candidates = integer_candidates(&formats);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].mSampleRate, 96_000.0);
    }

    fn ranged_format(
        sample_rate: f64,
        minimum_rate: f64,
        maximum_rate: f64,
        flags: u32,
    ) -> AudioStreamRangedDescription {
        AudioStreamRangedDescription {
            mFormat: AudioStreamBasicDescription {
                mSampleRate: sample_rate,
                mFormatID: kAudioFormatLinearPCM,
                mFormatFlags: flags,
                mBytesPerPacket: 8,
                mFramesPerPacket: 1,
                mBytesPerFrame: 8,
                mChannelsPerFrame: 2,
                mBitsPerChannel: 32,
                mReserved: 0,
            },
            mSampleRateRange: AudioValueRange {
                mMinimum: minimum_rate,
                mMaximum: maximum_rate,
            },
        }
    }
}
