use super::{
    DsdLayout, ParsedDsd, checked_add, decode_error, expect_bytes, read_array, read_u8,
    read_u16_be, read_u32_be, read_u64_be, u8_from_u16, validate_dsd_rate,
};
use crate::EngineError;
use std::{
    fs::File,
    io::{Seek, SeekFrom},
};

pub(super) fn parse_dff(file: &mut File, file_len: u64) -> Result<ParsedDsd, EngineError> {
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
