//! Where the fields of each record lie, by version: one table, resolved once per file. Each field
//! names the `PROVENANCE.md` row it rests on.

/// The layouts of one range of versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Profile {
    /// Coordinates are i16 in 1/1000 inch, record headers, counts and angles 16-bit (R5, R6, R7).
    pub(crate) short: bool,
    /// Bytes before the page size in `mcfg` (R10).
    pub(crate) mcfg_skip: usize,
    /// Bytes before a `trfd` entry's type (R8).
    pub(crate) trfd_head: usize,
    /// Bytes between a matrix entry's type and its six numbers (R8).
    pub(crate) matrix_skip: usize,
    /// Fill and outline records start with tagged records (R11, R12).
    pub(crate) tagged: bool,
    /// Outline: bytes after the join, after the stretch, after the nib angle and after the color
    /// (R12).
    pub(crate) outl_after_join: usize,
    pub(crate) outl_after_stretch: usize,
    pub(crate) outl_after_angle: usize,
    pub(crate) outl_after_color: usize,
    /// Where the start marker id lies after the first dash (R13); `None` where no source says.
    pub(crate) marker_after_dash: Option<usize>,
    /// Rectangles carry four corner radii, not one (R15).
    pub(crate) four_radii: bool,
    /// Rectangles are f64 sizes with radii as shares of half the shorter side (R29).
    pub(crate) f64_rect: bool,
    /// The master page flags its guides and desktop layers the way of versions 7 to 13 (R19).
    pub(crate) old_master_flags: bool,
    /// Object type 0x25 is a path (R30).
    pub(crate) paths: bool,
    /// Names are UTF-16LE, not Windows-1252 (R9).
    pub(crate) utf16_names: bool,
    /// An object holds its transform, fill and outline as arguments; there are no `trfd`, `fill`
    /// or `outl` records (R33, R34, R35).
    pub(crate) inline_styles: bool,
    /// Object types count from 0: rectangle 0, ellipse 1, curve 2, text 3, bitmap 4 (R32).
    pub(crate) types_from_zero: bool,
    /// How a color is laid out (R14, R36).
    pub(crate) color: ColorLayout,
    /// A matrix is six i32 with its factors in 16.16 fixed point, not six f64 (R37).
    pub(crate) fixed_matrix: bool,
    /// `mcfg` holds 2 bytes and the page's two corners, not its width and height (R38).
    pub(crate) mcfg_corners: bool,
}

/// The layouts of a color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColorLayout {
    /// A model byte and four value bytes (R36).
    Bytes5,
    /// A model u16, four u16 values and 2 bytes (R36).
    Words12,
    /// A model u16, a palette u16, 4 bytes and four value bytes (R14).
    Record12,
}

const fn profile(from: u16) -> Profile {
    let short = from < 600;
    Profile {
        short,
        inline_styles: from < 400,
        types_from_zero: from < 400,
        color: if from < 400 {
            ColorLayout::Bytes5
        } else if from < 500 {
            ColorLayout::Words12
        } else {
            ColorLayout::Record12
        },
        fixed_matrix: from < 500,
        mcfg_corners: from < 400,
        mcfg_skip: if from >= 1300 {
            12
        } else if from >= 900 {
            4
        } else if from >= 600 && from < 700 {
            28
        } else {
            0
        },
        trfd_head: if from >= 1300 { 8 } else { 0 },
        matrix_skip: if short { 0 } else { 6 },
        tagged: from >= 1300,
        outl_after_join: if from >= 600 && from < 1300 { 2 } else { 0 },
        outl_after_stretch: if short { 0 } else { 2 },
        outl_after_angle: if from >= 1300 {
            46
        } else if !short {
            52
        } else {
            0
        },
        outl_after_color: if short { 10 } else { 16 },
        marker_after_dash: if from >= 1300 {
            Some(20)
        } else if from >= 600 {
            Some(22)
        } else {
            None
        },
        four_radii: from >= 900,
        f64_rect: from >= 1500,
        old_master_flags: from < 1400,
        paths: from >= 1000,
        utf16_names: from >= 1200,
    }
}

/// The newest version read: 27, as the `vrsn` chunk names it (R1, R2).
pub(crate) const NEWEST: u16 = 2799;

/// The first version of each range, with its layouts. Ranges start where a field moves.
const PROFILES: &[(u16, Profile)] = &[
    (300, profile(300)),
    (400, profile(400)),
    (500, profile(500)),
    (600, profile(600)),
    (700, profile(700)),
    (800, profile(800)),
    (900, profile(900)),
    (1000, profile(1000)),
    (1100, profile(1100)),
    (1200, profile(1200)),
    (1300, profile(1300)),
    (1400, profile(1400)),
    (1500, profile(1500)),
    (1600, profile(1600)),
];

/// The oldest version of a RIFF file read: 3 (R31). Versions 1 and 2 are not RIFF files (`wl.rs`).
pub(crate) const OLDEST_RIFF: u16 = 300;

/// The layouts of the RIFF file of `version` (hundreds; 801 is the 8.01 variant, 2430 is 24.3), or
/// `None` outside 3 to 27.
pub(crate) fn for_version(version: u16) -> Option<Profile> {
    if !(OLDEST_RIFF..=NEWEST).contains(&version) {
        return None;
    }
    PROFILES.iter().rev().find(|(from, _)| *from <= version).map(|(_, p)| *p)
}

impl Profile {
    /// Points per coordinate unit.
    pub(crate) fn unit(&self) -> f64 {
        if self.short { 72.0 / 1000.0 } else { 72.0 / 254_000.0 }
    }

    /// Bytes of a coordinate, and of header fields and counts.
    pub(crate) fn word(&self) -> usize {
        if self.short { 2 } else { 4 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_version_from_3_to_27_has_one_profile() {
        assert!(for_version(299).is_none() && for_version(2800).is_none());
        assert!(for_version(300).is_some_and(|p| p.short && p.inline_styles && p.types_from_zero && p.mcfg_corners && p.fixed_matrix));
        assert!(for_version(399).is_some_and(|p| p.color == ColorLayout::Bytes5));
        assert!(for_version(400).is_some_and(|p| p.short && !p.inline_styles && !p.mcfg_corners && p.fixed_matrix));
        assert!(for_version(499).is_some_and(|p| p.color == ColorLayout::Words12));
        assert!(for_version(500).is_some_and(|p| !p.fixed_matrix && !p.types_from_zero && p.color == ColorLayout::Record12));
        assert!(for_version(1300).is_some_and(|p| p.old_master_flags));
        assert!(for_version(1400).is_some_and(|p| p.tagged && !p.f64_rect && !p.old_master_flags));
        assert!(for_version(1500).is_some_and(|p| p.tagged && p.f64_rect && p.mcfg_skip == 12 && p.paths));
        assert!(for_version(2430).is_some_and(|p| p.f64_rect && p.paths && p.utf16_names));
        assert!(for_version(500).is_some_and(|p| p.short && p.mcfg_skip == 0 && p.marker_after_dash.is_none()));
        assert!(for_version(600).is_some_and(|p| !p.short && p.mcfg_skip == 28 && p.marker_after_dash == Some(22)));
        assert!(for_version(801).is_some_and(|p| p.mcfg_skip == 0 && p.marker_after_dash == Some(22)));
        assert!(for_version(999).is_some_and(|p| !p.paths && p.marker_after_dash == Some(22)));
        assert!(for_version(1000).is_some_and(|p| p.mcfg_skip == 4 && p.four_radii && !p.utf16_names && p.paths));
        assert!(for_version(1300).is_some_and(|p| p.tagged && p.trfd_head == 8 && p.outl_after_angle == 46));
        // The table is in order, so the search finds each range's own row.
        assert!(PROFILES.windows(2).all(|w| w[0].0 < w[1].0));
    }
}
