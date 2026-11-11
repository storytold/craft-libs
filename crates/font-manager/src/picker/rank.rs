//! What the picker knows about the installed families: names in the interface language, classes and
//! traits read from the faces' tables, the filtered list and its order, and the sample text of a row.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use craft_font_catalog::{Catalog, FaceInfo, LocalizedName};
use craft_i18n::matcher::{Matcher, Rank};
use craft_i18n::{Collator, Folder};
use skrifa::MetadataProvider as _;
use skrifa::raw::{FontRef, TableProvider};

use super::{FamilyRow, FontClass, FontFilters, FontPickerSettings, SIMILAR_COUNT};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the catalog says about a family, read once per catalog generation.
pub(super) struct Info {
    pub(super) localized: Vec<LocalizedName>,
    pub(super) variable: bool,
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

/// The families of one catalog generation with their traits and sample texts, filled on first use.
pub(super) struct Infos {
    pub(super) families: HashMap<String, Info>,
    traits: Mutex<HashMap<String, Option<Traits>>>,
    samples: Mutex<HashMap<(String, String), String>>,
}

impl Infos {
    fn build(catalog: &Catalog) -> Infos {
        let mut groups: HashMap<String, Vec<FaceInfo>> = HashMap::new();
        for face in catalog.faces().iter() {
            groups.entry(face.family.clone()).or_default().push(face.clone());
        }
        let families = groups
            .into_iter()
            .filter_map(|(family, faces)| {
                let regular = regular_face(&faces)?.clone();
                let localized = faces.iter().flat_map(|f| f.localized_families.iter().cloned()).collect();
                Some((family, Info { localized, variable: faces.iter().any(|f| f.variable), regular }))
            })
            .collect();
        Infos { families, traits: Mutex::new(HashMap::new()), samples: Mutex::new(HashMap::new()) }
    }

    /// The family's traits, read from its regular face on first use.
    pub(super) fn traits(&self, catalog: &Catalog, family: &str) -> Option<Traits> {
        if let Some(t) = lock(&self.traits).get(family) {
            return t.clone();
        }
        let traits = self.families.get(family).and_then(|i| read_traits(catalog, &i.regular));
        lock(&self.traits).insert(family.to_owned(), traits.clone());
        traits
    }

    /// The class if the family's traits were read already.
    pub(super) fn known_class(&self, family: &str) -> Option<FontClass> {
        lock(&self.traits).get(family).and_then(|t| t.as_ref()).and_then(|t| t.class)
    }

    /// What the row's sample says: `sample` when the family's face draws it, else its name, else the
    /// first glyphs the face has (a symbol or non-Latin face).
    pub(super) fn sample_text(&self, catalog: &Catalog, family: &str, sample: &str, shown: &str) -> String {
        let key = (family.to_owned(), sample.to_owned());
        if let Some(text) = lock(&self.samples).get(&key) {
            return text.clone();
        }
        let text = self.families.get(family).and_then(|info| draws(catalog, &info.regular, sample, shown)).unwrap_or_default();
        lock(&self.samples).insert(key, text.clone());
        text
    }
}

fn draws(catalog: &Catalog, face: &FaceInfo, sample: &str, shown: &str) -> Option<String> {
    let data = catalog.load(face).ok()?;
    let font = FontRef::from_index(&data.bytes, face.index).ok()?;
    let map = font.charmap();
    let drawn = |t: &str| t.chars().filter(|c| !c.is_whitespace()).all(|c| map.map(c).is_some());
    if drawn(sample) {
        return Some(sample.to_owned());
    }
    if drawn(shown) {
        return Some(shown.to_owned());
    }
    Some(map.mappings().filter_map(|(cp, _)| char::from_u32(cp)).filter(|c| !c.is_whitespace() && !c.is_control()).take(6).collect())
}

type CacheEntry = (Weak<Catalog>, u64, Arc<Infos>);
static CACHE: Mutex<Vec<CacheEntry>> = Mutex::new(Vec::new());

/// The family data of `catalog`, rebuilt when its generation moves.
pub(super) fn infos(catalog: &Arc<Catalog>) -> Arc<Infos> {
    let generation = catalog.generation();
    let same = |w: &Weak<Catalog>| std::ptr::eq(w.as_ptr(), Arc::as_ptr(catalog));
    let mut cache = lock(&CACHE);
    cache.retain(|(w, _, _)| w.strong_count() > 0);
    if let Some((_, _, infos)) = cache.iter().find(|(w, g, _)| *g == generation && same(w)) {
        return Arc::clone(infos);
    }
    cache.retain(|(w, _, _)| !same(w));
    let infos = Arc::new(Infos::build(catalog));
    cache.push((Arc::downgrade(catalog), generation, Arc::clone(&infos)));
    infos
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

/// A face's class, PANOSE and weight from its `OS/2` and `post` tables.
fn read_traits(catalog: &Catalog, face: &FaceInfo) -> Option<Traits> {
    let data = catalog.load(face).ok()?;
    let font = FontRef::from_index(&data.bytes, face.index).ok()?;
    let os2 = font.os2().ok();
    let family_class = os2.as_ref().map_or(0, |t| t.s_family_class());
    let mut panose = [0u8; 10];
    if let Some(bytes) = os2.as_ref().map(|t| t.panose_10()) {
        for (slot, b) in panose.iter_mut().zip(bytes.iter()) {
            *slot = *b;
        }
    }
    let weight = os2.as_ref().map_or(400, |t| t.us_weight_class()).clamp(1, 1000);
    let fixed_pitch = font.post().ok().is_some_and(|p| p.is_fixed_pitch() != 0);
    Some(Traits { class: classify(&face.family, family_class, panose, fixed_pitch), panose, weight })
}

/// The class of a family from its regular face, `None` when the face can't be read.
pub(super) fn class_of(catalog: &Catalog, family: &str) -> Option<FontClass> {
    let faces = catalog.styles(family);
    let face = regular_face(&faces)?;
    read_traits(catalog, face)?.class
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
    /// The committed family, the reference of Show Similar Fonts.
    pub(super) current: &'a str,
    /// The interface language code the names are shown in.
    pub(super) language: &'a str,
}

/// How the interface language folds and orders text.
pub(super) struct Language<'a> {
    pub(super) folder: &'a Folder,
    pub(super) collator: &'a Collator,
}

/// The rows the list shows for `listing` and whether Show Similar Fonts had a reference to compare with: recents
/// first (when the query is empty and no filter is on), then the matching families, the best match first and ties
/// in the interface language's order. A list that is not `narrowed` has no filters and no recents.
pub(super) fn list(catalog: &Arc<Catalog>, infos: &Infos, listing: &Listing<'_>, language: &Language<'_>, narrowed: bool) -> (Vec<FamilyRow>, bool) {
    let default_filters = FontFilters::default();
    let filters = if narrowed { listing.filters } else { &default_filters };
    let settings = listing.settings;
    let english = settings.is_some_and(|s| s.names_in_english);
    let matcher = Matcher::new(language.folder, listing.query);
    let mut ranked: Vec<(Rank, FamilyRow)> = catalog
        .families()
        .iter()
        .filter_map(|family| {
            let info = infos.families.get(family)?;
            let shown = shown_name(info, family, listing.language, english);
            let names = [family.as_str(), shown.as_str()].into_iter().chain(info.localized.iter().map(|n| n.name.as_str()));
            let rank = names.filter_map(|name| matcher.score(name)).map(|m| m.rank).min()?;
            let row = FamilyRow {
                family: family.clone(),
                starred: settings.is_some_and(|s| s.favorites.iter().any(|f| f == family)),
                class: infos.known_class(family),
                variable: info.variable,
                recent: false,
                shown,
            };
            Some((rank, row))
        })
        .collect();
    ranked.sort_by(|(a_rank, a), (b_rank, b)| a_rank.cmp(b_rank).then_with(|| language.collator.compare(&a.shown, &b.shown)));
    let mut rows: Vec<FamilyRow> = ranked.into_iter().map(|(_, row)| row).collect();
    if filters.favorites {
        rows.retain(|r| r.starred);
    }
    if filters.variable {
        rows.retain(|r| r.variable);
    }
    if let Some(class) = filters.class {
        rows.retain_mut(|r| {
            r.class = infos.traits(catalog, &r.family).and_then(|t| t.class);
            r.class == Some(class)
        });
    }
    let mut comparable = true;
    if filters.similar {
        let reference = infos.traits(catalog, listing.current).filter(|t| t.class.is_some() || t.panose.iter().any(|d| *d > 1));
        if let Some(reference) = reference {
            let mut scored: Vec<(f32, FamilyRow)> = rows
                .drain(..)
                .filter_map(|r| {
                    let traits = infos.traits(catalog, &r.family)?;
                    let distance = if r.family == listing.current { -1.0 } else { reference.difference(&traits) };
                    Some((distance, r))
                })
                .collect();
            scored.sort_by(|a, b| a.0.total_cmp(&b.0));
            rows = scored.into_iter().take(SIMILAR_COUNT).map(|(_, r)| r).collect();
        } else {
            comparable = false;
        }
    }
    if narrowed
        && matcher.is_empty()
        && !filters.any()
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
