//! Named colours: the `ncl2` tag (namedColor2Type, ICC.1:2022 §10.17) of a named-colour
//! profile, such as a spot-colour library.
//!
//! Each entry pairs a name with a PCS value and, optionally, device coordinates. PCS values are
//! relative colorimetric. Lab is stored in the legacy 16-bit encoding (`0xFF00` is L* 100) in
//! a profile of any version, unlike the v4 encoding the LUT tags use.

use crate::profile::{Profile, Reader};
use crate::{Builtin, CmsError, Intent, Pcs, TransformOptions, math};

/// Bytes before the first entry: type, reserved, vendor flags, count, device-coordinate count,
/// prefix and suffix.
const HEADER: usize = 84;
/// Root name, then three PCS coordinates; the device coordinates follow.
const ENTRY: usize = 38;
/// Most device coordinates an entry may carry: the largest ICC colour space has 15 channels.
pub(crate) const MAX_DEVICE_COORDS: usize = 15;

/// One named colour.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedColor {
    /// Prefix, root name and suffix joined, each taken up to its first NUL.
    pub name: String,
    /// The PCS value: CIE L*a*b* (L* 0–100) when the profile's PCS is Lab, otherwise XYZ
    /// (D50, Y 1.0 for white).
    pub pcs: [f64; 3],
    /// Device coordinates scaled to 0–1; empty when the profile gives none.
    pub device: Vec<f64>,
}

/// A profile's named-colour table, in stored order.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedColors {
    /// The connection space [`NamedColor::pcs`] is in.
    pub pcs: Pcs,
    colors: Vec<NamedColor>,
}

impl Profile {
    /// The profile's named colours, or `None` when it has no `ncl2` tag or no stored bytes.
    pub fn named_colors(&self) -> Result<Option<NamedColors>, CmsError> {
        let Some(bytes) = self.bytes.as_deref() else {
            return Ok(None);
        };
        match tag(bytes, *b"ncl2")? {
            Some(data) => NamedColors::parse(data, self.pcs).map(Some),
            None => Ok(None),
        }
    }
}

impl NamedColors {
    /// Parses an `ncl2` tag's bytes; `pcs` is the profile header's connection space.
    pub fn parse(data: &[u8], pcs: Pcs) -> Result<NamedColors, CmsError> {
        let r = Reader { b: data };
        if r.slice(0, 4)? != b"ncl2" {
            return Err(CmsError::Invalid("named colour tag is not namedColor2Type".into()));
        }
        let count = r.u32(12)? as usize;
        let coords = r.u32(16)? as usize;
        if coords > MAX_DEVICE_COORDS {
            return Err(CmsError::Invalid(format!("{coords} device coordinates per named colour")));
        }
        let size = ENTRY + 2 * coords;
        // Every entry must lie inside the tag. This also bounds the allocation below by the
        // tag's size; a count the tag cannot hold would otherwise reserve up to ~300 GB.
        let needed = count.checked_mul(size).and_then(|n| n.checked_add(HEADER)).ok_or(CmsError::Truncated)?;
        if needed > data.len() {
            return Err(CmsError::Truncated);
        }
        let prefix = text(r.slice(20, 32)?);
        let suffix = text(r.slice(52, 32)?);
        let mut colors = Vec::with_capacity(count);
        for i in 0..count {
            let at = HEADER + i * size;
            let name = format!("{prefix}{}{suffix}", text(r.slice(at, 32)?));
            let code = |k: usize| r.u16(at + 32 + 2 * k).map(f64::from);
            let raw = [code(0)?, code(1)?, code(2)?];
            let device = (0..coords).map(|k| Ok(code(3 + k)? / 65535.0)).collect::<Result<_, CmsError>>()?;
            colors.push(NamedColor { name, pcs: decode(raw, pcs), device });
        }
        Ok(NamedColors { pcs, colors })
    }

    /// How many named colours the table holds.
    pub fn len(&self) -> usize {
        self.colors.len()
    }

    /// Whether the table holds no named colours.
    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    /// The named colours, in stored order.
    pub fn iter(&self) -> impl Iterator<Item = &NamedColor> {
        self.colors.iter()
    }

    /// The colour named exactly `name` (case-sensitive), the first if several share it.
    pub fn find(&self, name: &str) -> Option<&NamedColor> {
        self.colors.iter().find(|c| c.name == name)
    }

    /// `name`'s device values in `dst`, or `None` when no colour has that name. The PCS value
    /// enters `dst` relative colorimetric, as stored, without black point compensation. A `dst`
    /// with no PCS → device path (a device link, named-colour or abstract profile) is an error.
    pub fn to_device(&self, name: &str, dst: &Profile) -> Result<Option<Vec<f32>>, CmsError> {
        let Some(c) = self.find(name) else {
            return Ok(None);
        };
        let lab = match self.pcs {
            Pcs::Lab => c.pcs,
            Pcs::Xyz => math::xyz_to_lab(c.pcs, math::D50),
        };
        let opts = TransformOptions { intent: Intent::RelativeColorimetric, bpc: false, precise_float: true };
        let t = crate::cached(Builtin::LabD50.profile(), dst, opts)?;
        // Legacy codes reach a* and b* up to about 127.0; the Lab identity input is 0–1.
        let input = [lab[0] / 100.0, (lab[1] + 128.0) / 255.0, (lab[2] + 128.0) / 255.0].map(|v| v.clamp(0.0, 1.0) as f32);
        let mut out = vec![0.0; t.outputs()];
        t.eval(&input, &mut out)?;
        Ok(Some(out))
    }
}

/// The bytes of tag `sig` in profile `bytes`, or `None` when the profile has no such tag. Reads
/// within the declared profile size, as [`Profile::parse`] does.
fn tag(bytes: &[u8], sig: [u8; 4]) -> Result<Option<&[u8]>, CmsError> {
    let declared = Reader { b: bytes }.u32(0)? as usize;
    let end = if (132..=bytes.len()).contains(&declared) { declared } else { bytes.len() };
    let r = Reader { b: bytes.get(..end).unwrap_or(bytes) };
    // The same cap `Profile::parse` applies, so the offsets below cannot overflow on 32-bit.
    let count = (r.u32(128)? as usize).min(1024);
    for i in 0..count {
        let o = 132 + i * 12;
        if r.u32(o)? == u32::from_be_bytes(sig) {
            return Ok(Some(r.slice(r.u32(o + 4)? as usize, r.u32(o + 8)? as usize)?));
        }
    }
    Ok(None)
}

/// A fixed-size ASCII field up to its first NUL.
fn text(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(field.get(..end).unwrap_or_default()).into_owned()
}

/// Three stored PCS codes: legacy 16-bit Lab, or u1Fixed15 XYZ.
fn decode(raw: [f64; 3], pcs: Pcs) -> [f64; 3] {
    match pcs {
        Pcs::Lab => [raw[0] * 100.0 / 65280.0, raw[1] * 255.0 / 65280.0 - 128.0, raw[2] * 255.0 / 65280.0 - 128.0],
        Pcs::Xyz => raw.map(|v| v / 32768.0),
    }
}
