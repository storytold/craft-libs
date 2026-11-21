//! Ogg Opus timing through `Decoder` and `decode`: a granule position that jumps far past what
//! the file could hold is damage, not a lost page.

use super::common;

use common::*;
use craft_audio::opus::encoder::OpusEncoder;
use craft_audio::{DecodeOptions, Decoder, decode};
use craft_media::ogg::PageWriter;

/// An Ogg Opus file of `packets`, four to a page, for `len` samples; the pages from packet 12 on
/// carry granule positions `jump` higher.
fn ogg_file(enc: &OpusEncoder, packets: &[Vec<u8>], len: usize, jump: i64) -> Vec<u8> {
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&0u32.to_le_bytes());
    tags.extend_from_slice(&0u32.to_le_bytes());
    let mut w = PageWriter::new(7).with_page_target(1 << 20, 4);
    let mut out = w.header(&enc.opus_head());
    out.extend(w.header(&tags));
    for (k, p) in packets.iter().enumerate() {
        let end = ((k + 1) * FRAME) as i64 + if k >= 12 { jump } else { 0 };
        out.extend(w.packet(p, end));
    }
    out.extend(w.finish(Some(i64::from(enc.pre_skip()) + len as i64 + jump)));
    out
}

/// A jump of 2^40 samples counts as one damaged packet, no silence is made for it, and the audio
/// after it follows on: the length and the decode are those of the same file without the jump.
#[test]
fn a_granule_jump_past_what_the_file_holds_is_damage() {
    let len = 48_000;
    let (enc, packets) = encode(OpusEncoder::new(1, 64_000).expect("encoder"), &[music(len)]);
    let clean = ogg_file(&enc, &packets, len, 0);
    let jumped = ogg_file(&enc, &packets, len, 1 << 40);
    let dec = Decoder::from_bytes(&jumped, None, &DecodeOptions::default()).expect("open");
    assert_eq!(dec.info().frames, len as u64);
    let (clean_info, clean_buf) = decode(&clean, None).expect("decode");
    let (info, buf) = decode(&jumped, None).expect("decode");
    assert_eq!((clean_info.frames, clean_info.damaged), (len as u64, 0));
    assert_eq!((info.frames, info.damaged), (len as u64, 1));
    assert_eq!(buf.channels, clean_buf.channels);
}
