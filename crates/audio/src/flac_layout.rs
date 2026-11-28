//! FLAC speaker layout from STREAMINFO and Vorbis comments (RFC 9639).

use crate::ChannelLayout;
use crate::pcm::{be_u32, le_u32};

/// The layout of the FLAC stream at `at`, from its STREAMINFO channel count and its channel mask
/// comment, or `None` when it does not start with a STREAMINFO block.
#[cfg(feature = "symphonia")]
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
pub(crate) fn layout_of(b: &[u8], at: usize, channels: u16) -> ChannelLayout {
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

/// The channel mask in a Vorbis comment block's body.
pub(crate) fn comments_mask(block: &[u8]) -> Option<u32> {
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
