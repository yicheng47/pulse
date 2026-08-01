use std::{
    fs::{self, File},
    io,
    io::BufReader,
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
pub(super) struct AudioMetadata {
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
pub(super) struct EmbeddedArtwork {
    pub data: Vec<u8>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Error)]
pub(super) enum MetadataError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Lofty(#[from] lofty::error::LoftyError),
    #[error("unsupported audio format: {0}")]
    Unsupported(String),
    #[error("audio duration is too large to store")]
    DurationOutOfRange,
}

pub(super) fn extract_metadata(path: &Path) -> Result<AudioMetadata, MetadataError> {
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

pub(super) fn folder_artwork(path: &Path) -> io::Result<Option<EmbeddedArtwork>> {
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
pub(super) fn write_test_wav(
    path: &Path,
    title: &str,
    artist: &str,
    album: &str,
) -> std::io::Result<()> {
    write_test_wav_with_format(path, title, artist, album, 1)
}

#[cfg(test)]
pub(super) fn write_test_wav_with_format(
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
pub(super) fn write_test_wav_with_artwork(
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

#[cfg(test)]
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
}
