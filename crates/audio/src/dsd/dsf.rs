//! Sony DSF (Direct Stream Digital File) parser and writer.
//!
//! A DSF file consists of three contiguous little-endian chunks:
//! - `'DSD '` chunk (28 bytes): file identification, total size, and metadata offset.
//! - `'fmt '` chunk (52 bytes): sample rate, channel count, sample count, block size.
//! - `'data'` chunk (12 bytes + data): 1-bit audio organized into channel blocks.
//! - Optional ID3v2 metadata chunk located at `metadata_offset`.

use super::{BitOrder, Error, Format, MAX_BLOCK_SIZE, MAX_CHANNELS, MAX_DECODE_BYTES, MAX_SAMPLE_RATE, MIN_SAMPLE_RATE, Metadata, RawAudio};

/// Channel configuration names for DSF channel types 1..=7.
fn dsf_channel_names(channel_type: u32, channel_count: u16) -> Vec<String> {
    match channel_type {
        1 => vec!["Mono".to_string()],
        2 => vec!["Left".to_string(), "Right".to_string()],
        3 => vec!["Left".to_string(), "Right".to_string(), "Center".to_string()],
        4 => vec!["FrontLeft".to_string(), "FrontRight".to_string(), "BackLeft".to_string(), "BackRight".to_string()],
        5 => vec!["FrontLeft".to_string(), "FrontRight".to_string(), "Center".to_string(), "LFE".to_string()],
        6 => vec!["FrontLeft".to_string(), "FrontRight".to_string(), "Center".to_string(), "BackLeft".to_string(), "BackRight".to_string()],
        7 => vec![
            "FrontLeft".to_string(),
            "FrontRight".to_string(),
            "Center".to_string(),
            "LFE".to_string(),
            "BackLeft".to_string(),
            "BackRight".to_string(),
        ],
        _ => (0..channel_count).map(|c| format!("Channel {}", c + 1)).collect(),
    }
}

/// Attempts to parse basic ID3v2 title and artist tags if present at `offset`.
fn parse_id3v2(bytes: &[u8], offset: usize) -> (Option<String>, Option<String>) {
    let Some(id3) = bytes.get(offset..) else {
        return (None, None);
    };
    if id3.len() < 10 || &id3[..3] != b"ID3" {
        return (None, None);
    }

    let mut title = None;
    let mut artist = None;

    // ID3v2 size is 4 synchsafe bytes at offset 6.
    let tag_size = ((id3[6] as usize & 0x7F) << 21) | ((id3[7] as usize & 0x7F) << 14) | ((id3[8] as usize & 0x7F) << 7) | (id3[9] as usize & 0x7F);

    let end = (10 + tag_size).min(id3.len());
    let mut pos = 10;

    // Scan ID3v2.3 / ID3v2.4 frames.
    while pos + 10 <= end {
        let frame_id = &id3[pos..pos + 4];
        if frame_id[0] == 0 {
            break;
        }
        let raw = [id3[pos + 4], id3[pos + 5], id3[pos + 6], id3[pos + 7]];
        // ID3v2.4 frame sizes are synchsafe; v2.3 ones are plain big-endian.
        let frame_size = if id3[3] >= 4 {
            if raw.iter().any(|b| b & 0x80 != 0) {
                break;
            }
            ((raw[0] as usize) << 21) | ((raw[1] as usize) << 14) | ((raw[2] as usize) << 7) | raw[3] as usize
        } else {
            u32::from_be_bytes(raw) as usize
        };
        pos += 10;
        if frame_size > end.saturating_sub(pos) {
            break;
        }

        let frame_data = &id3[pos..pos + frame_size];
        pos += frame_size;

        if frame_id == b"TIT2" && title.is_none() {
            title = parse_text_frame(frame_data);
        } else if frame_id == b"TPE1" && artist.is_none() {
            artist = parse_text_frame(frame_data);
        }
    }

    (title, artist)
}

fn parse_text_frame(data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return None;
    }
    let encoding = data[0];
    let content = &data[1..];
    let text = match encoding {
        // ISO-8859-1: every byte is the code point of the same value.
        0 => content.iter().map(|&b| char::from(b)).collect::<String>(),
        3 => String::from_utf8_lossy(content).into_owned(),
        // UTF-16 with a byte order mark (which is removed); without one, assume big-endian.
        1 => {
            let (big_endian, body) = match content {
                [0xFE, 0xFF, rest @ ..] => (true, rest),
                [0xFF, 0xFE, rest @ ..] => (false, rest),
                _ => (true, content),
            };
            utf16(body, big_endian)
        }
        // UTF-16BE without a byte order mark.
        2 => utf16(content, true),
        _ => return None,
    };
    let s = text.trim_end_matches('\0').trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn utf16(bytes: &[u8], big_endian: bool) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| if big_endian { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
        .collect();
    String::from_utf16_lossy(&units)
}

/// Parses DSF file metadata, returning the metadata, audio data offset, and block size per channel.
pub fn parse_dsf_header(bytes: &[u8]) -> Result<(Metadata, usize, u32), Error> {
    let (metadata, offset, _, block_size) = parse_dsf_header_bounded(bytes)?;
    Ok((metadata, offset, block_size))
}

fn parse_dsf_header_bounded(bytes: &[u8]) -> Result<(Metadata, usize, usize, u32), Error> {
    if bytes.len() < 28 + 52 + 12 {
        return Err(Error::UnexpectedEof("DSF file header is truncated"));
    }

    // 1. 'DSD ' chunk
    if &bytes[..4] != b"DSD " {
        return Err(Error::InvalidHeader("missing 'DSD ' magic header"));
    }
    let dsd_chunk_size = u64::from_le_bytes(bytes[4..12].try_into().map_err(|_| Error::UnexpectedEof("DSD chunk size"))?);
    if dsd_chunk_size != 28 {
        return Err(Error::InvalidHeader("invalid 'DSD ' chunk size (must be 28)"));
    }
    let file_size = u64::from_le_bytes(bytes[12..20].try_into().map_err(|_| Error::UnexpectedEof("file size"))?);
    let metadata_offset = u64::from_le_bytes(bytes[20..28].try_into().map_err(|_| Error::UnexpectedEof("metadata offset"))?);

    if file_size > bytes.len() as u64 {
        return Err(Error::UnexpectedEof("declared file size extends beyond buffer"));
    }
    let file_len = usize::try_from(file_size).map_err(|_| Error::MemoryLimitExceeded)?;
    if file_len < 28 + 52 + 12 {
        return Err(Error::InvalidHeader("declared file size is smaller than required headers"));
    }
    let file_bytes = &bytes[..file_len];

    // 2. 'fmt ' chunk
    let fmt_offset = 28;
    if &file_bytes[fmt_offset..fmt_offset + 4] != b"fmt " {
        return Err(Error::InvalidHeader("missing 'fmt ' chunk header"));
    }
    let fmt_chunk_size =
        u64::from_le_bytes(file_bytes[fmt_offset + 4..fmt_offset + 12].try_into().map_err(|_| Error::UnexpectedEof("fmt chunk size"))?);
    if fmt_chunk_size < 52 {
        return Err(Error::InvalidHeader("invalid 'fmt ' chunk size (must be at least 52)"));
    }
    let fmt_size = usize::try_from(fmt_chunk_size).map_err(|_| Error::MemoryLimitExceeded)?;
    if fmt_size > file_bytes.len().saturating_sub(fmt_offset) {
        return Err(Error::UnexpectedEof("fmt chunk extends beyond file bounds"));
    }

    let format_version =
        u32::from_le_bytes(file_bytes[fmt_offset + 12..fmt_offset + 16].try_into().map_err(|_| Error::UnexpectedEof("format version"))?);
    if format_version != 1 {
        return Err(Error::Unsupported("unsupported DSF format version (only version 1 is supported)"));
    }

    let format_id = u32::from_le_bytes(file_bytes[fmt_offset + 16..fmt_offset + 20].try_into().map_err(|_| Error::UnexpectedEof("format id"))?);
    if format_id != 0 {
        return Err(Error::Unsupported("unsupported DSF format id (only raw DSD id=0 is supported)"));
    }

    let channel_type = u32::from_le_bytes(file_bytes[fmt_offset + 20..fmt_offset + 24].try_into().map_err(|_| Error::UnexpectedEof("channel type"))?);
    let channel_count =
        u32::from_le_bytes(file_bytes[fmt_offset + 24..fmt_offset + 28].try_into().map_err(|_| Error::UnexpectedEof("channel count"))?);
    if channel_count == 0 || channel_count > MAX_CHANNELS as u32 {
        return Err(Error::Unsupported("DSF channel count must be between 1 and 8"));
    }
    let channels = channel_count as u16;

    let sample_rate =
        u32::from_le_bytes(file_bytes[fmt_offset + 28..fmt_offset + 32].try_into().map_err(|_| Error::UnexpectedEof("sampling frequency"))?);
    if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate) {
        return Err(Error::InvalidHeader("DSF sampling frequency out of valid range"));
    }

    let bits_per_sample =
        u32::from_le_bytes(file_bytes[fmt_offset + 32..fmt_offset + 36].try_into().map_err(|_| Error::UnexpectedEof("bits per sample"))?);
    if bits_per_sample != 1 {
        return Err(Error::Unsupported("DSF bits per sample must be 1"));
    }

    let sample_count = u64::from_le_bytes(file_bytes[fmt_offset + 36..fmt_offset + 44].try_into().map_err(|_| Error::UnexpectedEof("sample count"))?);

    let block_size_per_channel =
        u32::from_le_bytes(file_bytes[fmt_offset + 44..fmt_offset + 48].try_into().map_err(|_| Error::UnexpectedEof("block size per channel"))?);
    if block_size_per_channel == 0 || block_size_per_channel > MAX_BLOCK_SIZE {
        return Err(Error::InvalidHeader("DSF block size per channel is invalid or too large"));
    }

    // 3. 'data' chunk
    let data_offset = fmt_offset.saturating_add(fmt_size);
    if data_offset.saturating_add(12) > file_bytes.len() {
        return Err(Error::UnexpectedEof("missing 'data' chunk header"));
    }
    if &file_bytes[data_offset..data_offset + 4] != b"data" {
        return Err(Error::InvalidHeader("missing 'data' chunk header"));
    }
    let data_chunk_size =
        u64::from_le_bytes(file_bytes[data_offset + 4..data_offset + 12].try_into().map_err(|_| Error::UnexpectedEof("data chunk size"))?);
    if data_chunk_size < 12 {
        return Err(Error::InvalidHeader("invalid 'data' chunk size (must be at least 12)"));
    }

    let payload_offset = data_offset + 12;
    let payload_size = usize::try_from(data_chunk_size - 12).map_err(|_| Error::MemoryLimitExceeded)?;
    if payload_size > file_bytes.len().saturating_sub(payload_offset) {
        return Err(Error::UnexpectedEof("data chunk payload extends beyond file bounds"));
    }
    let data_end = payload_offset.saturating_add(payload_size);

    // Parse ID3v2 metadata if pointer is valid.
    let (title, artist) = if metadata_offset == 0 {
        (None, None)
    } else {
        let meta_offset = usize::try_from(metadata_offset).map_err(|_| Error::MemoryLimitExceeded)?;
        if meta_offset < data_end || meta_offset >= file_len {
            return Err(Error::InvalidHeader("DSF metadata offset outside declared file bounds"));
        }
        parse_id3v2(file_bytes, meta_offset)
    };

    let channel_names = dsf_channel_names(channel_type, channels);

    let metadata = Metadata { format: Format::Dsf, sample_rate, channels, sample_count, channel_names, title, artist };

    Ok((metadata, payload_offset, payload_size, block_size_per_channel))
}

/// Parses a DSF file into raw 1-bit planar channel streams.
pub fn parse_dsf(bytes: &[u8]) -> Result<RawAudio, Error> {
    let (metadata, payload_offset, payload_size, block_size) = parse_dsf_header_bounded(bytes)?;
    let block_size = block_size as usize;
    let channels = metadata.channels as usize;

    let target_bytes_per_channel = usize::try_from(metadata.sample_count.div_ceil(8)).map_err(|_| Error::MemoryLimitExceeded)?;
    if target_bytes_per_channel.saturating_mul(channels) > MAX_DECODE_BYTES {
        return Err(Error::MemoryLimitExceeded);
    }

    let slice_size = channels.saturating_mul(block_size);
    if slice_size == 0 {
        return Err(Error::InvalidHeader("zero slice size"));
    }

    // The header validated this range. Trailing ID3 or other bytes are not audio,
    // even if sample_count asks for more samples than the data chunk contains.
    let payload = &bytes[payload_offset..payload_offset + payload_size];
    let num_slices = payload.len() / slice_size;

    let mut channel_bytes: Vec<Vec<u8>> = Vec::with_capacity(channels);
    for _ in 0..channels {
        let mut ch_vec = Vec::new();
        ch_vec.try_reserve_exact(target_bytes_per_channel).map_err(|_| Error::MemoryLimitExceeded)?;
        channel_bytes.push(ch_vec);
    }

    for s in 0..num_slices {
        let slice_base = s * slice_size;
        for (c, ch_vec) in channel_bytes.iter_mut().enumerate() {
            let ch_start = slice_base + c * block_size;
            let ch_end = ch_start + block_size;
            if let Some(block) = payload.get(ch_start..ch_end) {
                let remaining = target_bytes_per_channel.saturating_sub(ch_vec.len());
                let take = block.len().min(remaining);
                ch_vec.extend_from_slice(&block[..take]);
            }
        }
    }

    // Handle any trailing partial slice.
    let remainder = payload.len() % slice_size;
    if remainder > 0 {
        let slice_base = num_slices * slice_size;
        for (c, ch_vec) in channel_bytes.iter_mut().enumerate() {
            let ch_start = slice_base + c * block_size;
            if ch_start < payload.len() {
                let ch_end = (ch_start + block_size).min(payload.len());
                let block = &payload[ch_start..ch_end];
                let remaining = target_bytes_per_channel.saturating_sub(ch_vec.len());
                let take = block.len().min(remaining);
                ch_vec.extend_from_slice(&block[..take]);
            }
        }
    }

    // Pad with silence (0x69) if the file had fewer bytes than required by sample_count.
    for ch_vec in &mut channel_bytes {
        if ch_vec.len() < target_bytes_per_channel {
            ch_vec.resize(target_bytes_per_channel, super::filter::SILENCE_LSBF);
        }
    }

    Ok(RawAudio { metadata, bit_order: BitOrder::LsbFirst, channel_bytes })
}

fn unsupported_channel_layout() -> Result<u32, Error> {
    Err(Error::Unsupported("DSF has no channel type for this channel count (1 to 6 are supported)"))
}

/// Encodes 1-bit planar audio channels into a valid Sony DSF file.
pub fn write_dsf(sample_rate: u32, channels: &[&[u8]], sample_count: u64, block_size: u32) -> Result<Vec<u8>, Error> {
    if channels.is_empty() || channels.len() > MAX_CHANNELS {
        return Err(Error::Unsupported("channel count must be between 1 and 8"));
    }
    if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate) {
        return Err(Error::InvalidData("sample rate out of bounds"));
    }
    if block_size > MAX_BLOCK_SIZE {
        return Err(Error::InvalidData("DSF block size exceeds maximum"));
    }
    let block_size = if block_size == 0 { 4096 } else { block_size as usize };

    let num_channels = channels.len();
    let bytes_per_ch = usize::try_from(sample_count.div_ceil(8)).map_err(|_| Error::MemoryLimitExceeded)?;
    let num_blocks = bytes_per_ch.div_ceil(block_size);
    let payload_size = num_blocks.checked_mul(num_channels).and_then(|n| n.checked_mul(block_size)).ok_or(Error::MemoryLimitExceeded)?;
    let file_size = super::encoded_size(payload_size, 92)?;

    let data_chunk_size = 12u64 + payload_size as u64;

    let mut out = Vec::new();
    out.try_reserve_exact(file_size).map_err(|_| Error::MemoryLimitExceeded)?;

    // 1. 'DSD ' chunk (28 bytes)
    out.extend_from_slice(b"DSD ");
    out.extend_from_slice(&28u64.to_le_bytes());
    out.extend_from_slice(&(file_size as u64).to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes()); // metadata offset = 0

    // 2. 'fmt ' chunk (52 bytes)
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&52u64.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes()); // format_version = 1
    out.extend_from_slice(&0u32.to_le_bytes()); // format_id = 0 (raw DSD)
    // DSF channel types: 1 mono, 2 stereo, 3 L R C, 4 quad, 6 five channels, 7 5.1.
    let channel_type = match num_channels {
        1 => 1u32,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 6,
        6 => 7,
        _ => unsupported_channel_layout()?,
    };
    out.extend_from_slice(&channel_type.to_le_bytes());
    out.extend_from_slice(&(num_channels as u32).to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes()); // bits_per_sample = 1
    out.extend_from_slice(&sample_count.to_le_bytes());
    out.extend_from_slice(&(block_size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved

    // 3. 'data' chunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_chunk_size.to_le_bytes());

    for b in 0..num_blocks {
        let blk_start = b * block_size;
        let blk_end = blk_start + block_size;
        for ch_data in channels {
            let available = if blk_start < ch_data.len() {
                let end = blk_end.min(ch_data.len());
                &ch_data[blk_start..end]
            } else {
                &[]
            };
            out.extend_from_slice(available);
            if available.len() < block_size {
                out.resize(out.len() + (block_size - available.len()), super::filter::SILENCE_LSBF);
            }
        }
    }

    Ok(out)
}
