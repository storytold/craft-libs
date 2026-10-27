//! Native WAV / BWF / RF64 reader (RIFF chunk layout per the Microsoft multimedia
//! specification, `bext` per EBU Tech 3285, `ds64` per EBU Tech 3306).

use super::pcm::{PcmKind, deinterleave, le_u16, le_u32, le_u64, tag};
use crate::wav::{GUID_TAIL, WAVE_FORMAT_EXTENSIBLE, WAVE_FORMAT_IEEE_FLOAT, WAVE_FORMAT_PCM};
use crate::{AudioBuffer, AudioError, AudioInfo, BwfInfo, FileFormat, Result, SampleFormat};

/// Tail of the Ambisonic subtype GUIDs {0000000x-0721-11D3-8644-C8C1CA000000}: PCM (x = 1) and float (x = 3) as well.
const AMBISONIC_TAIL: [u8; 14] = [0x00, 0x00, 0x21, 0x07, 0xD3, 0x11, 0x86, 0x44, 0xC8, 0xC1, 0xCA, 0x00, 0x00, 0x00];

/// Parsed WAV layout (no sample data decoded yet).
pub(crate) struct WavLayout {
    pub info: AudioInfo,
    pub kind: PcmKind,
    pub data_start: usize,
    pub data_len: usize,
}

/// Outcome of parsing a WAV header.
pub(crate) enum WavParse {
    /// A PCM / float layout we decode natively.
    Native(WavLayout),
    /// A valid WAV whose codec we hand to symphonia (ADPCM, A-law, ...).
    Foreign { bwf: Option<BwfInfo> },
}

struct Fmt {
    tag: u16,
    channels: u16,
    sample_rate: u32,
    block_align: u16,
    bits: u16,
    valid_bits: u16,
}

fn parse_fmt(c: &[u8]) -> Result<Fmt> {
    let short = || AudioError::Malformed("fmt chunk too short".into());
    let mut fmt = Fmt {
        tag: le_u16(c, 0).ok_or_else(short)?,
        channels: le_u16(c, 2).ok_or_else(short)?,
        sample_rate: le_u32(c, 4).ok_or_else(short)?,
        block_align: le_u16(c, 12).ok_or_else(short)?,
        bits: le_u16(c, 14).unwrap_or(0),
        valid_bits: 0,
    };
    fmt.valid_bits = fmt.bits;
    if fmt.tag == WAVE_FORMAT_EXTENSIBLE {
        // cbSize(2) validBits(2) channelMask(4) subFormat GUID(16)
        let (Some(cb_size), Some(valid), Some(sub)) = (le_u16(c, 16), le_u16(c, 18), le_u16(c, 24)) else {
            return Err(AudioError::Malformed("WAVE_FORMAT_EXTENSIBLE fmt chunk too short".into()));
        };
        let Some(tail) = c.get(26..40) else {
            return Err(AudioError::Malformed("WAVE_FORMAT_EXTENSIBLE fmt chunk too short".into()));
        };
        if cb_size < 22 {
            return Err(AudioError::Malformed("WAVE_FORMAT_EXTENSIBLE cbSize is below 22".into()));
        }
        if valid > fmt.bits {
            return Err(AudioError::Malformed("WAVE_FORMAT_EXTENSIBLE valid bits exceed the container size".into()));
        }
        if valid != 0 {
            fmt.valid_bits = valid;
        }
        // A subtype outside the KSDATAFORMAT_SUBTYPE_* family is not PCM/float, whatever its first two bytes say:
        // keep the extensible tag so the file goes to the foreign decoder.
        if tail == GUID_TAIL || tail == AMBISONIC_TAIL {
            fmt.tag = sub;
        }
    }
    Ok(fmt)
}

/// Read a NUL-padded ASCII field.
fn fixed_str(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(b.get(..end).unwrap_or_default()).trim_end().to_string()
}

fn parse_bext(c: &[u8]) -> Option<BwfInfo> {
    if c.len() < 346 {
        return None;
    }
    let lo = u64::from(le_u32(c, 338)?);
    let hi = u64::from(le_u32(c, 342)?);
    Some(BwfInfo {
        description: fixed_str(c.get(0..256)?),
        originator: fixed_str(c.get(256..288)?),
        originator_reference: fixed_str(c.get(288..320)?),
        origination_date: fixed_str(c.get(320..330)?),
        origination_time: fixed_str(c.get(330..338)?),
        time_reference: (hi << 32) | lo,
    })
}

/// Refuse the WAVE file at `at`, which symphonia reads (A-law, ADPCM, ...), if an INFO entry
/// claims more bytes than the file holds: symphonia allocates each entry's length. Chunks are
/// walked where symphonia reads them: after the bytes a chunk's parser used, not its length.
pub(crate) fn check_info(b: &[u8], at: usize) -> Result<()> {
    if tag(b, at + 8) != Some(*b"WAVE") {
        return Ok(());
    }
    let mut pos = at + 12;
    while let Some(id) = tag(b, pos) {
        let Some(len) = le_u32(b, pos + 4).map(|n| n as usize) else { break };
        let (body, pad) = (pos + 8, len & 1);
        let used = match &id {
            b"data" => break,
            b"fmt " => fmt_used(b, body, len),
            b"LIST" if len < 4 => break,
            b"LIST" => {
                let entries = if tag(b, body) == Some(*b"INFO") { info_entries(b, body + 4, len - 4)? } else { 0 };
                // The list pads itself, then the chunk is padded again.
                4 + entries.max(len - 4) + pad
            }
            _ => len,
        };
        pos = body.saturating_add(used).saturating_add(pad);
    }
    Ok(())
}

/// The bytes symphonia reads of a `fmt ` chunk: some formats' parsers go by their own extra size.
fn fmt_used(b: &[u8], body: usize, len: usize) -> usize {
    match le_u16(b, body) {
        Some(2 | 6 | 7 | 0x11) => le_u16(b, body + 16).map_or(len, |extra| 18 + usize::from(extra)),
        Some(0xfffe) => 40,
        _ => len,
    }
}

/// Check the INFO entries in the `len` bytes from `start`; returns the bytes symphonia reads of
/// them, with the pad it reads after an odd entry.
fn info_entries(b: &[u8], start: usize, len: usize) -> Result<usize> {
    let mut used = 0;
    while used + 8 <= len {
        let Some(n) = le_u32(b, start + used + 4).map(|n| n as usize) else { break };
        if n > b.len().saturating_sub(start + used + 8) {
            return Err(AudioError::Malformed("a WAV INFO entry is larger than the file".into()));
        }
        used += 8 + n;
        used += used & 1;
    }
    Ok(used)
}

/// True when the bytes start like a RIFF/RF64/BW64 WAVE file.
pub(crate) fn is_wav(b: &[u8]) -> bool {
    matches!(tag(b, 0), Some(t) if &t == b"RIFF" || &t == b"RF64" || &t == b"BW64") && tag(b, 8).is_some_and(|t| &t == b"WAVE")
}

/// True when everything from `from` to the end of the file is whole RIFF chunks (four printable
/// ASCII id bytes, a size that fits, the pad byte after an odd size; the last chunk's pad may be
/// missing): the metadata of a finished file, not samples.
fn only_chunks_follow(b: &[u8], from: usize) -> bool {
    let mut pos = from;
    while pos < b.len() {
        let (Some(id), Some(size)) = (tag(b, pos), le_u32(b, pos + 4)) else { return false };
        if !id.iter().all(|c| (0x20..0x7f).contains(c)) {
            return false;
        }
        let size = size as usize;
        let Some(end) = pos.checked_add(8).and_then(|body| body.checked_add(size)) else { return false };
        if end > b.len() {
            return false;
        }
        let next = end + (size & 1);
        if end == b.len() || next == b.len() {
            return true;
        }
        pos = next;
    }
    false
}

/// A `data` chunk header as found in the file, for [`data_chunk_len`].
struct DataChunk {
    body: usize,
    size32: u32,
    is_rf64: bool,
    remaining: usize,
    declared: usize,
}

/// Bytes of audio a `data` chunk holds: leniently, whatever is present when the size is wrong or
/// the recording was never finalised.
fn data_chunk_len(b: &[u8], c: DataChunk) -> usize {
    let finished = u64::from(le_u32(b, 4).unwrap_or(0)) + 8 == b.len() as u64;
    let empty = c.size32 == 0 && !c.is_rf64 && finished && only_chunks_follow(b, c.body);
    if c.size32 == 0 && !c.is_rf64 && !empty { c.remaining } else { c.declared.min(c.remaining) }
}

/// The sample layout of a `fmt` chunk, `None` for a format we do not read natively.
fn pcm_layout(fmt: &Fmt) -> Result<Option<(PcmKind, SampleFormat)>> {
    Ok(Some(match (fmt.tag, fmt.bits) {
        (WAVE_FORMAT_PCM, bits @ 1..=32) => {
            let bytes = usize::from(bits.div_ceil(8));
            let sf = match fmt.valid_bits.min(bits) {
                0..=8 => SampleFormat::Int8,
                9..=16 => SampleFormat::Int16,
                17..=24 => SampleFormat::Int24,
                _ => SampleFormat::Int32,
            };
            (PcmKind::Int { bytes, big_endian: false, unsigned: bytes == 1 }, sf)
        }
        (WAVE_FORMAT_IEEE_FLOAT, 32) => (PcmKind::Float { bytes: 4, big_endian: false }, SampleFormat::Float32),
        (WAVE_FORMAT_IEEE_FLOAT, 64) => (PcmKind::Float { bytes: 8, big_endian: false }, SampleFormat::Float64),
        (WAVE_FORMAT_PCM | WAVE_FORMAT_IEEE_FLOAT, bits) => {
            return Err(AudioError::Unsupported(format!("WAV format {} with {bits} bits per sample", fmt.tag)));
        }
        _ => return Ok(None),
    }))
}

pub(crate) fn parse(b: &[u8]) -> Result<WavParse> {
    if !is_wav(b) {
        return Err(AudioError::Malformed("not a RIFF/RF64 WAVE file".into()));
    }
    let is_rf64 = tag(b, 0).is_some_and(|t| &t != b"RIFF");
    let mut ds64_data: Option<u64> = None;
    let mut fmt: Option<Fmt> = None;
    let mut bwf: Option<BwfInfo> = None;
    let mut data: Option<(usize, usize)> = None;

    let mut pos: usize = 12;
    // Bound the number of chunks we look at so a file of tiny chunks can't spin forever.
    let mut chunks = 0usize;
    while let (Some(id), Some(size32)) = (tag(b, pos), le_u32(b, pos + 4)) {
        chunks += 1;
        if chunks > 100_000 {
            break;
        }
        let body = pos.checked_add(8).ok_or_else(|| AudioError::Malformed("chunk offset overflow".into()))?;
        let remaining = b.len().saturating_sub(body);
        let mut size = u64::from(size32);
        if &id == b"data" && is_rf64 && size32 == u32::MAX {
            size = ds64_data.unwrap_or(remaining as u64);
        }
        let declared = usize::try_from(size).unwrap_or(usize::MAX);
        if &id == b"data" {
            // Lenient: a truncated recording (or a streaming writer's 0 / 0xFFFFFFFF size) still
            // yields whatever audio is actually present.
            // A size of 0 in a file whose RIFF size is right and that has only chunks after it is an
            // empty data chunk followed by metadata (LIST, bext, …); otherwise it is a recording
            // that was never finalised, and what follows is audio.
            let len = data_chunk_len(b, DataChunk { body, size32, is_rf64, remaining, declared });
            if data.is_none() {
                data = Some((body, len));
            }
            if declared > remaining {
                break;
            }
        } else {
            if declared > remaining {
                // A truncated non-audio chunk at the end of the file: stop scanning.
                break;
            }
            let c = b.get(body..body + declared).unwrap_or_default();
            match &id {
                b"fmt " => fmt = Some(parse_fmt(c)?),
                b"bext" => bwf = parse_bext(c),
                b"ds64" => ds64_data = le_u64(c, 8),
                _ => {}
            }
        }
        let padded = declared.saturating_add(declared & 1);
        pos = match body.checked_add(padded) {
            Some(p) => p,
            None => break,
        };
    }

    let fmt = fmt.ok_or_else(|| AudioError::Malformed("WAV has no fmt chunk".into()))?;
    if fmt.channels == 0 {
        return Err(AudioError::Malformed("WAV declares zero channels".into()));
    }
    if fmt.sample_rate == 0 {
        return Err(AudioError::Malformed("WAV declares a zero sample rate".into()));
    }
    let channels = usize::from(fmt.channels);
    let Some((kind, sample_format)) = pcm_layout(&fmt)? else { return Ok(WavParse::Foreign { bwf }) };
    let block = kind.bytes().checked_mul(channels).ok_or_else(|| AudioError::Malformed("block size overflow".into()))?;
    if usize::from(fmt.block_align) != block {
        log::debug!("WAV block_align {} disagrees with computed {block}; using computed", fmt.block_align);
    }
    let (data_start, data_len) = data.ok_or_else(|| AudioError::Malformed("WAV has no data chunk".into()))?;
    let frames = (data_len / block) as u64;
    Ok(WavParse::Native(WavLayout {
        info: AudioInfo { format: FileFormat::Wav, sample_format, sample_rate: fmt.sample_rate, channels: fmt.channels, frames, bwf },
        kind,
        data_start,
        data_len,
    }))
}

pub(crate) fn decode(b: &[u8], layout: &WavLayout) -> Result<AudioBuffer> {
    let end = layout.data_start.saturating_add(layout.data_len).min(b.len());
    let data = b.get(layout.data_start..end).unwrap_or_default();
    let channels = deinterleave(data, layout.kind, usize::from(layout.info.channels), layout.info.frames)?;
    Ok(AudioBuffer { sample_rate: layout.info.sample_rate, channels })
}

/// The `dwChannelMask` of a WAVE_FORMAT_EXTENSIBLE file (None for plain PCM/float headers).
pub(crate) fn read_channel_mask(b: &[u8]) -> Option<u32> {
    if !is_wav(b) {
        return None;
    }
    let mut pos = 12usize;
    for _ in 0..10_000 {
        let (id, size) = (tag(b, pos)?, usize::try_from(le_u32(b, pos + 4)?).ok()?);
        if &id == b"fmt " {
            return (le_u16(b, pos + 8)? == WAVE_FORMAT_EXTENSIBLE).then(|| le_u32(b, pos + 8 + 20)).flatten();
        }
        pos = pos.checked_add(8)?.checked_add(size)?.checked_add(size & 1)?;
    }
    None
}
