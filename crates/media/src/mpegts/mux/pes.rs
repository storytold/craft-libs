//! PES packet headers (H.222.0 §2.4.3.6), the adaptation field's PCR (§2.4.3.4), and the bytes
//! written ahead of a frame: access unit delimiters, ADTS headers and Opus control headers.

use super::{Codec, Error, MAX_VIDEO_FRAME, OpusTrim, av1};

/// 2^33: PTS, DTS and the PCR base are 33-bit counters.
pub(crate) const WRAP_90K: i64 = 1 << 33;

/// Largest PES header this writer produces: 9 bytes, PTS and DTS.
pub(crate) const HEADER_MAX: usize = 19;

/// Largest prefix: an Opus control header for a 65 535-byte packet (2 bytes, 257 size bytes, a
/// start and an end trim).
pub(crate) const PREFIX_MAX: usize = 2 + 0xFFFF / 255 + 1 + 4;

/// What [`prepare`] found: the prefix it wrote, and the bytes the frame takes after it.
pub(crate) struct Prepared {
    pub(crate) prefix_len: usize,
    pub(crate) body_len: usize,
}

/// Most frame bytes `codec` allows when the PES header has `header` bytes after
/// PES_packet_length and an Opus control header carries `trim` trim bytes.
pub(crate) fn data_limit(codec: Codec, header: usize, trim: usize) -> usize {
    let room = 0xFFFF - header;
    match codec {
        c if c.is_video() => MAX_VIDEO_FRAME,
        // ADTS frame_length is 13 bits
        Codec::Aac { .. } => (0x1FFF - 7).min(room - 7),
        Codec::Opus { .. } | Codec::OpusMapped(_) => {
            // the control header takes 3 bytes, a size byte per 255 bytes, and the trims
            let budget = room - 3 - trim;
            let mut l = budget * 255 / 256;
            while l + 1 + (l + 1) / 255 <= budget {
                l += 1;
            }
            while l + l / 255 > budget {
                l -= 1;
            }
            l
        }
        _ => room,
    }
}

/// Samples per channel at 48 kHz in an Opus packet, from its table of contents (RFC 6716 §3.1).
pub(crate) fn opus_samples(data: &[u8]) -> Option<u32> {
    let toc = *data.first()?;
    let config = usize::from(toc >> 3);
    let frame = match config {
        0..=11 => [480, 960, 1920, 2880][config % 4],
        12..=15 => [480, 960][config % 2],
        _ => [120, 240, 480, 960][config % 4],
    };
    let count = match toc & 3 {
        0 => 1,
        1 | 2 => 2,
        _ => u32::from(*data.get(1)? & 0x3F),
    };
    Some(frame * count)
}

/// Checks that an ADTS frame or a run of them fills `data` exactly.
fn check_adts(data: &[u8]) -> Result<(), Error> {
    let mut at = 0;
    while at < data.len() {
        let f = &data[at..];
        let &[0xFF, b1, _, b3, b4, b5, _, ..] = f else { return Err(Error::BadFrame("an AAC frame must start with an ADTS header")) };
        if b1 & 0xF6 != 0xF0 {
            return Err(Error::BadFrame("an AAC frame must start with an ADTS header"));
        }
        let header = if b1 & 1 == 0 { 9 } else { 7 };
        let len = usize::from(b3 & 3) << 11 | usize::from(b4) << 3 | usize::from(b5 >> 5);
        if len < header || len > f.len() {
            return Err(Error::BadFrame("an ADTS frame_length does not match the frame"));
        }
        at += len;
    }
    Ok(())
}

/// A 33-bit timestamp with its 4-bit prefix and marker bits (§2.4.3.7).
fn timestamp(prefix: u8, v: i64) -> [u8; 5] {
    let v = v.rem_euclid(WRAP_90K);
    [prefix << 4 | ((v >> 29) & 0x0E) as u8 | 1, (v >> 22) as u8, ((v >> 14) & 0xFE) as u8 | 1, (v >> 7) as u8, ((v << 1) & 0xFE) as u8 | 1]
}

/// Writes a PES header into `out` and returns its length. `length` is the PES_packet_length field
/// (0 for an unbounded video packet).
pub(crate) fn header(out: &mut [u8; HEADER_MAX], stream_id: u8, length: u16, pts: i64, dts: Option<i64>) -> usize {
    let [hi, lo] = length.to_be_bytes();
    // '10', not scrambled, no priority, data_alignment_indicator set, not copyrighted, a copy
    out[..7].copy_from_slice(&[0, 0, 1, stream_id, hi, lo, 0x84]);
    match dts {
        Some(dts) => {
            out[7] = 0xC0;
            out[8] = 10;
            out[9..14].copy_from_slice(&timestamp(3, pts));
            out[14..19].copy_from_slice(&timestamp(1, dts));
            19
        }
        None => {
            out[7] = 0x80;
            out[8] = 5;
            out[9..14].copy_from_slice(&timestamp(2, pts));
            14
        }
    }
}

/// The six program_clock_reference bytes for a 27 MHz clock value: the 33-bit base (clock / 300),
/// six reserved bits, and the 9-bit extension (clock % 300).
pub(crate) fn pcr(clock: i64) -> [u8; 6] {
    let v = clock.rem_euclid(WRAP_90K * 300);
    let (base, ext) = (v / 300, v % 300);
    [(base >> 25) as u8, (base >> 17) as u8, (base >> 9) as u8, (base >> 1) as u8, ((base & 1) << 7) as u8 | 0x7E | (ext >> 8) as u8, ext as u8]
}

/// The first NAL unit's first header byte, when `data` starts with an Annex B start code.
fn first_nal(data: &[u8]) -> Option<u8> {
    let at = if data.starts_with(&[0, 0, 1]) {
        3
    } else if data.starts_with(&[0, 0, 0, 1]) {
        4
    } else {
        return None;
    };
    data.get(at).copied()
}

/// Checks that `data` is a frame of `codec`, writes what goes in front of it into `out`, and
/// returns the prefix length and the bytes the frame itself takes.
pub(crate) fn prepare(codec: Codec, data: &[u8], key: bool, trim: OpusTrim, out: &mut [u8; PREFIX_MAX]) -> Result<Prepared, Error> {
    if let Codec::Av1(_) = codec {
        return Ok(Prepared { prefix_len: 0, body_len: av1::check(data, key)? });
    }
    let prefix_len = prefix(codec, data, trim, out)?;
    Ok(Prepared { prefix_len, body_len: data.len() })
}

/// Checks a frame of a codec carried as it is and writes its prefix. Returns the prefix length.
fn prefix(codec: Codec, data: &[u8], trim: OpusTrim, out: &mut [u8; PREFIX_MAX]) -> Result<usize, Error> {
    match codec {
        Codec::H264 => {
            let nal = first_nal(data).ok_or(Error::BadFrame("an H.264 frame must start with an Annex B start code"))?;
            if nal & 0x1F == 9 {
                return Ok(0);
            }
            // access unit delimiter, primary_pic_type 7 (any slice type) (H.222.0 §2.14)
            out[..6].copy_from_slice(&[0, 0, 0, 1, 0x09, 0xF0]);
            Ok(6)
        }
        Codec::Hevc => {
            let nal = first_nal(data).ok_or(Error::BadFrame("an HEVC frame must start with an Annex B start code"))?;
            if (nal >> 1) & 0x3F == 35 {
                return Ok(0);
            }
            // access unit delimiter (nal_unit_type 35), pic_type 2 (any slice type) (H.222.0 §2.17)
            out[..7].copy_from_slice(&[0, 0, 0, 1, 0x46, 0x01, 0x50]);
            Ok(7)
        }
        Codec::Mpeg1Video | Codec::Mpeg2Video | Codec::Mpeg4Part2 => {
            first_nal(data).ok_or(Error::BadFrame("an MPEG video frame must start with a start code"))?;
            Ok(0)
        }
        Codec::AacAdts => {
            check_adts(data)?;
            Ok(0)
        }
        Codec::Aac { object_type, sample_rate_index, channels } => {
            let len = data.len() + 7;
            if len > 0x1FFF {
                return Err(Error::FrameTooLarge { size: data.len(), limit: 0x1FFF - 7 });
            }
            // ADTS fixed and variable header (ISO/IEC 13818-7 §6.2): MPEG-4, no CRC, buffer
            // fullness 0x7FF (variable rate), one raw data block
            out[..7].copy_from_slice(&[
                0xFF,
                0xF1,
                (object_type - 1) << 6 | sample_rate_index << 2 | channels >> 2,
                (channels & 3) << 6 | (len >> 11) as u8,
                (len >> 3) as u8,
                ((len & 7) as u8) << 5 | 0x1F,
                0xFC,
            ]);
            Ok(7)
        }
        Codec::Opus { .. } | Codec::OpusMapped(_) => {
            // opus_control_header: the 11-bit prefix 0x3FF, the trim flags, no extension, the
            // payload size as 0xFF bytes and a last byte, then the trims (3 reserved bits and
            // 13 bits each)
            let n = data.len() / 255;
            let trims = if trim.start > 0 { 2 } else { 0 } + if trim.end > 0 { 2 } else { 0 };
            if 3 + n + trims > PREFIX_MAX {
                return Err(Error::FrameTooLarge { size: data.len(), limit: data_limit(codec, 8, trims) });
            }
            out[0] = 0x7F;
            out[1] = 0xE0 | if trim.start > 0 { 0x10 } else { 0 } | if trim.end > 0 { 0x08 } else { 0 };
            out[2..2 + n].fill(0xFF);
            out[2 + n] = (data.len() % 255) as u8;
            let mut at = 3 + n;
            for t in [trim.start, trim.end] {
                if t > 0 {
                    out[at..at + 2].copy_from_slice(&(t & 0x1FFF).to_be_bytes());
                    at += 2;
                }
            }
            Ok(at)
        }
        Codec::Ac3 => match data {
            [0x0B, 0x77, ..] => Ok(0),
            _ => Err(Error::BadFrame("an AC-3 frame must start with its sync word")),
        },
        Codec::Eac3 => match data {
            // bsid, the top five bits of byte 5, is 11 to 16 for E-AC-3 (ATSC A/52)
            [0x0B, 0x77, _, _, _, b5, ..] if (11..=16).contains(&(b5 >> 3)) => Ok(0),
            _ => Err(Error::BadFrame("an E-AC-3 frame must start with its sync word and a bsid of 11 to 16")),
        },
        Codec::MpegAudio { low_sample_rate } => match data {
            [0xFF, b, ..] if b & 0xE0 == 0xE0 => {
                // version: 3 MPEG-1, 2 MPEG-2, 0 MPEG 2.5, 1 reserved; layer 0 is reserved
                let version = (b >> 3) & 3;
                if version == 1 || (b >> 1) & 3 == 0 {
                    return Err(Error::BadFrame("the MPEG audio frame header has a reserved version or layer"));
                }
                if (version == 3) == low_sample_rate {
                    return Err(Error::BadFrame("the MPEG audio frame's version does not match the stream's low_sample_rate"));
                }
                Ok(0)
            }
            _ => Err(Error::BadFrame("an MPEG audio frame must start with its sync word")),
        },
        Codec::Av1(_) => Err(Error::BadFrame("AV1 frames are not carried with a prefix")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads a 33-bit timestamp back from its five bytes.
    fn read(b: &[u8]) -> i64 {
        (((b[0] >> 1) & 7) as i64) << 30 | (b[1] as i64) << 22 | ((b[2] >> 1) as i64) << 15 | (b[3] as i64) << 7 | (b[4] >> 1) as i64
    }

    #[test]
    fn timestamps_carry_their_prefix_and_markers() {
        for v in [0, 1, 90_000, (1 << 32) + 12_345, WRAP_90K - 1] {
            let b = timestamp(2, v);
            assert_eq!(b[0] >> 4, 2);
            assert_eq!([b[0] & 1, b[2] & 1, b[4] & 1], [1, 1, 1]);
            assert_eq!(read(&b), v);
        }
        assert_eq!(read(&timestamp(2, WRAP_90K + 5)), 5, "wraps at 2^33");
        assert_eq!(read(&timestamp(2, -1)), WRAP_90K - 1);
    }

    #[test]
    fn headers_hold_pts_and_dts() {
        let mut h = [0u8; HEADER_MAX];
        assert_eq!(header(&mut h, 0xE0, 0, 3000, Some(0)), 19);
        assert_eq!(&h[..9], &[0, 0, 1, 0xE0, 0, 0, 0x84, 0xC0, 10]);
        assert_eq!((h[9] >> 4, read(&h[9..14]), h[14] >> 4, read(&h[14..19])), (3, 3000, 1, 0));
        assert_eq!(header(&mut h, 0xC0, 300, 7, None), 14);
        assert_eq!(&h[4..9], &[1, 44, 0x84, 0x80, 5]);
    }

    #[test]
    fn pcr_splits_base_and_extension() {
        let clock = 123_456_789i64 * 300 + 299;
        let b = pcr(clock);
        let base = (b[0] as i64) << 25 | (b[1] as i64) << 17 | (b[2] as i64) << 9 | (b[3] as i64) << 1 | (b[4] >> 7) as i64;
        let ext = ((b[4] & 1) as i64) << 8 | b[5] as i64;
        assert_eq!((base, ext, b[4] & 0x7E), (123_456_789, 299, 0x7E));
    }

    const NO_TRIM: OpusTrim = OpusTrim { start: 0, end: 0 };

    #[test]
    fn access_unit_delimiters_are_added_once() {
        let mut p = [0u8; PREFIX_MAX];
        assert_eq!(prefix(Codec::H264, &[0, 0, 0, 1, 0x65, 0x88], NO_TRIM, &mut p).unwrap(), 6);
        assert_eq!(prefix(Codec::H264, &[0, 0, 1, 0x09, 0xF0, 0, 0, 1, 0x65], NO_TRIM, &mut p).unwrap(), 0);
        assert_eq!(prefix(Codec::Hevc, &[0, 0, 0, 1, 0x26, 0x01], NO_TRIM, &mut p).unwrap(), 7);
        assert_eq!(prefix(Codec::Hevc, &[0, 0, 1, 0x46, 0x01, 0x50], NO_TRIM, &mut p).unwrap(), 0);
        assert!(matches!(prefix(Codec::H264, &[0x65, 0x88], NO_TRIM, &mut p), Err(Error::BadFrame(_))));
        assert!(matches!(prefix(Codec::H264, &[0, 0, 1], NO_TRIM, &mut p), Err(Error::BadFrame(_))));
    }

    #[test]
    fn opus_control_header_codes_the_size_and_trims() {
        let mut p = [0u8; PREFIX_MAX];
        let opus = Codec::Opus { channels: 2 };
        assert_eq!(prefix(opus, &[0xFC; 254], NO_TRIM, &mut p).unwrap(), 3);
        assert_eq!(&p[..3], &[0x7F, 0xE0, 254]);
        assert_eq!(prefix(opus, &[0xFC; 510], NO_TRIM, &mut p).unwrap(), 5);
        assert_eq!(&p[..5], &[0x7F, 0xE0, 0xFF, 0xFF, 0]);
        assert_eq!(prefix(opus, &[0xFC; 10], OpusTrim { start: 312, end: 0 }, &mut p).unwrap(), 5);
        assert_eq!(&p[..5], &[0x7F, 0xF0, 10, 0x01, 0x38]);
        assert_eq!(prefix(opus, &[0xFC; 10], OpusTrim { start: 1, end: 8191 }, &mut p).unwrap(), 7);
        assert_eq!(&p[..7], &[0x7F, 0xF8, 10, 0, 1, 0x1F, 0xFF]);
        assert_eq!(prefix(opus, &[0xFC; 10], OpusTrim { start: 0, end: 2 }, &mut p).unwrap(), 5);
        assert_eq!(&p[..5], &[0x7F, 0xE8, 10, 0, 2]);
    }

    #[test]
    fn opus_limits_match_the_pes_length() {
        for (header, trim) in [(8, 0), (13, 0), (8, 4), (13, 2)] {
            let l = data_limit(Codec::Opus { channels: 2 }, header, trim);
            let body = |n: usize| header + 3 + n / 255 + trim + n;
            assert!(body(l) <= 0xFFFF && body(l + 1) > 0xFFFF, "{header} {trim}: {l}");
        }
        assert_eq!(data_limit(Codec::Ac3, 8, 0), 0xFFFF - 8);
        assert_eq!(data_limit(Codec::Mpeg4Part2, 13, 0), MAX_VIDEO_FRAME);
    }

    #[test]
    fn opus_durations_from_the_table_of_contents() {
        assert_eq!(opus_samples(&[0xFC]), Some(960), "CELT 20 ms, one frame");
        assert_eq!(opus_samples(&[0x08 | 1]), Some(1920), "SILK 20 ms, two frames");
        assert_eq!(opus_samples(&[0x18 | 3, 3]), Some(3 * 2880), "SILK 60 ms, three frames");
        assert_eq!(opus_samples(&[0x80]), Some(120), "CELT 2.5 ms");
        assert_eq!(opus_samples(&[0x03]), None);
        assert_eq!(opus_samples(&[]), None);
    }

    #[test]
    fn adts_header_for_raw_aac() {
        let mut p = [0u8; PREFIX_MAX];
        let codec = Codec::Aac { object_type: 2, sample_rate_index: 3, channels: 2 };
        assert_eq!(prefix(codec, &[0; 100], NO_TRIM, &mut p).unwrap(), 7);
        // AAC LC, 48 kHz, 2 channels, 107 bytes
        assert_eq!(&p[..7], &[0xFF, 0xF1, 0x4C, 0x80, 0x0D, 0x7F, 0xFC]);
        assert!(matches!(prefix(codec, &[0; 8185], NO_TRIM, &mut p), Err(Error::FrameTooLarge { .. })));
        assert_eq!(data_limit(codec, 8, 0), 8184);
    }

    fn adts(len: usize) -> Vec<u8> {
        let mut v = vec![0xFF, 0xF1, 0x4C, 0x80 | (len >> 11) as u8, (len >> 3) as u8, ((len & 7) as u8) << 5 | 0x1F, 0xFC];
        v.resize(len, 0x11);
        v
    }

    #[test]
    fn adts_frame_lengths_must_tile_the_frame() {
        let mut p = [0u8; PREFIX_MAX];
        assert_eq!(prefix(Codec::AacAdts, &adts(100), NO_TRIM, &mut p).unwrap(), 0);
        let two = [adts(100), adts(57)].concat();
        assert_eq!(prefix(Codec::AacAdts, &two, NO_TRIM, &mut p).unwrap(), 0);
        let mut short = adts(100);
        short.pop();
        assert!(matches!(prefix(Codec::AacAdts, &short, NO_TRIM, &mut p), Err(Error::BadFrame(_))), "one byte short");
        let mut long = adts(100);
        long.push(0);
        assert!(matches!(prefix(Codec::AacAdts, &long, NO_TRIM, &mut p), Err(Error::BadFrame(_))), "a trailing byte");
        let mut zero = adts(100);
        zero[3] &= 0xFC;
        zero[4] = 0;
        zero[5] &= 0x1F;
        assert!(matches!(prefix(Codec::AacAdts, &zero, NO_TRIM, &mut p), Err(Error::BadFrame(_))), "frame_length 0");
        // a header with a CRC is 9 bytes, so a frame_length of 8 does not hold it
        let mut crc = adts(8);
        crc[1] = 0xF0;
        assert!(matches!(prefix(Codec::AacAdts, &crc, NO_TRIM, &mut p), Err(Error::BadFrame(_))));
    }

    #[test]
    fn mpeg_audio_versions_match_the_stream_type() {
        let mut p = [0u8; PREFIX_MAX];
        let (mpeg1, mpeg2, mpeg25, reserved) = ([0xFF, 0xFB, 0x90], [0xFF, 0xF3, 0x90], [0xFF, 0xE3, 0x90], [0xFF, 0xEB, 0x90]);
        let high = Codec::MpegAudio { low_sample_rate: false };
        let low = Codec::MpegAudio { low_sample_rate: true };
        assert!(prefix(high, &mpeg1, NO_TRIM, &mut p).is_ok());
        assert!(prefix(low, &mpeg2, NO_TRIM, &mut p).is_ok());
        assert!(prefix(low, &mpeg25, NO_TRIM, &mut p).is_ok());
        assert!(prefix(low, &mpeg1, NO_TRIM, &mut p).is_err());
        assert!(prefix(high, &mpeg2, NO_TRIM, &mut p).is_err());
        assert!(prefix(high, &reserved, NO_TRIM, &mut p).is_err());
        assert!(prefix(high, &[0xFF, 0xF9, 0x90], NO_TRIM, &mut p).is_err(), "layer 0");
    }

    #[test]
    fn eac3_needs_its_bsid() {
        let mut p = [0u8; PREFIX_MAX];
        assert!(prefix(Codec::Eac3, &[0x0B, 0x77, 0, 0x7F, 0x34, 16 << 3], NO_TRIM, &mut p).is_ok());
        assert!(prefix(Codec::Eac3, &[0x0B, 0x77, 0, 0x7F, 0x34, 8 << 3], NO_TRIM, &mut p).is_err(), "AC-3's bsid");
        assert!(prefix(Codec::Eac3, &[0x0B, 0x77, 0, 0x7F, 0x34], NO_TRIM, &mut p).is_err());
    }
}
