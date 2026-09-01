use std::{
    fs::{self, File},
    io::{self, BufReader, Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use lofty::{
    config::ParseOptions,
    file::{AudioFile, TaggedFile, TaggedFileExt},
    flac::FlacFile,
    iff::{
        aiff::{AiffCompressionType, AiffFile},
        wav::{WavFile, WavFormat},
    },
    mp4::{Mp4Codec, Mp4File},
    picture::PictureType,
    tag::{Accessor, ItemKey, Tag},
};
#[cfg(test)]
use lofty::{
    config::WriteOptions,
    picture::{MimeType, Picture},
    tag::{TagExt, TagType},
};
use thiserror::Error;

#[derive(Debug)]
pub(in crate::backend) struct AudioMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration_ms: Option<i64>,
    pub sample_rate_hz: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    pub artwork: Option<EmbeddedArtwork>,
}

#[derive(Debug)]
pub(in crate::backend) struct EmbeddedArtwork {
    pub data: Vec<u8>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Error)]
pub(in crate::backend) enum MetadataError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Lofty(#[from] lofty::error::LoftyError),
    #[error("unsupported audio format: {0}")]
    Unsupported(String),
    #[error("audio duration is too large to store")]
    DurationOutOfRange,
}

pub(in crate::backend) fn extract_metadata(path: &Path) -> Result<AudioMetadata, MetadataError> {
    if is_dsd_path(path) {
        return extract_dsd_metadata(path);
    }
    let tagged_file = read_pcm_file(path)?;
    let properties = tagged_file.properties();
    let duration_ms = (!properties.duration().is_zero())
        .then(|| i64::try_from(properties.duration().as_millis()))
        .transpose()
        .map_err(|_| MetadataError::DurationOutOfRange)?;

    Ok(AudioMetadata {
        title: tag_text(&tagged_file, ItemKey::TrackTitle),
        artist: tag_text(&tagged_file, ItemKey::TrackArtist),
        album: tag_text(&tagged_file, ItemKey::AlbumTitle),
        album_artist: tag_text(&tagged_file, ItemKey::AlbumArtist),
        year: tags_in_priority_order(&tagged_file)
            .find_map(Accessor::date)
            .map(|date| u32::from(date.year)),
        genre: tag_text(&tagged_file, ItemKey::Genre),
        track_number: tags_in_priority_order(&tagged_file).find_map(Accessor::track),
        disc_number: tags_in_priority_order(&tagged_file).find_map(Accessor::disk),
        duration_ms,
        sample_rate_hz: properties.sample_rate(),
        bit_depth: properties.bit_depth(),
        channels: properties.channels(),
        artwork: embedded_artwork(&tagged_file),
    })
}

pub(in crate::backend) fn dsd_sample_rate(path: &Path) -> Result<u32, MetadataError> {
    Ok(read_dsd_properties(path)?.sample_rate_hz)
}

struct DsdProperties {
    sample_rate_hz: u32,
    channels: u8,
    duration_ms: i64,
    metadata_offset: Option<u64>,
}

fn extract_dsd_metadata(path: &Path) -> Result<AudioMetadata, MetadataError> {
    let properties = read_dsd_properties(path)?;
    let tagged_file = properties
        .metadata_offset
        .map(|offset| {
            let mut file = File::open(path)?;
            let file_len = file.metadata()?.len();
            read_dsf_id3(&mut file, file_len, offset)
        })
        .transpose()?;
    let fallback_title = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned());

    Ok(AudioMetadata {
        title: tagged_file
            .as_ref()
            .and_then(|tagged| tag_text(tagged, ItemKey::TrackTitle))
            .or(fallback_title),
        artist: tagged_file
            .as_ref()
            .and_then(|tagged| tag_text(tagged, ItemKey::TrackArtist)),
        album: tagged_file
            .as_ref()
            .and_then(|tagged| tag_text(tagged, ItemKey::AlbumTitle)),
        album_artist: tagged_file
            .as_ref()
            .and_then(|tagged| tag_text(tagged, ItemKey::AlbumArtist)),
        year: tagged_file
            .as_ref()
            .and_then(|tagged| tags_in_priority_order(tagged).find_map(Accessor::date))
            .map(|date| u32::from(date.year)),
        genre: tagged_file
            .as_ref()
            .and_then(|tagged| tag_text(tagged, ItemKey::Genre)),
        track_number: tagged_file
            .as_ref()
            .and_then(|tagged| tags_in_priority_order(tagged).find_map(Accessor::track)),
        disc_number: tagged_file
            .as_ref()
            .and_then(|tagged| tags_in_priority_order(tagged).find_map(Accessor::disk)),
        duration_ms: Some(properties.duration_ms),
        sample_rate_hz: Some(properties.sample_rate_hz),
        bit_depth: Some(1),
        channels: Some(properties.channels),
        artwork: tagged_file.as_ref().and_then(embedded_artwork),
    })
}

fn read_dsd_properties(path: &Path) -> Result<DsdProperties, MetadataError> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    match &read_array::<4>(&mut file)? {
        b"DSD " => parse_dsf_properties(&mut file, file_len),
        b"FRM8" => parse_dff_properties(&mut file, file_len),
        _ => Err(MetadataError::Unsupported(
            "invalid DSD container".to_string(),
        )),
    }
}

fn parse_dsf_properties(file: &mut File, file_len: u64) -> Result<DsdProperties, MetadataError> {
    if read_u64_le(file)? != 28 {
        return Err(MetadataError::Unsupported(
            "invalid DSF header size".to_string(),
        ));
    }
    let declared_file_size = read_u64_le(file)?;
    if declared_file_size > file_len {
        return Err(MetadataError::Unsupported(
            "truncated DSF container".to_string(),
        ));
    }
    let metadata_offset = match read_u64_le(file)? {
        0 => None,
        offset => Some(offset),
    };
    if read_array::<4>(file)? != *b"fmt " || read_u64_le(file)? != 52 {
        return Err(MetadataError::Unsupported(
            "invalid DSF format chunk".to_string(),
        ));
    }
    let version = read_u32_le(file)?;
    let format_id = read_u32_le(file)?;
    if version != 1 || format_id != 0 {
        return Err(MetadataError::Unsupported(format!(
            "unsupported DSF format version {version}, ID {format_id}"
        )));
    }
    let _channel_type = read_u32_le(file)?;
    let channels = channel_count(read_u32_le(file)?, "DSF")?;
    let sample_rate_hz = read_u32_le(file)?;
    validate_dsd_rate(sample_rate_hz)?;
    let bits_per_sample = read_u32_le(file)?;
    if bits_per_sample == 8 {
        return Err(MetadataError::Unsupported(
            "MSB-first DSF is not supported".to_string(),
        ));
    }
    if bits_per_sample != 1 {
        return Err(MetadataError::Unsupported(format!(
            "unsupported DSF bits-per-sample value {bits_per_sample}"
        )));
    }
    let sample_count = read_u64_le(file)?;
    let block_size = read_u32_le(file)?;
    if block_size != 4_096 {
        return Err(MetadataError::Unsupported(format!(
            "unsupported DSF channel block size {block_size}"
        )));
    }
    let _reserved = read_u32_le(file)?;
    if read_array::<4>(file)? != *b"data" {
        return Err(MetadataError::Unsupported(
            "DSF data chunk is missing".to_string(),
        ));
    }
    let data_size = read_u64_le(file)?;
    if data_size < 12 {
        return Err(MetadataError::Unsupported(
            "truncated DSF data chunk".to_string(),
        ));
    }
    let data_end = file
        .stream_position()?
        .checked_add(data_size - 12)
        .ok_or_else(|| MetadataError::Unsupported("DSF data size overflow".to_string()))?;
    if data_end > file_len {
        return Err(MetadataError::Unsupported(
            "truncated DSF data chunk".to_string(),
        ));
    }
    let bytes_per_channel = sample_count.div_ceil(8);
    let blocks_per_channel = bytes_per_channel.div_ceil(u64::from(block_size));
    let required_data_size = blocks_per_channel
        .checked_mul(u64::from(block_size))
        .and_then(|size| size.checked_mul(u64::from(channels)))
        .ok_or_else(|| MetadataError::Unsupported("DSF data size overflow".to_string()))?;
    if data_size - 12 < required_data_size {
        return Err(MetadataError::Unsupported(
            "DSF data chunk is shorter than its sample count".to_string(),
        ));
    }

    Ok(DsdProperties {
        sample_rate_hz,
        channels,
        duration_ms: dsd_duration_ms(u128::from(sample_count), sample_rate_hz)?,
        metadata_offset,
    })
}

fn parse_dff_properties(file: &mut File, file_len: u64) -> Result<DsdProperties, MetadataError> {
    let form_size = read_u64_be(file)?;
    let form_end = 12_u64
        .checked_add(form_size)
        .ok_or_else(|| MetadataError::Unsupported("DFF FRM8 size overflow".to_string()))?;
    if form_end != file_len || form_size < 4 || read_array::<4>(file)? != *b"DSD " {
        return Err(MetadataError::Unsupported(
            "invalid DFF FRM8 container".to_string(),
        ));
    }

    let mut sample_rate_hz = None;
    let mut channels = None;
    let mut compression = None;
    let mut data_size = None;
    while file.stream_position()? < form_end {
        let (chunk_id, chunk_size, data_offset, padded_end) =
            read_dff_chunk_header(file, form_end)?;
        match &chunk_id {
            b"PROP" => {
                let parsed = parse_dff_sound_properties(file, data_offset, chunk_size)?;
                sample_rate_hz = parsed.0;
                channels = parsed.1;
                compression = parsed.2;
            }
            b"DSD " => data_size = Some(chunk_size),
            b"DST " => {
                return Err(MetadataError::Unsupported(
                    "DST-compressed DFF is not supported".to_string(),
                ));
            }
            _ => {}
        }
        file.seek(SeekFrom::Start(padded_end))?;
    }

    let sample_rate_hz = sample_rate_hz
        .ok_or_else(|| MetadataError::Unsupported("DFF sample rate is missing".to_string()))?;
    validate_dsd_rate(sample_rate_hz)?;
    let channels = channels
        .ok_or_else(|| MetadataError::Unsupported("DFF channels are missing".to_string()))?;
    let compression = compression
        .ok_or_else(|| MetadataError::Unsupported("DFF compression is missing".to_string()))?;
    if compression == *b"DST " {
        return Err(MetadataError::Unsupported(
            "DST-compressed DFF is not supported".to_string(),
        ));
    }
    if compression != *b"DSD " {
        return Err(MetadataError::Unsupported(format!(
            "unsupported DFF compression {}",
            String::from_utf8_lossy(&compression)
        )));
    }
    let data_size = data_size
        .ok_or_else(|| MetadataError::Unsupported("DFF sound data is missing".to_string()))?;
    if data_size % u64::from(channels) != 0 {
        return Err(MetadataError::Unsupported(
            "DFF sound data has incomplete channel clusters".to_string(),
        ));
    }
    let bits_per_channel = u128::from(data_size / u64::from(channels)) * 8;

    Ok(DsdProperties {
        sample_rate_hz,
        channels,
        duration_ms: dsd_duration_ms(bits_per_channel, sample_rate_hz)?,
        metadata_offset: None,
    })
}

type DffSoundProperties = (Option<u32>, Option<u8>, Option<[u8; 4]>);

fn parse_dff_sound_properties(
    file: &mut File,
    data_offset: u64,
    size: u64,
) -> Result<DffSoundProperties, MetadataError> {
    if size < 4 {
        return Err(MetadataError::Unsupported(
            "DFF PROP chunk is too short".to_string(),
        ));
    }
    let end = data_offset
        .checked_add(size)
        .ok_or_else(|| MetadataError::Unsupported("DFF PROP size overflow".to_string()))?;
    file.seek(SeekFrom::Start(data_offset))?;
    if read_array::<4>(file)? != *b"SND " {
        return Err(MetadataError::Unsupported(
            "DFF PROP is not sound properties".to_string(),
        ));
    }
    let mut sample_rate_hz = None;
    let mut channels = None;
    let mut compression = None;
    while file.stream_position()? < end {
        let (chunk_id, chunk_size, _, padded_end) = read_dff_chunk_header(file, end)?;
        match &chunk_id {
            b"FS  " if chunk_size == 4 => sample_rate_hz = Some(read_u32_be(file)?),
            b"CHNL" if chunk_size >= 2 => {
                let count = read_u16_be(file)?;
                if chunk_size != 2 + u64::from(count) * 4 {
                    return Err(MetadataError::Unsupported(
                        "DFF CHNL size does not match its channel count".to_string(),
                    ));
                }
                channels = Some(channel_count(u32::from(count), "DFF")?);
            }
            b"CMPR" if chunk_size >= 5 => {
                let parsed_compression = read_array::<4>(file)?;
                let name_len = read_array::<1>(file)?[0];
                if u64::from(name_len) > chunk_size - 5 {
                    return Err(MetadataError::Unsupported(
                        "DFF compression name exceeds its chunk".to_string(),
                    ));
                }
                compression = Some(parsed_compression);
            }
            b"FS  " => {
                return Err(MetadataError::Unsupported(
                    "invalid DFF FS chunk".to_string(),
                ));
            }
            b"CHNL" => {
                return Err(MetadataError::Unsupported(
                    "invalid DFF CHNL chunk".to_string(),
                ));
            }
            b"CMPR" => {
                return Err(MetadataError::Unsupported(
                    "invalid DFF CMPR chunk".to_string(),
                ));
            }
            _ => {}
        }
        file.seek(SeekFrom::Start(padded_end))?;
    }
    Ok((sample_rate_hz, channels, compression))
}

fn read_dff_chunk_header(
    file: &mut File,
    limit: u64,
) -> Result<([u8; 4], u64, u64, u64), MetadataError> {
    let header_offset = file.stream_position()?;
    if header_offset.checked_add(12).is_none_or(|end| end > limit) {
        return Err(MetadataError::Unsupported(
            "truncated DFF chunk header".to_string(),
        ));
    }
    let chunk_id = read_array::<4>(file)?;
    let chunk_size = read_u64_be(file)?;
    let data_offset = file.stream_position()?;
    let padded_end = data_offset
        .checked_add(chunk_size)
        .and_then(|end| end.checked_add(chunk_size & 1))
        .ok_or_else(|| MetadataError::Unsupported("DFF chunk size overflow".to_string()))?;
    if padded_end > limit {
        return Err(MetadataError::Unsupported(format!(
            "DFF chunk {} exceeds its container",
            String::from_utf8_lossy(&chunk_id)
        )));
    }
    Ok((chunk_id, chunk_size, data_offset, padded_end))
}

fn read_dsf_id3(
    file: &mut File,
    file_len: u64,
    metadata_offset: u64,
) -> Result<TaggedFile, MetadataError> {
    if metadata_offset
        .checked_add(10)
        .is_none_or(|end| end > file_len)
    {
        return Err(MetadataError::Unsupported(
            "truncated DSF ID3v2 header".to_string(),
        ));
    }
    file.seek(SeekFrom::Start(metadata_offset))?;
    let header = read_array::<10>(file)?;
    if &header[..3] != b"ID3" || header[6..10].iter().any(|byte| byte & 0x80 != 0) {
        return Err(MetadataError::Unsupported(
            "DSF metadata offset does not point to ID3v2".to_string(),
        ));
    }
    let content_size = header[6..10]
        .iter()
        .fold(0_u64, |size, byte| (size << 7) | u64::from(*byte));
    let footer_size = if header[5] & 0x10 == 0 { 0 } else { 10 };
    let tag_size = 10_u64
        .checked_add(content_size)
        .and_then(|size| size.checked_add(footer_size))
        .ok_or_else(|| MetadataError::Unsupported("DSF ID3v2 size overflow".to_string()))?;
    if metadata_offset
        .checked_add(tag_size)
        .is_none_or(|end| end > file_len)
    {
        return Err(MetadataError::Unsupported(
            "truncated DSF ID3v2 tag".to_string(),
        ));
    }
    let tag_len = usize::try_from(tag_size)
        .map_err(|_| MetadataError::Unsupported("DSF ID3v2 tag is too large".to_string()))?;
    let mut tag = vec![0; tag_len];
    tag[..10].copy_from_slice(&header);
    file.read_exact(&mut tag[10..])?;

    // Lofty 0.24 keeps its ID3v2 reader private, so expose the tag through a
    // minimal WAV wrapper. The PCM format is arbitrary; only the tag is used.
    let mut wave = Vec::with_capacity(tag.len() + 58);
    wave.extend_from_slice(b"WAVE");
    let mut format = Vec::with_capacity(16);
    format.extend_from_slice(&1_u16.to_le_bytes());
    format.extend_from_slice(&2_u16.to_le_bytes());
    format.extend_from_slice(&44_100_u32.to_le_bytes());
    format.extend_from_slice(&176_400_u32.to_le_bytes());
    format.extend_from_slice(&4_u16.to_le_bytes());
    format.extend_from_slice(&16_u16.to_le_bytes());
    push_riff_chunk(&mut wave, b"fmt ", &format);
    push_riff_chunk(&mut wave, b"data", &[0; 4]);
    // push_riff_chunk unwraps this conversion for test-built chunks; validate
    // the untrusted on-disk tag before passing it through that helper.
    u32::try_from(tag.len())
        .map_err(|_| MetadataError::Unsupported("DSF ID3v2 tag is too large".to_string()))?;
    push_riff_chunk(&mut wave, b"ID3 ", &tag);
    let wave_len = u32::try_from(wave.len())
        .map_err(|_| MetadataError::Unsupported("DSF ID3v2 tag is too large".to_string()))?;
    let mut wrapped = Vec::with_capacity(wave.len() + 8);
    wrapped.extend_from_slice(b"RIFF");
    wrapped.extend_from_slice(&wave_len.to_le_bytes());
    wrapped.extend_from_slice(&wave);
    Ok(WavFile::read_from(&mut Cursor::new(wrapped), ParseOptions::new())?.into())
}

fn is_dsd_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("dsf") || extension.eq_ignore_ascii_case("dff")
        })
}

fn validate_dsd_rate(sample_rate_hz: u32) -> Result<(), MetadataError> {
    if !matches!(sample_rate_hz, 2_822_400 | 5_644_800) {
        return Err(MetadataError::Unsupported(format!(
            "unsupported DSD sample rate {sample_rate_hz}"
        )));
    }
    Ok(())
}

fn channel_count(value: u32, container: &str) -> Result<u8, MetadataError> {
    if value == 0 {
        return Err(MetadataError::Unsupported(format!(
            "{container} channel count is zero"
        )));
    }
    u8::try_from(value)
        .map_err(|_| MetadataError::Unsupported(format!("{container} channel count exceeds 255")))
}

fn dsd_duration_ms(bits_per_channel: u128, sample_rate_hz: u32) -> Result<i64, MetadataError> {
    i64::try_from(bits_per_channel * 1_000 / u128::from(sample_rate_hz))
        .map_err(|_| MetadataError::DurationOutOfRange)
}

fn read_u16_be(file: &mut File) -> io::Result<u16> {
    Ok(u16::from_be_bytes(read_array(file)?))
}

fn read_u32_le(file: &mut File) -> io::Result<u32> {
    Ok(u32::from_le_bytes(read_array(file)?))
}

fn read_u32_be(file: &mut File) -> io::Result<u32> {
    Ok(u32::from_be_bytes(read_array(file)?))
}

fn read_u64_le(file: &mut File) -> io::Result<u64> {
    Ok(u64::from_le_bytes(read_array(file)?))
}

fn read_u64_be(file: &mut File) -> io::Result<u64> {
    Ok(u64::from_be_bytes(read_array(file)?))
}

fn read_array<const N: usize>(file: &mut File) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub(in crate::backend) fn folder_artwork(path: &Path) -> io::Result<Option<EmbeddedArtwork>> {
    const STEMS: &[&str] = &["cover", "folder", "front"];
    const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

    let Some(directory) = path.parent() else {
        return Ok(None);
    };
    let mut best: Option<(usize, PathBuf, &'static str)> = None;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let candidate = entry.path();
        let Some(stem) = candidate.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Some(extension) = candidate
            .extension()
            .and_then(|extension| extension.to_str())
        else {
            continue;
        };
        let stem = stem.to_ascii_lowercase();
        let extension = extension.to_ascii_lowercase();
        let Some(stem_rank) = STEMS.iter().position(|allowed| *allowed == stem) else {
            continue;
        };
        let Some(extension_rank) = EXTENSIONS.iter().position(|allowed| *allowed == extension)
        else {
            continue;
        };
        let rank = stem_rank * EXTENSIONS.len() + extension_rank;
        if best
            .as_ref()
            .is_none_or(|(best_rank, _, _)| rank < *best_rank)
        {
            let mime_type = if extension == "png" {
                "image/png"
            } else {
                "image/jpeg"
            };
            best = Some((rank, candidate, mime_type));
        }
    }

    let Some((_, path, mime_type)) = best else {
        return Ok(None);
    };
    Ok(Some(EmbeddedArtwork {
        data: fs::read(path)?,
        mime_type: Some(mime_type.to_string()),
    }))
}

fn read_pcm_file(path: &Path) -> Result<TaggedFile, MetadataError> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| MetadataError::Unsupported("missing file extension".to_string()))?;
    let mut reader = BufReader::new(File::open(path)?);
    let options = ParseOptions::new();

    match extension.as_str() {
        "flac" => Ok(FlacFile::read_from(&mut reader, options)?.into()),
        "m4a" => {
            let file = Mp4File::read_from(&mut reader, options)?;
            if file.properties().codec() != &Mp4Codec::ALAC {
                return Err(MetadataError::Unsupported(format!(
                    "M4A codec {:?} is not ALAC",
                    file.properties().codec()
                )));
            }
            Ok(file.into())
        }
        "aif" | "aiff" => {
            let file = AiffFile::read_from(&mut reader, options)?;
            if !is_pcm_aiff(file.properties().compression_type()) {
                return Err(MetadataError::Unsupported(format!(
                    "AIFF compression {} is not PCM",
                    file.properties()
                        .compression_type()
                        .expect("non-PCM AIFF has a compression type")
                        .compression_name()
                )));
            }
            Ok(file.into())
        }
        "wav" => {
            let file = WavFile::read_from(&mut reader, options)?;
            if file.properties().format() != &WavFormat::PCM {
                return Err(MetadataError::Unsupported(format!(
                    "WAV format {:?} is not integer PCM",
                    file.properties().format()
                )));
            }
            Ok(file.into())
        }
        _ => Err(MetadataError::Unsupported(extension)),
    }
}

fn is_pcm_aiff(compression: Option<&AiffCompressionType>) -> bool {
    match compression {
        None | Some(AiffCompressionType::None | AiffCompressionType::sowt) => true,
        Some(AiffCompressionType::Other {
            compression_type, ..
        }) => matches!(compression_type, b"twos" | b"TWOS" | b"none"),
        _ => false,
    }
}

fn tags_in_priority_order(file: &TaggedFile) -> impl Iterator<Item = &Tag> {
    let primary_type = file.primary_tag_type();
    file.primary_tag().into_iter().chain(
        file.tags()
            .iter()
            .filter(move |tag| tag.tag_type() != primary_type),
    )
}

fn tag_text(file: &TaggedFile, key: ItemKey) -> Option<String> {
    tags_in_priority_order(file).find_map(|tag| {
        tag.get_string(key).and_then(|value| {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        })
    })
}

fn embedded_artwork(file: &TaggedFile) -> Option<EmbeddedArtwork> {
    let mut first = None;
    for tag in tags_in_priority_order(file) {
        for picture in tag.pictures() {
            if picture.pic_type() == PictureType::CoverFront {
                return Some(EmbeddedArtwork {
                    data: picture.data().to_vec(),
                    mime_type: picture.mime_type().map(ToString::to_string),
                });
            }
            first.get_or_insert(picture);
        }
    }

    first.map(|picture| EmbeddedArtwork {
        data: picture.data().to_vec(),
        mime_type: picture.mime_type().map(ToString::to_string),
    })
}

#[cfg(test)]
pub(crate) fn write_test_wav(
    path: &Path,
    title: &str,
    artist: &str,
    album: &str,
) -> std::io::Result<()> {
    write_test_wav_with_format(path, title, artist, album, 1)
}

#[cfg(test)]
pub(in crate::backend) fn write_test_wav_with_format(
    path: &Path,
    title: &str,
    artist: &str,
    album: &str,
    audio_format: u16,
) -> std::io::Result<()> {
    const CHANNELS: u16 = 2;
    const SAMPLE_RATE: u32 = 44_100;
    const BITS_PER_SAMPLE: u16 = 16;
    const FRAMES: usize = 441;

    let mut wave = Vec::new();
    wave.extend_from_slice(b"WAVE");

    let mut format = Vec::new();
    format.extend_from_slice(&audio_format.to_le_bytes());
    format.extend_from_slice(&CHANNELS.to_le_bytes());
    format.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);
    format.extend_from_slice(&(SAMPLE_RATE * u32::from(block_align)).to_le_bytes());
    format.extend_from_slice(&block_align.to_le_bytes());
    format.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    push_riff_chunk(&mut wave, b"fmt ", &format);

    let samples = vec![0_u8; FRAMES * usize::from(block_align)];
    push_riff_chunk(&mut wave, b"data", &samples);

    let mut info = Vec::from(&b"INFO"[..]);
    push_info_item(&mut info, b"INAM", title);
    push_info_item(&mut info, b"IART", artist);
    push_info_item(&mut info, b"IPRD", album);
    push_info_item(&mut info, b"ICRD", "2024");
    push_info_item(&mut info, b"IGNR", "Electronic");
    push_info_item(&mut info, b"ITRK", "7");
    push_riff_chunk(&mut wave, b"LIST", &info);

    let mut file = Vec::from(&b"RIFF"[..]);
    file.extend_from_slice(&(u32::try_from(wave.len()).unwrap()).to_le_bytes());
    file.extend_from_slice(&wave);
    std::fs::write(path, file)
}

#[cfg(test)]
pub(in crate::backend) fn write_test_wav_with_artwork(
    path: &Path,
    title: &str,
    artist: &str,
    album: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    write_test_wav(path, title, artist, album)?;
    let mut tag = Tag::new(TagType::Id3v2);
    tag.push_picture(
        Picture::unchecked(vec![1, 2, 3])
            .pic_type(PictureType::CoverFront)
            .mime_type(MimeType::Png)
            .build(),
    );
    tag.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

#[cfg(test)]
fn push_info_item(info: &mut Vec<u8>, id: &[u8; 4], value: &str) {
    let mut data = value.as_bytes().to_vec();
    data.push(0);
    push_riff_chunk(info, id, &data);
}

fn push_riff_chunk(target: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    target.extend_from_slice(id);
    target.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_le_bytes());
    target.extend_from_slice(data);
    if !data.len().is_multiple_of(2) {
        target.push(0);
    }
}

#[cfg(test)]
mod tests {
    use lofty::tag::TagType;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn extracts_tags_and_properties_from_generated_wav() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("generated.wav");
        write_test_wav(&path, "Test Title", "Test Artist", "Test Album").unwrap();

        let metadata = extract_metadata(&path).unwrap();
        let engine = pulse_engine::decode::open(&path).unwrap();
        let engine_duration_ms =
            engine.frames.unwrap() * 1_000 / u64::from(engine.format.sample_rate);

        assert_eq!(metadata.title.as_deref(), Some("Test Title"));
        assert_eq!(metadata.artist.as_deref(), Some("Test Artist"));
        assert_eq!(metadata.album.as_deref(), Some("Test Album"));
        assert_eq!(metadata.year, Some(2024));
        assert_eq!(metadata.genre.as_deref(), Some("Electronic"));
        assert_eq!(metadata.track_number, Some(7));
        assert_eq!(metadata.sample_rate_hz, Some(44_100));
        assert_eq!(metadata.bit_depth, Some(16));
        assert_eq!(metadata.channels, Some(2));
        assert_eq!(metadata.duration_ms, Some(10));
        assert_eq!(metadata.duration_ms, Some(engine_duration_ms as i64));
        assert!(metadata.artwork.is_none());
    }

    #[test]
    fn recognizes_engine_supported_big_endian_pcm_aiff_codes() {
        for compression_type in [*b"twos", *b"TWOS", *b"none"] {
            let compression = AiffCompressionType::Other {
                compression_type,
                compression_name: String::new(),
            };
            assert!(is_pcm_aiff(Some(&compression)));
        }

        let float = AiffCompressionType::fl32;
        assert!(!is_pcm_aiff(Some(&float)));
    }

    #[test]
    fn extracts_dsf_id3_and_true_dsd_properties() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("tagged.dsf");
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title("DSF Title".to_string());
        tag.set_artist("DSF Artist".to_string());
        tag.set_album("DSF Album".to_string());
        tag.insert_text(ItemKey::AlbumArtist, "Album Artist".to_string());
        tag.set_genre("Jazz".to_string());
        tag.set_track(3);
        tag.set_disk(2);
        tag.push_picture(
            Picture::unchecked(vec![1, 2, 3])
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Png)
                .build(),
        );
        let mut id3 = Vec::new();
        tag.dump_to(&mut id3, WriteOptions::default()).unwrap();
        write_test_dsf(&path, 2_822_400, 28_224, &id3);

        let metadata = extract_metadata(&path).unwrap();

        assert_eq!(metadata.title.as_deref(), Some("DSF Title"));
        assert_eq!(metadata.artist.as_deref(), Some("DSF Artist"));
        assert_eq!(metadata.album.as_deref(), Some("DSF Album"));
        assert_eq!(metadata.album_artist.as_deref(), Some("Album Artist"));
        assert_eq!(metadata.genre.as_deref(), Some("Jazz"));
        assert_eq!(metadata.track_number, Some(3));
        assert_eq!(metadata.disc_number, Some(2));
        assert_eq!(metadata.duration_ms, Some(10));
        assert_eq!(metadata.sample_rate_hz, Some(2_822_400));
        assert_eq!(metadata.bit_depth, Some(1));
        assert_eq!(metadata.channels, Some(2));
        assert_eq!(metadata.artwork.unwrap().data, [1, 2, 3]);
    }

    #[test]
    fn extracts_tagless_dff_properties_and_filename_title() {
        let path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pulse-engine/tests/fixtures/dsd-interleave.dff"
        ));

        let metadata = extract_metadata(path).unwrap();

        assert_eq!(metadata.title.as_deref(), Some("dsd-interleave"));
        assert_eq!(metadata.duration_ms, Some(1));
        assert_eq!(metadata.sample_rate_hz, Some(2_822_400));
        assert_eq!(metadata.bit_depth, Some(1));
        assert_eq!(metadata.channels, Some(2));
        assert!(metadata.artist.is_none());
    }

    #[test]
    fn refuses_dst_during_metadata_scan() {
        let path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pulse-engine/tests/fixtures/dst-refusal.dff"
        ));

        assert_eq!(
            extract_metadata(path).unwrap_err().to_string(),
            "unsupported audio format: DST-compressed DFF is not supported"
        );
    }

    fn write_test_dsf(path: &Path, sample_rate: u32, sample_count: u64, id3: &[u8]) {
        let data = vec![0; 8_192];
        let metadata_offset = 92 + data.len() as u64;
        let file_size = metadata_offset + id3.len() as u64;
        let mut dsf = Vec::with_capacity(file_size as usize);
        dsf.extend_from_slice(b"DSD ");
        dsf.extend_from_slice(&28_u64.to_le_bytes());
        dsf.extend_from_slice(&file_size.to_le_bytes());
        dsf.extend_from_slice(&metadata_offset.to_le_bytes());
        dsf.extend_from_slice(b"fmt ");
        dsf.extend_from_slice(&52_u64.to_le_bytes());
        for value in [1, 0, 2, 2, sample_rate, 1] {
            dsf.extend_from_slice(&value.to_le_bytes());
        }
        dsf.extend_from_slice(&sample_count.to_le_bytes());
        dsf.extend_from_slice(&4_096_u32.to_le_bytes());
        dsf.extend_from_slice(&0_u32.to_le_bytes());
        dsf.extend_from_slice(b"data");
        dsf.extend_from_slice(&(12 + data.len() as u64).to_le_bytes());
        dsf.extend_from_slice(&data);
        dsf.extend_from_slice(id3);
        std::fs::write(path, dsf).unwrap();
    }
}
