//! Philips DSDIFF (Direct Stream Digital Interchange File Format, `.dff`) parser and writer.
//!
//! DSDIFF files use an IFF-style big-endian chunk format starting with `FRM8` of type `DSD `:
//! - `FRM8` container chunk containing:
//!   - `FVER`: format version (typically 1.5.0.0).
//!   - `PROP`: properties chunk with subtype `SND `, containing:
//!     - `FS  `: sampling frequency (e.g. 2,822,400 Hz for DSD64).
//!     - `CHNL`: channel count and 4-character channel ID codes (`SLFT`, `SRGT`, etc.).
//!     - `CMPR`: compression type (`DSD ` for uncompressed, `DST ` for compressed).
//!   - `DSD `: audio data chunk containing channel-interleaved 1-bit samples (MSB-first).
//!   - Optional `DIIN`: disc and track information (title, artist).

use super::{BitOrder, Error, Format, MAX_CHANNELS, MAX_DECODE_BYTES, MAX_SAMPLE_RATE, MIN_SAMPLE_RATE, Metadata, RawAudio};

/// Decodes 4-byte ASCII channel identifiers into human-readable channel names.
fn channel_id_to_name(id: &[u8; 4]) -> String {
    match id {
        b"SLFT" => "FrontLeft".to_string(),
        b"SRGT" => "FrontRight".to_string(),
        b"MLFT" => "Mono/Left".to_string(),
        b"MRGT" => "Right".to_string(),
        b"C   " => "Center".to_string(),
        b"LFE " => "LFE".to_string(),
        b"LS  " => "LeftSurround".to_string(),
        b"RS  " => "RightSurround".to_string(),
        b"CO  " => "CenterOverhead".to_string(),
        b"LH  " => "LeftHeight".to_string(),
        b"RH  " => "RightHeight".to_string(),
        _ => {
            if let Ok(s) = std::str::from_utf8(id) {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
            format!("Channel {:02X}{:02X}{:02X}{:02X}", id[0], id[1], id[2], id[3])
        }
    }
}

/// The text of a `DITI` or `DIAR` chunk: a four-byte big-endian character count, then that many
/// bytes. `None` when the count is missing or longer than the chunk.
fn diin_text(sub_data: &[u8]) -> Option<String> {
    let count = usize::try_from(u32::from_be_bytes(sub_data.get(..4)?.try_into().ok()?)).ok()?;
    let text = sub_data.get(4..)?.get(..count)?;
    Some(String::from_utf8_lossy(text).trim().to_string())
}

/// The channel identifiers DSDIFF 1.5 defines for a channel count (stereo, 5 and 5.1 channels);
/// other counts use the generic `C001`, `C002`, ... identifiers.
fn channel_ids(num_channels: usize) -> Vec<[u8; 4]> {
    let named: &[&[u8; 4]] = match num_channels {
        1 => &[b"SLFT"],
        2 => &[b"SLFT", b"SRGT"],
        5 => &[b"MLFT", b"MRGT", b"C   ", b"LS  ", b"RS  "],
        6 => &[b"MLFT", b"MRGT", b"C   ", b"LFE ", b"LS  ", b"RS  "],
        _ => &[],
    };
    if !named.is_empty() {
        return named.iter().map(|id| **id).collect();
    }
    (1..=num_channels)
        .map(|n| {
            let s = format!("C{n:03}");
            let mut id = [b' '; 4];
            id.copy_from_slice(&s.as_bytes()[..4]);
            id
        })
        .collect()
}

/// Parses the metadata from a DSDIFF file.
pub fn parse_dsdiff_header(bytes: &[u8]) -> Result<(Metadata, usize, usize), Error> {
    if bytes.len() < 16 {
        return Err(Error::UnexpectedEof("DSDIFF file is too short"));
    }

    if &bytes[0..4] != b"FRM8" {
        return Err(Error::InvalidHeader("missing 'FRM8' magic chunk"));
    }
    let form_size = u64::from_be_bytes(bytes[4..12].try_into().map_err(|_| Error::UnexpectedEof("FRM8 size"))?);
    if &bytes[12..16] != b"DSD " {
        return Err(Error::InvalidHeader("invalid FRM8 form type (expected 'DSD ')"));
    }

    let total_file_data = bytes.len().saturating_sub(12);
    let form_size = usize::try_from(form_size).map_err(|_| Error::MemoryLimitExceeded)?;
    if form_size > total_file_data {
        return Err(Error::UnexpectedEof("FRM8 chunk extends beyond file size"));
    }

    let mut pos = 16;
    let end = 12 + form_size;

    let mut sample_rate = None;
    let mut channel_names = Vec::new();
    let mut num_channels = None;
    let mut is_compressed_dst = false;
    let mut dsd_data_offset = None;
    let mut dsd_data_len = None;
    let mut title = None;
    let mut artist = None;

    while pos + 12 <= end {
        let chunk_id: [u8; 4] = bytes[pos..pos + 4].try_into().map_err(|_| Error::UnexpectedEof("chunk ID"))?;
        let chunk_size = usize::try_from(u64::from_be_bytes(bytes[pos + 4..pos + 12].try_into().map_err(|_| Error::UnexpectedEof("chunk size"))?))
            .map_err(|_| Error::MemoryLimitExceeded)?;
        pos += 12;

        if chunk_size > end.saturating_sub(pos) {
            return Err(Error::UnexpectedEof("subchunk extends beyond container bounds"));
        }

        let chunk_data = &bytes[pos..pos + chunk_size];
        let pad = chunk_size & 1;
        pos = pos.saturating_add(chunk_size).saturating_add(pad);

        match &chunk_id {
            b"PROP" => {
                if chunk_data.len() < 4 {
                    continue;
                }
                if &chunk_data[0..4] != b"SND " {
                    continue;
                }
                let mut prop_pos = 4;
                while prop_pos + 12 <= chunk_data.len() {
                    let sub_id: [u8; 4] = chunk_data[prop_pos..prop_pos + 4].try_into().map_err(|_| Error::UnexpectedEof("PROP subchunk ID"))?;
                    let sub_size = usize::try_from(u64::from_be_bytes(
                        chunk_data[prop_pos + 4..prop_pos + 12].try_into().map_err(|_| Error::UnexpectedEof("PROP subchunk size"))?,
                    ))
                    .map_err(|_| Error::MemoryLimitExceeded)?;
                    prop_pos += 12;

                    if sub_size > chunk_data.len().saturating_sub(prop_pos) {
                        return Err(Error::UnexpectedEof("PROP subchunk extends beyond bounds"));
                    }

                    let sub_data = &chunk_data[prop_pos..prop_pos + sub_size];
                    let sub_pad = sub_size & 1;
                    prop_pos = prop_pos.saturating_add(sub_size).saturating_add(sub_pad);

                    match &sub_id {
                        b"FS  " => {
                            if sub_data.len() < 4 {
                                return Err(Error::UnexpectedEof("truncated FS chunk"));
                            }
                            if sample_rate.is_some() {
                                return Err(Error::InvalidHeader("duplicate FS chunk"));
                            }
                            let sr = u32::from_be_bytes([sub_data[0], sub_data[1], sub_data[2], sub_data[3]]);
                            if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sr) {
                                return Err(Error::InvalidHeader("sampling frequency out of range"));
                            }
                            sample_rate = Some(sr);
                        }
                        b"CHNL" => {
                            if sub_data.len() < 2 {
                                return Err(Error::UnexpectedEof("truncated CHNL chunk"));
                            }
                            if num_channels.is_some() {
                                return Err(Error::InvalidHeader("duplicate CHNL chunk"));
                            }
                            let n = u16::from_be_bytes([sub_data[0], sub_data[1]]);
                            if n == 0 || n > MAX_CHANNELS as u16 {
                                return Err(Error::Unsupported("channel count must be between 1 and 8"));
                            }
                            num_channels = Some(n);
                            channel_names.clear();
                            let mut ch_pos = 2;
                            while ch_pos + 4 <= sub_data.len() && channel_names.len() < n as usize {
                                let id: [u8; 4] = [sub_data[ch_pos], sub_data[ch_pos + 1], sub_data[ch_pos + 2], sub_data[ch_pos + 3]];
                                channel_names.push(channel_id_to_name(&id));
                                ch_pos += 4;
                            }
                        }
                        b"CMPR" if sub_data.len() >= 4 => {
                            if &sub_data[0..4] == b"DST " {
                                is_compressed_dst = true;
                            } else if &sub_data[0..4] != b"DSD " {
                                return Err(Error::Unsupported("unknown DSDIFF compression type"));
                            }
                        }
                        _ => {}
                    }
                }
            }
            b"DSD " => {
                dsd_data_offset = Some(pos - chunk_size - pad);
                dsd_data_len = Some(chunk_size);
            }
            b"DST " => {
                is_compressed_dst = true;
            }
            b"DIIN" => {
                // Parse optional disc info for title and artist.
                let mut diin_pos = 0;
                while diin_pos + 12 <= chunk_data.len() {
                    let sub_id = &chunk_data[diin_pos..diin_pos + 4];
                    let sub_size = usize::try_from(u64::from_be_bytes(
                        chunk_data[diin_pos + 4..diin_pos + 12].try_into().map_err(|_| Error::UnexpectedEof("DIIN subchunk size"))?,
                    ))
                    .map_err(|_| Error::MemoryLimitExceeded)?;
                    diin_pos += 12;
                    if sub_size > chunk_data.len().saturating_sub(diin_pos) {
                        break;
                    }
                    let sub_data = &chunk_data[diin_pos..diin_pos + sub_size];
                    let sub_pad = sub_size & 1;
                    diin_pos = diin_pos.saturating_add(sub_size).saturating_add(sub_pad);

                    if sub_id == b"DITI" && title.is_none() {
                        title = diin_text(sub_data);
                    } else if sub_id == b"DIAR" && artist.is_none() {
                        artist = diin_text(sub_data);
                    }
                }
            }
            _ => {}
        }
    }

    if is_compressed_dst {
        return Err(Error::Unsupported("DST compression in DSDIFF is not yet supported"));
    }

    let sample_rate = sample_rate.ok_or(Error::InvalidHeader("missing sample rate ('FS  ' chunk)"))?;
    if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate) {
        return Err(Error::InvalidHeader("sampling frequency out of range"));
    }

    let channels = num_channels.ok_or(Error::InvalidHeader("missing channel info ('CHNL' chunk)"))?;
    if channels == 0 || channels > MAX_CHANNELS as u16 {
        return Err(Error::Unsupported("channel count must be between 1 and 8"));
    }

    let data_offset = dsd_data_offset.ok_or(Error::InvalidHeader("missing audio data ('DSD ' chunk)"))?;
    let data_len = dsd_data_len.ok_or(Error::InvalidHeader("missing audio data length"))?;

    let bytes_per_channel = data_len / (channels as usize);
    let sample_count = (bytes_per_channel as u64).saturating_mul(8);

    if channel_names.len() < channels as usize {
        for c in channel_names.len()..(channels as usize) {
            channel_names.push(format!("Channel {}", c + 1));
        }
    }

    let metadata = Metadata { format: Format::Dsdiff, sample_rate, channels, sample_count, channel_names, title, artist };

    Ok((metadata, data_offset, data_len))
}

/// Parses a DSDIFF file into raw 1-bit planar channel streams.
pub fn parse_dsdiff(bytes: &[u8]) -> Result<RawAudio, Error> {
    let (metadata, data_offset, data_len) = parse_dsdiff_header(bytes)?;
    let channels = metadata.channels as usize;
    let bytes_per_channel = data_len / channels;

    if bytes_per_channel.saturating_mul(channels) > MAX_DECODE_BYTES {
        return Err(Error::MemoryLimitExceeded);
    }

    let end_offset = data_offset + data_len;
    if end_offset > bytes.len() {
        return Err(Error::UnexpectedEof("DSD sound data extends beyond file bounds"));
    }
    let data_slice = &bytes[data_offset..end_offset];

    let mut channel_bytes: Vec<Vec<u8>> = Vec::with_capacity(channels);
    for _ in 0..channels {
        let mut ch_vec = Vec::new();
        ch_vec.try_reserve_exact(bytes_per_channel).map_err(|_| Error::MemoryLimitExceeded)?;
        channel_bytes.push(ch_vec);
    }

    // De-interleave byte-by-byte: Ch0, Ch1, ... Ch(N-1), Ch0, Ch1 ...
    for chunk in data_slice.chunks_exact(channels) {
        for (c, &byte) in chunk.iter().enumerate() {
            if let Some(ch_vec) = channel_bytes.get_mut(c) {
                ch_vec.push(byte);
            }
        }
    }

    Ok(RawAudio { metadata, bit_order: BitOrder::MsbFirst, channel_bytes })
}

/// Encodes 1-bit planar audio channels into a valid Philips DSDIFF (`.dff`) file.
pub fn write_dsdiff(sample_rate: u32, channels: &[&[u8]], sample_count: u64) -> Result<Vec<u8>, Error> {
    if channels.is_empty() || channels.len() > MAX_CHANNELS {
        return Err(Error::Unsupported("channel count must be between 1 and 8"));
    }
    if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate) {
        return Err(Error::InvalidData("sample rate out of bounds"));
    }

    let num_channels = channels.len();
    let bytes_per_ch = usize::try_from(sample_count.div_ceil(8)).map_err(|_| Error::MemoryLimitExceeded)?;
    let data_size = bytes_per_ch.checked_mul(num_channels).ok_or(Error::MemoryLimitExceeded)?;
    // Fixed headers plus four bytes for each channel identifier.
    // An odd-sized chunk payload is followed by a pad byte that its size field does not count.
    let data_pad = data_size & 1;
    let padded_size = data_size.checked_add(data_pad).ok_or(Error::MemoryLimitExceeded)?;
    let file_size = super::encoded_size(padded_size, 110 + 4 * num_channels)?;

    // Build PROP subchunks:
    // 1. 'FS  ' (4 bytes + 12 chunk header)
    let mut fs_chunk = Vec::with_capacity(16);
    fs_chunk.extend_from_slice(b"FS  ");
    fs_chunk.extend_from_slice(&4u64.to_be_bytes());
    fs_chunk.extend_from_slice(&sample_rate.to_be_bytes());

    // 2. 'CHNL' (2 + 4 * num_channels + 12 chunk header)
    let chnl_data_len = 2 + 4 * num_channels;
    let mut chnl_chunk = Vec::with_capacity(12 + chnl_data_len + 1);
    chnl_chunk.extend_from_slice(b"CHNL");
    chnl_chunk.extend_from_slice(&(chnl_data_len as u64).to_be_bytes());
    chnl_chunk.extend_from_slice(&(num_channels as u16).to_be_bytes());
    for id in channel_ids(num_channels) {
        chnl_chunk.extend_from_slice(&id);
    }
    if !chnl_data_len.is_multiple_of(2) {
        chnl_chunk.push(0);
    }

    // 3. 'CMPR' (4 + 1 + 3 + pad = 8 bytes + 12 chunk header)
    let mut cmpr_chunk = Vec::with_capacity(20);
    cmpr_chunk.extend_from_slice(b"CMPR");
    cmpr_chunk.extend_from_slice(&8u64.to_be_bytes());
    cmpr_chunk.extend_from_slice(b"DSD ");
    cmpr_chunk.push(3); // name len
    cmpr_chunk.extend_from_slice(b"DSD");

    let prop_payload_len = 4 + fs_chunk.len() + chnl_chunk.len() + cmpr_chunk.len();
    let mut prop_chunk = Vec::with_capacity(12 + prop_payload_len);
    prop_chunk.extend_from_slice(b"PROP");
    prop_chunk.extend_from_slice(&(prop_payload_len as u64).to_be_bytes());
    prop_chunk.extend_from_slice(b"SND ");
    prop_chunk.extend_from_slice(&fs_chunk);
    prop_chunk.extend_from_slice(&chnl_chunk);
    prop_chunk.extend_from_slice(&cmpr_chunk);

    // FVER chunk (4 bytes payload + 12 chunk header)
    let mut fver_chunk = Vec::with_capacity(16);
    fver_chunk.extend_from_slice(b"FVER");
    fver_chunk.extend_from_slice(&4u64.to_be_bytes());
    fver_chunk.extend_from_slice(&0x01050000u32.to_be_bytes());

    // DSD chunk (data_size payload + 12 chunk header)
    let mut dsd_chunk_hdr = Vec::with_capacity(12);
    dsd_chunk_hdr.extend_from_slice(b"DSD ");
    dsd_chunk_hdr.extend_from_slice(&(data_size as u64).to_be_bytes());

    let frm8_payload_len = file_size - 12;
    let mut out = Vec::new();
    out.try_reserve_exact(file_size).map_err(|_| Error::MemoryLimitExceeded)?;

    out.extend_from_slice(b"FRM8");
    out.extend_from_slice(&(frm8_payload_len as u64).to_be_bytes());
    out.extend_from_slice(b"DSD ");
    out.extend_from_slice(&fver_chunk);
    out.extend_from_slice(&prop_chunk);
    out.extend_from_slice(&dsd_chunk_hdr);

    // Interleave channels byte-by-byte.
    for i in 0..bytes_per_ch {
        for ch_data in channels {
            let byte = ch_data.get(i).copied().unwrap_or(super::filter::SILENCE_MSBF);
            out.push(byte);
        }
    }
    if data_pad == 1 {
        out.push(0);
    }

    Ok(out)
}
