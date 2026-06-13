//! Symphonia decode: FLAC / ALAC / AIFF / WAV → interleaved integer PCM.
//! Runs on the decode thread, pushes into the rtrb producer.

use std::{fs::File, path::Path};

use symphonia::core::{
    errors::Error as SymphoniaError,
    formats::FormatOptions,
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
    probe::Hint,
};

use crate::{PcmFormat, error::EngineError};

pub struct DecodedStream {
    pub format: PcmFormat,
    pub codec: String,
    pub frames: Option<u64>,
}

pub fn open(path: &Path) -> Result<DecodedStream, EngineError> {
    let file = Box::new(File::open(path)?);
    let source = MediaSourceStream::new(file, MediaSourceStreamOptions::default());

    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(decode_error)?;

    let track = probed
        .format
        .default_track()
        .ok_or_else(|| EngineError::Decode("no default audio track".to_string()))?;
    let params = &track.codec_params;

    let sample_rate = params
        .sample_rate
        .ok_or_else(|| EngineError::Decode("missing sample rate".to_string()))?;
    let bits_per_sample = params
        .bits_per_sample
        .or_else(|| params.sample_format.map(sample_format_bits))
        .ok_or_else(|| EngineError::Decode("missing bit depth".to_string()))?;
    let channels = params
        .channels
        .map(|channels| channels.count())
        .ok_or_else(|| EngineError::Decode("missing channel count".to_string()))?;

    Ok(DecodedStream {
        format: PcmFormat {
            sample_rate,
            bits_per_sample: u8::try_from(bits_per_sample).map_err(|_| {
                EngineError::Decode(format!("unsupported bit depth {bits_per_sample}"))
            })?,
            channels: u8::try_from(channels).map_err(|_| {
                EngineError::Decode(format!("unsupported channel count {channels}"))
            })?,
        },
        codec: params.codec.to_string(),
        frames: params.n_frames,
    })
}

fn sample_format_bits(format: symphonia::core::sample::SampleFormat) -> u32 {
    match format {
        symphonia::core::sample::SampleFormat::U8 | symphonia::core::sample::SampleFormat::S8 => 8,
        symphonia::core::sample::SampleFormat::U16 | symphonia::core::sample::SampleFormat::S16 => {
            16
        }
        symphonia::core::sample::SampleFormat::U24 | symphonia::core::sample::SampleFormat::S24 => {
            24
        }
        symphonia::core::sample::SampleFormat::U32
        | symphonia::core::sample::SampleFormat::S32
        | symphonia::core::sample::SampleFormat::F32 => 32,
        symphonia::core::sample::SampleFormat::F64 => 64,
    }
}

fn decode_error(err: SymphoniaError) -> EngineError {
    EngineError::Decode(err.to_string())
}
