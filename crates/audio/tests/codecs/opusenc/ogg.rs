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

#[test]
fn an_invalid_duration_does_not_discard_the_last_intact_packet() {
    let (enc, packets) = encode(OpusEncoder::new(1, 64_000).expect("encoder"), &[music(8 * FRAME)]);
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&[0; 8]);
    let mut head = enc.opus_head();
    head[10..12].copy_from_slice(&0u16.to_le_bytes());
    let damaged_file = |damaged: &[u8]| {
        let mut writer = PageWriter::new(7).with_page_target(1 << 20, 1);
        let mut file = writer.header(&head);
        file.extend(writer.header(&tags));
        for (index, packet) in packets[..3].iter().enumerate() {
            file.extend(writer.packet(if index == 1 { damaged } else { packet }, ((index + 1) * FRAME) as i64));
        }
        file.extend(writer.finish(Some((3 * FRAME) as i64)));
        file
    };
    let (_, actual) = decode(&damaged_file(&[packets[1][0] | 3, 0]), None).expect("decode");
    let (_, known_duration) = decode(&damaged_file(&[packets[1][0] & !3]), None).expect("decode with known duration");
    assert_eq!(actual.frames(), 3 * FRAME);
    assert_eq!(actual.channels[0][2 * FRAME..], known_duration.channels[0][2 * FRAME..]);
    assert!(actual.channels[0][2 * FRAME..].iter().any(|sample| sample.abs() > 0.0001));
}

#[test]
fn a_malformed_packet_with_readable_toc_keeps_its_toc_duration() {
    let (enc, packets) = encode(OpusEncoder::new(1, 64_000).expect("encoder"), &[music(8 * FRAME)]);
    let parsed = craft_audio::opus::Packet::parse(&packets[0]).expect("packet");
    assert_eq!(parsed.frames.len(), 1);
    let mut first = vec![packets[0][0] & !3 | 1];
    first.extend_from_slice(parsed.frames[0]);
    first.extend_from_slice(parsed.frames[0]);
    let mut head = enc.opus_head();
    head[10..12].copy_from_slice(&0u16.to_le_bytes());
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&[0; 8]);
    let file = |middle: &[u8]| {
        let mut writer = PageWriter::new(7).with_page_target(1 << 20, 1);
        let mut out = writer.header(&head);
        out.extend(writer.header(&tags));
        out.extend(writer.packet(&first, (2 * FRAME) as i64));
        out.extend(writer.packet(middle, (3 * FRAME) as i64));
        out.extend(writer.packet(&packets[2], (4 * FRAME) as i64));
        out.extend(writer.finish(Some((4 * FRAME) as i64)));
        out
    };
    let (_, actual) = decode(&file(&[0x62]), None).expect("decode malformed framing");
    let (_, reference) = decode(&file(&[packets[1][0] & !3]), None).expect("decode readable single-frame duration");
    assert_eq!(actual.frames(), 4 * FRAME);
    assert_eq!(actual.channels[0][3 * FRAME..], reference.channels[0][3 * FRAME..]);
    assert!(actual.channels[0][3 * FRAME..].iter().any(|sample| sample.abs() > 0.0001));
}
