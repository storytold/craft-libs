//! What the picker knows about the installed families: names in the interface language, classes and
//! traits read from the faces' tables, the filtered list and its order, and the sample text of a row.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use craft_font_catalog::{Catalog, FaceInfo, LocalizedName, Outlines, is_private_family};
use craft_i18n::matcher::{Matcher, Rank};
use craft_i18n::{Collator, Folder};
use craft_ui::fonts::FaceBytes;
use skrifa::GlyphId;
use skrifa::MetadataProvider as _;
use skrifa::raw::{FontRef, TableProvider};

use super::{FamilyRow, FamilySet, FontClass, FontFilters, FontFormat, FontPickerSettings, FontScript, HostFontGroup, HostFontKind, SIMILAR_COUNT};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the catalog says about a family, read once per catalog generation.
pub(super) struct Info {
    pub(super) localized: Vec<LocalizedName>,
    /// Variable when any face of the family is; else the outlines of the regular face.
    pub(super) format: FontFormat,
    /// The number of faces of the family, named instances included.
    pub(super) styles: usize,
    /// The upright face nearest weight 400: the one that stands for the family.
    pub(super) regular: FaceInfo,
}

/// What a face's tables say about its family, as the class filter and similar fonts read them.
#[derive(Clone, Debug)]
pub(super) struct Traits {
    pub(super) class: Option<FontClass>,
    pub(super) panose: [u8; 10],
    pub(super) weight: u16,
}

impl Traits {
    /// How unlike `other` is to `self`: 0 for the same traits, larger the more they differ. The
    /// class matters most, then the PANOSE digits both fonts state, then the weight.
    fn difference(&self, other: &Traits) -> f32 {
        let mut d = 0.0;
        if self.class != other.class {
            d += 20.0;
        }
        // Digits 1..=9; 0 and 1 mean "any"/"no fit", so only stated digits count.
        for (x, y) in self.panose.iter().zip(other.panose.iter()).skip(1) {
            if *x > 1 && *y > 1 {
                d += f32::from(x.abs_diff(*y)).min(6.0);
            }
        }
        d + f32::from(self.weight.abs_diff(other.weight)) / 100.0
    }
}

/// The families of one catalog generation with their scripts and sample texts, filled on first use. Built by
/// [`FamilyInfos::of`] and held by the one that lists with it: the picker keeps it in a derived value keyed by the
/// catalog's generation, and an app that calls [`family_rows`](super::family_rows) keeps its own while the catalog is unchanged.
pub struct FamilyInfos {
    pub(super) families: HashMap<String, Info>,
    scripts: Mutex<HashMap<String, Vec<FontScript>>>,
    samples: Mutex<HashMap<(String, String), String>>,
}

impl FamilyInfos {
    /// The family data of `catalog` as it is now; build it again when the catalog's generation moves.
    pub fn of(catalog: &Catalog) -> FamilyInfos {
        let mut groups: HashMap<String, Vec<FaceInfo>> = HashMap::new();
        for face in catalog.faces().iter() {
            groups.entry(face.family.clone()).or_default().push(face.clone());
        }
        let families = groups
            .into_iter()
            .filter_map(|(family, faces)| {
                let regular = regular_face(&faces)?.clone();
                let localized = faces.iter().flat_map(|f| f.localized_families.iter().cloned()).collect();
                let format = if faces.iter().any(|f| f.variable) {
                    FontFormat::Variable
                } else if regular.outlines == Outlines::TrueType {
                    FontFormat::TrueType
                } else {
                    FontFormat::OpenType
                };
                Some((family, Info { localized, format, styles: faces.len(), regular }))
            })
            .collect();
        FamilyInfos { families, scripts: Mutex::new(HashMap::new()), samples: Mutex::new(HashMap::new()) }
    }

    /// The family's traits, from its regular face's record.
    pub(super) fn traits(&self, family: &str) -> Option<Traits> {
        self.families.get(family).map(|i| read_traits(&i.regular))
    }

    /// The scripts the family's regular face covers, read from its `cmap` on first use; none when the face can't be read.
    pub(super) fn scripts(&self, catalog: &Catalog, family: &str) -> Vec<FontScript> {
        if let Some(scripts) = lock(&self.scripts).get(family) {
            return scripts.clone();
        }
        let scripts = self.families.get(family).and_then(|i| read_scripts(catalog, &i.regular)).unwrap_or_default();
        lock(&self.scripts).insert(family.to_owned(), scripts.clone());
        scripts
    }

    /// What a family's sample says, in its row and in the banner: `sample` when the family's regular face (`face`, as
    /// the face registry loaded it) draws it, else its name, else the first glyphs the face has (a symbol face, or a
    /// face without the sample's script).
    pub(super) fn sample_text(&self, family: &str, sample: &str, shown: &str, face: &FaceBytes) -> String {
        let key = (family.to_owned(), sample.to_owned());
        if let Some(text) = lock(&self.samples).get(&key) {
            return text.clone();
        }
        let text = draws(face, sample, shown).unwrap_or_default();
        lock(&self.samples).insert(key, text.clone());
        text
    }
}

fn draws(face: &FaceBytes, sample: &str, shown: &str) -> Option<String> {
    let font = FontRef::from_index(&face.bytes, face.index).ok()?;
    let map = font.charmap();
    let inked = inked(&font);
    let drawn = |t: &str| t.chars().filter(|c| !c.is_whitespace()).all(|c| map.map(c).is_some_and(&inked));
    if drawn(sample) {
        return Some(sample.to_owned());
    }
    if drawn(shown) {
        return Some(shown.to_owned());
    }
    let first = map
        .mappings()
        .filter(|(_, glyph)| inked(*glyph))
        .filter_map(|(cp, _)| char::from_u32(cp))
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .take(6)
        .collect();
    Some(first)
}

/// Whether a glyph of `font` has an outline, which is all the sample painter draws: a TrueType glyph with contours, or
/// any glyph of a CFF face. A colour bitmap face (sbix, CBDT) maps its characters to glyphs without outlines.
fn inked<'a>(font: &FontRef<'a>) -> impl Fn(GlyphId) -> bool + 'a {
    let truetype = font.loca(None).ok().zip(font.glyf().ok());
    let cff = font.cff().is_ok() || font.cff2().is_ok();
    move |glyph| match &truetype {
        Some((loca, glyf)) => loca.get_glyf(glyph, glyf).ok().flatten().is_some(),
        None => cff,
    }
}

/// The upright face nearest weight 400.
pub(super) fn regular_face(faces: &[FaceInfo]) -> Option<&FaceInfo> {
    faces.iter().min_by_key(|f| (f.italic, (f.weight - 400.0).abs() as u32))
}

/// The name shown for a family: its record in the interface language, else the family name.
pub(super) fn shown_name(info: &Info, family: &str, language: &str, english: bool) -> String {
    let primary = |tag: &str| tag.split(['-', '_']).next().unwrap_or(tag).to_ascii_lowercase();
    if english || primary(language) == "en" {
        return family.to_owned();
    }
    let exact = |n: &&LocalizedName| n.language.as_deref().is_some_and(|l| l.replace('_', "-").eq_ignore_ascii_case(&language.replace('_', "-")));
    let same_language = |n: &&LocalizedName| n.language.as_deref().is_some_and(|l| primary(l) == primary(language));
    info.localized.iter().find(exact).or_else(|| info.localized.iter().find(same_language)).map_or_else(|| family.to_owned(), |n| n.name.clone())
}

/// Whether a family name of the face is in Japanese.
fn japanese_name(face: &FaceInfo) -> bool {
    face.localized_families
        .iter()
        .any(|n| n.language.as_deref().and_then(|l| l.split(['-', '_']).next()).is_some_and(|l| l.eq_ignore_ascii_case("ja")))
}

/// A face's class, PANOSE and weight from its `OS/2` and `post` tables, as the catalog's scan recorded them.
fn read_traits(face: &FaceInfo) -> Traits {
    let weight = face.weight_class.unwrap_or(400).clamp(1, 1000);
    Traits { class: classify(&face.family, face.family_class, face.panose, face.fixed_pitch), panose: face.panose, weight }
}

/// The JIS/Japan bit of `ulCodePageRange1`.
const JIS_CODE_PAGE: u32 = 1 << 17;
/// The Hiragana (49), Katakana (50) and CJK Unified Ideographs (59) bits of `ulUnicodeRange2`, which holds bits 32 to 63.
const JAPANESE_RANGES: u32 = (1 << (49 - 32)) | (1 << (50 - 32)) | (1 << (59 - 32));

/// The scripts a face covers: Latin, the basic Latin letters in its `cmap`; Japanese, the JIS code page, the kana or
/// CJK Unicode ranges of its `OS/2` table, or a Japanese family name; Thai, ก and า in its `cmap`.
fn read_scripts(catalog: &Catalog, face: &FaceInfo) -> Option<Vec<FontScript>> {
    let data = catalog.load(face).ok()?;
    let font = FontRef::from_index(&data.bytes, face.index).ok()?;
    let map = font.charmap();
    let mapped = |c: char| map.map(c).is_some();
    let latin = ('A'..='Z').chain('a'..='z').all(&mapped);
    let ranges = font.os2().ok().map_or(0, |os2| os2.ul_unicode_range_2());
    let japanese = face.code_pages[0] & JIS_CODE_PAGE != 0 || ranges & JAPANESE_RANGES != 0 || japanese_name(face);
    let thai = ['\u{0E01}', '\u{0E32}'].into_iter().all(&mapped);
    let scripts = [(FontScript::Latin, latin), (FontScript::Japanese, japanese), (FontScript::Thai, thai)];
    Some(scripts.into_iter().filter_map(|(script, covered)| covered.then_some(script)).collect())
}

/// The class of a family from its regular face, `None` when the catalog lacks it.
pub(super) fn class_of(catalog: &Catalog, family: &str) -> Option<FontClass> {
    let faces = catalog.styles(family);
    read_traits(regular_face(&faces)?).class
}

/// Name fragments (lowercase) that mark a blackletter face; the tables have no class for it.
const BLACKLETTER: [&str; 7] = ["fraktur", "blackletter", "black letter", "textura", "old english", "schwabacher", "rotunda"];

/// Classify from the OS/2 `sFamilyClass` (high byte: the IBM class), the PANOSE digits, the
/// `post` fixed-pitch flag and, last, the family name.
pub(super) fn classify(family: &str, family_class: i16, panose: [u8; 10], fixed_pitch: bool) -> Option<FontClass> {
    let name = family.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    // Whole words of the name, so "Encode Sans" isn't read as a "code" font.
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    // PANOSE Latin Text proportion 9 is monospaced.
    if fixed_pitch || (panose[0] == 2 && panose[3] == 9) {
        return Some(FontClass::Monospace);
    }
    if has(&BLACKLETTER) {
        return Some(FontClass::Blackletter);
    }
    // PANOSE family kind: 3 Latin Hand Written, 4 Latin Decoratives, 5 Latin Symbol.
    match panose[0] {
        3 => return Some(if family_class >> 8 == 10 { FontClass::Script } else { FontClass::Handwritten }),
        4 | 5 => return Some(FontClass::Decorative),
        _ => {}
    }
    match family_class >> 8 {
        // 4 is Clarendon: bracketed serifs (Georgia), not slabs.
        1..=4 | 7 => return Some(FontClass::Serif),
        5 => return Some(FontClass::SlabSerif),
        8 => return Some(FontClass::SansSerif),
        9 | 12 => return Some(FontClass::Decorative),
        10 => return Some(if has(&["hand", "marker", "brush", "pen", "chalk"]) { FontClass::Handwritten } else { FontClass::Script }),
        _ => {}
    }
    // PANOSE Latin Text serif style: 11-15 are sans (normal, obtuse, perpendicular, flared,
    // rounded); square (6) is a slab; the rest, the square coves (bracketed serifs, as Georgia's)
    // included, are serifs.
    if panose[0] == 2 {
        match panose[1] {
            11..=15 => return Some(FontClass::SansSerif),
            6 => return Some(FontClass::SlabSerif),
            2..=10 => return Some(FontClass::Serif),
            _ => {}
        }
    }
    // Last, the name.
    if words.iter().any(|w| matches!(*w, "mono" | "monospace" | "monospaced" | "code" | "courier") || w.starts_with("consol")) {
        Some(FontClass::Monospace)
    } else if has(&["slab"]) {
        Some(FontClass::SlabSerif)
    } else if has(&["sans", "grotesk", "grotesque", "gothic"]) {
        Some(FontClass::SansSerif)
    } else if has(&["serif", "roman", "garamond", "times", "bodoni", "baskerville", "caslon"]) {
        Some(FontClass::Serif)
    } else if has(&["hand", "marker", "brush", "chalk"]) {
        Some(FontClass::Handwritten)
    } else if has(&["script"]) {
        Some(FontClass::Script)
    } else {
        None
    }
}

/// What a list is made for.
pub(super) struct Listing<'a> {
    /// The typed text that narrows the list.
    pub(super) query: &'a str,
    /// Stars, recents and the name language; without it no stars or recents show.
    pub(super) settings: Option<&'a FontPickerSettings>,
    /// The filters; they apply only to a `narrowed` list.
    pub(super) filters: &'a FontFilters,
    /// The interface language code the names are shown in.
    pub(super) language: &'a str,
    /// Families the host can fetch but the catalog lacks; they list with the matches, by rank and name, with no class, format or sample.
    pub(super) served: &'a [String],
    /// The host's family sets; those whose key is in the filters and that are enabled narrow the list.
    pub(super) sets: &'a [FamilySet<'a>],
}

/// The sets that filter: their key is in `filters.sets` and they are enabled. A key naming a disabled set or no set
/// filters nothing, so a toggle the user cannot turn off never hides families.
fn active<'s, 'a>(sets: &'s [FamilySet<'a>], filters: &FontFilters) -> Vec<&'s FamilySet<'a>> {
    sets.iter().filter(|set| set.enabled && filters.sets.iter().any(|key| key == set.key)).collect()
}

/// How the interface language folds and orders text.
pub(super) struct Language<'a> {
    pub(super) folder: &'a Folder,
    pub(super) collator: &'a Collator,
}

/// The rows the list shows for `listing` and whether Show Similar Fonts had a reference to compare with: recents
/// first (when the query is empty and no filter is on), then the matching families, the best match first and ties
/// in the interface language's order. A list that is not `narrowed` has no filters and no recents.
pub(super) fn list(
    catalog: &Arc<Catalog>,
    infos: &FamilyInfos,
    listing: &Listing<'_>,
    language: &Language<'_>,
    narrowed: bool,
) -> (Vec<FamilyRow>, bool) {
    let default_filters = FontFilters::default();
    let filters = if narrowed { listing.filters } else { &default_filters };
    let settings = listing.settings;
    let english = settings.is_some_and(|s| s.names_in_english);
    let matcher = Matcher::new(language.folder, listing.query);
    let mut ranked: Vec<(Rank, FamilyRow)> = catalog
        .families()
        .iter()
        .filter(|family| !is_private_family(family))
        .filter_map(|family| {
            let info = infos.families.get(family)?;
            let shown = shown_name(info, family, listing.language, english);
            let names = [family.as_str(), shown.as_str()].into_iter().chain(info.localized.iter().map(|n| n.name.as_str()));
            let rank = names.filter_map(|name| matcher.score(name)).map(|m| m.rank).min()?;
            let row = FamilyRow {
                family: family.clone(),
                starred: settings.is_some_and(|s| s.favorites.iter().any(|f| f == family)),
                class: infos.traits(family).and_then(|t| t.class),
                format: Some(info.format),
                styles: info.styles,
                recent: false,
                shown,
            };
            Some((rank, row))
        })
        .collect();
    for name in listing.served {
        if infos.families.contains_key(name) || ranked.iter().any(|(_, r)| r.family == *name) {
            continue;
        }
        if let Some(found) = matcher.score(name) {
            let starred = settings.is_some_and(|s| s.favorites.iter().any(|f| f == name));
            let row = FamilyRow { family: name.clone(), shown: name.clone(), starred, class: None, format: None, styles: 0, recent: false };
            ranked.push((found.rank, row));
        }
    }
    ranked.sort_by(|(a_rank, a), (b_rank, b)| a_rank.cmp(b_rank).then_with(|| language.collator.compare(&a.shown, &b.shown)));
    let mut rows: Vec<FamilyRow> = ranked.into_iter().map(|(_, row)| row).collect();
    if filters.favorites {
        rows.retain(|r| r.starred);
    }
    if filters.variable {
        rows.retain(|r| r.format == Some(FontFormat::Variable));
    }
    if let Some(class) = filters.class {
        rows.retain(|r| r.class == Some(class));
    }
    if let Some(script) = filters.script {
        rows.retain(|r| infos.scripts(catalog, &r.family).contains(&script));
    }
    let on = active(listing.sets, filters);
    for set in &on {
        rows.retain(|r| set.position(&r.family).is_some());
    }
    // The first set that is on and ordered gives the order; the others only filter.
    if let Some(set) = on.iter().find(|set| set.ordered) {
        rows.sort_by_cached_key(|r| set.position(&r.family));
    }
    let mut comparable = true;
    if let Some(reference_family) = filters.similar.as_deref() {
        let reference = infos.traits(reference_family).filter(|t| t.class.is_some() || t.panose.iter().any(|d| *d > 1));
        if let Some(reference) = reference {
            let mut scored: Vec<(f32, FamilyRow)> = rows
                .drain(..)
                .filter_map(|r| {
                    let traits = infos.traits(&r.family)?;
                    let distance = if r.family == reference_family { -1.0 } else { reference.difference(&traits) };
                    Some((distance, r))
                })
                .collect();
            scored.sort_by(|a, b| a.0.total_cmp(&b.0));
            rows = scored.into_iter().take(SIMILAR_COUNT).map(|(_, r)| r).collect();
        } else {
            comparable = false;
        }
    }
    let narrowing = !on.is_empty() || (FontFilters { sets: Vec::new(), ..filters.clone() }).any();
    if narrowed
        && matcher.is_empty()
        && !narrowing
        && let Some(settings) = settings
    {
        let recent: Vec<FamilyRow> = settings
            .recent
            .iter()
            .take(settings.recent_limit)
            .filter_map(|f| rows.iter().find(|r| r.family == *f))
            .map(|r| FamilyRow { recent: true, ..r.clone() })
            .collect();
        rows.splice(0..0, recent);
    }
    (rows, comparable)
}

/// The host rows the list shows for `query`, as (group, row) in group order: every row for an empty query, else the
/// face and action rows whose name matches as a family name would. A notice speaks of its group as a whole, so it
/// shows only in the unfiltered list.
pub(super) fn host_rows(groups: &[HostFontGroup<'_>], query: &str, folder: &Folder) -> Vec<(usize, usize)> {
    let matcher = Matcher::new(folder, query);
    let listed =
        |row: &super::HostFontRow| matcher.is_empty() || (!matches!(row.kind, HostFontKind::Notice) && matcher.score(row.name.as_str()).is_some());
    groups
        .iter()
        .enumerate()
        .flat_map(|(group, rows)| rows.rows.iter().enumerate().map(move |(at, row)| (group, at, row)))
        .filter(|(_, _, row)| listed(row))
        .map(|(group, at, _)| (group, at))
        .collect()
}
