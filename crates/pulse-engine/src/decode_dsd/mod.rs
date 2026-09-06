mod dff;
mod dsf;

use dff::parse_dff;
use dsf::parse_dsf;

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
    const DSF_DOP: &[u8] = include_bytes!("../../tests/fixtures/dsd-bit-reversal.dop");
    const DFF: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/dsd-interleave.dff"
    );
    const DFF_DOP: &[u8] = include_bytes!("../../tests/fixtures/dsd-interleave.dop");
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
