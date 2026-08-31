use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("{call} failed (OSStatus {status})")]
    Os { call: &'static str, status: i32 },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("no Core Audio output device is available")]
    NoOutputDevice,
    #[error("output device {0} advertises no supported PCM physical formats")]
    NoOutputCapabilities(u32),
    #[error("device hogged by pid {0}")]
    Hogged(i32),
    #[error("device does not support the nominal sample rate in {0:?}")]
    UnsupportedNominalSampleRate(crate::PcmFormat),
    #[error("no physical format matches {0:?}")]
    NoMatchingPhysicalFormat(crate::PcmFormat),
    #[error("audio unit: {0}")]
    AudioUnit(String),
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("timed out waiting for {0}")]
    Timeout(&'static str),
    #[error("playback backend release failed: {0}")]
    BackendRelease(String),
    #[error("decode: {0}")]
    Decode(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORMAT: crate::PcmFormat = crate::PcmFormat {
        sample_rate: 44_100,
        bits_per_sample: 16,
        channels: 2,
    };

    #[test]
    fn nominal_rate_and_physical_format_errors_are_distinct() {
        assert_eq!(
            EngineError::UnsupportedNominalSampleRate(FORMAT).to_string(),
            "device does not support the nominal sample rate in PcmFormat { sample_rate: 44100, bits_per_sample: 16, channels: 2 }"
        );
        assert_eq!(
            EngineError::NoMatchingPhysicalFormat(FORMAT).to_string(),
            "no physical format matches PcmFormat { sample_rate: 44100, bits_per_sample: 16, channels: 2 }"
        );
    }
}
