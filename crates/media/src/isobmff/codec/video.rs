//! Visual sample entry parameters and the video codec configuration records.

use super::*;

/// Fields common to all visual sample entries plus the optional descriptive boxes.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct VideoParams {
    /// Coded width in pixels from the sample entry.
    pub width: u16,
    /// Coded height in pixels from the sample entry.
    pub height: u16,
    /// Horizontal/vertical resolution in pixels per inch (16.16 fixed point; 72 dpi = 0x0048_0000).
    pub horiz_resolution: u32,
    /// Vertical resolution in pixels per inch (16.16 fixed point).
    pub vert_resolution: u32,
    /// Frames per sample; 1 in practice.
    pub frame_count: u16,
    /// Compressor name from the sample entry's 32-byte Pascal string.
    pub compressor_name: String,
    /// The sample entry's `depth` field, as read.
    pub depth: u16,
    /// QuickTime `vendor` / version fields (zero in ISO files).
    pub vendor: FourCc,
    /// The QuickTime `temporalQuality` field, as read.
    pub temporal_quality: u32,
    /// The QuickTime `spatialQuality` field, as read.
    pub spatial_quality: u32,
    /// `colr` box.
    pub color: Option<ColorInfo>,
    /// QuickTime `logs`: the CoreMedia log transfer-function identifier (for example
    /// `com.apple.rec2020.apple-log`). Non-empty UTF-8 without NUL, at most 255 bytes; anything
    /// else, or a second `logs` box, reads as `None`.
    pub log_transfer_function: Option<String>,
    /// `pasp`: (hSpacing, vSpacing).
    pub pixel_aspect: Option<(u32, u32)>,
    /// `clap` box.
    pub clean_aperture: Option<CleanAperture>,
    /// `fiel` box.
    pub field_info: Option<FieldInfo>,
    /// QuickTime `gama` (16.16 fixed point).
    pub gamma: Option<u32>,
    /// `mdcv`: mastering display color volume (SMPTE ST 2086).
    pub mastering_display: Option<MasteringDisplay>,
    /// `clli`: content light level (MaxCLL, MaxFALL in cd/m²; 0 = unknown).
    pub content_light: Option<(u16, u16)>,
}

/// SMPTE ST 2086 mastering display metadata, in the units of the `mdcv` box and the H.264/HEVC
/// SEI: chromaticities in 0.00002 steps (order G, B, R as the SEI), luminance in 0.0001 cd/m².
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MasteringDisplay {
    /// Chromaticity (x, y) of the green, blue and red primaries, in 0.00002 steps.
    pub primaries: [(u16, u16); 3],
    /// Chromaticity (x, y) of the white point, in 0.00002 steps.
    pub white_point: (u16, u16),
    /// Peak display luminance in 0.0001 cd/m².
    pub max_luminance: u32,
    /// Minimum display luminance in 0.0001 cd/m².
    pub min_luminance: u32,
}

impl MasteringDisplay {
    /// BT.2020 primaries, D65, with the given peak and black in cd/m².
    pub fn bt2020(max_nits: f64, min_nits: f64) -> Self {
        let q = |v: f64| (v / 0.00002).round() as u16;
        MasteringDisplay {
            primaries: [(q(0.170), q(0.797)), (q(0.131), q(0.046)), (q(0.708), q(0.292))],
            white_point: (q(0.3127), q(0.3290)),
            max_luminance: (max_nits * 10_000.0).round() as u32,
            min_luminance: (min_nits * 10_000.0).round() as u32,
        }
    }
    /// Peak luminance in cd/m².
    pub fn max_nits(&self) -> f64 {
        self.max_luminance as f64 / 10_000.0
    }
    /// The 24-byte payload shared by `mdcv` and the SEI message.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(24);
        for (x, y) in self.primaries {
            v.extend_from_slice(&x.to_be_bytes());
            v.extend_from_slice(&y.to_be_bytes());
        }
        v.extend_from_slice(&self.white_point.0.to_be_bytes());
        v.extend_from_slice(&self.white_point.1.to_be_bytes());
        v.extend_from_slice(&self.max_luminance.to_be_bytes());
        v.extend_from_slice(&self.min_luminance.to_be_bytes());
        v
    }
    /// Parses the 24-byte `mdcv` payload. Returns `None` if `p` is shorter than 24 bytes.
    pub fn parse(p: &[u8]) -> Option<Self> {
        if p.len() < 24 {
            return None;
        }
        let u16_at = |i: usize| u16::from_be_bytes([p[i], p[i + 1]]);
        let u32_at = |i: usize| u32::from_be_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]);
        Some(MasteringDisplay {
            primaries: [(u16_at(0), u16_at(2)), (u16_at(4), u16_at(6)), (u16_at(8), u16_at(10))],
            white_point: (u16_at(12), u16_at(14)),
            max_luminance: u32_at(16),
            min_luminance: u32_at(20),
        })
    }
}

/// `colr` box contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorInfo {
    /// ISO `nclx` (with full-range flag).
    Nclx {
        /// Colour primaries code (ITU-T H.273 `ColourPrimaries`).
        primaries: u16,
        /// Transfer characteristics code (ITU-T H.273).
        transfer: u16,
        /// Matrix coefficients code (ITU-T H.273).
        matrix: u16,
        /// True when sample values use the full range rather than the video (limited) range.
        full_range: bool,
    },
    /// QuickTime `nclc`.
    Nclc {
        /// QuickTime colour primaries index.
        primaries: u16,
        /// QuickTime transfer function index.
        transfer: u16,
        /// QuickTime matrix index.
        matrix: u16,
    },
    /// Embedded ICC profile (`prof` restricted/unrestricted `rICC`).
    Icc {
        /// Colour type: `prof` or `rICC`.
        kind: FourCc,
        /// Raw ICC profile bytes.
        profile: Vec<u8>,
    },
}

/// `clap` clean aperture (all rationals num/den; offsets are signed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanAperture {
    /// Width as a fraction (numerator, denominator) in pixels.
    pub width: (u32, u32),
    /// Height as a fraction (numerator, denominator) in pixels.
    pub height: (u32, u32),
    /// Horizontal offset of the aperture centre from the image centre, as (signed numerator, denominator).
    pub horiz_offset: (i32, u32),
    /// Vertical offset of the aperture centre from the image centre, as (signed numerator, denominator).
    pub vert_offset: (i32, u32),
}

/// QuickTime `fiel`: field count (1 progressive, 2 interlaced) and ordering detail
/// (0, 1 = top first, 6 = bottom first, 9 / 14 per the QuickTime spec).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldInfo {
    /// Field count: 1 for progressive, 2 for interlaced.
    pub fields: u8,
    /// Field ordering code; see the type docs.
    pub detail: u8,
}

/// `avcC` — AVCDecoderConfigurationRecord (ISO/IEC 14496-15 §5.3.3).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AvcConfig {
    /// `profile_idc` of the stream (AVCProfileIndication).
    pub profile: u8,
    /// The `constraint_set` flags byte (profile_compatibility).
    pub compatibility: u8,
    /// `level_idc` of the stream (AVCLevelIndication).
    pub level: u8,
    /// NAL length field size in bytes (1, 2 or 4).
    pub length_size: u8,
    /// Sequence parameter set NAL units, without start codes.
    pub sps: Vec<Vec<u8>>,
    /// Picture parameter set NAL units, without start codes.
    pub pps: Vec<Vec<u8>>,
    /// Bytes after the PPS list (high-profile extension: chroma format, bit depths, SPS-ext).
    pub ext: Vec<u8>,
}

impl AvcConfig {
    /// Build from parameter sets (NAL units without start codes); profile/level taken from the first SPS.
    pub fn new(sps: Vec<Vec<u8>>, pps: Vec<Vec<u8>>, length_size: u8) -> Self {
        let (profile, compatibility, level) = sps.first().filter(|s| s.len() >= 4).map(|s| (s[1], s[2], s[3])).unwrap_or((0, 0, 0));
        AvcConfig { profile, compatibility, level, length_size, sps, pps, ext: Vec::new() }
    }

    /// Parses the payload of the box (the bytes after its header). Fails with [`Error::Truncated`] when a field runs past the end of `data`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut c = Cur::labeled(data, "avcC");
        let _version = c.u8()?;
        let profile = c.u8()?;
        let compatibility = c.u8()?;
        let level = c.u8()?;
        let length_size = (c.u8()? & 3) + 1;
        let nsps = (c.u8()? & 0x1F) as usize;
        let mut sps = Vec::with_capacity(nsps);
        for _ in 0..nsps {
            let n = c.u16()? as usize;
            sps.push(c.bytes(n)?.to_vec());
        }
        let npps = c.u8()? as usize;
        let mut pps = Vec::with_capacity(npps);
        for _ in 0..npps {
            let n = c.u16()? as usize;
            pps.push(c.bytes(n)?.to_vec());
        }
        Ok(AvcConfig { profile, compatibility, level, length_size, sps, pps, ext: c.rest().to_vec() })
    }

    /// Serializes to the payload of an `avcC` box (without the box header).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = BoxBuf::new();
        b.u8(1);
        b.u8(self.profile);
        b.u8(self.compatibility);
        b.u8(self.level);
        b.u8(0xFC | (self.length_size.clamp(1, 4) - 1));
        b.u8(0xE0 | (self.sps.len().min(31) as u8));
        for s in self.sps.iter().take(31) {
            b.u16(s.len() as u16);
            b.bytes(s);
        }
        b.u8(self.pps.len().min(255) as u8);
        for p in self.pps.iter().take(255) {
            b.u16(p.len() as u16);
            b.bytes(p);
        }
        b.bytes(&self.ext);
        b.buf
    }
}

/// One NAL array of an `hvcC` record.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct HevcNalArray {
    /// True when the array holds all NAL units of this type for the stream (`array_completeness`).
    pub completeness: bool,
    /// NAL unit type of every unit in the array (32 = VPS, 33 = SPS, 34 = PPS).
    pub nal_type: u8,
    /// NAL units, without start codes.
    pub nalus: Vec<Vec<u8>>,
}

/// `hvcC` — HEVCDecoderConfigurationRecord (ISO/IEC 14496-15 §8.3.3).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct HevcConfig {
    /// `general_profile_space` (2 bits).
    pub general_profile_space: u8,
    /// `general_tier_flag`: true for the high tier.
    pub general_tier_flag: bool,
    /// `general_profile_idc` (5 bits).
    pub general_profile_idc: u8,
    /// 32 flags, one per profile id, set for each profile the stream conforms to.
    pub general_profile_compatibility_flags: u32,
    /// 48 bits of general constraint indicator flags.
    pub general_constraint_indicator_flags: u64,
    /// The `general_level_idc` byte, as read.
    pub general_level_idc: u8,
    /// `min_spatial_segmentation_idc` (12 bits); 0 if unknown.
    pub min_spatial_segmentation_idc: u16,
    /// `parallelismType` (2 bits).
    pub parallelism_type: u8,
    /// `chroma_format_idc`: 0 monochrome, 1 for 4:2:0, 2 for 4:2:2, 3 for 4:4:4.
    pub chroma_format_idc: u8,
    /// Luma bit depth (8 or more).
    pub bit_depth_luma: u8,
    /// Chroma bit depth (8 or more).
    pub bit_depth_chroma: u8,
    /// The `avgFrameRate` field, as read.
    pub avg_frame_rate: u16,
    /// The 2-bit `constantFrameRate` field, as read.
    pub constant_frame_rate: u8,
    /// Number of temporal layers; 0 if unknown.
    pub num_temporal_layers: u8,
    /// True when all temporal layers are nested (`temporalIdNested`).
    pub temporal_id_nested: bool,
    /// NAL length field size in bytes (1, 2 or 4).
    pub length_size: u8,
    /// NAL unit arrays (VPS, SPS, PPS, SEI).
    pub arrays: Vec<HevcNalArray>,
}

impl HevcConfig {
    /// Parses the payload of the box (the bytes after its header). Fails with [`Error::Truncated`] when a field runs past the end of `data`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut c = Cur::labeled(data, "hvcC");
        let _version = c.u8()?;
        let b = c.u8()?;
        let compat = c.u32()?;
        let hi = c.u16()? as u64;
        let lo = c.u32()? as u64;
        let level = c.u8()?;
        let min_seg = c.u16()? & 0x0FFF;
        let par = c.u8()? & 3;
        let chroma = c.u8()? & 3;
        let bdl = (c.u8()? & 7) + 8;
        let bdc = (c.u8()? & 7) + 8;
        let afr = c.u16()?;
        let x = c.u8()?;
        let narrays = c.u8()? as usize;
        let mut arrays = Vec::new();
        for _ in 0..narrays {
            let t = c.u8()?;
            let n = c.u16()? as usize;
            c.check_count(n, 2)?;
            let mut nalus = Vec::with_capacity(n);
            for _ in 0..n {
                let len = c.u16()? as usize;
                nalus.push(c.bytes(len)?.to_vec());
            }
            arrays.push(HevcNalArray { completeness: t & 0x80 != 0, nal_type: t & 0x3F, nalus });
        }
        Ok(HevcConfig {
            general_profile_space: b >> 6,
            general_tier_flag: b & 0x20 != 0,
            general_profile_idc: b & 0x1F,
            general_profile_compatibility_flags: compat,
            general_constraint_indicator_flags: (hi << 32) | lo,
            general_level_idc: level,
            min_spatial_segmentation_idc: min_seg,
            parallelism_type: par,
            chroma_format_idc: chroma,
            bit_depth_luma: bdl,
            bit_depth_chroma: bdc,
            avg_frame_rate: afr,
            constant_frame_rate: x >> 6,
            num_temporal_layers: (x >> 3) & 7,
            temporal_id_nested: x & 4 != 0,
            length_size: (x & 3) + 1,
            arrays,
        })
    }

    /// Serializes to the payload of an `hvcC` box (without the box header).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = BoxBuf::new();
        b.u8(1);
        b.u8((self.general_profile_space << 6) | ((self.general_tier_flag as u8) << 5) | (self.general_profile_idc & 0x1F));
        b.u32(self.general_profile_compatibility_flags);
        b.u16((self.general_constraint_indicator_flags >> 32) as u16);
        b.u32(self.general_constraint_indicator_flags as u32);
        b.u8(self.general_level_idc);
        b.u16(0xF000 | (self.min_spatial_segmentation_idc & 0x0FFF));
        b.u8(0xFC | (self.parallelism_type & 3));
        b.u8(0xFC | (self.chroma_format_idc & 3));
        b.u8(0xF8 | (self.bit_depth_luma.saturating_sub(8) & 7));
        b.u8(0xF8 | (self.bit_depth_chroma.saturating_sub(8) & 7));
        b.u16(self.avg_frame_rate);
        b.u8((self.constant_frame_rate << 6)
            | ((self.num_temporal_layers & 7) << 3)
            | ((self.temporal_id_nested as u8) << 2)
            | (self.length_size.clamp(1, 4) - 1));
        b.u8(self.arrays.len() as u8);
        for a in &self.arrays {
            b.u8(((a.completeness as u8) << 7) | (a.nal_type & 0x3F));
            b.u16(a.nalus.len() as u16);
            for n in &a.nalus {
                b.u16(n.len() as u16);
                b.bytes(n);
            }
        }
        b.buf
    }

    fn nalus_of(&self, t: u8) -> Vec<&[u8]> {
        self.arrays.iter().filter(|a| a.nal_type == t).flat_map(|a| a.nalus.iter().map(|n| n.as_slice())).collect()
    }
    /// Video parameter set NAL units from all arrays of type 32, without start codes.
    pub fn vps(&self) -> Vec<&[u8]> {
        self.nalus_of(32)
    }
    /// Sequence parameter set NAL units from all arrays of type 33, without start codes.
    pub fn sps(&self) -> Vec<&[u8]> {
        self.nalus_of(33)
    }
    /// Picture parameter set NAL units from all arrays of type 34, without start codes.
    pub fn pps(&self) -> Vec<&[u8]> {
        self.nalus_of(34)
    }
}

/// `av1C` — AV1CodecConfigurationRecord.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Av1Config {
    /// `seq_profile` (3 bits).
    pub seq_profile: u8,
    /// `seq_level_idx_0` (5 bits).
    pub seq_level_idx_0: u8,
    /// `seq_tier_0`: true for the high tier.
    pub seq_tier_0: bool,
    /// `high_bitdepth`: set for 10-bit (or 12-bit with `twelve_bit`).
    pub high_bitdepth: bool,
    /// `twelve_bit`: with `high_bitdepth`, selects 12-bit.
    pub twelve_bit: bool,
    /// True for monochrome video.
    pub monochrome: bool,
    /// `chroma_subsampling_x`.
    pub chroma_subsampling_x: bool,
    /// `chroma_subsampling_y`.
    pub chroma_subsampling_y: bool,
    /// The 2-bit `chroma_sample_position` field, as read.
    pub chroma_sample_position: u8,
    /// `initial_presentation_delay_minus_one` (4 bits), when signalled.
    pub initial_presentation_delay_minus_one: Option<u8>,
    /// Configuration OBUs (usually the sequence header) stored after the fixed fields.
    pub config_obus: Vec<u8>,
}

impl Av1Config {
    /// Parses the payload of the box (the bytes after its header). Fails with [`Error::Truncated`] when a field runs past the end of `data`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut c = Cur::labeled(data, "av1C");
        let m = c.u8()?;
        if m & 0x80 == 0 {
            return Err(Error::Invalid("av1C marker bit not set".into()));
        }
        let a = c.u8()?;
        let b = c.u8()?;
        let d = c.u8()?;
        Ok(Av1Config {
            seq_profile: a >> 5,
            seq_level_idx_0: a & 0x1F,
            seq_tier_0: b & 0x80 != 0,
            high_bitdepth: b & 0x40 != 0,
            twelve_bit: b & 0x20 != 0,
            monochrome: b & 0x10 != 0,
            chroma_subsampling_x: b & 0x08 != 0,
            chroma_subsampling_y: b & 0x04 != 0,
            chroma_sample_position: b & 3,
            initial_presentation_delay_minus_one: if d & 0x10 != 0 { Some(d & 0x0F) } else { None },
            config_obus: c.rest().to_vec(),
        })
    }
    /// Serializes to the payload of an `av1C` box (without the box header).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = BoxBuf::new();
        b.u8(0x81);
        b.u8((self.seq_profile << 5) | (self.seq_level_idx_0 & 0x1F));
        b.u8(((self.seq_tier_0 as u8) << 7)
            | ((self.high_bitdepth as u8) << 6)
            | ((self.twelve_bit as u8) << 5)
            | ((self.monochrome as u8) << 4)
            | ((self.chroma_subsampling_x as u8) << 3)
            | ((self.chroma_subsampling_y as u8) << 2)
            | (self.chroma_sample_position & 3));
        b.u8(match self.initial_presentation_delay_minus_one {
            Some(d) => 0x10 | (d & 0x0F),
            None => 0,
        });
        b.bytes(&self.config_obus);
        b.buf
    }
}

/// `vpcC` — VP codec configuration (version 1).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct VpcConfig {
    /// VP profile (0 to 3).
    pub profile: u8,
    /// The `level` byte, as read.
    pub level: u8,
    /// Bit depth (8, 10 or 12).
    pub bit_depth: u8,
    /// `chromaSubsampling` code: 0 and 1 are 4:2:0, 2 is 4:2:2, 3 is 4:4:4.
    pub chroma_subsampling: u8,
    /// True for full-range sample values, false for video range.
    pub full_range: bool,
    /// Colour primaries code (ITU-T H.273).
    pub colour_primaries: u8,
    /// Transfer characteristics code (ITU-T H.273).
    pub transfer_characteristics: u8,
    /// Matrix coefficients code (ITU-T H.273).
    pub matrix_coefficients: u8,
    /// Codec initialization data (`codecInitializationData`); empty for VP8 and VP9.
    pub codec_init: Vec<u8>,
}

impl VpcConfig {
    /// Parses the payload of the box (the bytes after its header). Fails with [`Error::Truncated`] when a field runs past the end of `data`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut c = Cur::labeled(data, "vpcC");
        let (version, _) = c.full_header()?;
        if version == 0 {
            // Version 0 draft layout: profile, level, bitDepth(4)|colorSpace(4), chroma(4)|transfer(3)|range(1)
            let profile = c.u8()?;
            let level = c.u8()?;
            let a = c.u8()?;
            let b = c.u8()?;
            return Ok(VpcConfig { profile, level, bit_depth: a >> 4, chroma_subsampling: b >> 4, full_range: b & 1 != 0, ..Default::default() });
        }
        let profile = c.u8()?;
        let level = c.u8()?;
        let a = c.u8()?;
        let colour_primaries = c.u8()?;
        let transfer_characteristics = c.u8()?;
        let matrix_coefficients = c.u8()?;
        let n = c.u16()? as usize;
        let codec_init = c.bytes(n)?.to_vec();
        Ok(VpcConfig {
            profile,
            level,
            bit_depth: a >> 4,
            chroma_subsampling: (a >> 1) & 7,
            full_range: a & 1 != 0,
            colour_primaries,
            transfer_characteristics,
            matrix_coefficients,
            codec_init,
        })
    }
    /// Serializes to the payload of a `vpcC` box (without the box header).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = BoxBuf::new();
        b.u32(1 << 24);
        b.u8(self.profile);
        b.u8(self.level);
        b.u8((self.bit_depth << 4) | ((self.chroma_subsampling & 7) << 1) | self.full_range as u8);
        b.u8(self.colour_primaries);
        b.u8(self.transfer_characteristics);
        b.u8(self.matrix_coefficients);
        b.u16(self.codec_init.len() as u16);
        b.bytes(&self.codec_init);
        b.buf
    }
}

/// `apvC` — APVDecoderConfigurationBox (`FullBox('apvC', version = 0, flags = 0)` containing
/// `APVDecoderConfigurationRecord`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ApvConfig {
    /// APV profile code (`profile_idc`).
    pub profile_idc: u8,
    /// APV level code (`level_idc`).
    pub level_idc: u8,
    /// APV band code (`band_idc`).
    pub band_idc: u8,
    /// Frame width in pixels.
    pub frame_width: u32,
    /// Frame height in pixels.
    pub frame_height: u32,
    /// `chroma_format_idc`.
    pub chroma_format_idc: u8,
    /// Bit depth minus 8.
    pub bit_depth_minus8: u8,
    /// `capture_time_distance`.
    pub capture_time_distance: u8,
    /// True when `capture_time_distance` is to be ignored.
    pub capture_time_distance_ignored: bool,
    /// True when the colour description fields are present in the record.
    pub color_description_present: bool,
    /// Colour primaries code (ITU-T H.273).
    pub color_primaries: u8,
    /// Transfer characteristics code (ITU-T H.273).
    pub transfer_characteristics: u8,
    /// Matrix coefficients code (ITU-T H.273).
    pub matrix_coefficients: u8,
    /// True for full-range sample values.
    pub full_range: bool,
    /// Raw `APVDecoderConfigurationRecord` bytes (after the 4-byte FullBox header). When non-empty,
    /// serialization writes these bytes and ignores the parsed fields above.
    pub raw_record: Vec<u8>,
}

impl ApvConfig {
    /// Build from a raw `APVDecoderConfigurationRecord` (starting with `configurationVersion = 1`).
    pub fn from_record(record: Vec<u8>) -> Result<Self> {
        let mut c = Cur::labeled(&record, "apvC");
        let version = c.u8()?;
        if version != 1 {
            return Err(Error::Invalid(format!("unsupported APVDecoderConfigurationRecord version {version}")));
        }
        let num_entries = c.u8()?;
        let mut cfg = ApvConfig { raw_record: record.clone(), ..Default::default() };
        if num_entries >= 1 && c.remaining() >= 2 {
            let _pbu_type = c.u8()?;
            let num_fi = c.u8()?;
            if num_fi >= 1 && c.remaining() >= 14 {
                let flags = c.u8()?;
                cfg.color_description_present = (flags & 0x02) != 0;
                cfg.capture_time_distance_ignored = (flags & 0x01) != 0;
                cfg.profile_idc = c.u8()?;
                cfg.level_idc = c.u8()?;
                cfg.band_idc = c.u8()?;
                cfg.frame_width = c.u32()?;
                cfg.frame_height = c.u32()?;
                let cb = c.u8()?;
                cfg.chroma_format_idc = cb >> 4;
                cfg.bit_depth_minus8 = cb & 0x0F;
                cfg.capture_time_distance = c.u8()?;
                if cfg.color_description_present && c.remaining() >= 4 {
                    cfg.color_primaries = c.u8()?;
                    cfg.transfer_characteristics = c.u8()?;
                    cfg.matrix_coefficients = c.u8()?;
                    cfg.full_range = (c.u8()? & 0x80) != 0;
                }
            }
        }
        Ok(cfg)
    }

    /// Parse the payload of an `apvC` box (with or without the 4-byte `FullBox` header).
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() >= 5 && data[..4] == [0, 0, 0, 0] && data[4] == 1 {
            Self::from_record(data[4..].to_vec())
        } else if !data.is_empty() && data[0] == 1 {
            Self::from_record(data.to_vec())
        } else {
            let mut c = Cur::labeled(data, "apvC");
            let (v, _) = c.full_header()?;
            if v != 0 {
                return Err(Error::Invalid(format!("unsupported apvC FullBox version {v}")));
            }
            Self::from_record(c.rest().to_vec())
        }
    }

    /// Serialize the full `apvC` box payload (4-byte `FullBox` header + `APVDecoderConfigurationRecord`).
    pub fn to_bytes(&self) -> Vec<u8> {
        [&[0u8; 4][..], &self.record_bytes()].concat() // FullBox version = 0, flags = 0
    }

    /// Serialize the bare `APVDecoderConfigurationRecord` (Matroska's `CodecPrivate` for `V_APV`).
    pub fn record_bytes(&self) -> Vec<u8> {
        if !self.raw_record.is_empty() {
            return self.raw_record.clone();
        }
        let mut b = BoxBuf::new();
        b.u8(1); // configurationVersion = 1
        b.u8(1); // number_of_configuration_entry = 1
        b.u8(1); // pbu_type = 1 (primary frame)
        b.u8(1); // number_of_frame_info = 1
        let flags = (if self.color_description_present { 0x02 } else { 0 }) | (if self.capture_time_distance_ignored { 0x01 } else { 0 });
        b.u8(flags);
        b.u8(self.profile_idc);
        b.u8(self.level_idc);
        b.u8(self.band_idc);
        b.u32(self.frame_width);
        b.u32(self.frame_height);
        b.u8((self.chroma_format_idc << 4) | (self.bit_depth_minus8 & 0x0F));
        b.u8(self.capture_time_distance);
        if self.color_description_present {
            b.u8(self.color_primaries);
            b.u8(self.transfer_characteristics);
            b.u8(self.matrix_coefficients);
            b.u8(if self.full_range { 0x80 } else { 0 });
        }
        b.buf
    }
}
