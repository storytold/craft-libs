//! Program specific information: the program association section and the program map section
//! (H.222.0 §2.4.4), each ending with the CRC-32 of Annex A that the demuxer checks.

pub(crate) use super::super::ts::crc32_mpeg as crc32;
use super::{Codec, Error, OpusMapping, Stream};

/// Largest `section_length` of a program association or program map section (§2.4.4.3, §2.4.4.8).
pub(crate) const MAX_SECTION_LENGTH: usize = 1021;

/// Writes `section_length` into bytes 1 and 2 and appends the CRC. `s` holds the section from
/// `table_id` up to the CRC.
fn finish(mut s: Vec<u8>) -> Result<Vec<u8>, Error> {
    let length = s.len() + 4 - 3;
    if length > MAX_SECTION_LENGTH {
        return Err(Error::PsiTooLarge { length });
    }
    s[1] = 0xB0 | (length >> 8) as u8;
    s[2] = length as u8;
    let crc = crc32(&s);
    s.extend_from_slice(&crc.to_be_bytes());
    Ok(s)
}

/// The program association section with one program.
pub(crate) fn pat(transport_stream_id: u16, program_number: u16, pmt_pid: u16) -> Result<Vec<u8>, Error> {
    let mut s = vec![0x00, 0, 0];
    s.extend_from_slice(&transport_stream_id.to_be_bytes());
    // reserved, version_number 0, current_next_indicator 1; section_number 0, last_section_number 0
    s.extend_from_slice(&[0xC1, 0x00, 0x00]);
    s.extend_from_slice(&program_number.to_be_bytes());
    s.extend_from_slice(&(0xE000 | pmt_pid).to_be_bytes());
    finish(s)
}

/// The program map section.
pub(crate) fn pmt(program_number: u16, pcr_pid: u16, streams: &[Stream]) -> Result<Vec<u8>, Error> {
    let mut s = vec![0x02, 0, 0];
    s.extend_from_slice(&program_number.to_be_bytes());
    s.extend_from_slice(&[0xC1, 0x00, 0x00]);
    s.extend_from_slice(&(0xE000 | pcr_pid).to_be_bytes());
    // program_info_length 0
    s.extend_from_slice(&[0xF0, 0x00]);
    for st in streams {
        s.push(st.codec.stream_type());
        s.extend_from_slice(&(0xE000 | st.pid).to_be_bytes());
        let at = s.len();
        s.extend_from_slice(&[0xF0, 0x00]);
        descriptors(st, &mut s);
        let len = s.len() - at - 2;
        s[at] = 0xF0 | (len >> 8) as u8;
        s[at + 1] = len as u8;
    }
    finish(s)
}

/// The elementary stream descriptors of one stream.
fn descriptors(st: &Stream, s: &mut Vec<u8>) {
    match st.codec {
        // registration_descriptor (§2.6.8) with the format identifier "AC-3"
        Codec::Ac3 => s.extend_from_slice(&[0x05, 4, b'A', b'C', b'-', b'3']),
        // registration "EAC3" (ATSC A/52) with stream type 0x87
        Codec::Eac3 => s.extend_from_slice(&[0x05, 4, b'E', b'A', b'C', b'3']),
        Codec::Opus { channels } => {
            // registration_descriptor "Opus", then the Opus audio descriptor as an extension
            // descriptor (tag 0x7F) whose channel_config_code is the channel count (1 to 8)
            s.extend_from_slice(&[0x05, 4, b'O', b'p', b'u', b's']);
            s.extend_from_slice(&[0x7F, 2, OPUS_DESCRIPTOR_EXTENSION, channels]);
        }
        Codec::OpusMapped(m) => {
            s.extend_from_slice(&[0x05, 4, b'O', b'p', b'u', b's']);
            let mut body = [0u8; 10];
            let n = opus_explicit(&m, &mut body);
            s.extend_from_slice(&[0x7F, 1 + n as u8, OPUS_DESCRIPTOR_EXTENSION]);
            s.extend_from_slice(&body[..n]);
        }
        Codec::Av1(c) => {
            // registration_descriptor "AV01" first, then the AV1 video descriptor (tag 0x80)
            // (Carriage of AV1 in MPEG-2 TS, §2)
            s.extend_from_slice(&[0x05, 4, b'A', b'V', b'0', b'1']);
            let b = |v: bool, shift: u8| u8::from(v) << shift;
            s.extend_from_slice(&[
                0x80,
                4,
                0x81,
                c.seq_profile << 5 | c.seq_level_idx_0,
                b(c.seq_tier_0, 7)
                    | b(c.high_bitdepth, 6)
                    | b(c.twelve_bit, 5)
                    | b(c.monochrome, 4)
                    | b(c.chroma_subsampling_x, 3)
                    | b(c.chroma_subsampling_y, 2)
                    | c.chroma_sample_position,
                // hdr_wcg_idc, then reserved zeros and no initial_presentation_delay
                c.hdr_wcg_idc << 6,
            ]);
        }
        _ => {}
    }
    if let Some(lang) = st.language {
        // ISO_639_language_descriptor (§2.6.18), audio_type 0 (undefined)
        s.extend_from_slice(&[0x0A, 4, lang[0], lang[1], lang[2], 0]);
    }
}

/// `descriptor_tag_extension` of the Opus audio descriptor. The Opus transport draft leaves the
/// value open; 0x80 is the value in use for it.
pub(crate) const OPUS_DESCRIPTOR_EXTENSION: u8 = 0x80;

/// Bits needed for the values 0 to `n - 1`: ceil(log2(n)).
fn bits_for(n: u32) -> u32 {
    if n <= 1 { 0 } else { 32 - (n - 1).leading_zeros() }
}

/// The opus_audio_descriptor body for an explicit channel configuration (channel_config_code
/// 0x81): channel_count, mapping_family and, for a family above 0, stream_count_minus_one,
/// coupled_stream_count and the channel mapping in as few bits as the draft gives each, padded
/// with zero bits to a byte. Returns its length.
fn opus_explicit(m: &OpusMapping, out: &mut [u8; 10]) -> usize {
    out[..3].copy_from_slice(&[0x81, m.channels, m.family]);
    if m.family == 0 {
        return 3;
    }
    let (ch, streams, coupled) = (u32::from(m.channels), u32::from(m.streams), u32::from(m.coupled));
    let mut acc: u64 = 0;
    let mut bits = 0;
    let mut put = |v: u32, n: u32| {
        acc = acc << n | u64::from(v & ((1 << n) - 1));
        bits += n;
    };
    put(streams - 1, bits_for(ch));
    put(coupled, bits_for(streams + 1));
    let width = bits_for(streams + coupled + 1);
    for &c in &m.mapping[..usize::from(m.channels)] {
        put(if c == 255 { (1 << width) - 1 } else { u32::from(c) }, width);
    }
    let bytes = bits.div_ceil(8) as usize;
    let acc = acc << (bytes as u32 * 8 - bits);
    for (i, o) in out[3..3 + bytes].iter_mut().enumerate() {
        *o = (acc >> (8 * (bytes - 1 - i))) as u8;
    }
    3 + bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_check_value() {
        // the CRC-32/MPEG-2 check value of the ASCII digits 1 to 9
        assert_eq!(crc32(b"123456789"), 0x0376_E6E7);
    }

    #[test]
    fn sections_end_with_a_crc_over_the_whole_section() {
        let pat = pat(7, 1, 0x1000).unwrap();
        assert_eq!(pat.len(), 16);
        assert_eq!(crc32(&pat), 0);
        assert_eq!(&pat[..3], &[0x00, 0xB0, 13]);
        assert_eq!(&pat[8..12], &[0x00, 0x01, 0xF0, 0x00]);
        let st = [Stream::new(0x100, Codec::H264), Stream::new(0x101, Codec::Opus { channels: 2 }).with_language(*b"eng")];
        let pmt = pmt(1, 0x100, &st).unwrap();
        assert_eq!(crc32(&pmt), 0);
        assert_eq!(pmt.len(), 3 + ((pmt[1] as usize & 0x0F) << 8 | pmt[2] as usize));
        assert_eq!(&pmt[8..10], &[0xE1, 0x00], "PCR_PID");
        assert_eq!(&pmt[12..17], &[0x1B, 0xE1, 0x00, 0xF0, 0x00], "H.264 without descriptors");
        assert_eq!(&pmt[17..22], &[0x06, 0xE1, 0x01, 0xF0, 16]);
    }

    #[test]
    fn a_section_past_the_length_limit_is_refused() {
        let st: Vec<Stream> = (0..200).map(|i| Stream::new(0x100 + i, Codec::Ac3).with_language(*b"eng")).collect();
        assert!(matches!(pmt(1, 0x100, &st), Err(Error::PsiTooLarge { .. })));
    }

    /// The descriptors of one stream's PMT entry.
    fn stream_descriptors(codec: Codec) -> Vec<u8> {
        let pmt = pmt(1, 0x100, &[Stream::new(0x100, codec)]).unwrap();
        let len = ((pmt[15] as usize & 0x0F) << 8) | pmt[16] as usize;
        pmt[17..17 + len].to_vec()
    }

    #[test]
    fn explicit_opus_mappings_pack_their_fields() {
        // 6 channels, family 1, 4 streams, 2 coupled: 3 + 3 + 6 * 3 = 24 bits
        let m = OpusMapping { channels: 6, family: 1, streams: 4, coupled: 2, mapping: [0, 4, 1, 2, 3, 5, 0, 0] };
        let d = stream_descriptors(Codec::OpusMapped(m));
        let want = 0b011 << 21 | 0b010 << 18 | 0b000_100_001_010_011_101;
        assert_eq!(&d[6..], &[0x7F, 7, 0x80, 0x81, 6, 1, (want >> 16) as u8, (want >> 8) as u8, want as u8]);
        // 3 channels, family 255, 3 streams, none coupled, one silent: 2 + 2 + 3 * 2 = 10 bits
        let m = OpusMapping { channels: 3, family: 255, streams: 3, coupled: 0, mapping: [2, 255, 0, 0, 0, 0, 0, 0] };
        let d = stream_descriptors(Codec::OpusMapped(m));
        assert_eq!(&d[6..], &[0x7F, 6, 0x80, 0x81, 3, 255, 0b10_00_10_11, 0b00_000000]);
        // family 0 carries no stream fields
        let m = OpusMapping { channels: 2, family: 0, streams: 1, coupled: 1, mapping: [0, 1, 0, 0, 0, 0, 0, 0] };
        assert_eq!(&stream_descriptors(Codec::OpusMapped(m))[6..], &[0x7F, 4, 0x80, 0x81, 2, 0]);
    }

    #[test]
    fn av1_and_eac3_descriptors() {
        let c = crate::mpegts::mux::Av1Config {
            seq_profile: 1,
            seq_level_idx_0: 8,
            seq_tier_0: false,
            high_bitdepth: true,
            twelve_bit: false,
            monochrome: false,
            chroma_subsampling_x: false,
            chroma_subsampling_y: false,
            chroma_sample_position: 0,
            hdr_wcg_idc: 3,
        };
        assert_eq!(stream_descriptors(Codec::Av1(c)), [0x05, 4, b'A', b'V', b'0', b'1', 0x80, 4, 0x81, 0x28, 0x40, 0xC0]);
        assert_eq!(stream_descriptors(Codec::Eac3), [0x05, 4, b'E', b'A', b'C', b'3']);
    }
}
