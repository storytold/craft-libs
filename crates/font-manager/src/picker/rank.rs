//! What the picker knows about the installed families: names in the interface language, classes and
//! traits read from the faces' tables, the filtered list and its order, and the sample text of a row.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use craft_font_catalog::{Catalog, FaceInfo, LocalizedName, Outlines};
use craft_i18n::matcher::{Matcher, Rank};
use craft_i18n::{Collator, Folder};
use skrifa::MetadataProvider as _;
use skrifa::raw::{FontRef, TableProvider};

use super::{FamilyRow, FontClass, FontFilters, FontFormat, FontPickerSettings, SIMILAR_COUNT};

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

/// The families of one catalog generation with their traits and sample texts, filled on first use.
pub(super) struct Infos {
    pub(super) families: HashMap<String, Info>,
    traits: Mutex<HashMap<String, Option<Traits>>>,
    samples: Mutex<HashMap<(String, String, String), Samples>>,
}

/// The texts a row's thumbnails paint.
#[derive(Clone, Default)]
pub(super) struct Samples {
    /// The Latin sample, the name or the face's first glyphs.
    pub(super) latin: String,
    /// The Arabic second sample, when the face draws it and the Latin sample.
    pub(super) arabic: Option<String>,
}

/// The Arabic letters a face must map besides the Arabic sample: alef, lam, meem and ain.
const ARABIC_CORE: &str = "\u{627}\u{644}\u{645}\u{639}";

/// Whether a face gets the Arabic second sample: `drawn` (the face's character map) draws the Latin sample, the core
/// Arabic letters and the Arabic sample.
fn has_arabic_sample(drawn: &dyn Fn(&str) -> bool, latin: &str, arabic: &str) -> bool {
    drawn(latin) && drawn(ARABIC_CORE) && drawn(arabic)
}

/// The Japanese Kana sample a font that draws it gets: the canonical five basic vowels.
const KANA_SAMPLE: &str = "\u{3042}\u{3044}\u{3046}\u{3048}\u{304A}"; // "あいうえお"

/// Whether a face gets the Japanese Kana sample: `drawn` draws the five basic vowels.
fn has_kana_sample(drawn: &dyn Fn(&str) -> bool) -> bool {
    drawn(KANA_SAMPLE)
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

    /// What the row's samples say: `sample` when the family's face draws it, else its name, else the
    /// first glyphs the face has (a symbol or non-Latin face); and `arabic` as a second sample when the face
    /// draws that too. Both come from one read of the face's character map.
    pub(super) fn sample_text(&self, catalog: &Catalog, family: &str, sample: &str, arabic: &str, shown: &str) -> Samples {
        let key = (family.to_owned(), sample.to_owned(), arabic.to_owned());
        if let Some(text) = lock(&self.samples).get(&key) {
            return text.clone();
        }
        let text = self.families.get(family).and_then(|info| draws(catalog, &info.regular, sample, arabic, shown)).unwrap_or_default();
        lock(&self.samples).insert(key, text.clone());
        text
    }
}

fn draws(catalog: &Catalog, face: &FaceInfo, sample: &str, arabic: &str, shown: &str) -> Option<Samples> {
    let data = catalog.load(face).ok()?;
    let font = FontRef::from_index(&data.bytes, face.index).ok()?;
    let map = font.charmap();
    let drawn = |t: &str| t.chars().filter(|c| !c.is_whitespace()).all(|c| map.map(c).is_some());
    if has_kana_sample(&drawn) {
        let arabic = has_arabic_sample(&drawn, sample, arabic).then(|| arabic.to_owned());
        return Some(Samples { latin: KANA_SAMPLE.to_owned(), arabic });
    }
    if drawn(sample) {
        let arabic = has_arabic_sample(&drawn, sample, arabic).then(|| arabic.to_owned());
        return Some(Samples { latin: sample.to_owned(), arabic });
    }
    if drawn(shown) {
        return Some(Samples { latin: shown.to_owned(), arabic: None });
    }
    let first = map.mappings().filter_map(|(cp, _)| char::from_u32(cp)).filter(|c| !c.is_whitespace() && !c.is_control()).take(6).collect();
    Some(Samples { latin: first, arabic: None })
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
pub(super) fn shown_name(localized: &[LocalizedName], family: &str, language: &str, english: bool) -> String {
    let primary = |tag: &str| tag.split(['-', '_']).next().unwrap_or(tag).to_ascii_lowercase();
    let is_ja = primary(language) == "ja";
    // In a Japanese interface, Photoshop always lists Japanese font names in Japanese,
    // regardless of the English font names preference. In other languages, English names preference decides.
    if (!is_ja && english) || primary(language) == "en" {
        return family.to_owned();
    }
    let exact = |n: &&LocalizedName| n.language.as_deref().is_some_and(|l| l.replace('_', "-").eq_ignore_ascii_case(&language.replace('_', "-")));
    let same_language = |n: &&LocalizedName| n.language.as_deref().is_some_and(|l| primary(l) == primary(language));
    localized.iter().find(exact).or_else(|| localized.iter().find(same_language)).map_or_else(|| family.to_owned(), |n| n.name.clone())
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

fn is_cjk_boundary(c: char) -> bool {
    matches!(
        c,
        '\u{3040}'..='\u{309F}'
        | '\u{30A0}'..='\u{30FF}'
        | '\u{31F0}'..='\u{31FF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{AC00}'..='\u{D7AF}'
    )
}

/// A CJK Song (宋) serif's Latin name: a foundry-prefixed stem such as `STSong`, `STZhongsong` or `FZFangSong`.
/// A bare English word that ends in `song` is not one — `WindSong` is a script family, and reading its ending as the
/// Chinese serif marker filed a script face under Serif.
fn is_cjk_song(word: &str) -> bool {
    const FOUNDRIES: [&str; 9] = ["st", "fz", "hg", "hy", "df", "gb", "sim", "adobe", "noto"];
    ["song", "songti", "sungti"].iter().any(|ending| word.len() > ending.len() && word.ends_with(ending))
        && FOUNDRIES.iter().any(|prefix| word.starts_with(prefix))
}

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
    if words.iter().any(|w| matches!(*w, "mono" | "monospace" | "monospaced" | "code" | "courier" | "等幅") || w.starts_with("consol"))
        || has(&["等幅"])
    {
        Some(FontClass::Monospace)
    } else if has(&["slab"]) {
        Some(FontClass::SlabSerif)
    } else if has(&["sans", "grotesk", "grotesque", "gothic", "ゴシック", "ｺﾞｼｯｸ", "meiryo", "メイリオ", "黑体", "黑體", "丸ゴシック"])
    {
        Some(FontClass::SansSerif)
    } else if has(&["serif", "roman", "garamond", "times", "bodoni", "baskerville", "caslon", "mincho", "明朝", "宋体", "宋體"])
        || name
            .split(|c: char| !c.is_alphabetic() || is_cjk_boundary(c))
            .any(|w| matches!(w, "sung" | "song" | "sungti" | "songti") || is_cjk_song(w))
    {
        Some(FontClass::Serif)
    } else if has(&["hand", "marker", "brush", "chalk", "kyokasho", "教科書", "ポップ", "ﾎﾟｯﾌﾟ"])
        || name.ends_with("poptai")
        || name.split(|c: char| !c.is_alphabetic() || is_cjk_boundary(c)).any(|w| w == "poptai")
    {
        Some(FontClass::Handwritten)
    } else if has(&["script", "gyosho", "行書", "kaisho", "楷書", "sousho", "草書", "reisho", "隷書", "kanteiryu", "勘亭流"]) {
        Some(FontClass::Script)
    } else if name
        .split(|c: char| !c.is_alphabetic() || is_cjk_boundary(c))
        .any(|w| matches!(w, "hei" | "heiti") || w.ends_with("hei") || w.ends_with("heiti"))
    {
        // A `hei` (黑) name is a CJK sans serif, but a name that also carries a script marker is a script: this
        // waits until after the script markers, so `Xhei Script` keeps Script and `SimHei` stays a sans.
        Some(FontClass::SansSerif)
    } else if name.split(|c: char| !c.is_alphabetic() || is_cjk_boundary(c)).any(|w| w == "pop") {
        // A standalone `pop` is a rounded Japanese display face, not a script: after the script markers, so
        // `Pop Script` keeps Script.
        Some(FontClass::Handwritten)
    } else if name.split(|c: char| !c.is_alphabetic() || is_cjk_boundary(c)).any(|w| matches!(w, "maru")) {
        // A standalone `maru` is a round gothic, so a sans serif, but only after the handwritten
        // and script markers: `Maru Script` and `Maru Hand` keep their own class.
        Some(FontClass::SansSerif)
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
    /// Families the host can fetch but the catalog lacks; they list with the matches, by rank and name, with no class, format or sample.
    pub(super) served: &'a [String],
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
            let shown = shown_name(&info.localized, family, listing.language, english);
            let names = [family.as_str(), shown.as_str()].into_iter().chain(info.localized.iter().map(|n| n.name.as_str()));
            let rank = names.filter_map(|name| matcher.score(name)).map(|m| m.rank).min()?;
            let row = FamilyRow {
                family: family.clone(),
                starred: settings.is_some_and(|s| s.favorites.iter().any(|f| f == family)),
                class: infos.known_class(family),
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

#[cfg(test)]
mod tests {
    use craft_ui::testing::trial;

    use super::{ARABIC_CORE, has_arabic_sample};

    const LATIN: &str = "AaBbCc";
    /// "أبجد هوز".
    const ARABIC: &str = "\u{623}\u{628}\u{62C}\u{62F} \u{647}\u{648}\u{632}";

    fn maps(chars: &str) -> impl Fn(&str) -> bool + '_ {
        move |text| text.chars().filter(|c| !c.is_whitespace()).all(|c| chars.contains(c))
    }

    #[trial]
    fn an_arabic_sample_needs_the_latin_sample_and_every_arabic_test_letter() {
        let all = format!("{LATIN}{ARABIC_CORE}{ARABIC}");
        assert!(has_arabic_sample(&maps(&all), LATIN, ARABIC));
        assert!(!has_arabic_sample(&maps(LATIN), LATIN, ARABIC), "a Latin-only face");
        for missing in LATIN.chars().chain(ARABIC_CORE.chars()).chain(ARABIC.chars().filter(|c| !c.is_whitespace())) {
            let partial: String = all.chars().filter(|c| *c != missing).collect();
            assert!(!has_arabic_sample(&maps(&partial), LATIN, ARABIC), "without U+{:04X}", missing as u32);
        }
    }

    #[trial]
    fn a_kana_sample_needs_every_canonical_vowel() {
        assert!(super::has_kana_sample(&maps(super::KANA_SAMPLE)));
        assert!(!super::has_kana_sample(&maps(LATIN)), "a Latin-only face");
        for missing in super::KANA_SAMPLE.chars() {
            let partial: String = super::KANA_SAMPLE.chars().filter(|c| *c != missing).collect();
            assert!(!super::has_kana_sample(&maps(&partial)), "without U+{:04X}", missing as u32);
        }
    }

    #[trial]
    fn classifies_japanese_and_cjk_families() {
        use super::classify;
        use crate::picker::FontClass;
        const NONE: [u8; 10] = [0; 10];
        assert_eq!(classify("MS Mincho", 0, NONE, false), Some(FontClass::Serif));
        assert_eq!(classify("ＭＳ 明朝", 0, NONE, false), Some(FontClass::Serif));
        assert_eq!(classify("Yu Gothic", 0, NONE, false), Some(FontClass::SansSerif));
        assert_eq!(classify("游ゴシック", 0, NONE, false), Some(FontClass::SansSerif));
        assert_eq!(classify("Meiryo", 0, NONE, false), Some(FontClass::SansSerif));
        assert_eq!(classify("メイリオ", 0, NONE, false), Some(FontClass::SansSerif));
        assert_eq!(classify("HG行書体", 0, NONE, false), Some(FontClass::Script));
        assert_eq!(classify("HG正楷書体-PRO", 0, NONE, false), Some(FontClass::Script));
        assert_eq!(classify("UD デジタル 教科書体", 0, NONE, false), Some(FontClass::Handwritten));
        assert_eq!(classify("HG創英角ﾎﾟｯﾌﾟ体", 0, NONE, false), Some(FontClass::Handwritten));
        assert_eq!(classify("MS Gothic 等幅", 0, NONE, false), Some(FontClass::Monospace));
    }

    #[trial]
    fn a_japanese_interface_lists_japanese_family_names() {
        use super::shown_name;
        use craft_font_catalog::LocalizedName;

        let localized = vec![
            LocalizedName { language: Some("ja".into()), name: "ＭＳ ゴシック".into() },
            LocalizedName { language: Some("en".into()), name: "MS Gothic".into() },
        ];

        // In a Japanese interface, always Japanese regardless of the English font names preference:
        assert_eq!(shown_name(&localized, "MS Gothic", "ja", true), "ＭＳ ゴシック");
        assert_eq!(shown_name(&localized, "MS Gothic", "ja", false), "ＭＳ ゴシック");
        // In other languages:
        assert_eq!(shown_name(&localized, "MS Gothic", "en", true), "MS Gothic");
        assert_eq!(shown_name(&localized, "MS Gothic", "en", false), "MS Gothic");
        assert_eq!(shown_name(&localized, "MS Gothic", "fr", true), "MS Gothic");
    }
}
