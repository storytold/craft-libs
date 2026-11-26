//! Font definitions and the on-demand script coverage policy; the egui calls sit in `runtime::egui::fonts`.

mod defaults;
pub(crate) mod document;
mod faces;
pub(crate) mod on_demand;
pub(crate) mod script;
pub(crate) mod system;

pub use defaults::{deflated_default_definitions, emoji_icon_font, noto_emoji};
pub use document::{FaceBytes, FaceState, MAX_DOCUMENT_FACE_BYTES};
pub use faces::{FaceList, RegularWeight, set_weight};
pub use on_demand::{FontCandidates, MAX_SCRIPT_READS, NoCandidates, OnDemandFonts, SystemCandidates, han_for_locale, on_demand_registered};
pub use system::system_font_provider;
pub use system::ui_face::{SYSTEM_UI_OPTICAL_SIZE, SYSTEM_UI_REGULAR, SYSTEM_UI_SEMIBOLD, SystemUiFace, poll_system_ui_face, system_ui_face};

mod coverage;
mod language;
mod served;
mod stack;

pub use coverage::{Missing, Sample, assert_covers, assert_covers_with_system, missing_glyphs};
pub use craft_fonts_build::{EmbeddedFace, ServedFace};
pub use language::{I18nScripts, LanguageScripts};
pub use served::loader::{FetchDone, FetchWake, FontFetcher};
pub use served::loading;
pub use stack::{CjkRows, Emoji, Family, SEMIBOLD, Stack, SystemFonts, TAB_TITLE, UiFonts};
pub(crate) use stack::{StackFacts, stack_facts};

use crate::runtime::egui::fonts as egui_fonts;
use crate::runtime::frame;
use crate::runtime::layout_memory;
use egui::{Context, FontDefinitions, FontFamily};

/// The egui family name of [`FontRef::signature`]; the stack registers it in every build.
const SIGNATURE_FAMILY: &str = "signature";

/// A font handle: the family a run is set in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FontRef(FontFamily);

impl FontRef {
    /// The interface face.
    pub fn proportional() -> FontRef {
        FontRef(FontFamily::Proportional)
    }

    /// The monospace face.
    pub fn monospace() -> FontRef {
        FontRef(FontFamily::Monospace)
    }

    /// The signature face: always a family egui knows. Its chain is the craft-fonts faces of the signature role,
    /// then the proportional chain; without one in the build, the proportional chain.
    pub fn signature() -> FontRef {
        FontRef::named(SIGNATURE_FAMILY)
    }

    /// A family registered under `name`.
    pub fn named(name: &str) -> FontRef {
        FontRef(FontFamily::Name(name.into()))
    }

    pub(crate) fn from_family(family: FontFamily) -> FontRef {
        FontRef(family)
    }

    /// The egui family that lays the face out, for a layout job an app builds itself: the open paragraph of a text
    /// editor, whose galley its `TextEditorHost` lays out. Content painters lay document runs out through `DocText`,
    /// which takes the `FontRef` itself.
    pub fn family(&self) -> FontFamily {
        self.0.clone()
    }
}

/// Gives the UI `definitions`, as when on-demand script coverage loads the glyphs of another script. Text
/// measured in this pass used the old metrics, so the pass asks for one discard pass: layout memory and the
/// new metrics agree when the frame is shown. Between frames nothing was measured with the old metrics, and
/// the next frame measures with the new ones. What text layout reads of the definitions follows them: framework text
/// in a semibold role draws in the [`SEMIBOLD`] family only when `definitions` have it, else in the proportional one.
pub fn load(ctx: &Context, definitions: FontDefinitions) {
    load_with(ctx, definitions, Vec::new());
}

/// [`load`], with `providers` serving the characters the faces lack to the probe that measures the definitions' line
/// ratio: a stack's own providers.
pub(crate) fn load_with(ctx: &Context, definitions: FontDefinitions, providers: Vec<std::sync::Arc<dyn egui::FontProvider>>) {
    stack::record_facts(ctx, &definitions, providers);
    egui_fonts::set_definitions(ctx, definitions);
    if frame::in_pass(ctx) {
        layout_memory::ask_discard(ctx);
    }
}

#[cfg(all(feature = "testing", any(test, feature = "suite")))]
pub(crate) mod facts_tests {
    use crate::fonts::{SEMIBOLD, TAB_TITLE, deflated_default_definitions, load, stack_facts};
    use crate::testing::trial;
    use craft_theme::{Contrast, Mode, TypeRole};
    use egui::{Context, FontFamily};

    /// Definitions an app gives in place of the stack's carry what text layout reads: without the semibold family, a
    /// framework title draws in a family the definitions have.
    #[trial]
    fn a_title_draws_in_a_family_the_loaded_definitions_have() {
        let ctx = Context::default();
        let tokens = crate::testing::style::tokens(Mode::Light, Contrast::Standard);
        let semibold = FontFamily::Name(SEMIBOLD.into());
        let mut definitions = deflated_default_definitions();
        load(&ctx, definitions.clone());
        assert!(stack_facts(&ctx).semibold, "the default definitions have the semibold family");
        assert_eq!(tokens.font(stack_facts(&ctx), TypeRole::Title, false).family, semibold, "and a title draws in it");
        definitions.families.remove(&semibold);
        load(&ctx, definitions.clone());
        assert!(!stack_facts(&ctx).semibold, "definitions without it say so");
        let title = tokens.font(stack_facts(&ctx), TypeRole::Title, false).family;
        assert!(definitions.families.contains_key(&title), "a title draws in a family the definitions have, not {title:?}");
    }

    /// A tab title draws in the tab-title family when the loaded definitions have it, and in the semibold family, or
    /// the proportional one without that, when they don't; it keeps the label's size.
    #[trial]
    fn a_tab_title_draws_in_the_tab_title_family_only_where_the_definitions_have_it() {
        let ctx = Context::default();
        let tokens = crate::testing::style::tokens(Mode::Light, Contrast::Standard);
        let (semibold, tab) = (FontFamily::Name(SEMIBOLD.into()), FontFamily::Name(TAB_TITLE.into()));
        let font = |ctx: &Context| tokens.font_weighted(stack_facts(ctx), TypeRole::TabTitle, false, true);
        let mut definitions = deflated_default_definitions();
        load(&ctx, definitions.clone());
        assert!(!stack_facts(&ctx).tab_title, "the default definitions have no tab-title family");
        assert_eq!(font(&ctx).family, semibold, "a tab title draws in the semibold family");
        assert_eq!(font(&ctx).size, tokens.font(stack_facts(&ctx), TypeRole::Label, false).size, "at the label's size");
        let chain = definitions.families.get(&semibold).cloned().unwrap_or_default();
        definitions.families.insert(tab.clone(), chain);
        load(&ctx, definitions.clone());
        assert!(stack_facts(&ctx).tab_title);
        assert_eq!(font(&ctx).family, tab, "with the family a tab title draws in it");
        assert_eq!(tokens.font_weighted(stack_facts(&ctx), TypeRole::Label, false, true).family, semibold, "a strong label does not");
        definitions.families.remove(&tab);
        definitions.families.remove(&semibold);
        load(&ctx, definitions);
        assert_eq!(font(&ctx).family, FontFamily::Proportional, "without either family a tab title draws in the proportional one");
    }
}
