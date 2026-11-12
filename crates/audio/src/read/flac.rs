//! FLAC metadata checks run before symphonia decodes a stream (RFC 9639 metadata blocks).

use craft_media::ogg;

use crate::pcm::{be_u32, le_u32};
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
    Some(AudioInfo::new(Container::Flac, CodecId::Flac, sample_format, sample_rate, layout_of(b, at, channels), 0))
}

/// The layout of the FLAC stream at `at`, from its STREAMINFO channel count and its channel mask
/// comment, or `None` when it does not start with a STREAMINFO block.
pub(crate) fn layout(b: &[u8], at: usize) -> Option<ChannelLayout> {
    let head = be_u32(b, at.checked_add(4)?)?;
    if (head >> 24) & 0x7f != 0 {
        return None;
    }
    // In STREAMINFO: 3 bits of channels - 1 after the 20 bits of rate.
    let channels = u16::from((b.get(at.checked_add(4 + 4 + 10 + 2)?)? >> 1) & 0x7) + 1;
    Some(layout_of(b, at, channels))
}

/// `channels` channels described by the channel mask comment of the stream at `at`, or else in
/// the layout decoders assume for the count (RFC 9639 §8.6.2, §9.1.3).
fn layout_of(b: &[u8], at: usize, channels: u16) -> ChannelLayout {
    layout_from(channel_mask(b, at), channels)
}

/// `channels` channels described by a channel mask comment's value, or else in the layout decoders
/// assume for the count.
pub(crate) fn layout_from(mask: Option<u32>, channels: u16) -> ChannelLayout {
    let mask = mask.unwrap_or_else(|| crate::layout::flac_assumed_mask(usize::from(channels)));
    ChannelLayout::from_mask_and_channels(mask, channels)
}

/// The value of the first `WAVEFORMATEXTENSIBLE_CHANNEL_MASK` comment that holds a `0x` hexadecimal
/// number, in the stream at `at`.
fn channel_mask(b: &[u8], at: usize) -> Option<u32> {
    let mut pos = at.checked_add(4)?;
    loop {
        let head = be_u32(b, pos)?;
        let (body, len) = (pos.checked_add(4)?, (head & 0x00ff_ffff) as usize);
        let end = body.checked_add(len)?.min(b.len());
        if (head >> 24) & 0x7f == 4 {
            return comments_mask(b.get(body..end)?);
        }
        if head >> 31 == 1 {
            return None;
        }
        pos = body.checked_add(len)?;
    }
}

/// The mask in a comment, when its key is the channel mask's and its value a `0x` hexadecimal number.
fn comment_mask(key: &[u8], value: &[u8]) -> Option<u32> {
    if !key.eq_ignore_ascii_case(b"WAVEFORMATEXTENSIBLE_CHANNEL_MASK") {
        return None;
    }
    let hex = value.strip_prefix(b"0x").or_else(|| value.strip_prefix(b"0X"))?;
    u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()
}

/// The audio streams of the Ogg file `b` in file order, as `tracks` lists them: each one's serial
/// and, for FLAC, its channel mask comment. Empty when the demuxer cannot read the file.
pub(crate) fn ogg_audio_streams(b: &[u8]) -> Vec<(u32, Option<u32>)> {
    let Ok(file) = ogg::open(b) else { return Vec::new() };
    let mask = |s: &ogg::Stream| {
        // After the first packet, each header packet is a metadata block with its 4-byte header.
        let block = s.headers.iter().skip(1).find(|h| h.first().is_some_and(|t| t & 0x7f == 4))?;
        comments_mask(block.get(4..)?)
    };
    file.streams.iter().filter(|s| s.codec.codec_id().is_audio()).map(|s| (s.serial, mask(s).filter(|_| s.codec == ogg::Codec::Flac))).collect()
}

/// The channel mask in a Vorbis comment block's body.
fn comments_mask(block: &[u8]) -> Option<u32> {
    let vendor = le_u32(block, 0)? as usize;
    let mut at = 4usize.checked_add(vendor)?;
    let count = le_u32(block, at)?;
    at += 4;
    for _ in 0..count {
        let len = le_u32(block, at)? as usize;
        let comment = block.get(at + 4..(at + 4).checked_add(len)?)?;
        at += 4 + len;
        let Some(eq) = comment.iter().position(|&c| c == b'=') else { continue };
        if let Some(mask) = comment_mask(&comment[..eq], &comment[eq + 1..]) {
            return Some(mask);
        }
    }
    None
}

/// Whether a vendor string or comment in a (little-endian) Vorbis comment block claims more than
/// the block holds.
fn comments_overrun(b: &[u8], body: usize, end: usize) -> bool {
    let Some(vendor) = le_u32(b, body).filter(|_| body.saturating_add(4) <= end) else { return false };
    if vendor as usize > end - (body + 4) {
        return true;
    }
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

#[cfg(test)]
mod tests {
    use super::comments_mask;

    fn block(comments: &[&str]) -> Vec<u8> {
        let mut b = 3u32.to_le_bytes().to_vec();
        b.extend_from_slice(b"abc");
        b.extend_from_slice(&(comments.len() as u32).to_le_bytes());
        for c in comments {
            b.extend_from_slice(&(c.len() as u32).to_le_bytes());
            b.extend_from_slice(c.as_bytes());
        }
        b
    }

    #[test]
    fn the_channel_mask_comment_is_found_in_any_case() {
        assert_eq!(comments_mask(&block(&["TITLE=x", "WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x0107"])), Some(0x107));
        assert_eq!(comments_mask(&block(&["waveformatextensible_channel_mask=0X3f"])), Some(0x3F));
        assert_eq!(comments_mask(&block(&["TITLE=x"])), None);
    }

    #[test]
    fn a_bad_channel_mask_comment_is_ignored() {
        for value in ["63", "0x", "0xZZ", "0x1FFFFFFFF"] {
            assert_eq!(comments_mask(&block(&[&format!("WAVEFORMATEXTENSIBLE_CHANNEL_MASK={value}")])), None, "{value}");
        }
        assert_eq!(comments_mask(&block(&["WAVEFORMATEXTENSIBLE_CHANNEL_MASK=bad", "WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x3"])), Some(3));
    }

    #[test]
    fn a_truncated_comment_block_has_no_mask() {
        let whole = block(&["WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x3"]);
        for cut in 0..whole.len() {
            assert_eq!(comments_mask(&whole[..cut]), None, "{cut}");
        }
    }
}
