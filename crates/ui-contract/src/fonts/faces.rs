//! Faces and the providers that hold them: weight tweaks, the face list asked per family, the regular-weight
//! wrapper for the system font provider, and the baseline placement of fallback faces.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use egui::epaint::text::Tag;
use egui::{FallbackRequest, FontData, FontFamily, FontInsert, FontPriority, FontProvider, InsertFontFamily};
use skrifa::MetadataProvider as _;

use super::FontRef;

/// Sets a variable face's weight axis to `weight`, within the axis's range.
///
/// Leaves a face with no weight axis, or one whose weight is already set, as it is. Returns
/// whether it changed the face. Variable system fonts often default to their lightest instance,
/// e.g. Thin (100), so a face installed without this draws as hairlines.
pub fn set_weight(data: &mut FontData, weight: u16) -> bool {
    let wght = Tag::new(b"wght");
    if data.tweak.coords.as_ref().iter().any(|(tag, _)| *tag == wght) {
        return false;
    }
    let Some(axis) = data.variation_axes().into_iter().find(|axis| axis.tag == wght) else { return false };
    let value = f32::from(weight).clamp(axis.range.min, axis.range.max);
    if value == axis.default {
        return false;
    }
    data.tweak.coords.push(*b"wght", value);
    true
}

/// Wraps a font provider, usually the system font provider, so variable faces it finds draw
/// at each family's weight (regular, 400, unless set with [`Self::with_weight`]).
pub struct RegularWeight {
    inner: Arc<dyn FontProvider>,
    weights: Vec<(FontRef, u16)>,
}

impl RegularWeight {
    /// Wraps `inner`, with every family at the regular weight.
    pub fn new(inner: Arc<dyn FontProvider>) -> Self {
        Self { inner, weights: Vec::new() }
    }

    /// Draws faces found for `family` at `weight`, e.g. 600 for a semibold family.
    pub fn with_weight(mut self, family: FontRef, weight: u16) -> Self {
        self.weights.retain(|(f, _)| *f != family);
        self.weights.push((family, weight));
        self
    }

    fn weight(&self, family: &FontFamily) -> u16 {
        self.weights.iter().find(|(f, _)| f.family() == *family).map_or(400, |(_, w)| *w)
    }
}

impl FontProvider for RegularWeight {
    fn fonts_for_family(&self, family: &FontFamily) -> Vec<FontInsert> {
        let weight = self.weight(family);
        self.inner.fonts_for_family(family).into_iter().map(|insert| weighted(insert, weight)).collect()
    }

    fn font_for(&self, request: &FallbackRequest<'_>) -> Option<FontInsert> {
        let insert = self.inner.font_for(request)?;
        Some(weighted(insert, self.weight(request.family)))
    }
}

fn weighted(mut insert: FontInsert, weight: u16) -> FontInsert {
    // egui's face store keys faces by name, so a face set to another weight needs its own.
    if set_weight(&mut insert.data, weight) && weight != 400 {
        insert.name = format!("{} @{weight}", insert.name);
    }
    insert
}

/// A craft face's data, set to sit on the primary face's baseline (the suite's egui fork puts a fallback face's glyphs
/// on the primary face's baseline and grows the row to fit them).
pub(crate) fn baseline_fallback(mut data: FontData) -> FontData {
    data.tweak.set_baseline_fallback(true);
    data
}

/// Wraps the system font provider so the text faces it finds sit on the row's baseline as
/// fallbacks, as craft faces do. Emoji and symbol faces stay centred in the row.
struct OnBaseline(Arc<dyn FontProvider>);

impl FontProvider for OnBaseline {
    fn fonts_for_family(&self, family: &FontFamily) -> Vec<FontInsert> {
        self.0.fonts_for_family(family).into_iter().map(on_baseline_insert).collect()
    }

    fn font_for(&self, request: &FallbackRequest<'_>) -> Option<FontInsert> {
        self.0.font_for(request).map(on_baseline_insert)
    }
}

/// `provider` with its text faces on the row's baseline.
pub(crate) fn on_baseline(provider: Arc<dyn FontProvider>) -> Arc<dyn FontProvider> {
    Arc::new(OnBaseline(provider))
}

fn on_baseline_insert(mut insert: FontInsert) -> FontInsert {
    if is_text_face(&insert.name, &insert.data) {
        insert.data = baseline_fallback(insert.data);
    }
    insert
}

/// Whether a face draws text rather than emoji or symbols: it has no colour glyphs, and neither
/// its egui name nor its family name says emoji or symbols.
pub(crate) fn is_text_face(name: &str, data: &FontData) -> bool {
    let pictorial = |name: &str| {
        let name = name.to_lowercase();
        ["emoji", "symbol"].iter().any(|word| name.contains(word))
    };
    if pictorial(name) {
        return false;
    }
    let Ok(font) = skrifa::FontRef::from_index(data.bytes(), data.index) else { return true };
    let colour = [b"COLR", b"CBDT", b"sbix", b"SVG "].iter().any(|tag| font.table_data(skrifa::Tag::new(tag)).is_some());
    let family = font.localized_strings(skrifa::string::StringId::FAMILY_NAME).english_or_first().map(|s| s.to_string());
    !colour && !family.is_some_and(|family| pictorial(&family))
}

/// Embedded faces asked in order, per family, for a character no earlier face or provider has.
#[derive(Clone, Default)]
pub struct FaceList {
    faces: BTreeMap<String, FontData>,
    chains: BTreeMap<FontFamily, Vec<String>>,
}

impl FaceList {
    /// The faces asked for `family`, in order. A family without its own list uses the
    /// proportional one.
    pub fn chain(&self, family: &FontRef) -> &[String] {
        self.chain_of(&family.family())
    }

    fn chain_of(&self, family: &FontFamily) -> &[String] {
        self.chains.get(family).or_else(|| self.chains.get(&FontFamily::Proportional)).map_or(&[], Vec::as_slice)
    }

    /// Replaces `family`'s list. Names without data from `Self::insert` are skipped when asked.
    pub fn set_chain(&mut self, family: FontRef, names: Vec<String>) {
        self.chains.insert(family.family(), names);
    }

    /// Adds a face under `name` unless the list has one, and returns the name.
    pub(crate) fn add(&mut self, name: String, data: FontData) -> String {
        self.faces.entry(name.clone()).or_insert(data);
        name
    }

    /// Each family's list, for renaming entries in place.
    pub(crate) fn chains_mut(&mut self) -> impl Iterator<Item = (&FontFamily, &mut Vec<String>)> {
        self.chains.iter_mut()
    }

    /// Adds or replaces a face's data.
    pub fn insert(&mut self, name: &str, data: FontData) {
        self.faces.insert(name.to_owned(), data);
    }

    /// Whether the list has data for the face called `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.faces.contains_key(name)
    }

    /// The data of the face called `name`, with its tweak.
    pub fn get(&self, name: &str) -> Option<&FontData> {
        self.faces.get(name)
    }

    /// Every face in the list with its name, in name order.
    pub fn faces(&self) -> impl Iterator<Item = (&str, &FontData)> {
        self.faces.iter().map(|(name, data)| (name.as_str(), data))
    }

    /// Removes the face called `name` and drops it from every chain.
    pub fn remove(&mut self, name: &str) {
        self.faces.remove(name);
        for chain in self.chains.values_mut() {
            chain.retain(|n| n != name);
        }
    }
}

impl FontProvider for FaceList {
    /// The first face with the cluster's first character, preferring one that also has the
    /// cluster's other visible characters: egui shapes the whole cluster with this face.
    fn font_for(&self, request: &FallbackRequest<'_>) -> Option<FontInsert> {
        let mut chars = request.cluster.chars();
        let first = chars.next()?;
        let rest: Vec<char> = chars.filter(|&c| draws(c)).collect();
        let faces =
            self.chain_of(request.family).iter().filter_map(|name| Some((name, self.faces.get(name)?))).filter(|(_, data)| has_char(data, first));
        let (name, data) = faces.clone().find(|(_, data)| rest.iter().all(|&c| has_char(data, c))).or_else(|| faces.clone().next())?;
        Some(FontInsert::new(name, data.clone(), vec![InsertFontFamily { family: request.family.clone(), priority: FontPriority::Lowest }]))
    }
}

/// A context's installed [`FaceList`], swapped in place when the stack is installed again, so the
/// providers added before it keep their place, and the provider asked when the list misses too
/// (on the web, the one that fetches served faces).
#[derive(Default)]
pub(crate) struct Installed(RwLock<(FaceList, Option<Arc<dyn FontProvider>>)>);

impl Installed {
    /// Replaces the list and the provider asked when it misses.
    pub(crate) fn set(&self, faces: FaceList, miss: Option<Arc<dyn FontProvider>>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = (faces, miss);
    }
}

impl FontProvider for Installed {
    fn font_for(&self, request: &FallbackRequest<'_>) -> Option<FontInsert> {
        let (faces, miss) = &*self.0.read().unwrap_or_else(PoisonError::into_inner);
        faces.font_for(request).or_else(|| miss.as_ref()?.font_for(request))
    }
}

/// Whether the face's character map has `c`.
pub(crate) fn has_char(data: &FontData, c: char) -> bool {
    skrifa::FontRef::from_index(data.bytes(), data.index).is_ok_and(|font| font.charmap().map(c).is_some())
}

/// Whether `c` shows a glyph: not whitespace, a control, a format character or another invisible
/// mark.
pub(crate) fn draws(c: char) -> bool {
    !(c.is_whitespace()
        || c.is_control()
        || matches!(
            c,
            '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}'
        )
        || matches!(c, '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}'))
}
