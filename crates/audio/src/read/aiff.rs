//! Native AIFF / AIFF-C reader (Apple "Audio Interchange File Format" 1.3 and the AIFF-C
//! draft: big-endian IFF chunks, 80-bit IEEE extended sample rate).

use super::pcm::{PcmKind, be_u16, be_u32, tag};
use super::source::Src;
use crate::{AudioError, AudioInfo, ChannelLayout, CodecId, Container, Result, SampleFormat};

const MAX_SAMPLE_RATE: f64 = 10_000_000.0;

pub(crate) struct AiffLayout {
    pub info: AudioInfo,
    pub kind: PcmKind,
    pub data_start: u64,
    pub data_len: u64,
}

pub(crate) enum AiffParse {
    Native(AiffLayout),
    /// Compressed AIFF-C (ulaw, alaw, ima4, ...): handed to symphonia.
    Foreign,
}

/// True when the bytes start like an AIFF or AIFF-C file.
pub(crate) fn is_aiff(b: &[u8]) -> bool {
    tag(b, 0).is_some_and(|t| &t == b"FORM") && tag(b, 8).is_some_and(|t| &t == b"AIFF" || &t == b"AIFC")
}

/// Decode an 80-bit IEEE 754 extended-precision float.
pub(crate) fn extended_to_f64(b: &[u8]) -> Option<f64> {
    let se = be_u16(b, 0)?;
    let hi = u64::from(be_u32(b, 2)?);
    let lo = u64::from(be_u32(b, 6)?);
    let mant = (hi << 32) | lo;
    if mant == 0 {
        return Some(0.0);
    }
    let exp = i32::from(se & 0x7FFF);
    if exp == 0x7FFF {
        return None; // inf / NaN
    }
    let v = mant as f64 * 2f64.powi(exp - 16383 - 63);
    Some(if se & 0x8000 != 0 { -v } else { v })
}

/// Parse an AIFF file's chunks: its `COMM` format and where its `SSND` samples are. Only chunk
/// headers and the first bytes of `COMM` and `SSND` are read.
pub(crate) fn parse(src: &Src<'_>) -> Result<AiffParse> {
    let head = src.bytes_upto(0, 12);
    if !is_aiff(&head) {
        return Err(AudioError::Malformed("not an AIFF/AIFF-C file".into()));
    }
    let total = src.size();
    let is_aifc = tag(&head, 8).is_some_and(|t| &t == b"AIFC");
    let mut comm: Option<(i16, u32, i16, f64, [u8; 4])> = None;
    let mut ssnd: Option<(u64, u64)> = None;

    let mut pos: u64 = 12;
    let mut chunks = 0usize;
    while let (Some(id), Some(size32)) = (src.tag(pos), src.be_u32(pos + 4)) {
        chunks += 1;
        if chunks > 100_000 {
            break;
        }
        let body = pos + 8;
        let remaining = total.saturating_sub(body);
        let declared = u64::from(size32);
        let len = declared.min(remaining);
        match &id {
            b"COMM" => {
                let c = src.bytes(body, usize::try_from(len.min(64)).unwrap_or(0)).unwrap_or_default();
                let short = || AudioError::Malformed("COMM chunk too short".into());
                let channels = be_u16(&c, 0).ok_or_else(short)? as i16;
                let frames = be_u32(&c, 2).ok_or_else(short)?;
                let bits = be_u16(&c, 6).ok_or_else(short)? as i16;
                let rate = extended_to_f64(c.get(8..18).ok_or_else(short)?).ok_or_else(|| AudioError::Malformed("invalid sample rate".into()))?;
                let compression = if is_aifc { tag(&c, 18).ok_or_else(short)? } else { *b"NONE" };
                comm = Some((channels, frames, bits, rate, compression));
            }
            b"SSND" if ssnd.is_none() => {
                let offset = if len >= 4 { u64::from(src.be_u32(body).unwrap_or(0)) } else { 0 };
                let start = body.saturating_add(8).saturating_add(offset).min(total);
                let data_len = len.saturating_sub(8).saturating_sub(offset);
                ssnd = Some((start, data_len));
            }
            _ => {}
        }
        if declared > remaining {
            break;
        }
        let Some(next) = body.checked_add(declared).and_then(|p| p.checked_add(declared & 1)) else { break };
        pos = next;
    }

    let (channels, frames, bits, rate, compression) = comm.ok_or_else(|| AudioError::Malformed("AIFF has no COMM chunk".into()))?;
    if channels <= 0 {
        return Err(AudioError::Malformed("AIFF declares zero or negative channels".into()));
    }
    if !(1.0..=MAX_SAMPLE_RATE).contains(&rate) {
        return Err(AudioError::Malformed(format!("AIFF sample rate {rate} out of range")));
    }
    let sample_rate = rate.round() as u32;
    let int_kind = |big_endian: bool| -> Result<(PcmKind, SampleFormat)> {
        if !(1..=32).contains(&bits) {
            return Err(AudioError::Unsupported(format!("AIFF sample size {bits}")));
        }
        let bytes = (bits as usize).div_ceil(8);
        let sf = match bits {
            1..=8 => SampleFormat::Int8,
            9..=16 => SampleFormat::Int16,
            17..=24 => SampleFormat::Int24,
            _ => SampleFormat::Int32,
        };
        Ok((PcmKind::Int { bytes, big_endian, unsigned: false }, sf))
    };
    let (kind, sample_format) = match &compression {
        b"NONE" | b"twos" => int_kind(true)?,
        b"sowt" => int_kind(false)?,
        b"fl32" | b"FL32" => (PcmKind::Float { bytes: 4, big_endian: true }, SampleFormat::Float32),
        b"fl64" | b"FL64" => (PcmKind::Float { bytes: 8, big_endian: true }, SampleFormat::Float64),
        _ => return Ok(AiffParse::Foreign),
    };
    let (data_start, data_len) = ssnd.unwrap_or((total, 0));
    let block = kind.bytes().saturating_mul(channels as usize).max(1);
    let frames = u64::from(frames).min(data_len / block as u64);
    Ok(AiffParse::Native(AiffLayout {
        info: AudioInfo::new(Container::Aiff, CodecId::Pcm, sample_format, sample_rate, layout(usize::from(channels as u16)), frames),
        kind,
        data_start,
        data_len,
    }))
}

/// The AIFF channel order (AIFF 1.3, "Sample Points"): mono, stereo, L R C and quad map to
/// speakers; the six-channel order (L Lc C R Rc S) and larger counts have no WAV order.
fn layout(channels: usize) -> ChannelLayout {
    match channels {
        1 => ChannelLayout::MONO,
        2 => ChannelLayout::STEREO,
        3 => ChannelLayout::SURROUND_3_0,
        4 => ChannelLayout::QUAD,
        n => ChannelLayout::unordered(n),
    }
}
