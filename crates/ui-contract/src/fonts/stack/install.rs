//! Installing a stack in a context: definitions, providers, and the reinstall once the OS interface face resolves.

use std::sync::Arc;

use egui::{Context, FontFamily, FontProvider};

use super::{SEMIBOLD, Stack, SystemFonts};
use crate::fonts::faces::{Installed, on_baseline};
use crate::fonts::served::loader;
use crate::fonts::system::installed_system_font_provider;
use crate::fonts::system::ui_face::{self, SYSTEM_UI_REGULAR, SYSTEM_UI_SEMIBOLD, SystemUiFace};
use crate::fonts::{FontRef, RegularWeight, load, system_font_provider};
use crate::runtime::egui::fonts::{self as egui_fonts, GlyphProbe};
use crate::runtime::egui::memory;

/// Starts building the system font index for a context.
type StartIndex = fn(&Context);

/// What a context remembers of the stack installed in it.
#[derive(Default)]
struct State {
    /// Whether the OS's interface face draws in the proportional family of that stack.
    system_ui: bool,
    /// Whether that stack has the semibold family.
    semibold: bool,
    /// The line height per point of text of the face that leads the proportional family, measured at install.
    line_ratio: f32,
    /// The provider that serves the stack's faces after the system fonts, swapped in place by every install.
    installed: Option<Arc<Installed>>,
    /// A stack installed before the OS face resolved, to install again with it.
    pending: Option<Stack>,
}

/// The size, in points, at which the leading face's line height is measured. Large, so the pixel rounding of a row
/// height is negligible in the ratio.
const LINE_PROBE_SIZE: f32 = 100.0;

/// What text layout and control sizing read of the stack last installed in a context, read once per layout or pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct StackFacts {
    /// Whether the OS's interface face draws in the proportional family ([`craft_theme::InterfaceFont::System`], once
    /// the face resolved).
    pub system_ui: bool,
    /// Whether the semibold family exists.
    pub semibold: bool,
    /// The line height per point of text of the face that leads the proportional family; 0 before a stack installs.
    pub line_ratio: f32,
}

/// The facts of the stack last installed in `ctx`.
pub(crate) fn stack_facts(ctx: &Context) -> StackFacts {
    memory::with::<State, _>(ctx, |state| StackFacts { system_ui: state.system_ui, semibold: state.semibold, line_ratio: state.line_ratio })
}

/// Installs the stack kept for `ctx` once the OS face has resolved: the first pass after it resolves.
fn install_pending(ctx: &Context) {
    let Some(face) = ui_face::resolved() else { return };
    let pending = memory::with::<State, _>(ctx, |state| state.pending.take());
    if let Some(mut stack) = pending {
        stack.provide_system_ui(face);
        stack.install(ctx);
        egui_fonts::wake(ctx);
    }
}

impl Stack {
    /// Install the faces and providers in `ctx`. Call it again when the UI language changes. Inside
    /// a pass the new fonts ask one discard pass ([`crate::fonts::load`]).
    ///
    /// The stack's system font provider (the `system_fonts` feature on desktop, or
    /// [`super::UiFonts::system_provider`]) replaces the context's font providers, such as the one
    /// eframe adds, as does [`SystemFonts::Off`]. Without one, the context keeps its providers and
    /// the stack's faces are asked after them.
    ///
    /// A stack with [`super::UiFonts::served_faces`] and a [`super::UiFonts::fetcher`] also fetches
    /// them (see there); the faces already fetched for an earlier stack are added at once.
    ///
    /// A stack built with [`craft_theme::InterfaceFont::System`] before the system face resolved is
    /// installed without it, then again with it at the start of the first pass after it resolves,
    /// unless another stack is installed in `ctx` first (see [`super::UiFonts::interface_font`]).
    pub fn install(&self, ctx: &Context) {
        if self.system_ui_pending
            && let Some(face) = ui_face::resolved()
        {
            let mut stack = self.clone();
            stack.provide_system_ui(face);
            return stack.install(ctx);
        }
        if !self.served.is_empty()
            && let Some(fetcher) = &self.fetcher
        {
            return loader::install(ctx, self.clone(), Arc::clone(fetcher));
        }
        self.install_faces(ctx, None);
    }

    /// [`Self::install`] without fetching; `miss` is asked for characters no face has.
    pub(in crate::fonts) fn install_faces(&self, ctx: &Context, miss: Option<Arc<dyn FontProvider>>) {
        load(ctx, self.checked_definitions());
        let system_ui = self.has_system_ui();
        let semibold = self.definitions.families.contains_key(&FontFamily::Name(SEMIBOLD.into()));
        let line_ratio = self.probe(false).row_height(&FontRef::proportional(), LINE_PROBE_SIZE) / LINE_PROBE_SIZE;
        let (system, start_index) = self.context_lookup();
        let (installed, added, replaced) = memory::with::<State, _>(ctx, |state| {
            let replaced = state.pending.take();
            state.system_ui = system_ui;
            state.semibold = semibold;
            state.line_ratio = line_ratio;
            state.pending = self.system_ui_pending.then(|| self.clone());
            match &state.installed {
                Some(installed) => (Arc::clone(installed), false, replaced),
                None => {
                    let installed = Arc::new(Installed::default());
                    state.installed = Some(Arc::clone(&installed));
                    (installed, true, replaced)
                }
            }
        });
        drop(replaced);
        if added && system.is_none() {
            egui_fonts::add_provider(ctx, Arc::clone(&installed) as Arc<dyn FontProvider>);
        }
        installed.set(self.after_system.clone(), miss);
        if let Some(system) = system {
            let mut providers = system.into_iter().collect::<Vec<_>>();
            providers.push(installed as Arc<dyn FontProvider>);
            egui_fonts::set_providers(ctx, providers);
        }
        if let Some(start) = start_index {
            start(ctx);
        }
        if self.system_ui_pending {
            egui_fonts::on_begin_pass(ctx, install_pending);
            ui_face::wake_on_resolve(ctx);
        }
    }

    /// Add the system face's data to its slots ([`SYSTEM_UI_REGULAR`], [`SYSTEM_UI_SEMIBOLD`]) and
    /// fit the CJK faces into its rows, or with `None` (no face resolves) drop the empty slots.
    pub(in crate::fonts) fn provide_system_ui(&mut self, face: Option<&SystemUiFace>) {
        self.system_ui_pending = false;
        for (name, data) in [(SYSTEM_UI_REGULAR, face.map(|f| &f.regular)), (SYSTEM_UI_SEMIBOLD, face.map(|f| &f.semibold))] {
            match data {
                Some(data) => {
                    if self.definitions.families.values().any(|chain| chain.iter().any(|n| n == name)) {
                        self.definitions.font_data.insert(name.to_owned(), Arc::new(data.clone()));
                    }
                }
                None if !self.definitions.font_data.contains_key(name) => {
                    for chain in self.definitions.families.values_mut() {
                        chain.retain(|n| n != name);
                    }
                }
                None => {}
            }
        }
        if face.is_some() {
            let cjk_faces = self.cjk_faces.clone();
            self.fit_into_rows(&cjk_faces);
        }
    }

    /// Whether the system face draws in the proportional family: its slot is there, with data.
    fn has_system_ui(&self) -> bool {
        self.chain_of(&FontFamily::Proportional).iter().any(|n| n == SYSTEM_UI_REGULAR) && self.definitions.font_data.contains_key(SYSTEM_UI_REGULAR)
    }

    /// A glyph probe over the stack, for coverage checks. `with_system` adds the system font
    /// provider.
    pub(in crate::fonts) fn probe(&self, with_system: bool) -> GlyphProbe {
        GlyphProbe::new(self.checked_definitions(), self.providers(with_system))
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
        let provider = on_baseline(provider);
        Arc::new(
            self.weights.iter().fold(RegularWeight::new(provider), |p, (family, weight): &(FontRef, u16)| p.with_weight(family.clone(), *weight)),
        )
    }
}
