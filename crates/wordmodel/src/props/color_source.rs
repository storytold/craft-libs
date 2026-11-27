//! The theme colours a file gives text, underline, border and shading colours, kept beside the
//! colours they are shown in.
//!
//! WordprocessingML names such a colour by a colour of the document's theme (`w:themeColor`, a
//! shading's `w:themeFill`) with a tint or a shade, and stores a fixed colour beside it, which
//! only a reader without the theme shows: the theme colour supersedes it (ECMA-376 Part 1, 5th
//! edition, §17.3.2.6 `color`, §17.3.2.40 `u`, §17.3.4 `CT_Border`, §17.3.5 `CT_Shd`). The
//! properties ([`super::CharProps::color`] and the rest) hold the colour shown, which layout and
//! the other formats read. The records here hold the reference and the fixed colour as the file
//! gave them, so saving writes them back and the colour keeps following the theme; each owner
//! keeps its records in one box (`color_sources`), absent for colours without a theme colour.
//!
//! A colour chosen since replaces its record (`set_color` and the like, or a patch that sets the
//! colour). A record also stops counting once its property no longer has the colour it was shown
//! in ([`ColorSource::holds`]), so a colour changed in place is saved as it now is.

use super::color::{Rgb, TextColor};
use super::{Deserialize, Serialize};

/// A colour of the document's theme, as `w:themeColor` and `w:themeFill` name it
/// (`ST_ThemeColor`, ECMA-376 §17.18.97). `none`, no theme colour, isn't one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemeColor {
    Dark1,
    Light1,
    Dark2,
    Light2,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    Hyperlink,
    FollowedHyperlink,
    Background1,
    Text1,
    Background2,
    Text2,
}

impl ThemeColor {
    pub const ALL: [ThemeColor; 16] = [
        ThemeColor::Dark1,
        ThemeColor::Light1,
        ThemeColor::Dark2,
        ThemeColor::Light2,
        ThemeColor::Accent1,
        ThemeColor::Accent2,
        ThemeColor::Accent3,
        ThemeColor::Accent4,
        ThemeColor::Accent5,
        ThemeColor::Accent6,
        ThemeColor::Hyperlink,
        ThemeColor::FollowedHyperlink,
        ThemeColor::Background1,
        ThemeColor::Text1,
        ThemeColor::Background2,
        ThemeColor::Text2,
    ];

    /// The `ST_ThemeColor` name.
    pub fn ooxml(self) -> &'static str {
        match self {
            ThemeColor::Dark1 => "dark1",
            ThemeColor::Light1 => "light1",
            ThemeColor::Dark2 => "dark2",
            ThemeColor::Light2 => "light2",
            ThemeColor::Accent1 => "accent1",
            ThemeColor::Accent2 => "accent2",
            ThemeColor::Accent3 => "accent3",
            ThemeColor::Accent4 => "accent4",
            ThemeColor::Accent5 => "accent5",
            ThemeColor::Accent6 => "accent6",
            ThemeColor::Hyperlink => "hyperlink",
            ThemeColor::FollowedHyperlink => "followedHyperlink",
            ThemeColor::Background1 => "background1",
            ThemeColor::Text1 => "text1",
            ThemeColor::Background2 => "background2",
            ThemeColor::Text2 => "text2",
        }
    }

    /// The theme colour an `ST_ThemeColor` name gives; `None` for `none` and for a name the
    /// schema doesn't have.
    pub fn from_ooxml(s: &str) -> Option<ThemeColor> {
        ThemeColor::ALL.into_iter().find(|c| c.ooxml() == s)
    }

    /// Its colour in a theme's colour scheme, as an index into [`crate::Settings::theme_colors`]
    /// (dk1, lt1, dk2, lt2, accent1–6, hlink, folHlink). The dark and light colours are scheme
    /// colours themselves; the others go through the document's colour scheme mapping `map`
    /// (`w:clrSchemeMapping`: the scheme colour that text1, background1, text2, background2,
    /// accent1–6, hyperlink and followedHyperlink each stand for), or stand for their own when
    /// there is none.
    pub fn scheme_index(self, map: Option<&[u8; 12]>) -> usize {
        let mapped = |i: usize| map.and_then(|m| m.get(i)).map_or(i, |v| usize::from(*v));
        match self {
            ThemeColor::Dark1 => 0,
            ThemeColor::Light1 => 1,
            ThemeColor::Dark2 => 2,
            ThemeColor::Light2 => 3,
            ThemeColor::Text1 => mapped(0),
            ThemeColor::Background1 => mapped(1),
            ThemeColor::Text2 => mapped(2),
            ThemeColor::Background2 => mapped(3),
            ThemeColor::Accent1 => mapped(4),
            ThemeColor::Accent2 => mapped(5),
            ThemeColor::Accent3 => mapped(6),
            ThemeColor::Accent4 => mapped(7),
            ThemeColor::Accent5 => mapped(8),
            ThemeColor::Accent6 => mapped(9),
            ThemeColor::Hyperlink => mapped(10),
            ThemeColor::FollowedHyperlink => mapped(11),
        }
    }
}

/// A theme colour with the tint or shade a file gives it: `w:themeTint` and `w:themeShade`, or a
/// shading's `w:themeFillTint` and `w:themeFillShade` (hex bytes, `ST_UcharHexNumber`,
/// ECMA-376 §17.18.98). Both are kept when the file gives both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeRef {
    pub color: ThemeColor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shade: Option<u8>,
}

impl ThemeRef {
    /// The colour it names in `palette` (a theme's colours, in [`ThemeColor::scheme_index`]'s
    /// order) through the colour scheme mapping `map`, with its tint, or else its shade, applied
    /// to the colour's HSL luminance: a tint `t` makes it `L·t + (1 − t)`, a shade `s` makes it
    /// `L·s`, and a tint given with a shade is applied alone ([MS-OI29500] §2.1.72, notes to
    /// Part 1 §17.3.2.6). `None` when the palette has no such colour (the document has no theme).
    pub fn resolve(&self, palette: &[Rgb], map: Option<&[u8; 12]>) -> Option<Rgb> {
        let c = *palette.get(self.color.scheme_index(map))?;
        let byte = |v: u8| f64::from(v) / 255.0;
        Some(match (self.tint, self.shade) {
            (Some(t), _) => c.with_lum(|l| l * byte(t) + (1.0 - byte(t))),
            (None, Some(s)) => c.with_lum(|l| l * byte(s)),
            (None, None) => c,
        })
    }
}

/// A text, underline or border colour given as a theme colour: the reference, the fixed colour
/// the file stores beside it, and the colour the property was shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorSource {
    /// The theme colour (`w:themeColor`, `w:themeTint`, `w:themeShade`).
    pub theme: ThemeRef,
    /// The fixed colour stored with it (`w:val` of a text colour, `w:color` of an underline or a
    /// border): `None` when the file gives none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<TextColor>,
    /// The colour the property had when the reference was read or last resolved; a border's or
    /// an underline's automatic colour is `None`. See [`ColorSource::holds`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown: Option<TextColor>,
}

impl ColorSource {
    /// The reference still stands for its property, whose colour is `now`: the property has the
    /// colour the reference was shown in. A colour set since, without replacing the record,
    /// differs from it, and is saved as set.
    pub fn holds(&self, now: Option<TextColor>) -> bool {
        self.shown == now
    }
}

/// The shading patterns of `ST_Shd` (ECMA-376 §17.18.78).
pub const SHADING_PATTERNS: [&str; 38] = [
    "nil",
    "clear",
    "solid",
    "horzStripe",
    "vertStripe",
    "reverseDiagStripe",
    "diagStripe",
    "horzCross",
    "diagCross",
    "thinHorzStripe",
    "thinVertStripe",
    "thinReverseDiagStripe",
    "thinDiagStripe",
    "thinHorzCross",
    "thinDiagCross",
    "pct5",
    "pct10",
    "pct12",
    "pct15",
    "pct20",
    "pct25",
    "pct30",
    "pct35",
    "pct37",
    "pct40",
    "pct45",
    "pct50",
    "pct55",
    "pct60",
    "pct62",
    "pct65",
    "pct70",
    "pct75",
    "pct80",
    "pct85",
    "pct87",
    "pct90",
    "pct95",
];

/// A shading (`w:shd`, `CT_Shd`) with a theme colour on its pattern colour, its fill or both, as
/// the file gives it: every attribute, those of a colour the pattern doesn't show included.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShadingSource {
    /// The pattern (`w:val`), one of [`SHADING_PATTERNS`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// The pattern colour (`w:color`) and its theme colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<TextColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_theme: Option<ThemeRef>,
    /// The fill (`w:fill`) and its theme colour (`w:themeFill`, `w:themeFillTint`,
    /// `w:themeFillShade`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<TextColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_theme: Option<ThemeRef>,
    /// The shading the property had when read or last resolved ([`ColorSource::shown`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shown: Option<Rgb>,
}

impl ShadingSource {
    /// The record still stands for its shading, which is `now` ([`ColorSource::holds`]).
    pub fn holds(&self, now: Option<Rgb>) -> bool {
        self.shown == now
    }
}

/// The theme colours of a set of borders, side by side with [`super::Borders`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BorderColorSources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<ColorSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<ColorSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<ColorSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<ColorSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub between: Option<ColorSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inside_v: Option<ColorSource>,
}

impl BorderColorSources {
    pub fn is_empty(&self) -> bool {
        *self == BorderColorSources::default()
    }

    fn sides_mut(&mut self) -> [&mut Option<ColorSource>; 6] {
        [&mut self.top, &mut self.left, &mut self.bottom, &mut self.right, &mut self.between, &mut self.inside_v]
    }

    fn sides(&self) -> [Option<ColorSource>; 6] {
        [self.top, self.left, self.bottom, self.right, self.between, self.inside_v]
    }

    /// Side by side with [`super::Borders::overlay`]: a side `patch` sets takes its record from
    /// `sources` (none when it has none); the others keep theirs.
    pub fn overlay(&mut self, patch: &super::Borders, sources: &BorderColorSources) {
        let set = [patch.top, patch.left, patch.bottom, patch.right, patch.between, patch.inside_v].map(|s| s.is_some());
        for ((mine, theirs), set) in self.sides_mut().into_iter().zip(sources.sides()).zip(set) {
            if set || theirs.is_some() {
                *mine = theirs;
            }
        }
    }
}

/// The theme colours of a run's properties ([`super::CharProps::color_sources`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CharColorSources {
    /// The text colour's (`w:color`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorSource>,
    /// The underline colour's (`w:u`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<ColorSource>,
    /// The character border's (`w:bdr`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<ColorSource>,
    /// The character shading's (`w:shd`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shading: Option<ShadingSource>,
}

impl CharColorSources {
    pub fn is_empty(&self) -> bool {
        self.color.is_none() && self.underline.is_none() && self.border.is_none() && self.shading.is_none()
    }
}

/// The theme colours of a paragraph's, a table's, a cell's or a table style part's shading and
/// borders.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BlockColorSources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shading: Option<ShadingSource>,
    #[serde(skip_serializing_if = "BorderColorSources::is_empty")]
    pub borders: BorderColorSources,
}

impl BlockColorSources {
    pub fn is_empty(&self) -> bool {
        self.shading.is_none() && self.borders.is_empty()
    }
}

/// `holder`'s record, `None` when it has none, as `get` picks it out.
pub(crate) fn source<'a, H, T>(holder: &'a Option<Box<H>>, get: impl Fn(&'a H) -> &'a Option<T>) -> Option<&'a T> {
    holder.as_deref().and_then(|h| get(h).as_ref())
}

/// Put `value` in `holder`'s place `get` picks: the box comes when there is something to hold
/// and goes when it holds nothing.
pub(crate) fn put<H: Default, T>(holder: &mut Option<Box<H>>, get: impl Fn(&mut H) -> &mut Option<T>, value: Option<T>, empty: impl Fn(&H) -> bool) {
    match holder {
        Some(h) => {
            *get(h) = value;
            if empty(h) {
                *holder = None;
            }
        }
        None => {
            if value.is_some() {
                let mut h = H::default();
                *get(&mut h) = value;
                *holder = Some(Box::new(h));
            }
        }
    }
}

/// The borders' records of `holder`, the empty set when it has none.
pub(crate) fn borders_of(holder: &Option<Box<BlockColorSources>>) -> BorderColorSources {
    holder.as_deref().map(|h| h.borders).unwrap_or_default()
}

/// Replace the borders' records of `holder` with `borders`.
pub(crate) fn put_borders(holder: &mut Option<Box<BlockColorSources>>, borders: BorderColorSources) {
    match holder {
        Some(h) => {
            h.borders = borders;
            if h.is_empty() {
                *holder = None;
            }
        }
        None if !borders.is_empty() => *holder = Some(Box::new(BlockColorSources { borders, ..BlockColorSources::default() })),
        None => {}
    }
}

/// `edit_color_sources` (or the like) for each owner of theme colour records.
macro_rules! edit_sources {
    ($owner:ty, $field:ident, $edit:ident, $holder:ty) => {
        impl $owner {
            #[doc = concat!("Change the theme colour records of the colours (`", stringify!($field), "`) with `f`: the box comes when they hold one and goes when they hold none.")]
            pub fn $edit(&mut self, f: impl FnOnce(&mut $holder)) {
                let mut held = self.$field.take().map(|b| *b).unwrap_or_default();
                f(&mut held);
                self.$field = (!held.is_empty()).then(|| Box::new(held));
            }
        }
    };
}

edit_sources!(super::CharProps, color_sources, edit_color_sources, CharColorSources);
edit_sources!(super::ParaProps, color_sources, edit_color_sources, BlockColorSources);
edit_sources!(super::TableProps, color_sources, edit_color_sources, BlockColorSources);
edit_sources!(super::CellProps, color_sources, edit_color_sources, BlockColorSources);
edit_sources!(crate::styles::PartFormat, color_sources, edit_color_sources, BlockColorSources);
edit_sources!(crate::section::SectionProps, page_border_sources, edit_page_border_sources, BorderColorSources);
