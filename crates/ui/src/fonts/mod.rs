//! The UI font stack every Craft app installs.
//!
//! egui draws a character only if one of its installed faces has it. [`UiFonts`] builds the same
//! fallback order for every app, so a label or file name in any shipped script draws in every app:
//!
//! 1. the app's own faces for the family (UI face, semibold, mono), or egui's default faces for a
//!    family the app has none for;
//! 2. craft-fonts faces for the UI language;
//! 3. craft-fonts faces for every other script the app ships, so a Japanese interface still draws
//!    Arabic file names;
//! 4. installed system fonts ([`system_font_provider`]), at regular weight ([`RegularWeight`]);
//! 5. emoji faces ([`Emoji`]) and, behind the app's own faces, egui's default faces.
//!
//! When the faces of step 1 don't cover the UI language's main script (Arabic, Thai, Georgian),
//! that script's face goes first. egui takes a row's height from the first face of a family, so
//! the UI language's own face sets the row height and its marks aren't clipped. Han, kana and
//! Hangul sit inside the em square and fit a Latin row, so in Chinese, Japanese and Korean UIs the
//! Latin face stays first, and rows and widgets keep the same height as in a Latin UI: CJK faces
//! sit on the first face's baseline, scaled down only if a glyph would leave the row
//! ([`CjkRows::Fit`]). Monospace, and every family declared [`Family::monospaced`], always keeps
//! its Latin face first. On the suite's egui fork, the other craft faces and installed text faces
//! used as fallbacks sit on the row's baseline and rows grow to fit them by themselves; emoji and
//! symbol faces stay centred.
//!
//! Apps keep their font files: their `build.rs` embeds craft-fonts' faces with `craft-fonts-build`,
//! as [`CraftFace`]s or as `EmbeddedFace`s that convert into them, or, for a web build, copies them
//! beside the wasm and lists them as [`ServedFace`]s, which the stack orders the same way and
//! fetches when they are needed (see [`UiFonts::served_faces`]). For the interface they take
//! `Embed::subset_to(catalogs)`, which embeds or serves the CJK faces as craft-fonts' interface
//! faces (`Noto Sans TC UI`, BIZ UDPGothic Regular), cut at build time to the characters the
//! translation catalogs use. A character a cut face lacks, such as one in a file name, comes from
//! the next face that has it: another region's interface face, then the installed system fonts.
//! [`UiFonts::stack`] returns a [`Stack`] the app can still edit (add, replace and reorder faces)
//! before [`Stack::install`].

mod coverage;
mod fit;
mod providers;
mod served;
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "system_fonts"), allow(dead_code, reason = "used by the system font index, and its tests"))]
mod system;
#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
mod system_index;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use egui::epaint::text::{Fonts, TextOptions};
use egui::{FontData, FontDefinitions, FontFamily, FontId, FontProvider, Style};

pub use coverage::{Missing, Sample, assert_covers, assert_covers_with_system, missing_glyphs};
use providers::Installed;
pub use providers::{FaceList, RegularWeight, set_weight};
pub use served::{ServedFace, loading};

/// A font file from craft-fonts that the app embeds, one per line of `fonts/manifest.txt`.
#[derive(Clone, Copy, Debug)]
pub struct CraftFace {
    /// Family name, e.g. `"Noto Sans Arabic"`.
    pub family: &'static str,
    /// `"Regular"` or `"Bold"`; faces with other styles are not used for UI text.
    pub style: &'static str,
    /// The manifest's script tags: ISO 15924 codes such as `Arab`, `Thai`, `Jpan`, `Kore`, `Hans`
    /// and `Hant`. Hong Kong forms, which have no ISO 15924 code, are tagged `Hant-HK`.
    pub scripts: &'static [&'static str],
    pub bytes: &'static [u8],
    /// Face index in a font collection (`.ttc`); `0` for a single font.
    pub index: u32,
}

impl From<craft_fonts_build::EmbeddedFace> for CraftFace {
    fn from(face: craft_fonts_build::EmbeddedFace) -> Self {
        let craft_fonts_build::EmbeddedFace { family, style, scripts, index, bytes } = face;
        Self { family, style, scripts, bytes, index }
    }
}

impl CraftFace {
    /// The egui font name, e.g. `"Noto Sans Arabic Regular"`.
    pub fn name(&self) -> String {
        format!("{} {}", self.family, self.style)
    }

    fn data(&self) -> FontData {
        baseline_fallback(FontData { index: self.index, ..FontData::from_static(self.bytes) })
    }

    fn region(&self) -> Option<Han> {
        self.scripts.iter().find_map(|tag| match *tag {
            "Hans" => Some(Han::Simplified),
            "Hant" => Some(Han::Traditional),
            "Hant-HK" => Some(Han::HongKong),
            "Jpan" => Some(Han::Japanese),
            "Kore" => Some(Han::Korean),
            _ => None,
        })
    }

    fn covers(&self, script: &str) -> bool {
        self.scripts.iter().any(|tag| {
            *tag == script
                || match *tag {
                    "Jpan" => matches!(script, "Hani" | "Hira" | "Kana"),
                    "Kore" => matches!(script, "Hang" | "Hani"),
                    "Hans" | "Hant" | "Hant-HK" => script == "Hani",
                    _ => false,
                }
        })
    }

    /// `0` for the style a family of `weight` wants, `1` for an acceptable stand-in.
    fn style_rank(&self, weight: u16) -> Option<u8> {
        let regular = self.style.eq_ignore_ascii_case("regular");
        let bold = ["bold", "semibold"].iter().any(|s| self.style.eq_ignore_ascii_case(s));
        match (weight >= 600, regular, bold) {
            (false, true, _) | (true, _, true) => Some(0),
            (true, true, _) => Some(1),
            _ => None,
        }
    }
}

pub use craft_i18n::Han;

/// The suite's egui fork has `FontTweak::set_baseline_fallback`, which puts a fallback face's
/// glyphs on the primary face's baseline and grows the row to fit them; crates.io egui has
/// neither. A method call picks an inherent method over a trait method of the same name, so
/// craft faces reach the fork's method there and this no-op everywhere else, without a feature.
#[allow(dead_code, reason = "unused when egui has the inherent method")]
trait SetBaselineFallback {
    fn set_baseline_fallback(&mut self, _baseline: bool) {}
}

impl SetBaselineFallback for egui::FontTweak {}

/// A craft face's data, set to sit on the primary face's baseline where egui supports it.
fn baseline_fallback(mut data: FontData) -> FontData {
    data.tweak.set_baseline_fallback(true);
    data
}

/// The scripts a language is written in, by language tag. [`I18nScripts`] answers from
/// craft-i18n; an app with its own table implements this instead.
pub trait LanguageScripts {
    /// ISO 15924 codes of the language's interface text, main script first: `ja` → `Hani`, `Hira`,
    /// `Kana`; `ar` → `Arab`. The unions `Jpan`, `Kore`, `Hans` and `Hant` are accepted too.
    fn scripts(&self, language: &str) -> Vec<&'static str>;

    /// The regional Han forms the language uses, `None` for a language not written with Han.
    fn han(&self, language: &str) -> Option<Han>;
}

/// [`LanguageScripts`] from craft-i18n's script table ([`craft_i18n::scripts`],
/// [`craft_i18n::han`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct I18nScripts;

impl LanguageScripts for I18nScripts {
    fn scripts(&self, language: &str) -> Vec<&'static str> {
        craft_i18n::scripts(language).iter().map(|s| s.code()).collect()
    }

    fn han(&self, language: &str) -> Option<Han> {
        craft_i18n::han(language)
    }
}

/// A font family the app uses, beyond what its own faces imply.
#[derive(Clone, Debug)]
pub struct Family {
    family: FontFamily,
    weight: u16,
    lead: Option<String>,
    monospaced: bool,
}

impl Family {
    /// A named family, e.g. `Family::named("semibold").weight(600)`.
    pub fn named(name: &str) -> Self {
        Self { family: FontFamily::Name(name.into()), weight: 400, lead: None, monospaced: false }
    }

    /// egui's proportional family.
    pub fn proportional() -> Self {
        Self { family: FontFamily::Proportional, weight: 400, lead: None, monospaced: false }
    }

    /// egui's monospace family, e.g. to set its weight.
    pub fn monospace() -> Self {
        Self { family: FontFamily::Monospace, weight: 400, lead: None, monospaced: true }
    }

    /// The weight (`wght`) this family's faces draw at: bold craft faces are used from 600 up, and
    /// variable faces are set to this weight.
    pub fn weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    /// Put `script`'s faces first, e.g. `Family::named("arabic").script("Arab")` for the
    /// right-to-left runs an app shapes itself.
    pub fn script(mut self, script: &str) -> Self {
        self.lead = Some(script.to_owned());
        self
    }

    /// Lay the family out as egui's monospace one, e.g. `Family::named("mono").monospaced()` for
    /// an app's own code face: its app face stays first in every UI language, and a family without
    /// app faces starts with egui's monospace faces.
    pub fn monospaced(mut self) -> Self {
        self.monospaced = true;
        self
    }
}

/// Emoji faces the app embeds, asked in this order after the system fonts.
///
/// egui 0.37's default faces have only a few monochrome emoji and lack symbols such as ⊞ ☰ 🔍.
/// On the web there are no system fonts, so emoji draw as boxes unless one of these is set.
#[derive(Clone, Default)]
pub struct Emoji {
    /// A colour emoji font, e.g. Noto Color Emoji (COLRv1) on desktop.
    pub color: Option<FontData>,
    /// A lighter colour emoji font for web builds, e.g. Twemoji.
    pub web: Option<FontData>,
    /// A colour emoji font served beside the wasm, fetched the first time text needs an emoji;
    /// asked after `web`. Ignored outside the web.
    pub served: Option<ServedFace>,
    /// A monochrome emoji font, e.g. Noto Emoji; the last resort.
    pub monochrome: Option<FontData>,
}

/// How rows treat the CJK faces that follow the app's face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CjkRows {
    /// Rows keep the height of the family's first face in every language, and widgets the theme's
    /// height. CJK glyphs sit on that face's baseline, scaled down only if they would leave the row.
    #[default]
    Fit,
    /// Rows and widgets take the CJK face's taller line in Chinese, Japanese and Korean UIs. Rows
    /// grow only on the suite's egui fork; with crates.io egui only widgets do.
    Grow,
}

/// Where installed system fonts come in the stack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SystemFonts {
    /// After every craft face.
    #[default]
    AfterCraft,
    /// After the UI language's craft faces, before the faces for other scripts.
    BeforeCraft,
    /// Not used: text draws the same on every machine.
    Off,
}

/// Scripts the app's own UI faces cover unless [`UiFonts::app_scripts`] says otherwise.
const APP_SCRIPTS: &[&str] = &["Latn", "Cyrl", "Grek"];

/// Scripts whose characters sit inside the em square, so they fit a row of a Latin face.
const EM_SQUARE_SCRIPTS: &[&str] = &["Hani", "Hira", "Kana", "Hang", "Bopo"];

/// craft-fonts' interface face for Japanese; its manifest also lists serif Japanese faces.
const DEFAULT_PREFER: &[(&str, &str)] = &[("Jpan", "BIZ UDPGothic")];

struct AppFace {
    name: String,
    data: FontData,
    families: Vec<FontFamily>,
}

/// The configuration of an app's UI fonts. Build it, then [`Self::install`] it, or take its
/// [`Self::stack`] to edit first.
pub struct UiFonts<'a> {
    scripts: &'a dyn LanguageScripts,
    app_faces: Vec<AppFace>,
    egui_faces: Vec<(String, Vec<FontFamily>)>,
    app_scripts: Vec<String>,
    craft_faces: Vec<CraftFace>,
    /// For each of `craft_faces`, the served face it stands for, whose data is fetched later.
    served: Vec<Option<ServedFace>>,
    languages: Vec<String>,
    ui_language: String,
    prefer: Vec<(String, String)>,
    families: Vec<Family>,
    emoji: Emoji,
    system: SystemFonts,
    system_provider: Option<Arc<dyn FontProvider>>,
    cjk_rows: CjkRows,
}

impl<'a> UiFonts<'a> {
    pub fn new(scripts: &'a dyn LanguageScripts) -> Self {
        Self {
            scripts,
            app_faces: Vec::new(),
            egui_faces: Vec::new(),
            app_scripts: APP_SCRIPTS.iter().map(|s| (*s).to_owned()).collect(),
            craft_faces: Vec::new(),
            served: Vec::new(),
            languages: Vec::new(),
            ui_language: "en".to_owned(),
            prefer: Vec::new(),
            families: Vec::new(),
            emoji: Emoji::default(),
            system: SystemFonts::default(),
            system_provider: None,
            cjk_rows: CjkRows::default(),
        }
    }

    /// One of the app's own faces, heading `families` in the order the app adds them.
    pub fn app_face(mut self, name: &str, data: FontData, families: &[FontFamily]) -> Self {
        self.app_faces.push(AppFace { name: name.to_owned(), data, families: families.to_vec() });
        self
    }

    /// One of egui's default faces (`Hack`, `Ubuntu-Light`, `NotoEmoji-Regular`,
    /// `emoji-icon-font`) at the end of `families`' configured faces, ahead of installed system
    /// fonts, e.g. Hack in the proportional family for symbols the app's face lacks (⇧, ▸). It
    /// needs egui's `default_fonts` feature; a name egui doesn't ship is ignored.
    pub fn egui_face(mut self, name: &str, families: &[FontFamily]) -> Self {
        self.egui_faces.push((name.to_owned(), families.to_vec()));
        self
    }

    /// The scripts the app's own faces cover (default `Latn`, `Cyrl`, `Grek`). A UI language whose
    /// main script isn't among them gets its craft face ahead of the app's faces, unless the
    /// script fits a Latin row (Han, kana, Hangul).
    pub fn app_scripts(mut self, scripts: &[&str]) -> Self {
        self.app_scripts = scripts.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    /// The craft-fonts faces this build embedded; may be empty.
    pub fn craft_faces(mut self, faces: &[CraftFace]) -> Self {
        self.craft_faces.extend_from_slice(faces);
        self.served.resize(self.craft_faces.len(), None);
        self
    }

    /// The craft-fonts faces a web build serves beside its wasm (see `craft-fonts-build`'s
    /// `Embed::serve`); may be empty. They take the places embedded faces would, and on the web
    /// [`Stack::install`] fetches them: the UI language's faces at once ([`Stack::ui_downloads`]),
    /// others the first time text needs a character that their files map and the faces already
    /// installed lack ([`Stack::downloads_for`]). Until a face arrives, text falls back to the
    /// faces after it. Outside the web they are never fetched.
    pub fn served_faces(mut self, faces: &[ServedFace]) -> Self {
        for face in faces {
            self.craft_faces.push(face.craft_face());
            self.served.push(Some(*face));
        }
        self
    }

    /// Every language the app offers, in its menu order.
    pub fn languages<S: AsRef<str>>(mut self, languages: &[S]) -> Self {
        self.languages = languages.iter().map(|l| l.as_ref().to_owned()).collect();
        self
    }

    /// The current UI language tag.
    pub fn ui_language(mut self, language: &str) -> Self {
        language.clone_into(&mut self.ui_language);
        self
    }

    /// Prefer the craft face of `family` for `script` (a face tag such as `Jpan`), ahead of other
    /// faces for the same script and region. A family also names its interface face (`Noto Sans TC`
    /// names `Noto Sans TC UI`). Earlier calls win over later ones.
    pub fn prefer(mut self, script: &str, family: &str) -> Self {
        self.prefer.push((script.to_owned(), family.to_owned()));
        self
    }

    /// A family to build beyond those the app faces name, or a different weight or lead script for
    /// one of them.
    pub fn family(mut self, family: Family) -> Self {
        self.families.push(family);
        self
    }

    pub fn emoji(mut self, emoji: Emoji) -> Self {
        self.emoji = emoji;
        self
    }

    /// Whether CJK faces after the app's face keep its row height ([`CjkRows::Fit`], the default)
    /// or make rows and widgets taller.
    pub fn cjk_rows(mut self, rows: CjkRows) -> Self {
        self.cjk_rows = rows;
        self
    }

    pub fn system_fonts(mut self, system: SystemFonts) -> Self {
        self.system = system;
        self
    }

    /// The provider to use for installed fonts instead of [`system_font_provider`] (with the
    /// `system_fonts` feature, on desktop). It is wrapped in [`RegularWeight`].
    pub fn system_provider(mut self, provider: Arc<dyn FontProvider>) -> Self {
        self.system_provider = Some(provider);
        self
    }

    /// Build the stack and install it in `ctx`.
    pub fn install(&self, ctx: &egui::Context) -> Stack {
        let stack = self.stack();
        stack.install(ctx);
        stack
    }

    /// Build the stack without installing it.
    pub fn stack(&self) -> Stack {
        let mut stack = Stack {
            definitions: FontDefinitions::empty(),
            after_system: FaceList::default(),
            system: self.system,
            system_provider: self.system_provider.clone(),
            ui_language: self.ui_language.clone(),
            language_scripts: std::iter::once(&self.ui_language)
                .chain(&self.languages)
                .map(|language| (language.clone(), expand(&self.scripts.scripts(language))))
                .collect(),
            weights: Vec::new(),
            base_face: None,
            script_face: None,
            served: Vec::new(),
            ui_downloads: Vec::new(),
            cjk_rows: self.cjk_rows,
        };
        let defaults = FontDefinitions::default();
        let cjk_names: Vec<String> = match self.cjk_rows {
            CjkRows::Fit => self.craft_faces.iter().filter(|face| EM_SQUARE_SCRIPTS.iter().any(|s| face.covers(s))).map(CraftFace::name).collect(),
            CjkRows::Grow => Vec::new(),
        };
        let mut cjk_faces: Vec<String> = Vec::new();
        for family in self.family_specs() {
            let default_family = if family.monospaced { &FontFamily::Monospace } else { &family.family };
            let default_chain = defaults.families.get(default_family).or_else(|| defaults.families.get(&FontFamily::Proportional));
            let egui_faces: Vec<Face> =
                default_chain.into_iter().flatten().filter_map(|name| Some(Face::new(name, (**defaults.font_data.get(name)?).clone()))).collect();
            let mut chain = self.chain(&family, egui_faces);
            let cjk = chain.configured.iter().chain(&chain.after).filter(|face| cjk_names.contains(&face.name));
            cjk_faces.extend(cjk.map(|face| face.clone().weighted(family.weight).name));
            chain.configured.extend(
                self.egui_faces
                    .iter()
                    .filter(|(_, families)| families.contains(&family.family))
                    .filter_map(|(name, _)| Some(Face::new(name, (**defaults.font_data.get(name)?).clone()))),
            );
            let mut configured: Vec<String> = chain.configured.into_iter().map(|face| stack.add_configured(face.weighted(family.weight))).collect();
            let mut tail: Vec<String> = chain.after.into_iter().map(|face| stack.add_after(face.weighted(family.weight))).collect();
            for face in chain.ui_downloads {
                if !stack.ui_downloads.contains(&face) {
                    stack.ui_downloads.push(face);
                }
            }
            if family.family == FontFamily::Proportional {
                stack.base_face = chain.base.map(|face| face.weighted(family.weight).name);
                stack.script_face = chain.script.map(|face| face.weighted(family.weight).name);
            }
            dedup(&mut configured);
            tail.retain(|name| !configured.contains(name));
            dedup(&mut tail);
            stack.definitions.families.insert(family.family.clone(), configured);
            stack.after_system.set_chain(family.family.clone(), tail);
            stack.weights.push((family.family, family.weight));
        }
        stack.fit_into_rows(&cjk_faces);
        stack
    }

    /// Proportional and Monospace, every family an app face names, then the declared families.
    fn family_specs(&self) -> Vec<Family> {
        let mut specs = vec![Family::proportional(), Family::monospace()];
        for family in self.app_faces.iter().flat_map(|f| &f.families) {
            if !specs.iter().any(|s| s.family == *family) {
                specs.push(Family { family: family.clone(), weight: 400, lead: None, monospaced: false });
            }
        }
        for declared in &self.families {
            match specs.iter_mut().find(|s| s.family == declared.family) {
                Some(spec) => *spec = declared.clone(),
                None => specs.push(declared.clone()),
            }
        }
        specs
    }

    /// One family's faces. `egui_faces` are egui's defaults for it, which stand in for the app's
    /// faces when the app has none for the family.
    fn chain(&self, family: &Family, egui_faces: Vec<Face>) -> Chain {
        let weight = family.weight;
        let ui_scripts = expand(&self.scripts.scripts(&self.ui_language));
        let ui_han = self.han(&self.ui_language);

        let app = self.app_faces_for(family);
        let own_faces = !app.is_empty();
        let (base, egui_tail) = if own_faces { (app, egui_faces) } else { (egui_faces, Vec::new()) };
        // egui's default faces cover the scripts app faces are assumed to.
        let covers = |script: &str| if own_faces { self.app_covers(script) } else { APP_SCRIPTS.contains(&script) };

        let lead: Vec<usize> = expand(family.lead.as_slice()).iter().flat_map(|s| self.candidates(s, ui_han, weight)).collect();
        let language: Vec<usize> = ui_scripts.iter().filter(|s| !covers(s)).flat_map(|s| self.candidates(s, ui_han, weight)).collect();
        // Letters of a covered script that the base faces lack, such as Vietnamese ổ and ạ in
        // egui's Ubuntu-Light, come from that script's own faces before any other face with them.
        // CJK faces carry Latin at CJK proportions, so they don't stand in.
        let backup: Vec<usize> = ui_scripts
            .iter()
            .filter(|s| covers(s))
            .flat_map(|s| self.candidates(s, ui_han, weight))
            .filter(|&i| !self.craft_faces[i].covers("Hani"))
            .collect();
        let mut others: Vec<usize> = Vec::new();
        for tag in self.languages.iter().filter(|l| **l != self.ui_language) {
            let han = self.han(tag);
            for script in expand(&self.scripts.scripts(tag)).iter().filter(|s| !covers(s)) {
                others.extend(self.candidates(script, han, weight));
            }
        }
        let mut rest: Vec<(u8, usize)> =
            self.craft_faces.iter().enumerate().filter_map(|(i, face)| face.style_rank(weight).map(|rank| (rank, i))).collect();
        rest.sort();
        others.extend(rest.into_iter().map(|(_, i)| i));

        let main_needs_lead = ui_scripts.first().is_some_and(|main| !covers(main) && !EM_SQUARE_SCRIPTS.contains(&main.as_str()));
        let language_first = !family.monospaced && main_needs_lead;
        let craft = |indices: Vec<usize>| indices.into_iter().map(|i| self.craft_face(i)).collect::<Vec<_>>();
        // A face that only follows the app's face keeps the app's rows unless the app asks otherwise.
        let script = (language_first || self.cjk_rows == CjkRows::Grow).then(|| language.first().map(|i| self.craft_face(*i))).flatten();
        // The best face for each of the UI language's scripts, the ones a web build fetches first.
        let ui_downloads = ui_scripts
            .iter()
            .filter(|s| !covers(s))
            .filter_map(|s| self.candidates(s, ui_han, weight).first().map(|i| self.craft_face(*i)))
            .filter(|face| face.served.is_some())
            .map(|face| face.name)
            .collect();
        let base_face = base.first().cloned();

        let mut configured = craft(lead);
        if language_first {
            configured.extend(craft(language));
            configured.extend(base);
            configured.extend(craft(backup));
        } else {
            configured.extend(base);
            configured.extend(craft(backup));
            configured.extend(craft(language));
        }
        let mut after = Vec::new();
        match self.system {
            SystemFonts::BeforeCraft => after.extend(craft(others)),
            SystemFonts::AfterCraft | SystemFonts::Off => configured.extend(craft(others)),
        }
        let emoji = |name: &str, data: &Option<FontData>| Some(Face::new(name, data.clone()?));
        after.extend(emoji("emoji-color", &self.emoji.color));
        after.extend(emoji("emoji-web", &self.emoji.web));
        after.extend(self.emoji.served.map(|face| {
            let face = ServedFace { scripts: &[served::EMOJI], ..face };
            Face { served: Some(face), ..Face::new(&face.name(), FontData::from_static(&[])) }
        }));
        after.extend(emoji("emoji-monochrome", &self.emoji.monochrome));
        after.extend(egui_tail);
        Chain { configured, after, base: base_face, script, ui_downloads }
    }

    /// The app faces heading `family`; a named family without its own takes the monospace ones
    /// if it is [`Family::monospaced`], the proportional ones otherwise.
    fn app_faces_for(&self, family: &Family) -> Vec<Face> {
        let faces = |family: &FontFamily| -> Vec<Face> {
            self.app_faces.iter().filter(|face| face.families.contains(family)).map(|face| Face::new(&face.name, face.data.clone())).collect()
        };
        match faces(&family.family) {
            own if !own.is_empty() || !matches!(family.family, FontFamily::Name(_)) => own,
            _ if family.monospaced => faces(&FontFamily::Monospace),
            _ => faces(&FontFamily::Proportional),
        }
    }

    /// Craft faces for `script`, best first: the right Han region, then the app's preference,
    /// then the right style, then manifest order. craft-fonts' own preference ([`DEFAULT_PREFER`])
    /// orders only the faces of the best region found.
    fn candidates(&self, script: &str, han: Option<Han>, weight: u16) -> Vec<usize> {
        let found: Vec<(u8, u8, usize)> = self
            .craft_faces
            .iter()
            .enumerate()
            .filter(|(_, face)| face.covers(script))
            .filter_map(|(i, face)| {
                let style = face.style_rank(weight)?;
                let region = if script == "Hani" { region_rank(face.region(), han) } else { 0 };
                Some((region, style, i))
            })
            .collect();
        let best = found.iter().map(|(region, ..)| *region).min();
        let mut ranked: Vec<(u8, usize, u8, usize)> = found
            .into_iter()
            .map(|(region, style, i)| {
                let face = &self.craft_faces[i];
                let app = preference_of(self.prefer.iter().map(|(s, f)| (s.as_str(), f.as_str())), face);
                let default = || (Some(region) == best).then(|| preference_of(DEFAULT_PREFER.iter().copied(), face)).flatten();
                let preference = app.or_else(|| default().map(|p| self.prefer.len() + p)).unwrap_or(usize::MAX);
                (region, preference, style, i)
            })
            .collect();
        ranked.sort();
        ranked.into_iter().map(|(.., i)| i).collect()
    }

    fn craft_face(&self, index: usize) -> Face {
        let face = self.craft_faces.get(index);
        Face {
            name: face.map(CraftFace::name).unwrap_or_default(),
            data: face.map_or_else(|| FontData::from_static(&[]), CraftFace::data),
            served: self.served.get(index).copied().flatten(),
        }
    }

    fn app_covers(&self, script: &str) -> bool {
        self.app_scripts.iter().any(|s| s == script)
    }

    fn han(&self, language: &str) -> Option<Han> {
        self.scripts.han(language).or_else(|| {
            self.scripts.scripts(language).iter().find_map(|s| match *s {
                "Hans" => Some(Han::Simplified),
                "Hant" => Some(Han::Traditional),
                "Jpan" => Some(Han::Japanese),
                "Kore" => Some(Han::Korean),
                _ => None,
            })
        })
    }
}

/// The first of `prefer`'s `(script tag, family)` entries that names `face`, or the family whose
/// interface face `face` is (`Noto Sans TC` names `Noto Sans TC UI`).
fn preference_of<'p>(mut prefer: impl Iterator<Item = (&'p str, &'p str)>, face: &CraftFace) -> Option<usize> {
    let full_family = face.family.strip_suffix(" UI").unwrap_or(face.family);
    prefer.position(|(tag, family)| face.scripts.contains(&tag) && (face.family == family || full_family == family))
}

/// How well a face's Han region suits the one wanted: lower is better.
fn region_rank(face: Option<Han>, wanted: Option<Han>) -> u8 {
    match (face, wanted) {
        (_, None) => 0,
        (Some(f), Some(w)) if f == w => 0,
        (Some(Han::Traditional), Some(Han::HongKong)) | (Some(Han::HongKong), Some(Han::Traditional)) => 1,
        // The other Chinese forms are closer than Japanese or Korean ones.
        (Some(Han::Simplified), Some(Han::Traditional | Han::HongKong)) | (Some(Han::Traditional | Han::HongKong), Some(Han::Simplified)) => 2,
        (None, Some(_)) => 2,
        _ => 3,
    }
}

/// Script codes with the CLDR unions spelled out.
fn expand<S: AsRef<str>>(scripts: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for script in scripts {
        let parts: Vec<&str> = match script.as_ref() {
            "Jpan" => vec!["Hani", "Hira", "Kana"],
            "Kore" => vec!["Hang", "Hani"],
            "Hans" | "Hant" => vec!["Hani"],
            other => vec![other],
        };
        for part in parts {
            if !out.iter().any(|s| s == part) {
                out.push(part.to_owned());
            }
        }
    }
    out
}

fn dedup(names: &mut Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    names.retain(|n| !n.is_empty() && seen.insert(n.clone()));
}

/// A face with its egui name.
#[derive(Clone)]
struct Face {
    name: String,
    data: FontData,
    /// The served face this stands for; `data` is empty until it is fetched.
    served: Option<ServedFace>,
}

/// One family's faces, as [`UiFonts::chain`] orders them.
struct Chain {
    configured: Vec<Face>,
    /// Faces asked after the system fonts.
    after: Vec<Face>,
    /// The app's first face for the family, or egui's when the app has none.
    base: Option<Face>,
    /// The UI language's face for its main script, when the family's first faces lack it and the
    /// face leads, or with [`CjkRows::Grow`].
    script: Option<Face>,
    /// Names of the served faces the UI language needs in this family.
    ui_downloads: Vec<String>,
}

impl Face {
    fn new(name: &str, data: FontData) -> Self {
        Self { name: name.to_owned(), data, served: None }
    }

    /// Set a variable face to `weight`. A face that isn't at regular weight gets its own name, so
    /// the regular and bold uses of one file don't collide in egui's face store.
    fn weighted(mut self, weight: u16) -> Self {
        if set_weight(&mut self.data, weight) && weight != 400 {
            self.name = format!("{} @{weight}", self.name);
        }
        self
    }
}

/// A built font stack: the configured faces and chains, and the providers asked after them.
#[derive(Clone)]
pub struct Stack {
    /// The configured faces and each family's chain. Edit it to add, replace or reorder faces
    /// before [`Self::install`]; a chain entry without font data is skipped.
    pub definitions: FontDefinitions,
    /// Faces asked after the system fonts: emoji, egui's defaults and, with
    /// [`SystemFonts::BeforeCraft`], the craft faces for other scripts.
    pub after_system: FaceList,
    system: SystemFonts,
    system_provider: Option<Arc<dyn FontProvider>>,
    ui_language: String,
    /// The scripts of the UI language and every offered language.
    language_scripts: Vec<(String, Vec<String>)>,
    weights: Vec<(FontFamily, u16)>,
    /// The proportional family's first app face (or egui's), whose rows the app's theme fits.
    base_face: Option<String>,
    /// The UI language's main script face when it leads the stack, or with [`CjkRows::Grow`];
    /// widgets fit its rows.
    script_face: Option<String>,
    /// The served faces in the chains, by name; their data is added as it arrives.
    served: Vec<ServedFace>,
    cjk_rows: CjkRows,
    /// Names of the served faces to fetch at install.
    ui_downloads: Vec<String>,
}

impl Stack {
    /// Install the faces and providers in `ctx`, and fit the minimum widget height to a row of
    /// text. Call it again when the UI language changes.
    ///
    /// The app's theme sets the widget height for rows of the app's face. When the UI language's
    /// script face leads the stack (Arabic, Thai) and has taller rows, the context's styles get
    /// that much more height ([`Self::fit_interact_size`]), whether the app sets its theme before
    /// or after installing: a style the app sets later is fitted at the start of the next pass. A
    /// language whose face follows the app's (Chinese, Japanese, Korean, unless
    /// [`CjkRows::Grow`]) or has rows no taller than the app's face leaves the theme's height as
    /// it is.
    ///
    /// The stack's system font provider (the `system_fonts` feature on desktop, or
    /// [`UiFonts::system_provider`]) replaces the context's font providers, such as the one
    /// eframe adds, as does [`SystemFonts::Off`]. Without one, the context keeps its providers and
    /// the stack's faces are asked after them.
    ///
    /// On the web, a stack with [`UiFonts::served_faces`] also fetches them (see there); the faces
    /// already fetched for an earlier stack are added at once.
    pub fn install(&self, ctx: &egui::Context) {
        #[cfg(target_arch = "wasm32")]
        if !self.served.is_empty() {
            return served::web::install(ctx, self.clone());
        }
        self.install_faces(ctx, None);
    }

    /// [`Self::install`] without fetching; `miss` is asked for characters no face has.
    fn install_faces(&self, ctx: &egui::Context, miss: Option<Arc<dyn FontProvider>>) {
        ctx.set_fonts(self.checked_definitions());
        let id = egui::Id::unique("craft_ui::fonts::installed");
        let (system, start_index) = self.context_lookup();
        let installed: Option<Arc<Installed>> = ctx.data(|d| d.get_temp(id));
        let installed = installed.unwrap_or_else(|| {
            let new = Arc::new(Installed::default());
            ctx.data_mut(|d| d.insert_temp(id, Arc::clone(&new)));
            if system.is_none() {
                ctx.add_font_provider(Arc::clone(&new) as Arc<dyn FontProvider>);
            }
            new
        });
        installed.set(self.after_system.clone(), miss);
        if let Some(system) = system {
            let mut providers = system.into_iter().collect::<Vec<_>>();
            providers.push(installed as Arc<dyn FontProvider>);
            providers::set_font_providers(ctx, providers);
        }
        if let Some(start) = start_index {
            start(ctx);
        }
        fit::install(ctx, self.row_faces());
    }

    /// Each family's weight, for wrapping another provider in [`RegularWeight`].
    pub fn weights(&self) -> &[(FontFamily, u16)] {
        &self.weights
    }

    /// The chain of `family`, if the stack has it.
    pub fn chain(&self, family: &FontFamily) -> &[String] {
        self.definitions.families.get(family).map_or(&[], Vec::as_slice)
    }

    /// Insert a face into `family`'s chain at `index` (clamped), adding its data.
    pub fn insert(&mut self, family: &FontFamily, index: usize, name: &str, data: FontData) {
        self.definitions.font_data.insert(name.to_owned(), Arc::new(data));
        let chain = self.definitions.families.entry(family.clone()).or_default();
        chain.retain(|n| n != name);
        chain.insert(index.min(chain.len()), name.to_owned());
    }

    /// Replace the data of the face called `name`, wherever it is used.
    pub fn replace(&mut self, name: &str, data: FontData) {
        if self.after_system.contains(name) {
            self.after_system.insert(name, data.clone());
        }
        self.definitions.font_data.insert(name.to_owned(), Arc::new(data));
    }

    /// Remove the face called `name` from every chain, including those asked after the system
    /// fonts.
    pub fn remove(&mut self, name: &str) {
        self.definitions.font_data.remove(name);
        for chain in self.definitions.families.values_mut() {
            chain.retain(|n| n != name);
        }
        self.after_system.remove(name);
    }

    /// Add to `style`'s minimum interactive height how much taller a row of the UI language's
    /// script face is than a row of the app's face, at the style's body and button sizes, when
    /// that face leads the stack or [`CjkRows::Grow`] is set.
    /// [`Self::install`] does this for the context's styles; this is for a style used elsewhere.
    pub fn fit_interact_size(&self, style: &mut Style) {
        style.spacing.interact_size.y += fit::extra_height(self.row_faces().as_ref(), style);
    }

    /// Whether the proportional family can draw `language`'s scripts, e.g. to leave a language out
    /// of the app's menu on a machine that can't show it. `with_system` also asks the system font
    /// provider. The scripts come from the stack's [`LanguageScripts`] for the UI language and
    /// [`UiFonts::languages`], and from craft-i18n for any other language. Each script is checked
    /// with a few of its common letters. A served face counts for the scripts it is tagged with,
    /// fetched or not, since it is fetched when text needs it.
    pub fn covers_language(&self, language: &str, with_system: bool) -> bool {
        let scripts = match self.language_scripts.iter().find(|(l, _)| l == language) {
            Some((_, scripts)) => scripts.clone(),
            None => expand(&I18nScripts.scripts(language)),
        };
        let mut fonts = self.fonts(with_system);
        let font_id = FontId::new(14.0, FontFamily::Proportional);
        let served = |script: &str| self.served.iter().any(|face| face.craft_face().covers(script));
        scripts.iter().filter(|s| !served(s)).filter_map(|s| coverage::script_sample(s)).flat_map(str::chars).all(|c| fonts.has_glyph(&font_id, c))
    }

    /// The stack as one set of definitions, without providers: each family's chain followed by the
    /// faces asked after the system fonts (emoji and egui's default faces), for a context or
    /// [`Fonts`] that takes fonts only through `set_fonts`.
    pub fn flat_definitions(&self) -> FontDefinitions {
        let mut definitions = self.checked_definitions();
        let FontDefinitions { font_data, families, .. } = &mut definitions;
        for (family, chain) in families {
            for name in self.after_system.chain(family) {
                if let Some(data) = self.after_system.get(name)
                    && !chain.contains(name)
                {
                    font_data.entry(name.clone()).or_insert_with(|| Arc::new(data.clone()));
                    chain.push(name.clone());
                }
            }
        }
        definitions
    }

    /// An epaint [`Fonts`] with this stack, for tests and measurement. `with_system` adds the
    /// system font provider.
    pub fn fonts(&self, with_system: bool) -> Fonts {
        Fonts::new(TextOptions::default(), self.checked_definitions()).with_font_providers(self.providers(with_system))
    }

    /// The app's face and the UI language's script face, when the stack has both.
    fn row_faces(&self) -> Option<fit::RowFaces> {
        let data = |name: &Option<String>| self.definitions.font_data.get(name.as_ref()?).cloned();
        Some(fit::RowFaces::new(data(&self.base_face)?, data(&self.script_face)?))
    }

    fn checked_definitions(&self) -> FontDefinitions {
        let mut definitions = self.definitions.clone();
        for chain in definitions.families.values_mut() {
            chain.retain(|name| self.definitions.font_data.contains_key(name));
        }
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            definitions.families.entry(family).or_default();
        }
        definitions
    }

    fn providers(&self, with_system: bool) -> Vec<Arc<dyn FontProvider>> {
        let mut providers: Vec<Arc<dyn FontProvider>> = self.system_lookup(with_system).into_iter().flatten().collect();
        providers.push(Arc::new(self.after_system.clone()));
        providers
    }

    /// The system font lookup that goes before the stack's own providers: `Some(None)` for none
    /// ([`SystemFonts::Off`]), `None` when the stack has no provider of its own and leaves the
    /// lookup to whatever the context has.
    fn system_lookup(&self, with_system: bool) -> Option<Option<Arc<dyn FontProvider>>> {
        if !with_system || self.system == SystemFonts::Off {
            return Some(None);
        }
        let system = self.system_provider.clone().or_else(|| system_font_provider(&self.ui_language))?;
        Some(Some(self.regular_weight(system)))
    }

    /// [`Self::system_lookup`] for [`Self::install`]. The default system provider doesn't block
    /// the UI thread; the returned function starts its index for the context.
    fn context_lookup(&self) -> (Option<Option<Arc<dyn FontProvider>>>, Option<StartIndex>) {
        if self.system_provider.is_none()
            && self.system != SystemFonts::Off
            && let Some((provider, start)) = installed_system_font_provider(&self.ui_language)
        {
            return (Some(Some(self.regular_weight(provider))), Some(start));
        }
        (self.system_lookup(true), None)
    }

    /// `provider` at each family's weight, its text faces on the row's baseline.
    fn regular_weight(&self, provider: Arc<dyn FontProvider>) -> Arc<dyn FontProvider> {
        let provider: Arc<dyn FontProvider> = Arc::new(providers::OnBaseline(provider));
        Arc::new(self.weights.iter().fold(RegularWeight::new(provider), |p, (f, w)| p.with_weight(f.clone(), *w)))
    }

    /// Fit the CJK faces `names` into the rows of the proportional family's first face, except where
    /// one of them leads a family.
    fn fit_into_rows(&mut self, names: &[String]) {
        let Some(primary) = self.chain(&FontFamily::Proportional).first().and_then(|name| self.definitions.font_data.get(name)).cloned() else {
            return;
        };
        let leads = |name: &String| self.definitions.families.values().any(|chain| chain.first() == Some(name));
        let names: Vec<&String> = names.iter().filter(|name| !leads(name)).collect();
        for name in names {
            if let Some(data) = self.definitions.font_data.get_mut(name.as_str()) {
                fit::into_row(&primary, Arc::make_mut(data));
            }
            if let Some(data) = self.after_system.get(name) {
                let mut data = data.clone();
                fit::into_row(&primary, &mut data);
                self.after_system.insert(name, data);
            }
        }
    }

    fn add_configured(&mut self, face: Face) -> String {
        if !self.add_served(&face) {
            self.definitions.font_data.entry(face.name.clone()).or_insert_with(|| Arc::new(face.data));
        }
        face.name
    }

    fn add_after(&mut self, face: Face) -> String {
        if self.add_served(&face) { face.name } else { self.after_system.add(face) }
    }

    /// Record a served face, whose data comes later; false for any other face.
    fn add_served(&mut self, face: &Face) -> bool {
        let Some(served) = face.served else { return false };
        if !self.served.iter().any(|s| s.name() == face.name) {
            self.served.push(served);
        }
        true
    }
}

/// A system font provider that prefers `locale`'s fonts, shared by every stack and check that asks
/// for the same locale. Every locale's provider asks one index of the installed fonts, built once
/// on a background thread; a lookup before it is built waits for it. `None` without the
/// `system_fonts` feature and on the web.
///
/// To ask whether installed fonts can draw a language, pass it to
/// [`UiFonts::system_provider`] and check with [`assert_covers_with_system`] or
/// [`Stack::fonts`]`(true)`.
#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
pub fn system_font_provider(locale: &str) -> Option<Arc<dyn FontProvider>> {
    use std::collections::BTreeMap;
    use std::sync::{Mutex, OnceLock, PoisonError};
    static PROVIDERS: OnceLock<Mutex<BTreeMap<String, Arc<dyn FontProvider>>>> = OnceLock::new();
    let mut providers = PROVIDERS.get_or_init(Mutex::default).lock().unwrap_or_else(PoisonError::into_inner);
    Some(Arc::clone(providers.entry(locale.to_owned()).or_insert_with(|| system_index::provider(locale, true))))
}

/// A system font provider that prefers `locale`'s fonts, shared by every stack and check that asks
/// for the same locale. `None` without the `system_fonts` feature and on the web.
#[cfg(not(all(feature = "system_fonts", not(target_arch = "wasm32"))))]
pub fn system_font_provider(_locale: &str) -> Option<Arc<dyn FontProvider>> {
    None
}

/// Starts building the system font index for a context.
type StartIndex = fn(&egui::Context);

/// The system font provider [`Stack::install`] gives `ctx` when the app sets none. A lookup before
/// the index is built finds nothing, so the UI thread doesn't wait; `ctx` reloads its fonts and
/// repaints once it is.
#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
fn installed_system_font_provider(locale: &str) -> Option<(Arc<dyn FontProvider>, StartIndex)> {
    Some((system_index::provider(locale, false), system_index::start))
}

#[cfg(not(all(feature = "system_fonts", not(target_arch = "wasm32"))))]
fn installed_system_font_provider(_locale: &str) -> Option<(Arc<dyn FontProvider>, StartIndex)> {
    None
}
