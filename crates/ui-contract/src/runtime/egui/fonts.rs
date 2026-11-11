//! The egui font calls only; the top-level `fonts` module owns the definitions, the providers and the coverage policy.
//!
//! One fonts plugin is registered per context, on first need. Its begin-pass hook runs the steps added with
//! [`on_begin_pass`]; its output hook runs the on-demand scan. The plugin holds no state: the provider list, the steps
//! and the loader live in the runtime store.

use std::sync::Arc;

use egui::epaint::ClippedShape;
use egui::epaint::text::{Fonts, TextOptions};
use egui::{Context, FontData, FontDefinitions, FontFamily, FontId, FontInsert, FontPriority, FontProvider, FullOutput, InsertFontFamily};
use egui::{Plugin, Ui, ViewportId};

use crate::fonts::FontRef;
use crate::runtime::egui::{memory, repaint};

/// Gives egui `definitions`; the next pass lays text out with them.
pub(crate) fn set_definitions(ctx: &Context, definitions: FontDefinitions) {
    ctx.set_fonts(definitions);
}

/// The context's font providers, in the order egui asks them.
#[derive(Default)]
struct Providers(Vec<Arc<dyn FontProvider>>);

/// The steps the fonts plugin runs at the start of every pass.
#[derive(Default)]
struct Steps(Vec<fn(&Context)>);

/// Sets the context's font providers and remembers them for [`reload_providers`].
pub(crate) fn set_providers(ctx: &Context, providers: Vec<Arc<dyn FontProvider>>) {
    memory::with::<Providers, _>(ctx, |kept| kept.0.clone_from(&providers));
    ctx.set_font_providers(providers);
}

/// Adds a font provider after the context's others.
pub(crate) fn add_provider(ctx: &Context, provider: Arc<dyn FontProvider>) {
    memory::with::<Providers, _>(ctx, |kept| kept.0.push(Arc::clone(&provider)));
    ctx.add_font_provider(provider);
}

/// Sets the context's providers again, so egui forgets the characters they had no font for and repaints. Callable
/// from any thread.
pub(crate) fn reload_providers(ctx: &Context) {
    let providers = memory::with::<Providers, _>(ctx, |kept| kept.0.clone());
    ctx.set_font_providers(providers);
}

/// Adds `data` under `name` to every font family at the lowest priority.
pub(crate) fn add_face(ctx: &Context, name: String, data: FontData) {
    let mut families: Vec<FontFamily> = ctx.fonts(|fonts| fonts.definitions().families.keys().cloned().collect());
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        if !families.contains(&family) {
            families.push(family);
        }
    }
    let families = families.into_iter().map(|family| InsertFontFamily { family, priority: FontPriority::Lowest }).collect();
    ctx.add_font(FontInsert { name, data, families });
}

/// Adds `data` as a face named after `family` and makes it the highest-priority face of `family` alone. The face is
/// loaded at the next pass.
pub(crate) fn add_family_face(ctx: &Context, family: &FontRef, data: FontData) {
    let insert = InsertFontFamily { family: family.family(), priority: FontPriority::Highest };
    ctx.add_font(FontInsert { name: family.family().to_string(), data, families: vec![insert] });
}

/// Whether egui's definitions hold the family `family`.
pub(crate) fn has_family(ctx: &Context, family: &FontRef) -> bool {
    ctx.fonts(|fonts| fonts.definitions().families.contains_key(&family.family()))
}

/// Takes the faces called `names`, and the families of those names, out of egui's definitions.
/// It rebuilds the definitions from the applied ones, so call it only from a begin-pass step, before an install of the
/// pass can queue its own.
pub(crate) fn remove_faces(ctx: &Context, names: &[String]) {
    let mut definitions = ctx.fonts(|fonts| fonts.definitions().clone());
    definitions.font_data.retain(|name, _| !names.contains(name));
    definitions.families.retain(|family, _| !matches!(family, FontFamily::Name(name) if names.iter().any(|gone| gone.as_str() == &**name)));
    set_definitions(ctx, definitions);
}

/// Whether egui's definitions hold the face called `name`.
pub(crate) fn has_face(ctx: &Context, name: &str) -> bool {
    ctx.fonts(|fonts| fonts.definitions().font_data.contains_key(name))
}

/// The first of `chars` that the proportional family has no glyph for.
pub(crate) fn first_without_glyph(ctx: &Context, chars: impl IntoIterator<Item = char>) -> Option<char> {
    // Whether a face has a glyph does not depend on the size.
    let font = FontId::default();
    ctx.fonts_mut(|fonts| chars.into_iter().find(|c| !fonts.has_glyph(&font, *c)))
}

/// Asks every viewport for a pass. Callable from any thread.
pub(crate) fn wake(ctx: &Context) {
    let viewports: Vec<ViewportId> = ctx.input(|input| input.raw.viewports.keys().copied().collect());
    repaint::request_of(ctx, ViewportId::ROOT);
    for viewport in viewports {
        repaint::request_of(ctx, viewport);
    }
}

/// Adds a step to the fonts plugin's begin-pass hook. A step added twice runs once.
pub(crate) fn on_begin_pass(ctx: &Context, step: fn(&Context)) {
    memory::with::<Steps, _>(ctx, |steps| {
        if !steps.0.iter().any(|known| std::ptr::fn_addr_eq(*known, step)) {
            steps.0.push(step);
        }
    });
    ensure_plugin(ctx);
}

/// Registers the fonts plugin on `ctx` unless it is there.
pub(crate) fn ensure_plugin(ctx: &Context) {
    if ctx.plugin_opt::<FontsPlugin>().is_none() {
        ctx.add_plugin(FontsPlugin);
    }
}

/// The fonts plugin. egui holds a plugin's mutex while it calls a hook, so the struct holds no state.
struct FontsPlugin;

impl Plugin for FontsPlugin {
    fn debug_name(&self) -> &'static str {
        "craft_ui.fonts"
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        let steps = memory::with::<Steps, _>(ui.ctx(), |steps| steps.0.clone());
        for step in steps {
            step(ui.ctx());
        }
    }

    fn output_hook(&mut self, ctx: &Context, output: &mut FullOutput) {
        let shapes: &[ClippedShape] = &output.shapes;
        crate::fonts::on_demand::after_pass(ctx, shapes);
    }
}

/// Measures text with a font set that belongs to no context, for coverage checks and row heights.
pub(crate) struct GlyphProbe(Fonts);

impl GlyphProbe {
    /// A probe over `definitions`, with `providers` asked for characters the faces lack.
    pub(crate) fn new(definitions: FontDefinitions, providers: Vec<Arc<dyn FontProvider>>) -> Self {
        Self(Fonts::new(TextOptions::default(), definitions).with_font_providers(providers))
    }

    /// Whether the faces of `family` have a glyph for `c`.
    pub(crate) fn has(&mut self, family: &FontRef, c: char) -> bool {
        self.0.has_glyph(&FontId { family: family.family(), ..FontId::default() }, c)
    }

    /// The height of a row of `family` text at `size` points.
    pub(crate) fn row_height(&mut self, family: &FontRef, size: f32) -> f32 {
        self.0.with_pixels_per_point(1.0).row_height(&FontId::new(size, family.family()))
    }
}
