//! FLAC metadata checks run before symphonia decodes a stream (RFC 9639 metadata blocks).

use super::pcm::{be_u32, le_u32};
use crate::{AudioError, AudioInfo, ChannelLayout, CodecId, Container, Result, SampleFormat};

/// Refuse the FLAC stream at `at` if a comment or picture field claims more bytes than its
/// metadata block holds: symphonia allocates each field's length before reading it.
pub(crate) fn check_metadata(b: &[u8], at: usize) -> Result<()> {
    let mut pos = at + 4;
    while let Some(head) = be_u32(b, pos) {
        let (body, len) = (pos + 4, (head & 0x00ff_ffff) as usize);
        let end = (body + len).min(b.len());
        let overruns = match (head >> 24) & 0x7f {
            4 => comments_overrun(b, body, end),
            // Picture: type, MIME type, description, four size fields, data.
            6 => overruns(b, body + 4, end, be_u32, [0, 0, 16]),
            _ => false,
        };
        if overruns {
            return Err(AudioError::Malformed("a FLAC metadata field is larger than its block".into()));
        }
        if head >> 31 == 1 {
            break;
        }
        pos = body + len;
    }
    Ok(())
}

/// The info of a FLAC stream with no audio frames at all: STREAMINFO declares 0 samples and
/// nothing follows the metadata blocks. Symphonia's reader fails on these, as it looks for a first
/// frame, so the caller answers them itself.
pub(crate) fn empty_stream_info(b: &[u8], at: usize) -> Option<AudioInfo> {
    let mut pos = at.checked_add(4)?;
    let mut first = true;
    loop {
        let head = be_u32(b, pos)?;
        let (body, len) = (pos.checked_add(4)?, (head & 0x00ff_ffff) as usize);
        let end = body.checked_add(len)?;
        if end > b.len() {
            return None;
        }
        if first && ((head >> 24) & 0x7f != 0 || len != 34) {
            return None;
        }
        first = false;
        if head >> 31 == 1 {
            if end != b.len() {
                return None;
            }
            break;
        }
        pos = end;
    }
    // STREAMINFO, from the sample rate on: 20 bits rate, 3 channels - 1, 5 bits per sample - 1, 36 bits total.
    let si = at.checked_add(4 + 4 + 10)?;
    let packed = u64::from_be_bytes(b.get(si..si.checked_add(8)?)?.try_into().ok()?);
    let sample_rate = (packed >> 44) as u32;
    let channels = ((packed >> 41) & 0x7) as u16 + 1;
    let bps = ((packed >> 36) & 0x1f) as u32 + 1;
    if packed & 0xf_ffff_ffff != 0 || sample_rate == 0 {
        return None;
    }
    let sample_format = match bps {
        1..=8 => SampleFormat::Int8,
        9..=16 => SampleFormat::Int16,
        17..=24 => SampleFormat::Int24,
        _ => SampleFormat::Int32,
    };
    Some(AudioInfo::new(Container::Flac, CodecId::Flac, sample_format, sample_rate, flac_layout(usize::from(channels)), 0))
}

/// The FLAC channel assignment for a channel count (RFC 9639 §9.1.3).
pub(crate) fn flac_layout(channels: usize) -> ChannelLayout {
    match channels {
        7 => ChannelLayout::SURROUND_6_1,
        1..=8 => ChannelLayout::default_for(channels),
        n => ChannelLayout::unordered(n),
    }
}

/// Whether a comment in a (little-endian) Vorbis comment block claims more than the block holds.
/// The vendor string is skipped unread.
fn comments_overrun(b: &[u8], body: usize, end: usize) -> bool {
    let Some(vendor) = le_u32(b, body) else { return false };
    let at = (body + 4).saturating_add(vendor as usize);
    let Some(count) = le_u32(b, at).filter(|_| at.saturating_add(4) <= end) else { return false };
    overruns(b, at + 4, end, le_u32, std::iter::repeat_n(0, count as usize))
}

/// Whether one of the length-prefixed fields from `at` claims more than `end` leaves; `gaps` are
/// the bytes skipped before each length, which `read` reads.
fn overruns(b: &[u8], mut at: usize, end: usize, read: fn(&[u8], usize) -> Option<u32>, gaps: impl IntoIterator<Item = usize>) -> bool {
    for gap in gaps {
        at = at.saturating_add(gap);
        let Some(n) = read(b, at).filter(|_| at.saturating_add(4) <= end) else { return false };
        at += 4;
        if n as usize > end - at {
            return true;
        }
        at += n as usize;
    }
    false
}
