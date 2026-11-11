//! Font definitions and the on-demand script coverage policy; the egui calls sit in `runtime::egui::fonts`.

mod defaults;
pub(crate) mod document;
mod faces;
pub(crate) mod on_demand;
pub(crate) mod script;
pub(crate) mod system;

pub use defaults::deflated_default_definitions;
pub use document::{DOCUMENT_FACES_PER_PASS, FaceBytes, FaceState, MAX_DOCUMENT_FACES};
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
pub use stack::{CjkRows, Emoji, Family, SEMIBOLD, Stack, SystemFonts, UiFonts};
pub(crate) use stack::{StackFacts, stack_facts};

use crate::runtime::egui::fonts as egui_fonts;
use crate::runtime::frame;
use crate::runtime::layout_memory;
use egui::{Context, FontDefinitions, FontFamily};

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

    /// A family registered under `name`.
    pub fn named(name: &str) -> FontRef {
        FontRef(FontFamily::Name(name.into()))
    }

    pub(crate) fn from_family(family: FontFamily) -> FontRef {
        FontRef(family)
    }

    pub(crate) fn family(&self) -> FontFamily {
        self.0.clone()
    }
}

/// Gives the UI `definitions`, as when on-demand script coverage loads the glyphs of another script. Text
/// measured in this pass used the old metrics, so the pass asks for one discard pass: layout memory and the
/// new metrics agree when the frame is shown. Between frames nothing was measured with the old metrics, and
/// the next frame measures with the new ones.
pub fn load(ctx: &Context, definitions: FontDefinitions) {
    egui_fonts::set_definitions(ctx, definitions);
    if frame::in_pass(ctx) {
        layout_memory::ask_discard(ctx);
    }
}
