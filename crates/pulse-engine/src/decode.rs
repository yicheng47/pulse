//! Symphonia decode: FLAC / ALAC / AIFF / WAV → interleaved integer PCM.
//! Runs on the decode thread, pushes into the rtrb producer.

use std::{fs::File, io::ErrorKind, path::Path};

use symphonia::core::{
    audio::{AudioBuffer, AudioBufferRef, RawSampleBuffer, Signal},
    codecs::DecoderOptions,
    errors::Error as SymphoniaError,
    formats::FormatOptions,
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
    probe::Hint,
    sample::i24,
};

use crate::{PcmFormat, error::EngineError};

pub struct DecodedStream {
    pub format: PcmFormat,
    pub codec: String,
    pub frames: Option<u64>,
}

pub fn open(path: &Path) -> Result<DecodedStream, EngineError> {
    let probed = probe(path)?;
    let track = probed
        .format
        .default_track()
        .ok_or_else(|| EngineError::Decode("no default audio track".to_string()))?;
    decoded_stream_from_track(track)
}

pub fn stream_pcm<F>(path: &Path, expected: PcmFormat, mut on_pcm: F) -> Result<u64, EngineError>
where
    F: FnMut(&[u8]) -> Result<(), EngineError>,
{
    let mut probed = probe(path)?;
    let track = probed
        .format
        .default_track()
        .ok_or_else(|| EngineError::Decode("no default audio track".to_string()))?;
    let track_id = track.id;
    let actual = decoded_stream_from_track(track)?.format;
    if actual != expected {
        return Err(EngineError::UnsupportedFormat(format!(
            "decoded format {actual:?} does not match engine format {expected:?}"
        )));
    }

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(decode_error)?;
    let mut frames = 0_u64;

    loop {
        let packet = match probed.format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(err)) if err.kind() == ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(decode_error(err)),
        };
        if packet.track_id() != track_id {
            continue;
        }

        let audio_buf = decoder.decode(&packet).map_err(decode_error)?;
        frames = frames
            .checked_add(audio_buf.frames() as u64)
            .ok_or_else(|| EngineError::Decode("decoded frame count overflow".to_string()))?;
        write_interleaved_bytes(audio_buf, expected, &mut on_pcm)?;
    }

    Ok(frames)
}

fn probe(path: &Path) -> Result<symphonia::core::probe::ProbeResult, EngineError> {
    let file = Box::new(File::open(path)?);
    let source = MediaSourceStream::new(file, MediaSourceStreamOptions::default());

    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }

    symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(decode_error)
}

fn decoded_stream_from_track(
    track: &symphonia::core::formats::Track,
) -> Result<DecodedStream, EngineError> {
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

fn write_interleaved_bytes<F>(
    audio_buf: AudioBufferRef<'_>,
    expected: PcmFormat,
    on_pcm: &mut F,
) -> Result<(), EngineError>
where
    F: FnMut(&[u8]) -> Result<(), EngineError>,
{
    if audio_buf.spec().rate != expected.sample_rate {
        return Err(EngineError::UnsupportedFormat(format!(
            "decoded packet has sample rate {} for {} Hz stream",
            audio_buf.spec().rate,
            expected.sample_rate
        )));
    }
    if audio_buf.spec().channels.count() != usize::from(expected.channels) {
        return Err(EngineError::UnsupportedFormat(format!(
            "decoded packet has {} channels for {}-channel stream",
            audio_buf.spec().channels.count(),
            expected.channels
        )));
    }

    let duration = audio_buf.frames() as u64;
    let spec = *audio_buf.spec();
    match (expected.bits_per_sample, audio_buf) {
        (16, AudioBufferRef::S16(buf)) => {
            let mut raw = RawSampleBuffer::<i16>::new(duration, spec);
            raw.copy_interleaved_typed(buf.as_ref());
            on_pcm(raw.as_bytes())
        }
        (16, AudioBufferRef::S32(buf)) => {
            write_promoted_s32_as_i16(buf.as_ref(), expected.channels, on_pcm)
        }
        (24, AudioBufferRef::S24(buf)) => {
            let mut raw = RawSampleBuffer::<i24>::new(duration, spec);
            raw.copy_interleaved_typed(buf.as_ref());
            on_pcm(raw.as_bytes())
        }
        (24, AudioBufferRef::S32(buf)) => {
            write_promoted_s32_as_i24(buf.as_ref(), expected.channels, on_pcm)
        }
        (32, AudioBufferRef::S32(buf)) => {
            let mut raw = RawSampleBuffer::<i32>::new(duration, spec);
            raw.copy_interleaved_typed(buf.as_ref());
            on_pcm(raw.as_bytes())
        }
        (bits, decoded) => Err(EngineError::UnsupportedFormat(format!(
            "cannot directly stream {decoded_kind} decoder output as {bits}-bit PCM",
            decoded_kind = decoded_buffer_kind(&decoded),
        ))),
    }
}

fn write_promoted_s32_as_i16<F>(
    buf: &AudioBuffer<i32>,
    channels: u8,
    on_pcm: &mut F,
) -> Result<(), EngineError>
where
    F: FnMut(&[u8]) -> Result<(), EngineError>,
{
    let mut raw = Vec::with_capacity(buf.frames() * usize::from(channels) * 2);
    for frame in 0..buf.frames() {
        for channel in 0..usize::from(channels) {
            let sample = (buf.chan(channel)[frame] >> 16) as i16;
            raw.extend_from_slice(&sample.to_ne_bytes());
        }
    }
    on_pcm(&raw)
}

fn write_promoted_s32_as_i24<F>(
    buf: &AudioBuffer<i32>,
    channels: u8,
    on_pcm: &mut F,
) -> Result<(), EngineError>
where
    F: FnMut(&[u8]) -> Result<(), EngineError>,
{
    let mut raw = Vec::with_capacity(buf.frames() * usize::from(channels) * 3);
    for frame in 0..buf.frames() {
        for channel in 0..usize::from(channels) {
            let sample = i24::from(buf.chan(channel)[frame] >> 8);
            raw.extend_from_slice(&sample.to_ne_bytes());
        }
    }
    on_pcm(&raw)
}

fn decoded_buffer_kind(audio_buf: &AudioBufferRef<'_>) -> &'static str {
    match audio_buf {
        AudioBufferRef::U8(_) => "u8",
        AudioBufferRef::U16(_) => "u16",
        AudioBufferRef::U24(_) => "u24",
        AudioBufferRef::U32(_) => "u32",
        AudioBufferRef::S8(_) => "s8",
        AudioBufferRef::S16(_) => "s16",
        AudioBufferRef::S24(_) => "s24",
        AudioBufferRef::S32(_) => "s32",
        AudioBufferRef::F32(_) => "f32",
        AudioBufferRef::F64(_) => "f64",
    }
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

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Layout, Signal, SignalSpec};

    use super::*;

    #[test]
    fn write_interleaved_bytes_unpacks_promoted_s32_to_s16() {
        let mut buf =
            AudioBuffer::<i32>::new(2, SignalSpec::new_with_layout(44_100, Layout::Stereo));
        buf.render_reserved(Some(2));
        buf.chan_mut(0)
            .copy_from_slice(&[0x1234_0000, -0x1234_0000]);
        buf.chan_mut(1)
            .copy_from_slice(&[-0x0001_0000, 0x7fff_0000]);

        let mut bytes = Vec::new();
        write_interleaved_bytes(
            AudioBufferRef::S32(Cow::Borrowed(&buf)),
            PcmFormat {
                sample_rate: 44_100,
                bits_per_sample: 16,
                channels: 2,
            },
            &mut |pcm| {
                bytes.extend_from_slice(pcm);
                Ok(())
            },
        )
        .expect("promoted 16-bit samples should unpack");

        let mut expected = Vec::new();
        for sample in [0x1234_i16, -1_i16, -0x1234_i16, 0x7fff_i16] {
            expected.extend_from_slice(&sample.to_ne_bytes());
        }
        assert_eq!(bytes, expected);
    }

    #[test]
    fn write_interleaved_bytes_unpacks_promoted_s32_to_s24() {
        let originals = [0x0012_3456_i32, -1_i32, -0x0080_0000_i32, 0x007f_ffff_i32];
        let mut buf =
            AudioBuffer::<i32>::new(2, SignalSpec::new_with_layout(44_100, Layout::Stereo));
        buf.render_reserved(Some(2));
        buf.chan_mut(0)
            .copy_from_slice(&[originals[0] << 8, originals[2] << 8]);
        buf.chan_mut(1)
            .copy_from_slice(&[originals[1] << 8, originals[3] << 8]);

        let mut bytes = Vec::new();
        write_interleaved_bytes(
            AudioBufferRef::S32(Cow::Borrowed(&buf)),
            PcmFormat {
                sample_rate: 44_100,
                bits_per_sample: 24,
                channels: 2,
            },
            &mut |pcm| {
                bytes.extend_from_slice(pcm);
                Ok(())
            },
        )
        .expect("promoted 24-bit samples should unpack");

        let mut expected = Vec::new();
        for sample in originals {
            expected.extend_from_slice(&i24::from(sample).to_ne_bytes());
        }
        assert_eq!(bytes, expected);
    }
}
