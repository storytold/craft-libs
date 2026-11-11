//! The installed fonts, indexed once with fontique for every locale. The lookup is
//! egui_system_font_provider 0.37's, whose provider builds its own index per locale.

use std::sync::{Arc, LazyLock};

use egui::{FallbackRequest, FontData, FontFamily, FontInsert, FontPriority, FontProvider, InsertFontFamily};
use fontique::{
    Attributes, Collection, CollectionOptions, FallbackKey, FamilyId, FontStyle, FontWeight, FontWidth, GenericFamily, Language, QueryFamily,
    QueryFont, QueryStatus, Script, SourceCache, SourceCacheOptions,
};

use super::index::{FontIndex, Lookup, Shared};
use super::ui_face::{self, SystemUiFace, instance};
use crate::fonts::script;

pub(super) struct Index {
    collection: Collection,
    source_cache: SourceCache,
}

static INDEX: Shared<Index> = Shared::new(build, ui_face::index_built);

fn build() -> Index {
    Index {
        collection: Collection::new(CollectionOptions { shared: false, system_fonts: true }),
        source_cache: SourceCache::new(SourceCacheOptions::default()),
    }
}

/// A provider over the shared index that prefers `locale`'s fonts. With `wait`, a lookup before the
/// index is built blocks until it is; without, it finds nothing (see [`start`]).
pub(super) fn provider(locale: &str, wait: bool) -> Arc<dyn FontProvider> {
    Arc::new(Lookup::new(&INDEX, locale_language(locale), wait))
}

/// The language a locale prefers fonts for: a language tag (`zh-Hant`) or a POSIX locale
/// (`zh_TW.UTF-8`). `C`, `POSIX` and anything else without a language prefer none.
fn locale_language(locale: &str) -> Option<Language> {
    let tag = locale.split(['.', '@']).next().unwrap_or_default();
    if matches!(tag, "C" | "POSIX") {
        return None;
    }
    Language::parse(tag).ok()
}

/// Start building the index in the background. `ctx` reloads its fonts once it is built if a
/// lookup found nothing meanwhile.
pub(super) fn start(ctx: &egui::Context) {
    INDEX.start(Some(ctx));
}

/// Start building the index in the background, for a caller that doesn't look fonts up.
pub(super) fn start_building() {
    INDEX.start(None);
}

/// The OS's interface face from the shared index, waiting until it is built.
pub(super) fn resolve_system_ui_face() -> Option<SystemUiFace> {
    INDEX.with(|index| resolve_system_ui(&mut index.collection, &mut index.source_cache)).flatten()
}

/// Families Windows 11 names its interface face, ahead of fontique's `system-ui` (Segoe UI).
const PREFERRED_SYSTEM_UI: &[&str] = if cfg!(windows) { &["Segoe UI Variable", "Segoe UI Variable Text"] } else { &[] };

/// The first family among the preferred ones, `system-ui` and `sans-serif` with a regular face
/// that draws Latin, at regular and semibold weight.
fn resolve_system_ui(collection: &mut Collection, source_cache: &mut SourceCache) -> Option<SystemUiFace> {
    let mut families: Vec<FamilyId> = PREFERRED_SYSTEM_UI.iter().filter_map(|name| collection.family_id(name)).collect();
    for generic in [GenericFamily::SystemUi, GenericFamily::SansSerif] {
        families.extend(collection.generic_families(generic));
    }
    let mut seen = std::collections::BTreeSet::new();
    families.retain(|id| seen.insert(id.to_u64()));
    families.into_iter().find_map(|id| {
        let regular = match_weight(collection, source_cache, id, 400).filter(|(font, _)| can_render(font, 'a'))?;
        let family = collection.family_name(id)?.to_owned();
        let semibold = match_weight(collection, source_cache, id, 600).filter(|(font, _)| !font.synthesis.embolden());
        let regular = face_data(regular.0, 400);
        let (semibold, semibold_weight) = match semibold {
            Some((font, weight)) => {
                let data = face_data(font, 600);
                let weight = wght_of(&data).unwrap_or(weight);
                (data, weight)
            }
            None => (regular.clone(), 400),
        };
        Some(SystemUiFace::new(family, regular, semibold, semibold_weight))
    })
}

/// The family's face that best matches `weight`, upright at normal width, with its static weight.
fn match_weight(collection: &mut Collection, source_cache: &mut SourceCache, id: FamilyId, weight: u16) -> Option<(QueryFont, u16)> {
    let mut query = collection.query(source_cache);
    query.set_families([QueryFamily::Id(id)]);
    query.set_attributes(Attributes::new(FontWidth::NORMAL, FontStyle::Normal, FontWeight::new(f32::from(weight))));
    let mut found = None;
    query.matches_with(|font| {
        found = Some(font.clone());
        QueryStatus::Stop
    });
    drop(query);
    let font = found?;
    let info = collection.family(font.family.0)?.fonts().get(font.family.1)?.clone();
    // Static weights are whole numbers from 1 to 1000.
    Some((font, info.weight().value().round() as u16))
}

/// A matched font as egui face data, at the variation fontique suggests for `weight` and the
/// system face's optical size. A variable face's `wght` is always set, so the stack's family
/// weights ([`crate::fonts::set_weight`]) leave it as it is.
fn face_data(font: QueryFont, weight: u16) -> FontData {
    let coords = font.synthesis.variation_settings().iter().map(|(tag, value)| (tag.to_be_bytes(), *value));
    let coords: Vec<([u8; 4], f32)> = coords.chain([(*b"wght", f32::from(weight))]).collect();
    let (blob, _) = font.blob.into_raw_parts();
    instance(FontData::from_blob(blob, font.index), coords)
}

/// The `wght` coordinate a face is set to.
fn wght_of(data: &FontData) -> Option<u16> {
    let wght = egui::epaint::text::Tag::new(b"wght");
    // Clamped to the font's axis, which lies within 1 to 1000.
    data.tweak.coords.as_ref().iter().find(|(tag, _)| *tag == wght).map(|(_, value)| value.round() as u16)
}

impl FontIndex for Index {
    type Locale = Option<Language>;

    fn font_for(&mut self, request: &FallbackRequest<'_>, locale: &Option<Language>) -> Option<FontInsert> {
        let FallbackRequest { cluster, family } = request;
        let base_char = cluster.chars().next()?;
        let Self { collection, source_cache } = self;
        let generic = match family {
            FontFamily::Monospace => GenericFamily::Monospace,
            FontFamily::Proportional | FontFamily::Name(_) => GenericFamily::SansSerif,
        };

        let font = if let Some(script) = script_of(base_char) {
            // A script's fallback can be one locale's font, e.g. a Japanese font for Han that
            // lacks many Simplified Chinese characters, so the script's other locales follow.
            std::iter::once(locale.as_ref()).chain(other_locales(script).iter().map(Some)).find_map(|locale| {
                let mut query = collection.query(source_cache);
                query.set_families([QueryFamily::Generic(generic)]);
                query.set_fallbacks(fallback_key(script, locale));
                find_font(&mut query, base_char)
            })
        } else {
            // Punctuation, symbols and emoji belong to no script and have no fallback.
            let mut query = collection.query(source_cache);
            query.set_families(
                [generic, GenericFamily::SystemUi, GenericFamily::Emoji, GenericFamily::Math, GenericFamily::Serif, GenericFamily::Monospace]
                    .map(QueryFamily::Generic),
            );
            find_font(&mut query, base_char)
        }?;

        let family_name = collection.family_name(font.family.0).unwrap_or("unknown");
        let name = format!("system:{family_name}:{}:{}", font.family.1, font.index);
        let (blob, _) = font.blob.into_raw_parts();
        Some(FontInsert {
            name,
            data: FontData::from_blob(blob, font.index),
            families: vec![InsertFontFamily { family: (*family).clone(), priority: FontPriority::Lowest }],
        })
    }
}

/// The first font in the query that can draw `c`.
fn find_font(query: &mut fontique::Query<'_>, c: char) -> Option<QueryFont> {
    let mut found = None;
    query.matches_with(|font| {
        if can_render(font, c) {
            found = Some(font.clone());
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    found
}

const HAN: Script = Script::from_bytes(*b"Hani");

/// The locales of the Han fonts, Simplified Chinese first. That is `zh-SG` rather than `zh-CN`:
/// fontique's default Han locale is `zh-CN`, whose font is whatever the first locale-less Han
/// lookup returned, often the Japanese one.
static HAN_LOCALES: LazyLock<Vec<Language>> =
    LazyLock::new(|| ["zh-SG", "zh-TW", "ja", "ko"].iter().filter_map(|locale| Language::parse(locale).ok()).collect());

fn fallback_key(script: Script, locale: Option<&Language>) -> FallbackKey {
    let locale = match locale {
        Some(locale) if script == HAN && is_simplified_chinese(locale) => HAN_LOCALES.first(),
        locale => locale,
    };
    FallbackKey::new(script, locale)
}

/// `zh`, `zh-CN`, `zh-Hans` and the like, as fontique tells Simplified from Traditional Chinese.
fn is_simplified_chinese(locale: &Language) -> bool {
    locale.language() == "zh" && locale.script() != Some("Hant") && !matches!(locale.region(), Some("HK" | "TW" | "MO"))
}

/// Only Han needs other locales: its fonts are per locale, each with different characters.
fn other_locales(script: Script) -> &'static [Language] {
    if script == HAN { &HAN_LOCALES } else { &[] }
}

/// The script of a character by the one character-to-script table, `None` for symbols, mathematics, emoji and
/// characters of no listed block, which have no fallback.
fn script_of(c: char) -> Option<Script> {
    match script::script_of(c)? {
        script::EMOJI | script::SYMBOLS | script::MATH => None,
        tag => Script::parse(tag).ok(),
    }
}

/// Whether the font has an outline, bitmap or colour glyph for `c`.
fn can_render(font: &QueryFont, c: char) -> bool {
    use skrifa::MetadataProvider as _;
    let Some(glyph_id) = font.charmap().and_then(|charmap| charmap.map(c)) else { return false };
    let glyph_id = skrifa::GlyphId::new(glyph_id);
    if glyph_id == skrifa::GlyphId::NOTDEF {
        return false;
    }
    let Ok(font_ref) = skrifa::FontRef::from_index(font.blob.as_ref(), font.index) else { return false };
    font_ref.outline_glyphs().get(glyph_id).is_some()
        || font_ref.color_glyphs().get(glyph_id).is_some()
        || font_ref.bitmap_strikes().glyph_for_size(skrifa::instance::Size::unscaled(), glyph_id).is_some()
}
