use super::{
    DOP_BITS_PER_FRAME, DSF_BLOCK_SIZE, DsdLayout, ParsedDsd, checked_add, checked_mul,
    decode_error, expect_bytes, read_u32_le, read_u64_le, u8_from_u32, validate_dsd_rate,
};
use crate::EngineError;
use std::{
    fs::File,
    io::{Seek, SeekFrom},
};

pub(super) fn parse_dsf(file: &mut File, file_len: u64) -> Result<ParsedDsd, EngineError> {
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
