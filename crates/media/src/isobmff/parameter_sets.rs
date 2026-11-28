//! The `avcC` and `hvcC` records of a stream built from the parameter sets an encoder wrote, and
//! the fields of an H.264 or HEVC sequence parameter set the records repeat.

use crate::bitstream::{BitError, BitReader, unescape_rbsp};
use crate::isobmff::codec::{AvcConfig, HevcConfig, HevcNalArray};
use crate::isobmff::error::{Error, Result, invalid};

/// The fields of an H.264 sequence parameter set (ITU-T H.264 7.3.2.1.1) that an `avcC` record
/// repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AvcSps {
    pub profile_idc: u8,
    /// The `constraint_set` flags and reserved bits: the record's `profile_compatibility`.
    pub constraint_flags: u8,
    pub level_idc: u8,
    /// 1 for 4:2:0. An SPS of a profile without the field is 4:2:0.
    pub chroma_format_idc: u32,
    pub bit_depth_luma: u32,
    pub bit_depth_chroma: u32,
}

/// Whether an SPS of `profile_idc` carries the chroma format and bit depths (H.264 7.3.2.1.1,
/// and 144, the High 4:4:4 profile of the 2005 edition that ISO/IEC 14496-15 still lists).
fn has_chroma_info(profile_idc: u8) -> bool {
    matches!(profile_idc, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135 | 144)
}

/// Refuse a chroma format or bit depths a record's 2-bit and 3-bit fields cannot hold.
fn check_record_fields(chroma_format_idc: u32, bit_depth_luma: u32, bit_depth_chroma: u32) -> Result<()> {
    if chroma_format_idc > 3 {
        return invalid(format!("chroma_format_idc {chroma_format_idc}"));
    }
    for depth in [bit_depth_luma, bit_depth_chroma] {
        if !(8..=15).contains(&depth) {
            return invalid(format!("a bit depth of {depth}, outside the 8 to 15 a record can hold"));
        }
    }
    Ok(())
}

impl AvcSps {
    /// Read the SPS NAL unit `nal`: its one-byte header, then its payload with emulation
    /// prevention bytes, without a start code.
    pub fn parse(nal: &[u8]) -> Result<Self> {
        if nal.first().map(|b| b & 0x1F) != Some(7) {
            return invalid("not an H.264 SPS NAL unit");
        }
        let rbsp = unescape_rbsp(nal.get(1..).unwrap_or_default());
        Self::read(&mut BitReader::new(&rbsp)).map_err(|e| bit_error(e, "H.264 SPS"))
    }

    fn read(r: &mut BitReader) -> crate::bitstream::Result<Self> {
        let profile_idc = r.read_bits(8)? as u8;
        let constraint_flags = r.read_bits(8)? as u8;
        let level_idc = r.read_bits(8)? as u8;
        let _seq_parameter_set_id = r.read_ue()?;
        let (mut chroma_format_idc, mut bit_depth_luma, mut bit_depth_chroma) = (1, 8, 8);
        if has_chroma_info(profile_idc) {
            chroma_format_idc = r.read_ue()?;
            if chroma_format_idc == 3 {
                let _separate_colour_plane_flag = r.read_bit()?;
            }
            bit_depth_luma = r.read_ue_plus(8)?;
            bit_depth_chroma = r.read_ue_plus(8)?;
        }
        Ok(AvcSps { profile_idc, constraint_flags, level_idc, chroma_format_idc, bit_depth_luma, bit_depth_chroma })
    }
}

/// The fields of an HEVC sequence parameter set (ITU-T H.265 7.3.2.2.1 and 7.3.3) that an `hvcC`
/// record repeats, and the picture size after the conformance window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HevcSps {
    pub max_sub_layers_minus1: u8,
    pub temporal_id_nesting: bool,
    pub profile_space: u8,
    pub tier: bool,
    pub profile_idc: u8,
    pub profile_compatibility_flags: u32,
    /// The 48 bits after the compatibility flags, `general_progressive_source_flag` first.
    pub constraint_indicator_flags: u64,
    pub level_idc: u8,
    pub chroma_format_idc: u32,
    pub bit_depth_luma: u32,
    pub bit_depth_chroma: u32,
    /// Width and height in luma samples after the conformance window.
    pub picture_size: (u32, u32),
}

impl HevcSps {
    /// Read the SPS NAL unit `nal`: its two-byte header, then its payload with emulation
    /// prevention bytes, without a start code.
    pub fn parse(nal: &[u8]) -> Result<Self> {
        if hevc_type(nal) != Some(33) {
            return invalid("not an HEVC SPS NAL unit");
        }
        let rbsp = unescape_rbsp(nal.get(2..).unwrap_or_default());
        Self::read(&mut BitReader::new(&rbsp)).map_err(|e| bit_error(e, "HEVC SPS"))?
    }

    fn read(r: &mut BitReader) -> crate::bitstream::Result<Result<Self>> {
        let _sps_video_parameter_set_id = r.read_bits(4)?;
        let max_sub_layers_minus1 = r.read_bits(3)? as u8;
        let temporal_id_nesting = r.read_bit()?;
        let profile_space = r.read_bits(2)? as u8;
        let tier = r.read_bit()?;
        let profile_idc = r.read_bits(5)? as u8;
        let profile_compatibility_flags = r.read_bits(32)?;
        let constraint_indicator_flags = r.read_bits_u64(48)?;
        let level_idc = r.read_bits(8)? as u8;
        // H.265 7.4.3.2.1: at most 7 temporal sub-layers, which the record's 3-bit count holds
        if max_sub_layers_minus1 > 6 {
            return Ok(invalid(format!("sps_max_sub_layers_minus1 {max_sub_layers_minus1}")));
        }
        // H.265 7.3.3: the sub-layer present flags, padded to eight pairs, then each present
        // sub-layer's profile (88 bits) and level (8 bits)
        let subs = usize::from(max_sub_layers_minus1);
        let mut present = Vec::with_capacity(subs);
        for _ in 0..subs {
            present.push((r.read_bit()?, r.read_bit()?));
        }
        if subs > 0 {
            r.skip(2 * (8 - subs))?;
        }
        for (profile, level) in present {
            if profile {
                r.skip(88)?;
            }
            if level {
                r.skip(8)?;
            }
        }
        let _sps_seq_parameter_set_id = r.read_ue()?;
        let chroma_format_idc = r.read_ue()?;
        let separate_colour_planes = chroma_format_idc == 3 && r.read_bit()?;
        let width = r.read_ue()?;
        let height = r.read_ue()?;
        let (mut left, mut right, mut top, mut bottom) = (0, 0, 0, 0);
        if r.read_bit()? {
            (left, right, top, bottom) = (r.read_ue()?, r.read_ue()?, r.read_ue()?, r.read_ue()?);
        }
        let bit_depth_luma = r.read_ue_plus(8)?;
        let bit_depth_chroma = r.read_ue_plus(8)?;
        // H.265 Table 6-1: the conformance window counts chroma samples
        let (sub_width, sub_height) = match (chroma_format_idc, separate_colour_planes) {
            (1, _) => (2, 2),
            (2, _) => (2, 1),
            _ => (1, 1),
        };
        let crop_width = left.saturating_add(right).saturating_mul(sub_width);
        let crop_height = top.saturating_add(bottom).saturating_mul(sub_height);
        if crop_width >= width || crop_height >= height {
            return Ok(invalid(format!("the HEVC SPS crops its {width}x{height} pictures to nothing")));
        }
        Ok(Ok(HevcSps {
            max_sub_layers_minus1,
            temporal_id_nesting,
            profile_space,
            tier,
            profile_idc,
            profile_compatibility_flags,
            constraint_indicator_flags,
            level_idc,
            chroma_format_idc,
            bit_depth_luma,
            bit_depth_chroma,
            picture_size: (width - crop_width, height - crop_height),
        }))
    }
}

/// The error for a read of the parameter set `what` that ran past its end or met an Exp-Golomb
/// code too long for its field.
fn bit_error(e: BitError, what: &'static str) -> Error {
    match e {
        BitError::Eof => Error::Truncated(what),
        other => Error::Invalid(format!("{other} in the {what}")),
    }
}

/// The NAL unit type of an HEVC NAL unit, or `None` when it is shorter than its header.
fn hevc_type(nal: &[u8]) -> Option<u8> {
    match nal {
        [b, _, ..] => Some((b >> 1) & 0x3F),
        _ => None,
    }
}

/// Refuse a NAL unit a record cannot hold: one with nothing after its header, or longer than the
/// record's 16-bit length.
fn check_length(nal: &[u8], header: usize, name: &str) -> Result<()> {
    if nal.len() <= header {
        return invalid(format!("the {name} is empty"));
    }
    if u16::try_from(nal.len()).is_err() {
        return invalid(format!("the {name} is longer than a record's 65,535 bytes"));
    }
    Ok(())
}

/// A NAL length prefix size a record can declare.
fn check_length_size(length_size: u8) -> Result<()> {
    if matches!(length_size, 1 | 2 | 4) { Ok(()) } else { invalid(format!("a NAL length size of {length_size} bytes")) }
}

impl AvcConfig {
    /// The record of a stream with the one SPS and the one PPS an encoder wrote (NAL units with
    /// their headers, without start codes), with `length_size` bytes (1, 2 or 4) before each NAL
    /// unit in the samples. The profile, compatibility flags and level come from the SPS, and for
    /// every profile except Baseline (66), Main (77) and Extended (88) so do the chroma format and
    /// bit depths that ISO/IEC 14496-15 5.3.2.1 adds after the PPS list. An SPS of a profile that
    /// H.264 does not define is refused.
    pub fn from_parameter_sets(sps: &[u8], pps: &[u8], length_size: u8) -> Result<Self> {
        check_length_size(length_size)?;
        for (nal, nal_type, name) in [(sps, 7u8, "SPS"), (pps, 8, "PPS")] {
            if nal.first().map(|b| b & 0x1F) != Some(nal_type) {
                return invalid(format!("not an H.264 {name} NAL unit"));
            }
            check_length(nal, 1, name)?;
        }
        let s = AvcSps::parse(sps)?;
        let ext = if matches!(s.profile_idc, 66 | 77 | 88) {
            Vec::new()
        } else if has_chroma_info(s.profile_idc) {
            check_record_fields(s.chroma_format_idc, s.bit_depth_luma, s.bit_depth_chroma)?;
            vec![
                0xFC | (s.chroma_format_idc as u8 & 3),
                0xF8 | (s.bit_depth_luma.saturating_sub(8) as u8 & 7),
                0xF8 | (s.bit_depth_chroma.saturating_sub(8) as u8 & 7),
                // numOfSequenceParameterSetExt
                0,
            ]
        } else {
            return invalid(format!("an SPS of the unknown H.264 profile_idc {}", s.profile_idc));
        };
        Ok(AvcConfig {
            profile: s.profile_idc,
            compatibility: s.constraint_flags,
            level: s.level_idc,
            length_size,
            sps: vec![sps.to_vec()],
            pps: vec![pps.to_vec()],
            ext,
        })
    }
}

impl HevcConfig {
    /// The record of a stream with the one VPS, SPS and PPS an encoder wrote (NAL units with their
    /// headers, without start codes), with `length_size` bytes (1, 2 or 4) before each NAL unit in
    /// the samples. The profile, tier, compatibility and constraint flags, level, chroma format,
    /// bit depths and temporal layers come from the SPS. Each array is marked complete, so the
    /// samples carry no parameter sets. The spatial segmentation, parallelism and frame rate
    /// fields are 0, unspecified.
    pub fn from_parameter_sets(vps: &[u8], sps: &[u8], pps: &[u8], length_size: u8) -> Result<Self> {
        check_length_size(length_size)?;
        for (nal, nal_type, name) in [(vps, 32u8, "VPS"), (sps, 33, "SPS"), (pps, 34, "PPS")] {
            if hevc_type(nal) != Some(nal_type) {
                return invalid(format!("not an HEVC {name} NAL unit"));
            }
            check_length(nal, 2, name)?;
        }
        let s = HevcSps::parse(sps)?;
        check_record_fields(s.chroma_format_idc, s.bit_depth_luma, s.bit_depth_chroma)?;
        Ok(HevcConfig {
            general_profile_space: s.profile_space,
            general_tier_flag: s.tier,
            general_profile_idc: s.profile_idc,
            general_profile_compatibility_flags: s.profile_compatibility_flags,
            general_constraint_indicator_flags: s.constraint_indicator_flags,
            general_level_idc: s.level_idc,
            min_spatial_segmentation_idc: 0,
            parallelism_type: 0,
            chroma_format_idc: s.chroma_format_idc as u8,
            bit_depth_luma: s.bit_depth_luma as u8,
            bit_depth_chroma: s.bit_depth_chroma as u8,
            avg_frame_rate: 0,
            constant_frame_rate: 0,
            num_temporal_layers: s.max_sub_layers_minus1.saturating_add(1),
            temporal_id_nested: s.temporal_id_nesting,
            length_size,
            arrays: [(vps, 32u8), (sps, 33), (pps, 34)]
                .into_iter()
                .map(|(nal, nal_type)| HevcNalArray { completeness: true, nal_type, nalus: vec![nal.to_vec()] })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitstream::{BitWriter, escape_rbsp};

    fn unhex(h: &str) -> Vec<u8> {
        (0..h.len()).step_by(2).filter_map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok()).collect()
    }

    /// A High profile SPS: profile 100, level 4.0, then seq_parameter_set_id 0, chroma_format_idc
    /// 1 and both bit depths 8.
    const HIGH: [u8; 5] = [0x67, 100, 0x00, 40, 0b1010_1100];
    const PPS: [u8; 3] = [0x68, 0xEE, 0x3C];

    // The parameter sets of a 1280x720 HEVC Main stream coded at 1280x736 and cropped, and of a
    // Main 10 stream of the same size.
    const VPS: &str = "40010c01ffff01600000030090000003000003005d9940c0000003004000000614";
    const SPS: &str = "42010101600000030090000003000003005da00280802e1f1396654a421191bff0c05a8080808a0000030002000003003010";
    const HEVC_PPS: &str = "4401c1937c0cc9";
    const SPS10: &str = "420101222000000300900000030000030078a00280802e1f12d96654a421191bff0c05a8488048a000000300200000030301";

    #[test]
    fn an_avc_record_reads_back_as_the_parameter_sets_it_was_built_from() {
        let c = AvcConfig::from_parameter_sets(&HIGH, &PPS, 4).unwrap();
        let back = AvcConfig::parse(&c.to_bytes()).unwrap();
        assert_eq!(back, c);
        assert_eq!((back.profile, back.level), (HIGH[1], HIGH[3]));
        assert_eq!(back.ext.len(), 4, "a High profile record carries the chroma format and bit depths");
        let main = [0x67, 77, 0x40, 31, 0x80];
        assert!(AvcConfig::from_parameter_sets(&main, &PPS, 4).unwrap().ext.is_empty());
    }

    #[test]
    fn the_avc_sps_is_read_after_its_emulation_prevention_bytes_are_removed() {
        // profile 100, constraint flags 0, level 0, then `00 00 03`: without the 03 removed the
        // chroma format and bit depths would come from it
        let escaped = [0x67, 100, 0x00, 0x00, 0x03, 0b1010_1100];
        assert_eq!(AvcSps::parse(&escaped).unwrap(), AvcSps::parse(&[0x67, 100, 0, 0, 0b1010_1100]).unwrap());
    }

    #[test]
    fn an_hevc_record_repeats_the_sps_and_reads_back() {
        let (v, s, p) = (unhex(VPS), unhex(SPS), unhex(HEVC_PPS));
        let sps = HevcSps::parse(&s).unwrap();
        assert_eq!(sps.picture_size, (1280, 720), "the conformance window crops the 736 coded rows");
        let c = HevcConfig::from_parameter_sets(&v, &s, &p, 4).unwrap();
        let bytes = c.to_bytes();
        // the profile, tier and level fields are the SPS's own 12 bytes after its first RBSP byte
        let rbsp = unescape_rbsp(&s[2..]);
        assert_eq!(bytes.get(1..13), rbsp.get(1..13));
        let back = HevcConfig::parse(&bytes).unwrap();
        assert_eq!(back, c);
        assert_eq!((back.vps(), back.sps(), back.pps()), (vec![&v[..]], vec![&s[..]], vec![&p[..]]));
        let ten = HevcSps::parse(&unhex(SPS10)).unwrap();
        assert!(ten.bit_depth_luma > sps.bit_depth_luma && ten.profile_idc != sps.profile_idc);
    }

    #[test]
    fn units_of_the_wrong_type_size_or_length_size_are_refused() {
        let (v, s, p) = (unhex(VPS), unhex(SPS), unhex(HEVC_PPS));
        assert!(AvcConfig::from_parameter_sets(&PPS, &HIGH, 4).is_err(), "swapped");
        assert!(AvcConfig::from_parameter_sets(&HIGH, &[0x68], 4).is_err(), "an empty PPS");
        let mut long = PPS.to_vec();
        long.resize(70_000, 0x55);
        assert!(AvcConfig::from_parameter_sets(&HIGH, &long, 4).is_err());
        assert!(HevcConfig::from_parameter_sets(&s, &v, &p, 4).is_err(), "swapped");
        assert!(HevcConfig::from_parameter_sets(&v, &s, &[0x44, 0x01], 4).is_err(), "an empty PPS");
        for size in [0, 3, 5] {
            assert!(AvcConfig::from_parameter_sets(&HIGH, &PPS, size).is_err(), "{size}");
            assert!(HevcConfig::from_parameter_sets(&v, &s, &p, size).is_err(), "{size}");
        }
    }

    #[test]
    fn truncated_and_flipped_parameter_sets_are_errors_or_records() {
        for n in 0..HIGH.len() {
            assert!(AvcSps::parse(&HIGH[..n]).is_err(), "SPS cut at {n}");
        }
        let (v, s, p) = (unhex(VPS), unhex(SPS), unhex(HEVC_PPS));
        // the fields the record needs end in the SPS's 24th byte
        for n in 0..24 {
            assert!(HevcConfig::from_parameter_sets(&v, &s[..n], &p, 4).is_err(), "SPS cut at {n}");
        }
        for byte in 0..s.len() {
            for bit in 0..8 {
                let mut m = s.clone();
                m[byte] ^= 1 << bit;
                if let Ok(c) = HevcConfig::from_parameter_sets(&v, &m, &p, 4) {
                    assert_eq!(HevcConfig::parse(&c.to_bytes()).unwrap().sps(), vec![&m[..]]);
                }
            }
        }
        for byte in 0..HIGH.len() {
            for bit in 0..8 {
                let mut m = HIGH;
                m[byte] ^= 1 << bit;
                if let Ok(c) = AvcConfig::from_parameter_sets(&m, &PPS, 4) {
                    assert_eq!(AvcConfig::parse(&c.to_bytes()).unwrap(), c);
                }
            }
        }
    }

    /// An HEVC Main SPS NAL unit with one sub-layer, 4:2:0 8-bit pictures of `size` and a
    /// conformance window of `crop` (left, right, top, bottom) chroma samples.
    fn hevc_sps(size: (u32, u32), crop: (u32, u32, u32, u32)) -> Vec<u8> {
        hevc_sps_at(size, crop, 0, 0)
    }

    /// [`hevc_sps`] with both bit depths `8 + depth_minus8` and `sub_layers_minus1` sub-layers
    /// after the first, none with a profile or level of its own.
    fn hevc_sps_at(size: (u32, u32), crop: (u32, u32, u32, u32), depth_minus8: u32, sub_layers_minus1: u32) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write_bits(0, 4); // sps_video_parameter_set_id
        w.write_bits(sub_layers_minus1, 3); // sps_max_sub_layers_minus1
        w.write_bit(true); // sps_temporal_id_nesting_flag
        w.write_bits(0, 2); // general_profile_space
        w.write_bit(false); // general_tier_flag
        w.write_bits(1, 5); // general_profile_idc
        w.write_bits(0x6000_0000, 32);
        w.write_bits(0x9000, 16);
        w.write_bits(0, 32);
        w.write_bits(93, 8); // general_level_idc
        if sub_layers_minus1 > 0 {
            // the sub-layer profile and level present flags, padded to eight pairs
            w.write_bits(0, 2 * sub_layers_minus1);
            w.write_bits(0, 2 * (8 - sub_layers_minus1));
        }
        w.write_ue(0); // sps_seq_parameter_set_id
        w.write_ue(1); // chroma_format_idc
        w.write_ue(size.0);
        w.write_ue(size.1);
        w.write_bit(true); // conformance_window_flag
        for c in [crop.0, crop.1, crop.2, crop.3] {
            w.write_ue(c);
        }
        w.write_ue(depth_minus8); // bit_depth_luma_minus8
        w.write_ue(depth_minus8); // bit_depth_chroma_minus8
        w.rbsp_trailing();
        let mut nal = vec![0x42, 0x01];
        nal.extend(escape_rbsp(&w.finish()));
        nal
    }

    #[test]
    fn the_hevc_picture_size_is_the_size_inside_the_conformance_window() {
        let coded = (64, 48);
        let s = HevcSps::parse(&hevc_sps(coded, (1, 2, 0, 3))).unwrap();
        assert!(s.picture_size.0 < coded.0 && s.picture_size.1 < coded.1);
        assert_eq!(HevcSps::parse(&hevc_sps(coded, (0, 0, 0, 0))).unwrap().picture_size, coded);
        // a window as wide as the picture, in chroma samples, leaves nothing
        assert!(HevcSps::parse(&hevc_sps(coded, (16, 16, 0, 0))).is_err());
        assert!(HevcSps::parse(&hevc_sps(coded, (0, 0, u32::MAX - 1, 1))).is_err());
    }

    #[test]
    fn a_record_states_the_sps_chroma_format_and_depths_or_is_refused() {
        // profile 144 carries its chroma format: 4:4:4 (`00100`), separate planes 0, 8 bits
        let high444 = [0x67, 144, 0, 40, 0b1001_0001, 0b1000_0000];
        let c = AvcConfig::from_parameter_sets(&high444, &PPS, 4).unwrap();
        assert_eq!(c.ext.first().map(|b| b & 3), Some(3));
        // profile 244 (High 4:4:4 Predictive) at 4:2:0 (`010`) with both bit depths 10 (`011`)
        let high244 = [0x67, 244, 0, 40, 0b1010_0110, 0b1100_0000];
        assert_eq!(AvcConfig::from_parameter_sets(&high244, &PPS, 4).unwrap().ext, [0xFD, 0xFA, 0xFA, 0]);
        // only the Baseline, Main and Extended profiles leave the fields out
        for profile in [66, 77, 88] {
            let sps = [0x67, profile, 0, 30, 0x80];
            assert!(AvcConfig::from_parameter_sets(&sps, &PPS, 4).unwrap().ext.is_empty(), "profile {profile}");
        }
        // a profile H.264 does not define has no known place for the fields
        assert!(AvcConfig::from_parameter_sets(&[0x67, 1, 0, 30, 0x80], &PPS, 4).is_err());
        // HEVC allows bit depths up to 16; the record's 3-bit fields hold 8 to 15
        let (v, p) = (unhex(VPS), unhex(HEVC_PPS));
        let ten = hevc_sps_at((64, 48), (0, 0, 0, 0), 2, 0);
        assert_eq!(HevcConfig::from_parameter_sets(&v, &ten, &p, 4).unwrap().bit_depth_luma, 10);
        let sixteen = hevc_sps_at((64, 48), (0, 0, 0, 0), 8, 0);
        assert!(HevcSps::parse(&sixteen).is_ok());
        assert!(HevcConfig::from_parameter_sets(&v, &sixteen, &p, 4).is_err());
    }

    #[test]
    fn the_temporal_layer_count_is_the_sps_count_and_an_eighth_layer_is_refused() {
        let (v, p) = (unhex(VPS), unhex(HEVC_PPS));
        let six = hevc_sps_at((64, 48), (0, 0, 0, 0), 0, 6);
        let c = HevcConfig::from_parameter_sets(&v, &six, &p, 4).unwrap();
        assert_eq!(HevcConfig::parse(&c.to_bytes()).unwrap().num_temporal_layers, 7);
        assert!(HevcSps::parse(&hevc_sps_at((64, 48), (0, 0, 0, 0), 0, 7)).is_err());
    }
}
