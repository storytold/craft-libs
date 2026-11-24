//! The installed fonts, indexed once with fontique for every locale. The lookup is
//! egui_system_font_provider 0.37's, whose provider builds its own index per locale.

use std::sync::{Arc, LazyLock};

use egui::{FallbackRequest, FontData, FontFamily, FontInsert, FontPriority, FontProvider, InsertFontFamily};
use fontique::{
    Collection, CollectionOptions, FallbackKey, GenericFamily, Language, QueryFamily, QueryFont, QueryStatus, Script, SourceCache, SourceCacheOptions,
};

use super::system::{FontIndex, Lookup, Shared};

pub(super) struct Index {
    collection: Collection,
    source_cache: SourceCache,
}

static INDEX: Shared<Index> = Shared::new(build);

#[cfg(test)]
pub(super) static BUILDS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn build() -> Index {
    #[cfg(test)]
    BUILDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
pub(super) fn locale_language(locale: &str) -> Option<Language> {
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

/// The script of a character, `None` for characters shared between scripts.
fn script_of(c: char) -> Option<Script> {
    use unicode_script::UnicodeScript as _;
    match c.script() {
        unicode_script::Script::Common | unicode_script::Script::Inherited | unicode_script::Script::Unknown => None,
        script => Script::parse(script.short_name()).ok(),
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
