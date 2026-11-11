//! The UI font stack every Craft app installs.
//!
//! egui draws a character only if one of its installed faces has it. [`UiFonts`] builds the same
//! fallback order for every app, so a label or file name in any shipped script draws in every app:
//!
//! 1. the app's own faces for the family (UI face, semibold, mono), or egui's default faces for a
//!    family the app has none for; with [`InterfaceFont::System`]
//!    ([`UiFonts::interface_font`]), the OS's interface face goes ahead of them in the proportional
//!    family, and in the [`SEMIBOLD`] family if the app has one;
//! 2. craft-fonts faces for the UI language;
//! 3. craft-fonts faces for every other script the app ships, so a Japanese interface still draws
//!    Arabic file names;
//! 4. installed system fonts ([`system_font_provider`](super::system_font_provider)), at regular weight ([`RegularWeight`](super::RegularWeight));
//! 5. emoji faces ([`Emoji`]) and, behind the app's own faces, egui's default faces.
//!
//! When the faces of step 1 don't cover the UI language's main script (Arabic, Thai, Georgian),
//! that script's face goes first. egui takes a row's height from the first face of a family, so
//! the UI language's own face sets the row height and its marks aren't clipped. Han, kana and
//! Hangul sit inside the em square and fit a Latin row, so in Chinese, Japanese and Korean UIs the
//! Latin face stays first, and rows keep the same height as in a Latin UI: CJK faces sit on the
//! first face's baseline, scaled down only if a glyph would leave the row ([`CjkRows::Fit`]).
//! Monospace, and every family declared [`Family::monospaced`], always keeps its Latin face first.
//! On the suite's egui fork, the other craft faces and installed text faces used as fallbacks sit
//! on the row's baseline and rows grow to fit them by themselves; emoji and symbol faces stay
//! centred.
//!
//! Apps keep their font files: their `build.rs` embeds craft-fonts' faces with `craft-fonts-build`
//! as [`EmbeddedFace`]s or, for a web build, copies them beside the wasm and lists them as
//! [`ServedFace`]s, which the stack orders the same way and fetches when they are needed (see
//! [`UiFonts::served_faces`]). For the interface they take `Embed::subset_to(catalogs)`, which
//! embeds or serves the CJK faces as craft-fonts' interface faces (`Noto Sans TC UI`, BIZ
//! UDPGothic Regular), cut at build time to the characters the translation catalogs use. A
//! character a cut face lacks, such as one in a file name, comes from the next face that has it:
//! another region's interface face, then the installed system fonts. [`UiFonts::stack`] returns a
//! [`Stack`] the app can still edit (add, replace and reorder faces) before [`Stack::install`].

mod chain;
mod fit;
mod install;

use std::sync::Arc;

use craft_fonts_build::{EmbeddedFace, ServedFace};
use craft_theme::InterfaceFont;
use egui::{FontData, FontDefinitions, FontFamily, FontProvider};

pub(super) use chain::{EM_SQUARE_SCRIPTS, covers, is_cjk};
pub(crate) use install::{StackFacts, stack_facts};

use super::FontRef;
use super::coverage::script_sample;
use super::faces::FaceList;
use super::language::{I18nScripts, LanguageScripts};
use super::on_demand::{FontCandidates, OnDemandFonts, han_for_locale};
use super::served::loader::FontFetcher;
use super::system::ui_face;

/// The family [`crate::style`] draws the semibold type roles with (strong body text, titles,
/// headings): `Family::named(fonts::SEMIBOLD).weight(600)`. Without it, those roles draw with the
/// proportional family.
pub const SEMIBOLD: &str = "semibold";

/// A font family the app uses, beyond what its own faces imply.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Family {
    family: FontFamily,
    weight: u16,
    lead: Option<String>,
    monospaced: bool,
}

impl Family {
    /// A named family, e.g. `Family::named(fonts::SEMIBOLD).weight(600)`.
    pub fn named(name: &str) -> Self {
        Self::of(FontRef::named(name), false)
    }

    /// egui's proportional family.
    pub fn proportional() -> Self {
        Self::of(FontRef::proportional(), false)
    }

    /// egui's monospace family, e.g. to set its weight.
    pub fn monospace() -> Self {
        Self::of(FontRef::monospace(), true)
    }

    fn of(family: FontRef, monospaced: bool) -> Self {
        Self { family: family.family(), weight: REGULAR, lead: None, monospaced }
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

/// The weight of a regular face.
const REGULAR: u16 = 400;

/// Emoji faces the app embeds, asked in this order after the system fonts.
///
/// egui's default faces have only a few monochrome emoji and lack symbols such as ⊞ ☰ 🔍.
/// On the web there are no system fonts, so emoji draw as boxes unless one of these is set.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct Emoji {
    /// A colour emoji font, e.g. Noto Color Emoji (COLRv1) on desktop.
    pub color: Option<FontData>,
    /// A lighter colour emoji font for web builds, e.g. Twemoji.
    pub web: Option<FontData>,
    /// A colour emoji font served beside the wasm, fetched the first time text needs an emoji;
    /// asked after `web`. Needs a [`FontFetcher`].
    pub served: Option<ServedFace>,
    /// A monochrome emoji font, e.g. Noto Emoji; the last resort.
    pub monochrome: Option<FontData>,
}

/// How rows treat the CJK faces that follow the app's face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CjkRows {
    /// Rows keep the height of the family's first face in every language. CJK glyphs sit on that
    /// face's baseline, scaled down only if they would leave the row.
    #[default]
    Fit,
    /// CJK faces keep their own metrics, so rows take their taller line in Chinese, Japanese and
    /// Korean UIs. Rows grow only on the suite's egui fork.
    Grow,
}

/// Where installed system fonts come in the stack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SystemFonts {
    /// After every craft face.
    #[default]
    AfterCraft,
    /// After the UI language's craft faces, before the faces for other scripts.
    BeforeCraft,
    /// Not used: text draws the same on every machine.
    Off,
}

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
    craft_faces: Vec<EmbeddedFace>,
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
    interface: InterfaceFont,
    candidates: Option<Arc<dyn FontCandidates>>,
    os_locale: Option<String>,
    fetcher: Option<Arc<dyn FontFetcher>>,
}

impl<'a> UiFonts<'a> {
    /// A configuration with no faces, using `scripts` to map languages to scripts.
    pub fn new(scripts: &'a dyn LanguageScripts) -> Self {
        Self {
            scripts,
            app_faces: Vec::new(),
            egui_faces: Vec::new(),
            app_scripts: chain::APP_SCRIPTS.iter().map(|s| (*s).to_owned()).collect(),
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
            interface: InterfaceFont::default(),
            candidates: None,
            os_locale: None,
            fetcher: None,
        }
    }

    /// One of the app's own faces, heading `families` in the order the app adds them.
    pub fn app_face(mut self, name: &str, data: FontData, families: &[FontRef]) -> Self {
        self.app_faces.push(AppFace { name: name.to_owned(), data, families: families.iter().map(FontRef::family).collect() });
        self
    }

    /// One of egui's default faces (`Hack`, `Ubuntu-Light`, `NotoEmoji-Regular`,
    /// `emoji-icon-font`) at the end of `families`' configured faces, ahead of installed system
    /// fonts, e.g. Hack in the proportional family for symbols the app's face lacks (⇧, ▸). A name
    /// egui doesn't ship is ignored.
    pub fn egui_face(mut self, name: &str, families: &[FontRef]) -> Self {
        self.egui_faces.push((name.to_owned(), families.iter().map(FontRef::family).collect()));
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
    pub fn craft_faces(mut self, faces: &[EmbeddedFace]) -> Self {
        self.craft_faces.extend_from_slice(faces);
        self.served.resize(self.craft_faces.len(), None);
        self
    }

    /// The craft-fonts faces a web build serves beside its wasm (see `craft-fonts-build`'s
    /// `Embed::serve`); may be empty. They take the places embedded faces would, and with a
    /// [`Self::fetcher`] [`Stack::install`] fetches them: the UI language's faces at once
    /// ([`Stack::ui_downloads`]), others the first time text needs a character that their files
    /// map and the faces already installed lack ([`Stack::downloads_for`]). Until a face arrives,
    /// text falls back to the faces after it. Without a fetcher they are never fetched.
    pub fn served_faces(mut self, faces: &[ServedFace]) -> Self {
        for face in faces {
            self.craft_faces.push(chain::without_bytes(face));
            self.served.push(Some(*face));
        }
        self
    }

    /// What fetches the served faces: the browser's fetcher on the web. Without one, served faces
    /// are only ordered, never fetched.
    pub fn fetcher(mut self, fetcher: Arc<dyn FontFetcher>) -> Self {
        self.fetcher = Some(fetcher);
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

    /// How emoji are drawn.
    pub fn emoji(mut self, emoji: Emoji) -> Self {
        self.emoji = emoji;
        self
    }

    /// Whether CJK faces after the app's face keep its row height ([`CjkRows::Fit`], the default)
    /// or keep their own metrics ([`CjkRows::Grow`]).
    pub fn cjk_rows(mut self, rows: CjkRows) -> Self {
        self.cjk_rows = rows;
        self
    }

    /// The face the interface draws with: the app's own faces ([`InterfaceFont::Suite`], the
    /// default), or the OS's ([`InterfaceFont::System`], see [`super::system_ui_face`]). The system
    /// face's regular face goes ahead of the app's faces in the proportional family, and its
    /// semibold face ahead of them in the [`SEMIBOLD`] family if the app has one; the rest of each
    /// chain is the one [`InterfaceFont::Suite`] builds. The setting changes only the face: the
    /// families and their weights stay the app's, and the monospace family keeps the app's faces.
    /// Where no system face resolves (the web, no `system_fonts` feature), the app's faces stay
    /// first.
    ///
    /// Building the stack doesn't wait for the index of installed fonts. Until the face resolves,
    /// the chains hold [`super::SYSTEM_UI_REGULAR`] and [`super::SYSTEM_UI_SEMIBOLD`] without data,
    /// which install skips, so the app's faces draw; [`Stack::install`] then installs the stack
    /// again, with the face's data and any edits made to it, at the start of the first pass after
    /// the face resolves. Checks and tests that need the face call [`super::system_ui_face`] first,
    /// which waits.
    pub fn interface_font(mut self, font: InterfaceFont) -> Self {
        self.interface = font;
        self
    }

    /// Whether and how installed system fonts are used.
    pub fn system_fonts(mut self, system: SystemFonts) -> Self {
        self.system = system;
        self
    }

    /// The provider to use for installed fonts instead of [`super::system_font_provider`] (with the
    /// `system_fonts` feature, on desktop). It is wrapped in [`RegularWeight`](super::RegularWeight).
    pub fn system_provider(mut self, provider: Arc<dyn FontProvider>) -> Self {
        self.system_provider = Some(provider);
        self
    }

    /// Register faces for other scripts on demand from `candidates`: nothing beyond the app and
    /// craft faces is installed until a character needs it. Off by default.
    pub fn on_demand(mut self, candidates: Arc<dyn FontCandidates>) -> Self {
        self.candidates = Some(candidates);
        self
    }

    /// The OS locale (`zh-TW`), deciding the Han preference when the UI language is not CJK
    /// ([`han_for_locale`]).
    pub fn os_locale(mut self, locale: &str) -> Self {
        self.os_locale = Some(locale.to_owned());
        self
    }

    /// Build the stack and install it in `ctx`, with the on-demand loader when one is configured.
    pub fn install(&self, ctx: &egui::Context) -> Stack {
        let stack = self.stack();
        stack.install(ctx);
        if let Some(candidates) = &self.candidates {
            OnDemandFonts::new(candidates.clone(), han_for_locale(&self.ui_language, self.os_locale.as_deref())).install(ctx);
        }
        stack
    }

    /// Build the stack without installing it.
    pub fn stack(&self) -> Stack {
        self.stack_with_system_ui(ui_face::resolved())
    }
}

/// A built font stack: the configured faces and chains, and the providers asked after them.
#[derive(Clone)]
pub struct Stack {
    /// The configured faces and each family's chain. A chain entry without font data is skipped.
    pub(in crate::fonts) definitions: FontDefinitions,
    /// Faces asked after the system fonts: emoji, egui's defaults and, with
    /// [`SystemFonts::BeforeCraft`], the craft faces for other scripts.
    pub(in crate::fonts) after_system: FaceList,
    system: SystemFonts,
    system_provider: Option<Arc<dyn FontProvider>>,
    ui_language: String,
    /// The scripts of the UI language and every offered language.
    language_scripts: Vec<(String, Vec<String>)>,
    pub(in crate::fonts) weights: Vec<(FontRef, u16)>,
    /// The served faces in the chains, by name; their data is added as it arrives.
    pub(in crate::fonts) served: Vec<ServedFace>,
    pub(in crate::fonts) cjk_rows: CjkRows,
    /// Names of the served faces to fetch at install.
    pub(in crate::fonts) ui_downloads: Vec<String>,
    /// The CJK faces fitted into the rows of the proportional family's first face.
    cjk_faces: Vec<String>,
    /// The stack has [`InterfaceFont::System`] and the system face hasn't resolved yet.
    system_ui_pending: bool,
    pub(in crate::fonts) fetcher: Option<Arc<dyn FontFetcher>>,
}

impl Stack {
    /// Each family's weight, for wrapping another provider in [`RegularWeight`](super::RegularWeight).
    pub fn weights(&self) -> &[(FontRef, u16)] {
        &self.weights
    }

    /// The chain of `family`, if the stack has it.
    pub fn chain(&self, family: &FontRef) -> &[String] {
        self.chain_of(&family.family())
    }

    pub(in crate::fonts) fn chain_of(&self, family: &FontFamily) -> &[String] {
        self.definitions.families.get(family).map_or(&[], Vec::as_slice)
    }

    /// Insert a face into `family`'s chain at `index` (clamped), adding its data.
    pub fn insert(&mut self, family: &FontRef, index: usize, name: &str, data: FontData) {
        self.definitions.font_data.insert(name.to_owned(), Arc::new(data));
        let chain = self.definitions.families.entry(family.family()).or_default();
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

    /// Whether the proportional family can draw `language`'s scripts, e.g. to leave a language out
    /// of the app's menu on a machine that can't show it. `with_system` also asks the system font
    /// provider. The scripts come from the stack's [`LanguageScripts`] for the UI language and
    /// [`UiFonts::languages`], and from craft-i18n for any other language. Each script is checked
    /// with a few of its common letters. A served face counts for the scripts it is tagged with,
    /// fetched or not, since it is fetched when text needs it.
    pub fn covers_language(&self, language: &str, with_system: bool) -> bool {
        let scripts = match self.language_scripts.iter().find(|(l, _)| l == language) {
            Some((_, scripts)) => scripts.clone(),
            None => chain::expand(&I18nScripts.scripts(language)),
        };
        let mut probe = self.probe(with_system);
        let proportional = FontRef::proportional();
        let served = |script: &str| self.served.iter().any(|face| chain::covers(face.scripts, script));
        scripts.iter().filter(|s| !served(s)).filter_map(|s| script_sample(s)).flat_map(str::chars).all(|c| probe.has(&proportional, c))
    }

    /// The stack as one set of definitions, without providers: each family's chain followed by the
    /// faces asked after the system fonts (emoji and egui's default faces), for a context or
    /// font set that takes fonts only through definitions.
    pub fn flat_definitions(&self) -> FontDefinitions {
        let mut definitions = self.checked_definitions();
        let FontDefinitions { font_data, families, .. } = &mut definitions;
        for (family, chain) in families {
            for name in self.after_system.chain(&FontRef::from_family(family.clone())) {
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

    /// Fit the CJK faces `names` into the rows of the proportional family's first face, except where
    /// one of them leads a family. A system face slot without data is passed over: the face after
    /// it draws until the slot gets its data, which fits them again.
    pub(in crate::fonts) fn fit_into_rows(&mut self, names: &[String]) {
        let font_data = &self.definitions.font_data;
        let empty_slot = |name: &&String| *name == ui_face::SYSTEM_UI_REGULAR && !font_data.contains_key(name.as_str());
        let Some(primary) =
            self.chain_of(&FontFamily::Proportional).iter().find(|name| !empty_slot(name)).and_then(|name| font_data.get(name)).cloned()
        else {
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
}
