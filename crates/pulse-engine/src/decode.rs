//! Symphonia decode: FLAC / ALAC / AIFF / WAV → interleaved integer PCM.
//! Runs on the decode thread, pushes into the rtrb producer.

use std::{fs::File, io::ErrorKind, path::Path};

use symphonia::core::{
    audio::{AudioBuffer, AudioBufferRef, RawSampleBuffer, Signal},
    codecs::{Decoder, DecoderOptions},
    errors::Error as SymphoniaError,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo},
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
    probe::Hint,
    sample::i24,
    units::{Time, TimeBase},
};

use crate::{PcmFormat, error::EngineError};

pub struct DecodedStream {
    pub format: PcmFormat,
    pub codec: String,
    pub frames: Option<u64>,
}

pub(crate) struct PcmDecoder {
    format_reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    stream: DecodedStream,
    time_base: TimeBase,
    pending_pcm: Vec<u8>,
    pending_pcm_offset: usize,
    at_eof: bool,
}

impl PcmDecoder {
    pub(crate) fn open(path: &Path) -> Result<Self, EngineError> {
        let format_reader = probe(path)?.format;
        let track = format_reader
            .default_track()
            .ok_or_else(|| EngineError::Decode("no default audio track".to_string()))?;
        let track_id = track.id;
        let stream = decoded_stream_from_track(track)?;
        let time_base = track
            .codec_params
            .time_base
            .unwrap_or_else(|| TimeBase::new(1, stream.format.sample_rate));
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(decode_error)?;

        Ok(Self {
            format_reader,
            decoder,
            track_id,
            stream,
            time_base,
            pending_pcm: Vec::new(),
            pending_pcm_offset: 0,
            at_eof: false,
        })
    }

    pub(crate) fn format(&self) -> PcmFormat {
        self.stream.format
    }

    pub(crate) fn duration_ms(&self) -> Option<u64> {
        self.stream
            .frames
            .map(|frames| time_to_ms(self.time_base.calc_time(frames)))
    }

    pub(crate) fn seek(&mut self, position_ms: u64) -> Result<u64, EngineError> {
        self.pending_pcm.clear();
        self.pending_pcm_offset = 0;
        if let Some(duration_ms) = self.duration_ms()
            && position_ms >= duration_ms
        {
            self.at_eof = true;
            return Ok(duration_ms);
        }
        self.at_eof = false;
        let requested_ts = ms_to_timestamp_ceil(position_ms, self.time_base);
        let target_ts = self.stream.frames.map_or(requested_ts, |frames| {
            requested_ts.min(frames.saturating_sub(1))
        });
        let seeked = self
            .format_reader
            .seek(
                SeekMode::Accurate,
                SeekTo::TimeStamp {
                    ts: target_ts,
                    track_id: self.track_id,
                },
            )
            .map_err(decode_error)?;
        self.decoder.reset();

        let mut end_ts = seeked.actual_ts;
        loop {
            let packet = match self.format_reader.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                    return Ok(self
                        .duration_ms()
                        .unwrap_or_else(|| time_to_ms(self.time_base.calc_time(end_ts))));
                }
                Err(err) => return Err(decode_error(err)),
            };
            if packet.track_id() != self.track_id {
                continue;
            }

            let packet_ts = packet.ts();
            let packet_end_ts = packet_ts.saturating_add(packet.dur());
            let audio_buf = self.decoder.decode(&packet).map_err(decode_error)?;
            let decoded_frames = audio_buf.frames();
            end_ts = end_ts.max(packet_end_ts);
            if packet_end_ts <= target_ts || decoded_frames == 0 {
                continue;
            }

            let frames_to_drop = usize::try_from(frames_for_timestamp_delta_ceil(
                target_ts.saturating_sub(packet_ts),
                self.time_base,
                self.stream.format.sample_rate,
            ))
            .unwrap_or(usize::MAX)
            .min(decoded_frames);
            if frames_to_drop == decoded_frames {
                continue;
            }

            self.pending_pcm.clear();
            write_interleaved_bytes(audio_buf, self.stream.format, &mut |bytes| {
                self.pending_pcm.extend_from_slice(bytes);
                Ok(())
            })?;
            self.pending_pcm_offset = frames_to_drop * self.stream.format.bytes_per_frame();
            return Ok(frame_position_ms(
                packet_ts,
                frames_to_drop,
                self.time_base,
                self.stream.format.sample_rate,
            ));
        }
    }

    pub(crate) fn next_pcm(&mut self, pcm: &mut Vec<u8>) -> Result<Option<u64>, EngineError> {
        if self.at_eof {
            return Ok(None);
        }
        if self.pending_pcm_offset < self.pending_pcm.len() {
            pcm.clear();
            pcm.extend_from_slice(&self.pending_pcm[self.pending_pcm_offset..]);
            let frames = pcm.len() / self.stream.format.bytes_per_frame();
            self.pending_pcm.clear();
            self.pending_pcm_offset = 0;
            return Ok(Some(frames as u64));
        }

        loop {
            let packet = match self.format_reader.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(err) => return Err(decode_error(err)),
            };
            if packet.track_id() != self.track_id {
                continue;
            }

            let audio_buf = self.decoder.decode(&packet).map_err(decode_error)?;
            let frames = audio_buf.frames() as u64;
            pcm.clear();
            write_interleaved_bytes(audio_buf, self.stream.format, &mut |bytes| {
                pcm.extend_from_slice(bytes);
                Ok(())
            })?;
            return Ok(Some(frames));
        }
    }
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
    let mut decoder = PcmDecoder::open(path)?;
    let actual = decoder.format();
    if actual != expected {
        return Err(EngineError::UnsupportedFormat(format!(
            "decoded format {actual:?} does not match engine format {expected:?}"
        )));
    }

    let mut frames = 0_u64;
    let mut pcm = Vec::new();

    while let Some(decoded_frames) = decoder.next_pcm(&mut pcm)? {
        frames = frames
            .checked_add(decoded_frames)
            .ok_or_else(|| EngineError::Decode("decoded frame count overflow".to_string()))?;
        on_pcm(&pcm)?;
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

fn time_to_ms(time: Time) -> u64 {
    time.seconds
        .saturating_mul(1_000)
        .saturating_add((time.frac * 1_000.0) as u64)
}

fn ms_to_timestamp_ceil(position_ms: u64, time_base: TimeBase) -> u64 {
    let numerator = u128::from(position_ms) * u128::from(time_base.denom);
    let denominator = 1_000 * u128::from(time_base.numer);
    u64::try_from(numerator.div_ceil(denominator)).unwrap_or(u64::MAX)
}

fn frames_for_timestamp_delta_ceil(
    timestamp_delta: u64,
    time_base: TimeBase,
    sample_rate: u32,
) -> u64 {
    let numerator =
        u128::from(timestamp_delta) * u128::from(time_base.numer) * u128::from(sample_rate);
    u64::try_from(numerator.div_ceil(u128::from(time_base.denom))).unwrap_or(u64::MAX)
}

fn frame_position_ms(
    packet_ts: u64,
    frame_offset: usize,
    time_base: TimeBase,
    sample_rate: u32,
) -> u64 {
    let timestamp_units =
        u128::from(packet_ts) * u128::from(time_base.numer) * u128::from(sample_rate);
    let frame_units = frame_offset as u128 * u128::from(time_base.denom);
    let milliseconds = (timestamp_units + frame_units) * 1_000
        / (u128::from(time_base.denom) * u128::from(sample_rate));
    u64::try_from(milliseconds).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::{
        borrow::Cow,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Layout, Signal, SignalSpec};

    use super::*;

    static NEXT_TEST_FILE: AtomicU64 = AtomicU64::new(0);
    // Generated once with FLAC 1.5.0 from an 800 ms, 8 kHz stereo i16 ramp using 576-frame blocks, no seek table, and no padding.
    const RAMP_FLAC: &[u8] = include_bytes!("../tests/fixtures/ramp-800ms.flac");

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

    #[test]
    fn seek_between_packet_boundaries_starts_pcm_at_the_target_frame() {
        for bits_per_sample in [16, 24, 32] {
            let file = ramp_wav(4_000, bits_per_sample);
            let mut decoder = PcmDecoder::open(file.path()).unwrap();

            let actual_ms = decoder.seek(1_500).unwrap();
            let mut pcm = Vec::new();
            let decoded_frames = decoder.next_pcm(&mut pcm).unwrap().unwrap();

            assert!(actual_ms.abs_diff(1_500) <= 1);
            assert_eq!(first_sample(&pcm, bits_per_sample), 1_500);
            assert_eq!(
                pcm.len(),
                decoded_frames as usize * decoder.format().bytes_per_frame()
            );
            let mut next_pcm = Vec::new();
            decoder.next_pcm(&mut next_pcm).unwrap().unwrap();
            assert_eq!(
                first_sample(&next_pcm, bits_per_sample),
                1_500 + decoded_frames as i32
            );
        }
    }

    #[test]
    fn seek_to_packet_boundary_returns_that_boundary() {
        let file = ramp_wav(4_000, 16);
        let mut decoder = PcmDecoder::open(file.path()).unwrap();

        let actual_ms = decoder.seek(2_304).unwrap();
        let mut pcm = Vec::new();
        decoder.next_pcm(&mut pcm).unwrap().unwrap();

        assert_eq!(actual_ms, 2_304);
        assert_eq!(first_sample(&pcm, 16), 2_304);
    }

    #[test]
    fn seek_past_eof_clamps_to_the_file_end() {
        let file = ramp_wav(4_000, 16);
        let mut decoder = PcmDecoder::open(file.path()).unwrap();

        assert_eq!(decoder.seek(10_000).unwrap(), 4_000);
        assert_eq!(decoder.next_pcm(&mut Vec::new()).unwrap(), None);
    }

    #[test]
    fn flac_seek_between_packet_boundaries_starts_at_the_target_frame() {
        let file = TestAudioFile::new("flac", RAMP_FLAC);
        let mut decoder = PcmDecoder::open(file.path()).unwrap();

        let actual_ms = decoder.seek(333).unwrap();
        let mut pcm = Vec::new();
        decoder.next_pcm(&mut pcm).unwrap().unwrap();

        assert!(actual_ms.abs_diff(333) <= 1);
        assert_eq!(first_sample(&pcm, 16), 2_664);
    }

    #[test]
    fn flac_seek_to_exact_duration_returns_eof() {
        let file = TestAudioFile::new("flac", RAMP_FLAC);
        let mut decoder = PcmDecoder::open(file.path()).unwrap();

        assert_eq!(decoder.duration_ms(), Some(800));
        assert_eq!(decoder.seek(800).unwrap(), 800);
        assert_eq!(decoder.next_pcm(&mut Vec::new()).unwrap(), None);
    }

    fn ramp_wav(frames: u32, bits_per_sample: u16) -> TestAudioFile {
        const CHANNELS: u16 = 2;
        const SAMPLE_RATE: u32 = 1_000;
        let block_align = CHANNELS * (bits_per_sample / 8);
        let data_len = frames * u32::from(block_align);
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&CHANNELS.to_le_bytes());
        wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        wav.extend_from_slice(&(SAMPLE_RATE * u32::from(block_align)).to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits_per_sample.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for frame in 0..frames {
            for _ in 0..CHANNELS {
                let sample = (frame as i32).to_le_bytes();
                wav.extend_from_slice(&sample[..usize::from(bits_per_sample / 8)]);
            }
        }
        TestAudioFile::new("wav", &wav)
    }

    fn first_sample(pcm: &[u8], bits_per_sample: u16) -> i32 {
        match bits_per_sample {
            16 => i32::from(i16::from_ne_bytes(pcm[..2].try_into().unwrap())),
            24 => i32::from_ne_bytes([pcm[0], pcm[1], pcm[2], 0]),
            32 => i32::from_ne_bytes(pcm[..4].try_into().unwrap()),
            _ => unreachable!(),
        }
    }

    struct TestAudioFile {
        path: PathBuf,
    }

    impl TestAudioFile {
        fn new(extension: &str, bytes: &[u8]) -> Self {
            let sequence = NEXT_TEST_FILE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "pulse-engine-{}-{sequence}.{extension}",
                std::process::id()
            ));
            std::fs::write(&path, bytes).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestAudioFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
