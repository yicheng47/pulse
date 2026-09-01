use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use crate::{EngineError, PcmFormat};

const DSD64_RATE: u32 = 2_822_400;
const DSD128_RATE: u32 = 5_644_800;
const DSF_BLOCK_SIZE: u32 = 4_096;
const DFF_FRAMES_PER_READ: u64 = 8_192;
const DOP_BITS_PER_FRAME: u32 = 16;
const DOP_MARKER_1: u8 = 0x05;
const DOP_MARKER_2: u8 = 0xfa;

pub(crate) struct DsdDopDecoder {
    file: File,
    format: PcmFormat,
    layout: DsdLayout,
    total_frames: u64,
    frame_position: u64,
    next_marker: u8,
}

#[derive(Clone, Copy)]
enum DsdLayout {
    Dsf { data_offset: u64, block_size: u32 },
    Dff { data_offset: u64 },
}

struct ParsedDsd {
    sample_rate: u32,
    channels: u8,
    total_frames: u64,
    layout: DsdLayout,
}

impl DsdDopDecoder {
    pub(crate) fn open(path: &Path) -> Result<Self, EngineError> {
        let mut file = File::open(path)?;
        let file_len = file.metadata()?.len();
        let magic = read_array(&mut file, "DSD container ID")?;
        let parsed = match &magic {
            b"DSD " => parse_dsf(&mut file, file_len)?,
            b"FRM8" => parse_dff(&mut file, file_len)?,
            _ => {
                return decode_error("file is not a DSF or DFF container");
            }
        };

        let dop_rate = parsed.sample_rate / DOP_BITS_PER_FRAME;
        Ok(Self {
            file,
            format: PcmFormat {
                sample_rate: dop_rate,
                bits_per_sample: 24,
                channels: parsed.channels,
            },
            layout: parsed.layout,
            total_frames: parsed.total_frames,
            frame_position: 0,
            next_marker: DOP_MARKER_1,
        })
    }

    pub(crate) fn format(&self) -> PcmFormat {
        self.format
    }

    pub(crate) fn duration_ms(&self) -> u64 {
        frames_to_ms(self.total_frames, self.format.sample_rate)
    }

    pub(crate) fn seek(&mut self, position_ms: u64) -> Result<u64, EngineError> {
        let requested_frame = ms_to_frames_ceil(position_ms, self.format.sample_rate);
        self.frame_position = requested_frame.min(self.total_frames);
        self.next_marker = DOP_MARKER_1;
        Ok(frames_to_ms(self.frame_position, self.format.sample_rate))
    }

    pub(crate) fn next_pcm(&mut self, pcm: &mut Vec<u8>) -> Result<Option<u64>, EngineError> {
        if self.frame_position == self.total_frames {
            return Ok(None);
        }

        let remaining = self.total_frames - self.frame_position;
        let frame_count = match self.layout {
            DsdLayout::Dsf { block_size, .. } => {
                let byte_position = self.frame_position * 2;
                let bytes_left_in_block =
                    u64::from(block_size) - byte_position % u64::from(block_size);
                remaining.min(bytes_left_in_block / 2)
            }
            DsdLayout::Dff { .. } => remaining.min(DFF_FRAMES_PER_READ),
        };

        match self.layout {
            DsdLayout::Dsf {
                data_offset,
                block_size,
            } => self.read_dsf(data_offset, block_size, frame_count, pcm)?,
            DsdLayout::Dff { data_offset } => self.read_dff(data_offset, frame_count, pcm)?,
        }

        self.frame_position += frame_count;
        Ok(Some(frame_count))
    }

    fn read_dsf(
        &mut self,
        data_offset: u64,
        block_size: u32,
        frame_count: u64,
        pcm: &mut Vec<u8>,
    ) -> Result<(), EngineError> {
        let channels = u64::from(self.format.channels);
        let byte_position = self.frame_position * 2;
        let block_index = byte_position / u64::from(block_size);
        let byte_offset = byte_position % u64::from(block_size);
        let bytes_per_channel = usize_from_u64(frame_count * 2, "DSF read size")?;
        let block_group_offset = checked_add(
            data_offset,
            checked_mul(
                block_index,
                checked_mul(u64::from(block_size), channels, "DSF block group size")?,
                "DSF block offset",
            )?,
            "DSF data offset",
        )?;

        let mut channel_data = Vec::with_capacity(usize::from(self.format.channels));
        for channel in 0..channels {
            let channel_offset = checked_add(
                block_group_offset,
                checked_add(
                    checked_mul(channel, u64::from(block_size), "DSF channel offset")?,
                    byte_offset,
                    "DSF byte offset",
                )?,
                "DSF channel data offset",
            )?;
            self.file.seek(SeekFrom::Start(channel_offset))?;
            let mut data = vec![0; bytes_per_channel];
            read_exact(&mut self.file, &mut data, "DSF channel data")?;
            channel_data.push(data);
        }

        pcm.clear();
        pcm.reserve(pcm_capacity(frame_count, self.format.channels)?);
        for frame in 0..usize_from_u64(frame_count, "DSF frame count")? {
            let marker = self.take_marker();
            for channel in &channel_data {
                let earlier = reverse_bits(channel[frame * 2]);
                let later = reverse_bits(channel[frame * 2 + 1]);
                pcm.extend_from_slice(&[later, earlier, marker]);
            }
        }
        Ok(())
    }

    fn read_dff(
        &mut self,
        data_offset: u64,
        frame_count: u64,
        pcm: &mut Vec<u8>,
    ) -> Result<(), EngineError> {
        let channels = u64::from(self.format.channels);
        let bytes_per_frame = checked_mul(channels, 2, "DFF bytes per DoP frame")?;
        let source_offset = checked_add(
            data_offset,
            checked_mul(self.frame_position, bytes_per_frame, "DFF seek offset")?,
            "DFF data offset",
        )?;
        let source_len = usize_from_u64(
            checked_mul(frame_count, bytes_per_frame, "DFF read size")?,
            "DFF read size",
        )?;
        self.file.seek(SeekFrom::Start(source_offset))?;
        let mut data = vec![0; source_len];
        read_exact(&mut self.file, &mut data, "DFF sound data")?;

        let channel_count = usize::from(self.format.channels);
        pcm.clear();
        pcm.reserve(pcm_capacity(frame_count, self.format.channels)?);
        for frame in 0..usize_from_u64(frame_count, "DFF frame count")? {
            let marker = self.take_marker();
            let frame_offset = frame * channel_count * 2;
            for channel in 0..channel_count {
                let earlier = data[frame_offset + channel];
                let later = data[frame_offset + channel_count + channel];
                pcm.extend_from_slice(&[later, earlier, marker]);
            }
        }
        Ok(())
    }

    fn take_marker(&mut self) -> u8 {
        let marker = self.next_marker;
        self.next_marker = if marker == DOP_MARKER_1 {
            DOP_MARKER_2
        } else {
            DOP_MARKER_1
        };
        marker
    }
}

fn parse_dsf(file: &mut File, file_len: u64) -> Result<ParsedDsd, EngineError> {
    file.seek(SeekFrom::Start(0))?;
    expect_bytes(file, b"DSD ", "DSF file ID")?;
    let header_size = read_u64_le(file, "DSF header size")?;
    if header_size != 28 {
        return decode_error(format!(
            "DSF header size must be 28 bytes, found {header_size}"
        ));
    }
    let declared_file_size = read_u64_le(file, "DSF file size")?;
    if declared_file_size > file_len {
        return decode_error(format!(
            "DSF file is truncated: header declares {declared_file_size} bytes but file has {file_len}"
        ));
    }
    let _metadata_offset = read_u64_le(file, "DSF metadata offset")?;

    expect_bytes(file, b"fmt ", "DSF format chunk")?;
    let format_size = read_u64_le(file, "DSF format chunk size")?;
    if format_size != 52 {
        return decode_error(format!(
            "DSF format chunk size must be 52 bytes, found {format_size}"
        ));
    }
    let version = read_u32_le(file, "DSF format version")?;
    let format_id = read_u32_le(file, "DSF format ID")?;
    if version != 1 || format_id != 0 {
        return decode_error(format!(
            "unsupported DSF format version {version}, ID {format_id}"
        ));
    }
    let _channel_type = read_u32_le(file, "DSF channel type")?;
    let channels = u8_from_u32(read_u32_le(file, "DSF channel count")?, "DSF channel count")?;
    let sample_rate = read_u32_le(file, "DSF sample rate")?;
    validate_dsd_rate(sample_rate)?;
    let bits_per_sample = read_u32_le(file, "DSF bits per sample")?;
    if bits_per_sample == 8 {
        return decode_error("MSB-first DSF is not supported");
    }
    if bits_per_sample != 1 {
        return decode_error(format!(
            "unsupported DSF bits-per-sample value {bits_per_sample}"
        ));
    }
    let sample_count = read_u64_le(file, "DSF sample count")?;
    let block_size = read_u32_le(file, "DSF block size")?;
    if block_size != DSF_BLOCK_SIZE {
        return decode_error(format!(
            "DSF channel block size must be {DSF_BLOCK_SIZE} bytes, found {block_size}"
        ));
    }
    let _reserved = read_u32_le(file, "DSF reserved field")?;

    expect_bytes(file, b"data", "DSF data chunk")?;
    let data_chunk_size = read_u64_le(file, "DSF data chunk size")?;
    if data_chunk_size < 12 {
        return decode_error("DSF data chunk is shorter than its header");
    }
    let data_offset = file.stream_position()?;
    let data_size = data_chunk_size - 12;
    let data_end = checked_add(data_offset, data_size, "DSF data end")?;
    if data_end > file_len {
        return decode_error("DSF data chunk exceeds the file");
    }

    let bytes_per_channel = sample_count.div_ceil(8);
    let blocks_per_channel = bytes_per_channel.div_ceil(u64::from(block_size));
    let required_data_size = checked_mul(
        checked_mul(
            blocks_per_channel,
            u64::from(block_size),
            "DSF channel data size",
        )?,
        u64::from(channels),
        "DSF data size",
    )?;
    if data_size < required_data_size {
        return decode_error(format!(
            "DSF data chunk has {data_size} bytes but sample count requires {required_data_size}"
        ));
    }

    Ok(ParsedDsd {
        sample_rate,
        channels,
        total_frames: sample_count / u64::from(DOP_BITS_PER_FRAME),
        layout: DsdLayout::Dsf {
            data_offset,
            block_size,
        },
    })
}

fn parse_dff(file: &mut File, file_len: u64) -> Result<ParsedDsd, EngineError> {
    file.seek(SeekFrom::Start(0))?;
    expect_bytes(file, b"FRM8", "DFF file ID")?;
    let form_size = read_u64_be(file, "DFF FRM8 size")?;
    let form_end = checked_add(12, form_size, "DFF FRM8 end")?;
    if form_end != file_len {
        return decode_error(format!(
            "DFF file size is {file_len} bytes but FRM8 declares {form_end}"
        ));
    }
    if form_size < 4 {
        return decode_error("DFF FRM8 chunk is too short");
    }
    expect_bytes(file, b"DSD ", "DFF form type")?;

    let mut sample_rate = None;
    let mut channels = None;
    let mut compression = None;
    let mut data = None;
    while file.stream_position()? < form_end {
        let (chunk_id, chunk_size, data_offset, padded_end) =
            read_dff_chunk_header(file, form_end, "FRM8")?;
        match &chunk_id {
            b"PROP" => {
                let properties = parse_dff_properties(file, data_offset, chunk_size)?;
                sample_rate = properties.sample_rate;
                channels = properties.channels;
                compression = properties.compression;
            }
            b"DSD " => data = Some((data_offset, chunk_size)),
            b"DST " => return decode_error("DST-compressed DFF is not supported"),
            _ => {}
        }
        file.seek(SeekFrom::Start(padded_end))?;
    }

    let sample_rate = sample_rate
        .ok_or_else(|| EngineError::Decode("DFF is missing its FS chunk".to_string()))?;
    validate_dsd_rate(sample_rate)?;
    let channels =
        channels.ok_or_else(|| EngineError::Decode("DFF is missing its CHNL chunk".to_string()))?;
    let compression = compression
        .ok_or_else(|| EngineError::Decode("DFF is missing its CMPR chunk".to_string()))?;
    if compression == *b"DST " {
        return decode_error("DST-compressed DFF is not supported");
    }
    if compression != *b"DSD " {
        return decode_error(format!(
            "unsupported DFF compression {}",
            String::from_utf8_lossy(&compression)
        ));
    }
    let (data_offset, data_size) = data.ok_or_else(|| {
        EngineError::Decode("DFF is missing its DSD sound data chunk".to_string())
    })?;
    if data_size % u64::from(channels) != 0 {
        return decode_error("DFF sound data does not contain complete channel clusters");
    }

    Ok(ParsedDsd {
        sample_rate,
        channels,
        total_frames: data_size / u64::from(channels) / 2,
        layout: DsdLayout::Dff { data_offset },
    })
}

#[derive(Default)]
struct DffProperties {
    sample_rate: Option<u32>,
    channels: Option<u8>,
    compression: Option<[u8; 4]>,
}

fn parse_dff_properties(
    file: &mut File,
    data_offset: u64,
    size: u64,
) -> Result<DffProperties, EngineError> {
    if size < 4 {
        return decode_error("DFF PROP chunk is too short");
    }
    let end = checked_add(data_offset, size, "DFF PROP end")?;
    file.seek(SeekFrom::Start(data_offset))?;
    expect_bytes(file, b"SND ", "DFF PROP type")?;

    let mut properties = DffProperties::default();
    while file.stream_position()? < end {
        let (chunk_id, chunk_size, _chunk_data, padded_end) =
            read_dff_chunk_header(file, end, "PROP")?;
        match &chunk_id {
            b"FS  " => {
                if chunk_size != 4 {
                    return decode_error("DFF FS chunk must contain one 32-bit sample rate");
                }
                properties.sample_rate = Some(read_u32_be(file, "DFF sample rate")?);
            }
            b"CHNL" => {
                if chunk_size < 2 {
                    return decode_error("DFF CHNL chunk is too short");
                }
                let channel_count = read_u16_be(file, "DFF channel count")?;
                if chunk_size != 2 + u64::from(channel_count) * 4 {
                    return decode_error("DFF CHNL size does not match its channel count");
                }
                properties.channels = Some(u8_from_u16(channel_count, "DFF channel count")?);
            }
            b"CMPR" => {
                if chunk_size < 5 {
                    return decode_error("DFF CMPR chunk is too short");
                }
                let compression = read_array(file, "DFF compression type")?;
                let name_len = read_u8(file, "DFF compression name length")?;
                if u64::from(name_len) > chunk_size - 5 {
                    return decode_error("DFF compression name exceeds its chunk");
                }
                properties.compression = Some(compression);
            }
            _ => {}
        }
        file.seek(SeekFrom::Start(padded_end))?;
    }
    Ok(properties)
}

fn read_dff_chunk_header(
    file: &mut File,
    limit: u64,
    parent: &str,
) -> Result<([u8; 4], u64, u64, u64), EngineError> {
    let header_offset = file.stream_position()?;
    if checked_add(header_offset, 12, "DFF chunk header end")? > limit {
        return decode_error(format!("truncated DFF chunk header in {parent}"));
    }
    let chunk_id = read_array(file, "DFF chunk ID")?;
    let chunk_size = read_u64_be(file, "DFF chunk size")?;
    let data_offset = file.stream_position()?;
    let padded_end = checked_add(
        data_offset,
        checked_add(chunk_size, chunk_size & 1, "DFF padded chunk size")?,
        "DFF chunk end",
    )?;
    if padded_end > limit {
        return decode_error(format!(
            "DFF chunk {} exceeds its {parent} container",
            String::from_utf8_lossy(&chunk_id)
        ));
    }
    Ok((chunk_id, chunk_size, data_offset, padded_end))
}

fn validate_dsd_rate(sample_rate: u32) -> Result<(), EngineError> {
    if sample_rate != DSD64_RATE && sample_rate != DSD128_RATE {
        return decode_error(format!(
            "unsupported DSD sample rate {sample_rate}; only DSD64 and DSD128 are supported"
        ));
    }
    Ok(())
}

fn reverse_bits(byte: u8) -> u8 {
    const LUT: [u8; 256] = {
        let mut lut = [0; 256];
        let mut index = 0;
        while index < 256 {
            lut[index] = (index as u8).reverse_bits();
            index += 1;
        }
        lut
    };
    LUT[usize::from(byte)]
}

fn frames_to_ms(frames: u64, sample_rate: u32) -> u64 {
    u64::try_from(u128::from(frames) * 1_000 / u128::from(sample_rate)).unwrap_or(u64::MAX)
}

fn ms_to_frames_ceil(milliseconds: u64, sample_rate: u32) -> u64 {
    let frames = (u128::from(milliseconds) * u128::from(sample_rate)).div_ceil(1_000);
    u64::try_from(frames).unwrap_or(u64::MAX)
}

fn pcm_capacity(frames: u64, channels: u8) -> Result<usize, EngineError> {
    usize_from_u64(
        checked_mul(
            checked_mul(frames, u64::from(channels), "DoP sample count")?,
            3,
            "DoP byte count",
        )?,
        "DoP byte count",
    )
}

fn expect_bytes(file: &mut File, expected: &[u8], description: &str) -> Result<(), EngineError> {
    let mut actual = vec![0; expected.len()];
    read_exact(file, &mut actual, description)?;
    if actual != expected {
        return decode_error(format!("invalid {description}"));
    }
    Ok(())
}

fn read_u8(file: &mut File, description: &str) -> Result<u8, EngineError> {
    Ok(read_array::<1>(file, description)?[0])
}

fn read_u16_be(file: &mut File, description: &str) -> Result<u16, EngineError> {
    Ok(u16::from_be_bytes(read_array(file, description)?))
}

fn read_u32_le(file: &mut File, description: &str) -> Result<u32, EngineError> {
    Ok(u32::from_le_bytes(read_array(file, description)?))
}

fn read_u32_be(file: &mut File, description: &str) -> Result<u32, EngineError> {
    Ok(u32::from_be_bytes(read_array(file, description)?))
}

fn read_u64_le(file: &mut File, description: &str) -> Result<u64, EngineError> {
    Ok(u64::from_le_bytes(read_array(file, description)?))
}

fn read_u64_be(file: &mut File, description: &str) -> Result<u64, EngineError> {
    Ok(u64::from_be_bytes(read_array(file, description)?))
}

fn read_array<const N: usize>(file: &mut File, description: &str) -> Result<[u8; N], EngineError> {
    let mut bytes = [0; N];
    read_exact(file, &mut bytes, description)?;
    Ok(bytes)
}

fn read_exact(file: &mut File, bytes: &mut [u8], description: &str) -> Result<(), EngineError> {
    file.read_exact(bytes)
        .map_err(|error| EngineError::Decode(format!("truncated {description}: {error}")))
}

fn u8_from_u16(value: u16, description: &str) -> Result<u8, EngineError> {
    if value == 0 {
        return decode_error(format!("{description} must not be zero"));
    }
    u8::try_from(value).map_err(|_| EngineError::Decode(format!("{description} exceeds 255")))
}

fn u8_from_u32(value: u32, description: &str) -> Result<u8, EngineError> {
    if value == 0 {
        return decode_error(format!("{description} must not be zero"));
    }
    u8::try_from(value).map_err(|_| EngineError::Decode(format!("{description} exceeds 255")))
}

fn usize_from_u64(value: u64, description: &str) -> Result<usize, EngineError> {
    usize::try_from(value)
        .map_err(|_| EngineError::Decode(format!("{description} exceeds addressable memory")))
}

fn checked_add(left: u64, right: u64, description: &str) -> Result<u64, EngineError> {
    left.checked_add(right)
        .ok_or_else(|| EngineError::Decode(format!("{description} overflow")))
}

fn checked_mul(left: u64, right: u64, description: &str) -> Result<u64, EngineError> {
    left.checked_mul(right)
        .ok_or_else(|| EngineError::Decode(format!("{description} overflow")))
}

fn decode_error<T>(message: impl Into<String>) -> Result<T, EngineError> {
    Err(EngineError::Decode(message.into()))
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_TEST_FILE: AtomicU64 = AtomicU64::new(0);
    // Generated by script/generate_dsd_fixtures.py; the DFF expectation comes from dop_pack.py.
    const DSF: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/dsd-bit-reversal.dsf"
    );
    const DSF_DOP: &[u8] = include_bytes!("../tests/fixtures/dsd-bit-reversal.dop");
    const DFF: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/dsd-interleave.dff"
    );
    const DFF_DOP: &[u8] = include_bytes!("../tests/fixtures/dsd-interleave.dop");
    const DST_DFF: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/dst-refusal.dff"
    );

    #[test]
    fn dsf_reverses_lsb_first_channel_blocks() {
        let mut decoder = DsdDopDecoder::open(Path::new(DSF)).unwrap();
        assert_eq!(
            decoder.format(),
            PcmFormat {
                sample_rate: 176_400,
                bits_per_sample: 24,
                channels: 2,
            }
        );

        let mut pcm = Vec::new();
        assert_eq!(decoder.next_pcm(&mut pcm).unwrap(), Some(2));
        assert_eq!(pcm, DSF_DOP);
        assert_eq!(decoder.next_pcm(&mut pcm).unwrap(), None);
    }

    #[test]
    fn dsf_walks_block_groups_and_seeks_within_a_block() {
        let mut block_0_left = vec![0; DSF_BLOCK_SIZE as usize];
        let mut block_0_right = vec![0; DSF_BLOCK_SIZE as usize];
        let mut block_1_left = vec![0; DSF_BLOCK_SIZE as usize];
        let mut block_1_right = vec![0; DSF_BLOCK_SIZE as usize];
        let seek_frame = ms_to_frames_ceil(11, DSD64_RATE / DOP_BITS_PER_FRAME) as usize;
        let seek_byte = seek_frame * 2;
        block_0_left[seek_byte..seek_byte + 2].copy_from_slice(&[0x01, 0x02]);
        block_0_right[seek_byte..seek_byte + 2].copy_from_slice(&[0x04, 0x08]);
        block_1_left[..2].copy_from_slice(&[0x10, 0x20]);
        block_1_right[..2].copy_from_slice(&[0x40, 0x80]);
        let data = [block_0_left, block_0_right, block_1_left, block_1_right].concat();
        let file = TestDsf::new(DSD64_RATE, 1, u64::from(DSF_BLOCK_SIZE) * 16, &data);

        let mut decoder = DsdDopDecoder::open(file.path()).unwrap();
        let mut pcm = Vec::new();
        assert_eq!(decoder.next_pcm(&mut pcm).unwrap(), Some(2_048));
        assert_eq!(decoder.next_pcm(&mut pcm).unwrap(), Some(2_048));
        assert_eq!(&pcm[..6], &[0x04, 0x08, 0x05, 0x01, 0x02, 0x05]);

        let mut decoder = DsdDopDecoder::open(file.path()).unwrap();
        assert_eq!(decoder.seek(11).unwrap(), 11);
        assert_eq!(decoder.next_pcm(&mut pcm).unwrap(), Some(107));
        assert_eq!(&pcm[..6], &[0x40, 0x80, 0x05, 0x10, 0x20, 0x05]);
        assert_eq!(decoder.next_pcm(&mut pcm).unwrap(), Some(2_048));
    }

    #[test]
    fn dsf_refuses_msb_first_data() {
        let data = vec![0; DSF_BLOCK_SIZE as usize * 2];
        let file = TestDsf::new(DSD64_RATE, 8, 16, &data);

        let error = match DsdDopDecoder::open(file.path()) {
            Ok(_) => panic!("MSB-first DSF should be refused"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "decode: MSB-first DSF is not supported");
    }

    #[test]
    fn dsf_maps_dsd128_to_352k_dop() {
        let data = vec![0; DSF_BLOCK_SIZE as usize * 2];
        let file = TestDsf::new(DSD128_RATE, 1, 16, &data);

        assert_eq!(
            DsdDopDecoder::open(file.path()).unwrap().format(),
            PcmFormat {
                sample_rate: 352_800,
                bits_per_sample: 24,
                channels: 2,
            }
        );
    }

    #[test]
    fn dff_interleaves_channels_and_alternates_markers_byte_exactly() {
        let mut decoder = DsdDopDecoder::open(Path::new(DFF)).unwrap();
        let mut pcm = Vec::new();

        assert_eq!(decoder.next_pcm(&mut pcm).unwrap(), Some(200));
        assert_eq!(pcm, DFF_DOP);
        assert_eq!(decoder.next_pcm(&mut pcm).unwrap(), None);
    }

    #[test]
    fn seek_aligns_to_a_dop_frame_and_restarts_marker_phase() {
        let mut decoder = DsdDopDecoder::open(Path::new(DFF)).unwrap();
        let mut pcm = Vec::new();
        decoder.next_pcm(&mut pcm).unwrap();

        assert_eq!(decoder.seek(1).unwrap(), 1);
        assert_eq!(decoder.next_pcm(&mut pcm).unwrap(), Some(23));
        assert_eq!(&pcm[..6], &[0xc6, 0xc4, 0x05, 0xc7, 0xc5, 0x05]);

        assert_eq!(decoder.seek(0).unwrap(), 0);
        decoder.next_pcm(&mut pcm).unwrap();
        assert_eq!(
            &pcm[..12],
            &[2, 0, 0x05, 3, 1, 0x05, 6, 4, 0xfa, 7, 5, 0xfa]
        );
    }

    #[test]
    fn dst_compression_is_refused_clearly() {
        let error = match DsdDopDecoder::open(Path::new(DST_DFF)) {
            Ok(_) => panic!("DST compression should be refused"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "decode: DST-compressed DFF is not supported"
        );
    }

    struct TestDsf {
        path: PathBuf,
    }

    impl TestDsf {
        fn new(sample_rate: u32, bits_per_sample: u32, sample_count: u64, data: &[u8]) -> Self {
            let mut bytes = Vec::with_capacity(92 + data.len());
            let file_size = 92 + data.len() as u64;
            bytes.extend_from_slice(b"DSD ");
            bytes.extend_from_slice(&28_u64.to_le_bytes());
            bytes.extend_from_slice(&file_size.to_le_bytes());
            bytes.extend_from_slice(&0_u64.to_le_bytes());
            bytes.extend_from_slice(b"fmt ");
            bytes.extend_from_slice(&52_u64.to_le_bytes());
            for value in [1, 0, 2, 2, sample_rate, bits_per_sample] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes.extend_from_slice(&sample_count.to_le_bytes());
            bytes.extend_from_slice(&DSF_BLOCK_SIZE.to_le_bytes());
            bytes.extend_from_slice(&0_u32.to_le_bytes());
            bytes.extend_from_slice(b"data");
            bytes.extend_from_slice(&(12 + data.len() as u64).to_le_bytes());
            bytes.extend_from_slice(data);

            let sequence = NEXT_TEST_FILE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "pulse-engine-dsd-{}-{sequence}.dsf",
                std::process::id()
            ));
            std::fs::write(&path, bytes).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDsf {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
