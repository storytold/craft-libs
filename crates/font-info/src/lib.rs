#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! What one font face offers for fine typography: its variation axes with the font's own names,
//! its named instances, and its OpenType layout features with the names the font gives them
//! (`ss01` "Open digits", `cv11` "Single-story a") and the characters each feature changes, so a
//! preview can show exactly those. Also what naming its variable instances takes (Adobe Tech Note
//! #5902): the Variations PostScript Name Prefix, each named instance's PostScript name and the
//! `STAT` design axes; and [`FEATURE_NAMES`], the registered names of feature tags.
//!
//! [`read`] takes the font file's bytes and the face's index in it (0 for a single font; the
//! face in a `.ttc` collection). Fonts are untrusted input: a malformed table is left out, never a
//! panic, and every list read from the file is capped.
//!
//! ```no_run
//! let bytes = std::fs::read("Inter.ttf")?;
//! let details = craft_font_info::read(&bytes, 0)?;
//! for axis in &details.axes {
//!     println!("{} {:?} {}..{}", axis.tag, axis.name, axis.min, axis.max);
//! }
//! if let Some(ss01) = details.feature("ss01") {
//!     println!("ss01 is {:?} and changes {}", ss01.name, ss01.sample);
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::{BTreeMap, BTreeSet};

use skrifa::raw::tables::gsub::{SingleSubst, SubstitutionSubtables};
use skrifa::raw::tables::layout::{CoverageTable, FeatureParams};
use skrifa::raw::{ReadError, TableProvider, types::GlyphId16};
use skrifa::{FontRef, MetadataProvider, string::StringId};

/// Most characters kept as a feature's sample.
const SAMPLE_CHARS: usize = 24;
/// A sample takes this many characters of one kind before moving to the next kind.
const RUN: usize = 3;
/// Most features read from one table, most lookups followed per feature, and most glyphs read
/// from one coverage table: fonts are input, and a malformed one can claim anything.
const MAX_FEATURES: usize = 512;
const MAX_LOOKUPS: usize = 64;
const MAX_COVERAGE: usize = 4096;
/// Most `STAT` design axes and axis value records read.
const MAX_STAT: usize = 256;

/// A variation axis of the face, from its `fvar` table.
#[derive(Clone, Debug, PartialEq)]
pub struct AxisDetail {
    /// Four-character tag (`wght`, `GRAD`).
    pub tag: String,
    /// The font's own (English, else first) name for the axis ("Parametric Thick Stroke"), if it
    /// has one.
    pub name: Option<String>,
    pub min: f32,
    pub default: f32,
    pub max: f32,
    /// The font asks for the axis to stay out of user interfaces (`fvar` flag 0x0001).
    pub hidden: bool,
}

/// A named instance ("Bold Condensed"): a full set of axis coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceDetail {
    pub name: String,
    /// The instance's own PostScript name (`InterVariable-Bold`), if the font gives one.
    pub postscript_name: Option<String>,
    /// User-space coordinates, one per axis in [`FaceDetails::axes`] order: (tag, value).
    pub coords: Vec<(String, f32)>,
}

/// An OpenType layout feature the face has (in `GSUB` or `GPOS`).
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureDetail {
    pub tag: String,
    /// The font's UI name for a stylistic set or character variant (`ss01`–`ss20`,
    /// `cv01`–`cv99` feature parameters), if it gives one.
    pub name: Option<String>,
    /// Up to 24 of the characters the feature substitutes, a few of each kind in turn (figures,
    /// capitals, lower case, other letters, the rest: `012ABCabc345DEF…`), so any prefix of it
    /// shows every kind the feature changes. Empty for positioning features and for substitutions
    /// only reachable through context.
    pub sample: String,
    /// How many characters the feature substitutes in all (the sample shows at most 24).
    pub changed: usize,
}

/// A design axis of the face's style attributes (`STAT`), which may be one `fvar` lacks (Inter
/// Variable's `ital`: its upright and italic are two files).
#[derive(Clone, Debug, PartialEq)]
pub struct StatAxisDetail {
    pub tag: String,
    /// This face's value on the axis: its first single-axis value record (formats 1–3), if any.
    pub value: Option<f32>,
}

/// Everything [`read`] reads from one face.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceDetails {
    pub axes: Vec<AxisDetail>,
    pub instances: Vec<InstanceDetail>,
    /// Variations PostScript Name Prefix (name ID 25), if the font has one.
    pub variations_prefix: Option<String>,
    /// `STAT` design axes, in the table's order.
    pub stat_axes: Vec<StatAxisDetail>,
    /// Sorted by tag, each tag once.
    pub features: Vec<FeatureDetail>,
}

impl FaceDetails {
    /// Whether the face has the layout feature `tag`.
    pub fn has_feature(&self, tag: &str) -> bool {
        self.features.binary_search_by(|f| f.tag.as_str().cmp(tag)).is_ok()
    }

    /// The face's feature `tag`, if it has it.
    pub fn feature(&self, tag: &str) -> Option<&FeatureDetail> {
        self.features.binary_search_by(|f| f.tag.as_str().cmp(tag)).ok().and_then(|i| self.features.get(i))
    }

    /// The face's axis `tag`, if it has it.
    pub fn axis(&self, tag: &str) -> Option<&AxisDetail> {
        self.axes.iter().find(|a| a.tag == tag)
    }
}

/// The data isn't a font, or has no face at the index asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotAFont(String);

impl std::fmt::Display for NotAFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not a readable font: {}", self.0)
    }
}

impl std::error::Error for NotAFont {}

/// Reads [`FaceDetails`] from a font file's bytes: face `index` of a collection (`.ttc`), or 0.
/// Only data that isn't a font is an error; unreadable tables are left out (a font with a broken
/// `GSUB` still has its axes).
pub fn read(data: &[u8], index: u32) -> Result<FaceDetails, NotAFont> {
    let font = FontRef::from_index(data, index).map_err(|e| NotAFont(e.to_string()))?;
    Ok(read_face(&font))
}

fn read_face(font: &FontRef) -> FaceDetails {
    let name = |id: StringId| font.localized_strings(id).english_or_first().map(|s| s.to_string()).filter(|s| !s.trim().is_empty());
    let axes: Vec<AxisDetail> = font
        .axes()
        .iter()
        .filter(|a| a.min_value().is_finite() && a.default_value().is_finite() && a.max_value().is_finite() && a.min_value() <= a.max_value())
        .map(|a| AxisDetail {
            tag: a.tag().to_string(),
            name: name(a.name_id()),
            min: a.min_value(),
            default: a.default_value().clamp(a.min_value(), a.max_value()),
            max: a.max_value(),
            hidden: a.is_hidden(),
        })
        .collect();
    let all_axes: Vec<String> = font.axes().iter().map(|a| a.tag().to_string()).collect();
    let instances = font
        .named_instances()
        .iter()
        .filter_map(|inst| {
            let coords: Vec<(String, f32)> = all_axes.iter().cloned().zip(inst.user_coords()).filter(|(_, v)| v.is_finite()).collect();
            Some(InstanceDetail { name: name(inst.subfamily_name_id())?, postscript_name: inst.postscript_name_id().and_then(name), coords })
        })
        .collect();
    let mut features: BTreeMap<String, FeatureDetail> = BTreeMap::new();
    if let Ok(gsub) = font.gsub() {
        let _ = gsub_features(font, &gsub, &name, &mut features);
    }
    if let Ok(gpos) = font.gpos()
        && let Ok(list) = gpos.feature_list()
    {
        for record in list.feature_records().iter().take(MAX_FEATURES) {
            let tag = record.feature_tag().to_string();
            features.entry(tag.clone()).or_insert(FeatureDetail { tag, name: None, sample: String::new(), changed: 0 });
        }
    }
    FaceDetails {
        axes,
        instances,
        variations_prefix: name(StringId::VARIATIONS_POSTSCRIPT_NAME_PREFIX),
        stat_axes: stat_axes(font),
        features: features.into_values().collect(),
    }
}

/// The `STAT` design axes with this face's value on each.
fn stat_axes(font: &FontRef) -> Vec<StatAxisDetail> {
    use skrifa::raw::tables::stat::AxisValue;
    let Ok(stat) = font.stat() else { return Vec::new() };
    let Ok(design) = stat.design_axes() else { return Vec::new() };
    let mut values: BTreeMap<u16, f32> = BTreeMap::new();
    if let Some(Ok(list)) = stat.offset_to_axis_values() {
        for v in list.axis_values().iter().take(MAX_STAT).filter_map(Result::ok) {
            let (axis, value) = match v {
                AxisValue::Format1(v) => (v.axis_index(), v.value()),
                AxisValue::Format2(v) => (v.axis_index(), v.nominal_value()),
                AxisValue::Format3(v) => (v.axis_index(), v.value()),
                AxisValue::Format4(_) => continue,
            };
            values.entry(axis).or_insert(value.to_f32());
        }
    }
    design
        .iter()
        .take(MAX_STAT)
        .enumerate()
        .map(|(i, a)| StatAxisDetail {
            tag: a.axis_tag().to_string(),
            value: u16::try_from(i).ok().and_then(|i| values.get(&i)).copied().filter(|v| v.is_finite()),
        })
        .collect()
}

/// Registered OpenType layout features with their English names from the OpenType feature
/// registry (`tnum` "Tabular Figures", `salt` "Stylistic Alternates"), for an interface to show
/// and translate. Tags and names are copied from the registry
/// (<https://learn.microsoft.com/en-us/typography/opentype/spec/featurelist>, OpenType 1.9.1)
/// as it writes them, capitalisation included; an app that translates them keeps its own words.
/// The numbered `ssNN` and `cvNN` aren't here: [`FeatureDetail::name`] names them from the font
/// itself.
pub const FEATURE_NAMES: &[(&str, &str)] = &[
    ("liga", "Standard Ligatures"),
    ("dlig", "Discretionary Ligatures"),
    ("calt", "Contextual Alternates"),
    ("swsh", "Swash"),
    ("salt", "Stylistic Alternates"),
    ("titl", "Titling"),
    ("ordn", "Ordinals"),
    ("frac", "Fractions"),
    ("case", "Case-sensitive Forms"),
    ("cpsp", "Capital Spacing"),
    ("kern", "Kerning"),
    ("zero", "Slashed Zero"),
    ("subs", "Subscript"),
    ("sups", "Superscript"),
    ("sinf", "Scientific Inferiors"),
    ("smcp", "Small Capitals"),
    ("c2sc", "Small Capitals From Capitals"),
    ("pcap", "Petite Capitals"),
    ("c2pc", "Petite Capitals From Capitals"),
    ("unic", "Unicase"),
    ("dnom", "Denominators"),
    ("numr", "Numerators"),
    ("hist", "Historical Forms"),
    ("hlig", "Historical Ligatures"),
    ("ornm", "Ornaments"),
    ("nalt", "Alternate Annotation Forms"),
    ("mgrk", "Mathematical Greek"),
    ("lnum", "Lining Figures"),
    ("onum", "Oldstyle Figures"),
    ("pnum", "Proportional Figures"),
    ("tnum", "Tabular Figures"),
    ("afrc", "Alternative Fractions"),
    ("cswh", "Contextual Swash"),
    ("jalt", "Justification Alternates"),
    ("ital", "Italics"),
    ("rtbd", "Right Bounds"),
    ("lfbd", "Left Bounds"),
    ("opbd", "Optical Bounds"),
    ("palt", "Proportional Alternate Widths"),
    ("halt", "Alternate Half Widths"),
    ("pwid", "Proportional Widths"),
    ("fwid", "Full Widths"),
    ("hwid", "Half Widths"),
    ("ruby", "Ruby Notation Forms"),
    ("expt", "Expert Forms"),
    ("jp78", "JIS78 Forms"),
    ("jp83", "JIS83 Forms"),
    ("jp90", "JIS90 Forms"),
    ("jp04", "JIS2004 Forms"),
    ("smpl", "Simplified Forms"),
    ("trad", "Traditional Forms"),
    ("tnam", "Traditional Name Forms"),
];

/// The English name of the registered feature `tag` ([`FEATURE_NAMES`]), if it has one.
pub fn feature_name(tag: &str) -> Option<&'static str> {
    FEATURE_NAMES.iter().find(|(t, _)| *t == tag).map(|(_, name)| *name)
}

/// Adds the `GSUB` features, their UI names and their substituted characters to `out`.
fn gsub_features(
    font: &FontRef,
    gsub: &skrifa::raw::tables::gsub::Gsub,
    name: &dyn Fn(StringId) -> Option<String>,
    out: &mut BTreeMap<String, FeatureDetail>,
) -> Result<(), ReadError> {
    let list = gsub.feature_list()?;
    let lookups = gsub.lookup_list().ok();
    let reverse = reverse_cmap(font);
    let data = list.offset_data();
    // Glyphs per tag; one tag can have several records (one per script/language).
    let mut glyphs: BTreeMap<String, BTreeSet<u16>> = BTreeMap::new();
    for record in list.feature_records().iter().take(MAX_FEATURES) {
        let tag = record.feature_tag().to_string();
        let entry = out.entry(tag.clone()).or_insert(FeatureDetail { tag: tag.clone(), name: None, sample: String::new(), changed: 0 });
        let Ok(feature) = record.feature(data) else { continue };
        if entry.name.is_none() {
            entry.name = match feature.feature_params() {
                Some(Ok(FeatureParams::StylisticSet(p))) => name(p.ui_name_id()),
                Some(Ok(FeatureParams::CharacterVariant(p))) => name(p.feat_ui_label_name_id()),
                _ => None,
            };
        }
        let Some(lookups) = &lookups else { continue };
        let set = glyphs.entry(tag).or_default();
        for index in feature.lookup_list_indices().iter().take(MAX_LOOKUPS) {
            let Ok(lookup) = lookups.lookups().get(usize::from(index.get())) else { continue };
            let Ok(subtables) = lookup.subtables() else { continue };
            collect_coverage(&subtables, set);
        }
    }
    for (tag, set) in glyphs {
        if let Some(entry) = out.get_mut(&tag) {
            let chars: BTreeSet<char> = set.iter().filter_map(|g| reverse.get(g).copied()).collect();
            entry.changed = chars.len();
            entry.sample = sample(chars);
        }
    }
    Ok(())
}

/// The first glyphs each directly substituting subtable acts on. Contextual subtables are skipped:
/// what they change depends on the surrounding text, so their coverage would mislead.
fn collect_coverage(subtables: &SubstitutionSubtables, set: &mut BTreeSet<u16>) {
    let mut add = |c: Result<CoverageTable, ReadError>| {
        if let Ok(c) = c {
            set.extend(c.iter().take(MAX_COVERAGE).map(|g: GlyphId16| g.to_u16()));
        }
    };
    match subtables {
        SubstitutionSubtables::Single(s) => s.iter().flatten().for_each(|t| match t {
            SingleSubst::Format1(t) => add(t.coverage()),
            SingleSubst::Format2(t) => add(t.coverage()),
        }),
        SubstitutionSubtables::Multiple(s) => s.iter().flatten().for_each(|t| add(t.coverage())),
        SubstitutionSubtables::Alternate(s) => s.iter().flatten().for_each(|t| add(t.coverage())),
        SubstitutionSubtables::Ligature(s) => s.iter().flatten().for_each(|t| add(t.coverage())),
        _ => {}
    }
}

/// Nominal glyph → its lowest character, for the Basic Multilingual Plane (enough for samples).
fn reverse_cmap(font: &FontRef) -> BTreeMap<u16, char> {
    let mut out = BTreeMap::new();
    for (cp, gid) in font.charmap().mappings().take(1 << 17) {
        let (Some(c), Ok(g)) = (char::from_u32(cp), u16::try_from(gid.to_u32())) else { continue };
        if !c.is_control() && !c.is_whitespace() {
            out.entry(g).or_insert(c);
        }
    }
    out
}

/// A readable sample from substituted characters: runs of [`RUN`] of each kind in turn (ASCII
/// figures, ASCII capitals, ASCII lower case, other letters and figures, the rest), each kind in
/// code point order, at most [`SAMPLE_CHARS`]. Taking kinds in turn rather than one after the
/// other keeps a set of circled figures *and* letters from reading as figures only when a
/// preview shows just the start.
fn sample(chars: BTreeSet<char>) -> String {
    let kind = |c: char| match c {
        '0'..='9' => 0,
        'A'..='Z' => 1,
        'a'..='z' => 2,
        c if c.is_alphanumeric() => 3,
        _ => 4,
    };
    let mut kinds: [Vec<char>; 5] = Default::default();
    for c in chars {
        if let Some(k) = kinds.get_mut(kind(c)) {
            k.push(c);
        }
    }
    let mut queues: Vec<std::vec::IntoIter<char>> = kinds.into_iter().filter(|k| !k.is_empty()).map(Vec::into_iter).collect();
    let mut out = String::new();
    let mut n = 0;
    while n < SAMPLE_CHARS && !queues.is_empty() {
        queues.retain_mut(|q| {
            for c in q.by_ref().take(RUN.min(SAMPLE_CHARS - n)) {
                out.push(c);
                n += 1;
            }
            q.len() > 0
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_takes_each_kind_in_turn() {
        let set = |s: &str| s.chars().collect::<BTreeSet<char>>();
        assert_eq!(sample(set("é!a1Z")), "1Zaé!");
        assert_eq!(sample(set(&"x".repeat(100))), "x");
        // Squared figures and capitals: a short prefix already has both.
        let squared = sample(('0'..='9').chain('A'..='Z').collect());
        assert_eq!(squared, "012ABC345DEF678GHI9JKLMN");
        assert_eq!(squared.chars().count(), SAMPLE_CHARS);
        // One kind only keeps code point order.
        assert_eq!(sample(('a'..='z').collect()), "abcdefghijklmnopqrstuvwx");
        assert_eq!(sample(BTreeSet::new()), "");
    }
}
