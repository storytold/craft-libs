//! Native WAV / BWF / RF64 reader (RIFF chunk layout per the Microsoft multimedia
//! specification, `bext` per EBU Tech 3285, `ds64` per EBU Tech 3306).

use super::pcm::{PcmKind, le_u16, le_u32, le_u64, tag};
use super::source::Src;
use crate::wav::{GUID_TAIL, WAVE_FORMAT_EXTENSIBLE, WAVE_FORMAT_IEEE_FLOAT, WAVE_FORMAT_PCM};
use crate::{AudioError, AudioInfo, BwfInfo, ChannelLayout, CodecId, Container, Result, SampleFormat};

/// Tail of the Ambisonic subtype GUIDs {0000000x-0721-11D3-8644-C8C1CA000000}: PCM (x = 1) and float (x = 3) as well.
const AMBISONIC_TAIL: [u8; 14] = [0x00, 0x00, 0x21, 0x07, 0xD3, 0x11, 0x86, 0x44, 0xC8, 0xC1, 0xCA, 0x00, 0x00, 0x00];

/// Parsed WAV layout (no sample data decoded yet).
pub(crate) struct WavLayout {
    pub info: AudioInfo,
    pub kind: PcmKind,
    pub data_start: u64,
    pub data_len: u64,
}

/// Outcome of parsing a WAV header.
pub(crate) enum WavParse {
    /// A PCM / float layout we decode natively.
    Native(WavLayout),
    /// A valid WAV whose codec we hand to symphonia (ADPCM, A-law, ...).
    #[cfg_attr(not(feature = "symphonia"), allow(dead_code))]
    Foreign { bwf: Option<BwfInfo> },
}

struct Fmt {
    tag: u16,
    channels: u16,
    sample_rate: u32,
    block_align: u16,
    bits: u16,
    valid_bits: u16,
    /// `dwChannelMask` of a WAVE_FORMAT_EXTENSIBLE header.
    mask: Option<u32>,
    ambisonic: bool,
}

impl Fmt {
    /// The speaker layout: the extensible mask when there is one (0 or a mask naming another
    /// number of speakers leaves the channels unordered), unordered for Ambisonics, else the
    /// usual layout for the channel count.
    fn layout(&self) -> ChannelLayout {
        let channels = usize::from(self.channels);
        match self.mask {
            _ if self.ambisonic => ChannelLayout::unordered(channels),
            Some(mask) => ChannelLayout::from_mask_and_channels(mask, channels),
            None => ChannelLayout::default_for(channels),
        }
    }
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
        mask: None,
        ambisonic: false,
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
        fmt.mask = le_u32(c, 20);
        fmt.ambisonic = tail == AMBISONIC_TAIL;
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
#[cfg(feature = "symphonia")]
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
#[cfg(feature = "symphonia")]
fn fmt_used(b: &[u8], body: usize, len: usize) -> usize {
    match le_u16(b, body) {
        Some(2 | 6 | 7 | 0x11) => le_u16(b, body + 16).map_or(len, |extra| 18 + usize::from(extra)),
        Some(0xfffe) => 40,
        _ => len,
    }
}

/// Check the INFO entries in the `len` bytes from `start`; returns the bytes symphonia reads of
/// them, with the pad it reads after an odd entry.
#[cfg(feature = "symphonia")]
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
fn only_chunks_follow(src: &Src<'_>, from: u64) -> bool {
    let total = src.size();
    let mut pos = from;
    while pos < total {
        let (Some(id), Some(size)) = (src.tag(pos), src.le_u32(pos + 4)) else { return false };
        if !id.iter().all(|c| (0x20..0x7f).contains(c)) {
            return false;
        }
        let size = u64::from(size);
        let end = pos + 8 + size;
        if end > total {
            return false;
        }
        let next = end + (size & 1);
        if end == total || next == total {
            return true;
        }
        pos = next;
    }
    false
}

/// A `data` chunk header as found in the file, for [`data_chunk_len`].
struct DataChunk {
    body: u64,
    size32: u32,
    is_rf64: bool,
    remaining: u64,
    declared: u64,
}

/// Bytes of audio a `data` chunk holds: leniently, whatever is present when the size is wrong or
/// the recording was never finalised.
fn data_chunk_len(src: &Src<'_>, c: DataChunk) -> u64 {
    let finished = u64::from(src.le_u32(4).unwrap_or(0)) + 8 == src.size();
    let empty = c.size32 == 0 && !c.is_rf64 && finished && only_chunks_follow(src, c.body);
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

/// Parse a WAV file's chunks: its format, `bext` metadata and where its samples are. Only chunk
/// headers and the small `fmt `, `bext` and `ds64` bodies are read.
pub(crate) fn parse(src: &Src<'_>) -> Result<WavParse> {
    if !is_wav(&src.bytes_upto(0, 12)) {
        return Err(AudioError::Malformed("not a RIFF/RF64 WAVE file".into()));
    }
    let total = src.size();
    let is_rf64 = src.tag(0).is_some_and(|t| &t != b"RIFF");
    let mut ds64_data: Option<u64> = None;
    let mut fmt: Option<Fmt> = None;
    let mut bwf: Option<BwfInfo> = None;
    let mut data: Option<(u64, u64)> = None;

    let mut pos: u64 = 12;
    // Bound the number of chunks we look at so a file of tiny chunks can't spin forever.
    let mut chunks = 0usize;
    while let (Some(id), Some(size32)) = (src.tag(pos), src.le_u32(pos + 4)) {
        chunks += 1;
        if chunks > 100_000 {
            break;
        }
        let body = pos + 8;
        let remaining = total.saturating_sub(body);
        let mut declared = u64::from(size32);
        if &id == b"data" && is_rf64 && size32 == u32::MAX {
            declared = ds64_data.unwrap_or(remaining);
        }
        if &id == b"data" {
            // Lenient: a truncated recording (or a streaming writer's 0 / 0xFFFFFFFF size) still
            // yields whatever audio is actually present.
            // A size of 0 in a file whose RIFF size is right and that has only chunks after it is an
            // empty data chunk followed by metadata (LIST, bext, …); otherwise it is a recording
            // that was never finalised, and what follows is audio.
            let len = data_chunk_len(src, DataChunk { body, size32, is_rf64, remaining, declared });
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
            // The fields read from these chunks lie in their first bytes.
            let small = |n: u64| src.bytes(body, usize::try_from(declared.min(n)).unwrap_or(0)).unwrap_or_default();
            match &id {
                b"fmt " => fmt = Some(parse_fmt(&small(256))?),
                b"bext" => bwf = parse_bext(&small(4096)),
                b"ds64" => ds64_data = le_u64(&small(64), 8),
                _ => {}
            }
        }
        let Some(next) = body.checked_add(declared).and_then(|p| p.checked_add(declared & 1)) else { break };
        pos = next;
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
    let frames = data_len / block as u64;
    Ok(WavParse::Native(WavLayout {
        info: AudioInfo { bwf, ..AudioInfo::new(Container::Wav, CodecId::Pcm, sample_format, fmt.sample_rate, fmt.layout(), frames) },
        kind,
        data_start,
        data_len,
    }))
}
