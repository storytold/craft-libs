//! H.263 picture and GOB headers (ITU-T H.263 §5.1, §5.2).

use craft_media::bitstream::BitReader;

use super::DecodeError;

/// The 22-bit picture start code: `0000 0000 0000 0000 1 00000` (§5.1.1).
const PSC: u32 = 0x20;

/// Width and height of the five standard source formats (Table 2), indexed by the 3-bit source
/// format code minus one.
const STANDARD_SIZES: [(u32, u32); 5] = [(128, 96), (176, 144), (352, 288), (704, 576), (1408, 1152)];

/// Source format code of "custom source format" in OPPTYPE (§5.1.4.2).
const CUSTOM_SOURCE_FORMAT: u32 = 6;

/// Largest custom picture dimensions (§4.2.2).
const MAX_CUSTOM_WIDTH: u32 = 2048;
const MAX_CUSTOM_HEIGHT: u32 = 1152;

/// Pixel aspect ratios of the 4-bit codes 1 to 5 (Table 5).
const PIXEL_ASPECT: [(u16, u16); 5] = [(1, 1), (12, 11), (10, 11), (16, 11), (40, 33)];

/// What a picture codes (§5.1.3, §5.1.4.3). The scalability and B picture types are refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureType {
    /// An INTRA picture (I-picture).
    Intra,
    /// An INTER picture (P-picture).
    Inter,
}

/// How far motion vectors may reach in the Unrestricted Motion Vector mode (§5.1.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VectorLimit {
    /// Tables D.1 and D.2 apply (UUI `1`).
    ByFormat,
    /// Only the distance to the picture bounds limits a vector (UUI `01`).
    Unlimited,
}

/// The optional modes that stay in force from one picture to the next (§5.1.4.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Modes {
    /// Annex D.
    pub umv: bool,
    /// Annex F.
    pub advanced_prediction: bool,
    /// Annex J.
    pub deblocking: bool,
    /// Annex E.
    pub sac: bool,
    /// Annex I.
    pub advanced_intra: bool,
    /// Annex K.
    pub slices: bool,
    /// Annex N.
    pub reference_selection: bool,
    /// Annex R.
    pub independent_segments: bool,
    /// Annex S.
    pub alternative_inter_vlc: bool,
    /// Annex T.
    pub modified_quantization: bool,
}

impl Modes {
    /// The first mode this decoder does not implement, by its Annex.
    fn unsupported(&self) -> Option<&'static str> {
        [
            (self.sac, "Syntax-based Arithmetic Coding (Annex E)"),
            (self.advanced_intra, "Advanced INTRA Coding (Annex I)"),
            (self.slices, "Slice Structured mode (Annex K)"),
            (self.reference_selection, "Reference Picture Selection (Annex N)"),
            (self.independent_segments, "Independent Segment Decoding (Annex R)"),
            (self.alternative_inter_vlc, "Alternative INTER VLC (Annex S)"),
            (self.modified_quantization, "Modified Quantization (Annex T)"),
        ]
        .into_iter()
        .find_map(|(on, name)| on.then_some(name))
    }
}

/// What a picture header leaves in force for the pictures after it (§5.1.4.2, UFEP `000`).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct HeaderState {
    /// Set once a header carried the full optional PLUSPTYPE part (UFEP `001`).
    plus: Option<PlusState>,
}

#[derive(Clone, Copy, Debug)]
struct PlusState {
    modes: Modes,
    width: u32,
    height: u32,
    pixel_aspect: (u16, u16),
    custom_pcf: bool,
    vector_limit: VectorLimit,
}

/// A parsed picture header.
#[derive(Clone, Copy, Debug)]
pub(super) struct PictureHeader {
    pub picture_type: PictureType,
    pub width: u32,
    pub height: u32,
    pub pixel_aspect: (u16, u16),
    pub modes: Modes,
    /// The header carried PLUSPTYPE (§5.1.4).
    pub plus: bool,
    /// The rounding type bit (RTYPE, §5.1.4.3); always `false` without PLUSPTYPE.
    pub rounding: bool,
    pub quant: u8,
    pub vector_limit: VectorLimit,
}

impl PictureHeader {
    /// Macroblocks per row and column.
    pub fn mb_dims(&self) -> (usize, usize) {
        (self.width.div_ceil(16) as usize, self.height.div_ceil(16) as usize)
    }

    /// Macroblock rows in one group of blocks (`k` of Table 4).
    pub fn rows_per_gob(&self) -> usize {
        match self.height {
            0..=400 => 1,
            401..=800 => 2,
            _ => 4,
        }
    }

    /// Whether a macroblock may carry four vectors: Annex F or Annex J is in use (§5.3.8).
    pub fn four_vectors(&self) -> bool {
        self.modes.advanced_prediction || self.modes.deblocking
    }
}

/// Whether the 22 bits at `r` are a picture start code.
pub(super) fn at_picture_start(r: &BitReader<'_>) -> bool {
    r.bits_left() >= 22 && r.peek(22) == PSC
}

fn marker(r: &mut BitReader<'_>, what: &str) -> Result<(), DecodeError> {
    if r.read_bit()? { Ok(()) } else { Err(DecodeError::Invalid(format!("missing {what} marker bit"))) }
}

/// Parse the picture header at the start code (§5.1).
pub(super) fn parse_picture_header(r: &mut BitReader<'_>, state: &mut HeaderState) -> Result<PictureHeader, DecodeError> {
    if r.read_bits(22)? != PSC {
        return Err(DecodeError::Invalid("missing picture start code".into()));
    }
    let _temporal_reference = r.read_bits(8)?;
    marker(r, "PTYPE")?;
    if r.read_bit()? {
        return Err(DecodeError::Invalid("PTYPE bit 2 is 1 (ITU-T H.261 distinction bit)".into()));
    }
    r.skip(3)?; // split screen, document camera, freeze release (§5.1.3)
    let source = r.read_bits(3)?;
    let (picture_type, rounding, modes, size, pixel_aspect, vector_limit, plus);
    let mut full = false;
    if source != 7 {
        let &(w, h) = usize::try_from(source)
            .ok()
            .and_then(|s| s.checked_sub(1))
            .and_then(|s| STANDARD_SIZES.get(s))
            .ok_or_else(|| DecodeError::Invalid(format!("source format code {source} is forbidden or reserved")))?;
        picture_type = if r.read_bit()? { PictureType::Inter } else { PictureType::Intra };
        let umv = r.read_bit()?;
        let sac = r.read_bit()?;
        let advanced_prediction = r.read_bit()?;
        if r.read_bit()? {
            return Err(DecodeError::Unsupported("PB-frames mode (Annex G)".into()));
        }
        modes = Modes { umv, advanced_prediction, sac, ..Modes::default() };
        rounding = false;
        size = (w, h);
        pixel_aspect = (12, 11);
        vector_limit = VectorLimit::ByFormat;
        plus = false;
        // Modes not signalled in PTYPE fall back to "off" until the next UFEP `001` (§5.1.4.5).
        state.plus = None;
    } else {
        plus = true;
        let ufep = r.read_bits(3)?;
        let current = match ufep {
            0 => state.plus.ok_or_else(|| DecodeError::Invalid("UFEP 000 before any full PLUSPTYPE header".into()))?,
            1 => {
                full = true;
                let format = r.read_bits(3)?;
                let custom_pcf = r.read_bit()?;
                let umv = r.read_bit()?;
                let sac = r.read_bit()?;
                let advanced_prediction = r.read_bit()?;
                let advanced_intra = r.read_bit()?;
                let deblocking = r.read_bit()?;
                let slices = r.read_bit()?;
                let reference_selection = r.read_bit()?;
                let independent_segments = r.read_bit()?;
                let alternative_inter_vlc = r.read_bit()?;
                let modified_quantization = r.read_bit()?;
                marker(r, "OPPTYPE")?;
                r.skip(3)?; // reserved
                let (width, height) = if format == CUSTOM_SOURCE_FORMAT {
                    (0, 0)
                } else {
                    *usize::try_from(format)
                        .ok()
                        .and_then(|s| s.checked_sub(1))
                        .and_then(|s| STANDARD_SIZES.get(s))
                        .ok_or_else(|| DecodeError::Invalid(format!("OPPTYPE source format code {format} is reserved")))?
                };
                PlusState {
                    modes: Modes {
                        umv,
                        advanced_prediction,
                        deblocking,
                        sac,
                        advanced_intra,
                        slices,
                        reference_selection,
                        independent_segments,
                        alternative_inter_vlc,
                        modified_quantization,
                    },
                    width,
                    height,
                    pixel_aspect: (12, 11),
                    custom_pcf,
                    vector_limit: VectorLimit::ByFormat,
                }
            }
            other => return Err(DecodeError::Invalid(format!("reserved UFEP value {other}"))),
        };
        let mut current = current;
        // MPPTYPE (§5.1.4.3).
        picture_type = match r.read_bits(3)? {
            0 => PictureType::Intra,
            1 => PictureType::Inter,
            2 => return Err(DecodeError::Unsupported("Improved PB-frames (Annex M)".into())),
            code => return Err(DecodeError::Unsupported(format!("B, EI or EP picture type {code} (Annex O)"))),
        };
        if r.read_bit()? {
            return Err(DecodeError::Unsupported("Reference Picture Resampling (Annex P)".into()));
        }
        if r.read_bit()? {
            return Err(DecodeError::Unsupported("Reduced-Resolution Update (Annex Q)".into()));
        }
        rounding = r.read_bit()?;
        r.skip(2)?; // reserved
        marker(r, "MPPTYPE")?;
        if r.read_bit()? {
            return Err(DecodeError::Unsupported("Continuous Presence Multipoint (Annex C)".into()));
        }
        if full {
            // CPFMT, EPAR and CPCFC are present only in a full header (§5.1.5 to §5.1.7).
            if current.width == 0 {
                let par = r.read_bits(4)?;
                let width = (r.read_bits(9)? + 1) * 4;
                marker(r, "CPFMT")?;
                let height = r.read_bits(9)? * 4;
                if height == 0 || height > MAX_CUSTOM_HEIGHT || width > MAX_CUSTOM_WIDTH {
                    return Err(DecodeError::Invalid(format!("custom picture format {width}x{height} is out of range")));
                }
                current.pixel_aspect = match par {
                    0 => return Err(DecodeError::Invalid("forbidden pixel aspect ratio code 0".into())),
                    15 => {
                        let (pw, ph) = (r.read_bits(8)? as u16, r.read_bits(8)? as u16);
                        if pw == 0 || ph == 0 {
                            return Err(DecodeError::Invalid("extended pixel aspect ratio of 0".into()));
                        }
                        (pw, ph)
                    }
                    code => *PIXEL_ASPECT
                        .get(usize::from(code as u8) - 1)
                        .ok_or_else(|| DecodeError::Invalid(format!("reserved pixel aspect ratio code {code}")))?,
                };
                current.width = width;
                current.height = height;
            }
            if current.custom_pcf {
                let divisor = r.read_bits(8)? & 0x7F;
                if divisor == 0 {
                    return Err(DecodeError::Invalid("custom picture clock divisor of 0".into()));
                }
            }
        }
        if current.custom_pcf {
            r.skip(2)?; // ETR
        }
        if full {
            if current.modes.umv {
                current.vector_limit = if r.read_bit()? {
                    VectorLimit::ByFormat
                } else if r.read_bit()? {
                    VectorLimit::Unlimited
                } else {
                    return Err(DecodeError::Invalid("UUI is neither 1 nor 01".into()));
                };
            }
            if current.modes.slices {
                r.skip(2)?; // SSS
            }
        }
        state.plus = Some(current);
        modes = current.modes;
        size = (current.width, current.height);
        pixel_aspect = current.pixel_aspect;
        vector_limit = current.vector_limit;
    }
    if let Some(mode) = modes.unsupported() {
        return Err(DecodeError::Unsupported(mode.into()));
    }
    let quant = r.read_bits(5)? as u8;
    if quant == 0 {
        return Err(DecodeError::Invalid("PQUANT of 0".into()));
    }
    if !plus && r.read_bit()? {
        return Err(DecodeError::Unsupported("Continuous Presence Multipoint (Annex C)".into()));
    }
    while r.read_bit()? {
        r.skip(8)?; // PSUPP
    }
    Ok(PictureHeader { picture_type, width: size.0, height: size.1, pixel_aspect, modes, plus, rounding, quant, vector_limit })
}

/// Read the GOB header at the start of GOB `gob` if one is present (§5.2).
///
/// Returns `Some(GQUANT)` when a header was read, `None` when the GOB header is empty (the
/// macroblock data follows directly).
pub(super) fn read_gob_header(r: &mut BitReader<'_>, gob: usize) -> Result<Option<u8>, DecodeError> {
    // GSTUF: zero bits up to the next byte boundary, only before a start code.
    let before = r.position();
    if !r.is_byte_aligned() && r.bits_left() >= 8 {
        let pad = 8 - (before % 8);
        if r.peek(pad as u32) == 0 {
            let _ = r.skip(pad);
            if !(r.bits_left() >= 17 && r.peek(17) == 1) {
                r.seek_bits(before);
            }
        }
    }
    if !(r.bits_left() >= 17 && r.peek(17) == 1) {
        return Ok(None);
    }
    r.skip(17)?;
    let number = r.read_bits(5)? as usize;
    if number != gob {
        return Err(DecodeError::Invalid(format!("GOB number {number} where {gob} was expected")));
    }
    r.skip(2)?; // GFID
    let quant = r.read_bits(5)? as u8;
    if quant == 0 {
        return Err(DecodeError::Invalid("GQUANT of 0".into()));
    }
    Ok(Some(quant))
}
