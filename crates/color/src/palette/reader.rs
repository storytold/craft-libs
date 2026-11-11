//! Byte reader, name and count helpers and limits shared by the swatch codecs.

use super::*;

// ---------------------------------------------------------------- file helpers

/// Longest name in a binary swatch file, in UTF-16 code units.
pub(super) const MAX_NAME_UNITS: usize = 1024;
/// Longest swatch or group name kept from a file, in characters.
const MAX_NAME_CHARS: usize = 255;
pub(super) const GPL_HEADER: &str = "GIMP Palette";
pub(super) const GPL_GROUP: &str = "# Group:";
pub(super) const ASE_SIGNATURE: &[u8; 4] = b"ASEF";
pub(super) const ASE_GROUP_START: u16 = 0xC001;
pub(super) const ASE_GROUP_END: u16 = 0xC002;
pub(super) const ASE_COLOR: u16 = 0x0001;
pub(super) const ASE_MAX_SWATCHES: usize = 100_000;
pub(super) const ASE_MAX_GROUPS: usize = 10_000;
pub(super) const ACO_MAX_SWATCHES: usize = u16::MAX as usize;
pub(super) const ACT_COLORS: usize = 256;
pub(super) const ACT_PALETTE_LEN: usize = ACT_COLORS * 3;
pub(super) const ACT_TRAILER_LEN: usize = 4;

/// Transforms from the Lab D50 and sGray profiles to sRGB (relative colorimetric, black point
/// compensation); `None` when one does not link.
pub(super) struct WorkingTransforms {
    pub(super) lab: Option<std::sync::Arc<craft_cms::Transform>>,
    pub(super) gray: Option<std::sync::Arc<craft_cms::Transform>>,
}

pub(super) fn working_transforms() -> &'static WorkingTransforms {
    static TRANSFORMS: std::sync::OnceLock<WorkingTransforms> = std::sync::OnceLock::new();
    TRANSFORMS.get_or_init(|| {
        let opts = craft_cms::TransformOptions { intent: craft_cms::Intent::RelativeColorimetric, bpc: true, ..Default::default() };
        let srgb = craft_cms::Builtin::Srgb.profile();
        let link = |source: &craft_cms::Profile| craft_cms::cached(source, srgb, opts).ok();
        WorkingTransforms { lab: link(craft_cms::Builtin::LabD50.profile()), gray: link(craft_cms::Builtin::SGray.profile()) }
    })
}

pub(super) fn profile_to_srgb(transform: &Option<std::sync::Arc<craft_cms::Transform>>, input: &[f32]) -> Option<[f32; 3]> {
    let mut out = [0.0f32; 3];
    transform.as_ref()?.eval(input, &mut out).ok()?;
    Some(out.map(unit))
}

pub(super) fn malformed(format: SwatchFormat, message: impl Into<String>) -> SwatchError {
    SwatchError::Malformed(format, message.into())
}

pub(super) fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

pub(super) fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// One line of text, cut to the length a name may have.
pub(super) fn clip_name(name: &str) -> String {
    let line = name.split(['\n', '\r', '\t']).collect::<Vec<_>>().join(" ");
    line.trim().chars().take(MAX_NAME_CHARS).collect()
}

pub(super) fn entry_name(index: usize, name: &str) -> String {
    let name = clip_name(name);
    if name.is_empty() { format!("Swatch {}", index + 1) } else { name }
}

/// The group name for ungrouped colours: the file's stem.
pub(super) fn stem_of(file_name: &str) -> String {
    let stem = std::path::Path::new(file_name).file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let stem = clip_name(stem);
    if stem.is_empty() { "Imported Swatches".into() } else { stem }
}

pub(super) fn count_swatches(groups: &[SwatchGroup]) -> usize {
    groups.iter().map(|g| g.entries.len()).sum()
}

/// Whole UTF-16 scalar values of `name` that fit in `max` units.
pub(super) fn utf16_units(name: &str, max: usize) -> Vec<u16> {
    let mut units = Vec::new();
    for c in name.chars() {
        if units.len() + c.len_utf16() > max {
            break;
        }
        let mut buf = [0u16; 2];
        units.extend_from_slice(c.encode_utf16(&mut buf));
    }
    units
}

pub(super) fn names_cut(groups: &[SwatchGroup], include_groups: bool) -> usize {
    let cut = |name: &str| name.encode_utf16().count() > MAX_NAME_UNITS;
    let swatches = groups.iter().flat_map(|g| g.entries.iter()).filter(|e| cut(&e.name)).count();
    swatches + if include_groups { groups.iter().filter(|g| cut(&g.name)).count() } else { 0 }
}

/// A bounds-checked big-endian cursor.
pub(super) struct Reader<'a> {
    data: &'a [u8],
    pub(super) pos: usize,
}

impl<'a> Reader<'a> {
    pub(super) fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }
    pub(super) fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
    pub(super) fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        if n > self.remaining() {
            return Err(format!("the file is cut short at byte {}", self.pos));
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }
    pub(super) fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }
    pub(super) fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    pub(super) fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    pub(super) fn f32(&mut self) -> Result<f32, String> {
        Ok(finite(f32::from_bits(self.u32()?)))
    }
    /// Fails when `count` items of at least `min_size` bytes cannot fit in what is left.
    pub(super) fn check_count(&self, count: u64, min_size: u64) -> Result<(), String> {
        if count.saturating_mul(min_size) > self.remaining() as u64 { Err(format!("the file is cut short at byte {}", self.pos)) } else { Ok(()) }
    }
    /// A name of `n` UTF-16 units that include the trailing null; units past the limit are dropped.
    pub(super) fn units(&mut self, n: usize) -> Result<String, String> {
        self.check_count(n as u64, 2)?;
        let mut units = Vec::with_capacity(n.min(MAX_NAME_UNITS));
        for _ in 0..n {
            let u = self.u16()?;
            if units.len() < MAX_NAME_UNITS {
                units.push(u);
            }
        }
        while units.last() == Some(&0) {
            units.pop();
        }
        Ok(String::from_utf16_lossy(&units))
    }
}

/// Name, 8-bit colour and whether it is a spot colour.
pub(super) type RgbSwatch = (String, [u8; 3], bool);

/// The colours of a format that holds only 8-bit RGB: other models converted, colour-book colours
/// left out. Returns the colours per group with the `BookColors` and `ConvertedToRgb` counts.
pub(super) fn rgb_colors(groups: &[SwatchGroup]) -> (Vec<Vec<RgbSwatch>>, usize, usize) {
    let (mut books, mut converted) = (0, 0);
    let out = groups
        .iter()
        .map(|g| {
            g.entries
                .iter()
                .filter_map(|e| {
                    match e.color {
                        SwatchColor::Book { .. } => {
                            books += 1;
                            return None;
                        }
                        SwatchColor::Cmyk(_) | SwatchColor::Lab(_) | SwatchColor::Gray(_) => converted += 1,
                        SwatchColor::Rgb(_) | SwatchColor::Hsb(_) => {}
                    }
                    Some((e.name.clone(), rgb8(e.color.to_srgb(CmykConverter::coated()).to_rgb_array()), e.kind == SwatchKind::Spot))
                })
                .collect()
        })
        .collect();
    (out, books, converted)
}

pub(super) fn push_note(report: &mut SwatchReport, count: usize, note: impl FnOnce(usize) -> SwatchNote) {
    if count > 0 {
        report.notes.push(note(count));
    }
}
