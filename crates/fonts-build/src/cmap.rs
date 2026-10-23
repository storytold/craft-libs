//! Just enough of an OpenType reader to ask which characters a face maps: the `cmap` table's
//! Unicode subtables in formats 4 and 12, in a single font or one face of a collection.

/// The Unicode character maps of one face.
pub(crate) struct Cmap<'a> {
    subtables: Vec<&'a [u8]>,
}

impl<'a> Cmap<'a> {
    /// The cmap of face `index` of `font` (a `.ttf`/`.otf`, or a `.ttc` collection); `None` when the
    /// file isn't a font or has no Unicode subtable in format 4 or 12.
    pub(crate) fn read(font: &'a [u8], index: u32) -> Option<Self> {
        let face = if font.get(0..4)? == b"ttcf" {
            if index >= u32_at(font, 8)? {
                return None;
            }
            usize::try_from(u32_at(font, 12 + 4 * usize::try_from(index).ok()?)?).ok()?
        } else {
            0
        };
        let tables = usize::from(u16_at(font, face + 4)?);
        let record = (0..tables).map(|i| face + 12 + 16 * i).find(|&r| font.get(r..r + 4) == Some(b"cmap"))?;
        let cmap = slice(font, u32_at(font, record + 8)?, u32_at(font, record + 12)?)?;
        let mut subtables = Vec::new();
        for i in 0..usize::from(u16_at(cmap, 2)?) {
            let (platform, encoding) = (u16_at(cmap, 4 + 8 * i)?, u16_at(cmap, 6 + 8 * i)?);
            // Unicode platform, or Windows Unicode BMP (1) and full repertoire (10).
            if platform != 0 && !(platform == 3 && matches!(encoding, 1 | 10)) {
                continue;
            }
            let start = usize::try_from(u32_at(cmap, 8 + 8 * i)?).ok()?;
            let rest = cmap.get(start..)?;
            // Bound each subtable by its own length so that reads can't stray into the next one.
            // Some large CJK fonts declare a format 4 length past the end of the table, so that
            // one is clamped; a format 12 length that doesn't fit is skipped.
            let len = match u16_at(rest, 0) {
                Some(4) => u16_at(rest, 2).map(|len| usize::from(len).min(rest.len())),
                Some(12) => u32_at(rest, 4).and_then(|len| usize::try_from(len).ok()),
                _ => None,
            };
            if let Some(subtable) = len.and_then(|len| rest.get(..len)) {
                subtables.push(subtable);
            }
        }
        (!subtables.is_empty()).then_some(Self { subtables })
    }

    /// Whether any Unicode subtable maps `c` to a glyph other than `.notdef`.
    pub(crate) fn covers(&self, c: char) -> bool {
        self.subtables.iter().any(|t| glyph(t, u32::from(c)).is_some_and(|g| g != 0))
    }
}

fn glyph(subtable: &[u8], c: u32) -> Option<u32> {
    match u16_at(subtable, 0)? {
        4 => format4(subtable, u16::try_from(c).ok()?),
        12 => format12(subtable, c),
        _ => None,
    }
}

fn format4(t: &[u8], c: u16) -> Option<u32> {
    let seg_x2 = usize::from(u16_at(t, 6)?);
    let (ends, starts, deltas, range_offsets) = (14, 16 + seg_x2, 16 + 2 * seg_x2, 16 + 3 * seg_x2);
    // End codes are sorted: the first segment ending at or after `c` is the only candidate.
    let segments = seg_x2 / 2;
    let i = partition_point(segments, |i| u16_at(t, ends + 2 * i).is_some_and(|end| end < c));
    if i == segments {
        return None;
    }
    let start = u16_at(t, starts + 2 * i)?;
    if c < start {
        return None;
    }
    let delta = u16_at(t, deltas + 2 * i)?;
    let range_offset_at = range_offsets + 2 * i;
    let range_offset = usize::from(u16_at(t, range_offset_at)?);
    if range_offset == 0 {
        return Some(u32::from(c.wrapping_add(delta)));
    }
    let g = u16_at(t, range_offset_at + range_offset + 2 * usize::from(c - start))?;
    Some(if g == 0 { 0 } else { u32::from(g.wrapping_add(delta)) })
}

fn format12(t: &[u8], c: u32) -> Option<u32> {
    let groups = usize::try_from(u32_at(t, 12)?).ok()?;
    // Groups are sorted by start code and don't overlap.
    let i = partition_point(groups, |i| u32_at(t, 16 + 12 * i + 4).is_some_and(|end| end < c));
    if i == groups {
        return None;
    }
    let g = 16 + 12 * i;
    let (start, end, glyph) = (u32_at(t, g)?, u32_at(t, g + 4)?, u32_at(t, g + 8)?);
    (start..=end).contains(&c).then(|| glyph.wrapping_add(c - start))
}

/// The first index in `0..len` for which `before` is false, given that it is true for a prefix.
fn partition_point(len: usize, before: impl Fn(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0, len);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if before(mid) { lo = mid + 1 } else { hi = mid }
    }
    lo
}

fn slice(data: &[u8], offset: u32, len: u32) -> Option<&[u8]> {
    let offset = usize::try_from(offset).ok()?;
    data.get(offset..offset.checked_add(usize::try_from(len).ok()?)?)
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at.checked_add(4)?)?.try_into().ok()?))
}
