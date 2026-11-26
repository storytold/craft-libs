//! The egui font calls only; the top-level `fonts` module owns the definitions, the providers and the coverage policy.
//!
//! One fonts plugin is registered per context, on first need. Its begin-pass hook runs the steps added with
//! `on_begin_pass`; its output hook runs the on-demand scan. The plugin holds no state: the provider list, the steps
//! and the loader live in the runtime store.
//!
//! The faces served one family at a time (`serve_face`: the document faces) reach egui through one provider that is
//! always last in the context's list. egui asks a provider for a family's faces the first time text is laid out in
//! it, so serving a face drops nothing egui has parsed, laid out or put in its atlas, as adding it to the definitions
//! would.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use egui::epaint::ClippedShape;
use egui::epaint::text::{Fonts, TextOptions};
use egui::{Context, FontData, FontDefinitions, FontFamily, FontId, FontInsert, FontPriority, FontProvider, FullOutput, InsertFontFamily};
use egui::{Plugin, Ui, ViewportId};

use crate::fonts::FontRef;
use crate::runtime::egui::{memory, pass, repaint};
use crate::runtime::frame;

/// Gives egui `definitions`; the next pass lays text out with them. The first definitions bring the served faces'
/// provider with them, so the served faces are found from the first pass that lays text out.
pub(crate) fn set_definitions(ctx: &Context, definitions: FontDefinitions) {
    let (served, providers) = memory::with::<Providers, _>(ctx, |providers| {
        let first = providers.served_since.is_none();
        (Arc::clone(&providers.served), first.then(|| providers.all(ctx)))
    });
    served.set_interface(&definitions);
    ctx.set_fonts(definitions);
    if let Some(providers) = providers {
        ctx.set_font_providers(providers);
    }
}

/// A face served as a family of its own.
struct ServedFace {
    data: FontData,
    /// The proportional family's faces follow the face in its family.
    over_interface: bool,
    /// The served families whose faces follow the face, in order, before the interface faces: the sibling subsets of
    /// a font split over several.
    chain: Vec<String>,
}

/// The faces served one family at a time, by family name, and the proportional family's faces as the definitions
/// last given to egui list them (`None` until definitions are given or a face over them is served).
#[derive(Default)]
struct Served {
    families: RwLock<HashMap<String, ServedFace>>,
    interface: RwLock<Option<Vec<FontInsert>>>,
}

impl Served {
    fn set_interface(&self, definitions: &FontDefinitions) {
        let names = definitions.families.get(&FontFamily::Proportional).map(Vec::as_slice).unwrap_or_default();
        let faces =
            names.iter().filter_map(|name| definitions.font_data.get(name).map(|data| FontInsert::new(name, (**data).clone(), Vec::new()))).collect();
        *self.interface.write().unwrap_or_else(PoisonError::into_inner) = Some(faces);
    }
}

impl FontProvider for Served {
    fn fonts_for_family(&self, family: &FontFamily) -> Vec<FontInsert> {
        let FontFamily::Name(name) = family else { return Vec::new() };
        let families = self.families.read().unwrap_or_else(PoisonError::into_inner);
        let Some(face) = families.get(&**name) else { return Vec::new() };
        let mut faces = vec![FontInsert::new(name, face.data.clone(), Vec::new())];
        for link in face.chain.iter().filter(|link| **link != **name) {
            if let Some(linked) = families.get(link) {
                faces.push(FontInsert::new(link, linked.data.clone(), Vec::new()));
            }
        }
        if face.over_interface {
            let interface = self.interface.read().unwrap_or_else(PoisonError::into_inner);
            faces.extend(interface.iter().flatten().filter(|face| *face.name != **name).cloned());
        }
        faces
    }
}

/// The context's font providers, in the order egui asks them, and the served faces' provider, which follows them.
#[derive(Default)]
struct Providers {
    kept: Vec<Arc<dyn FontProvider>>,
    served: Arc<Served>,
    /// The root frame in which the served faces' provider was given to egui, and whether that was inside a pass.
    served_since: Option<(u64, bool)>,
}

impl Providers {
    /// The providers egui is given: the kept ones, then the served faces'.
    fn all(&mut self, ctx: &Context) -> Vec<Arc<dyn FontProvider>> {
        self.mark_served(ctx);
        self.kept.iter().cloned().chain(std::iter::once(Arc::clone(&self.served) as Arc<dyn FontProvider>)).collect()
    }

    fn mark_served(&mut self, ctx: &Context) {
        if self.served_since.is_none() {
            self.served_since = Some((pass::root_frame_nr(ctx), frame::in_pass(ctx)));
        }
    }
}

fn served(ctx: &Context) -> Arc<Served> {
    memory::with::<Providers, _>(ctx, |providers| Arc::clone(&providers.served))
}

/// The steps the fonts plugin runs at the start of every pass.
#[derive(Default)]
struct Steps(Vec<fn(&Context)>);

/// Sets the context's font providers and remembers them for [`reload_providers`].
pub(crate) fn set_providers(ctx: &Context, providers: Vec<Arc<dyn FontProvider>>) {
    let all = memory::with::<Providers, _>(ctx, |kept| {
        kept.kept.clone_from(&providers);
        kept.all(ctx)
    });
    ctx.set_font_providers(all);
}

/// Adds a font provider after the context's others.
pub(crate) fn add_provider(ctx: &Context, provider: Arc<dyn FontProvider>) {
    let all = memory::with::<Providers, _>(ctx, |kept| {
        kept.kept.push(Arc::clone(&provider));
        kept.all(ctx)
    });
    ctx.set_font_providers(all);
}

/// Sets the context's providers again, so egui forgets the characters they had no font for, drops the faces it
/// parsed (the served faces no longer served among them) and repaints. Callable from any thread.
pub(crate) fn reload_providers(ctx: &Context) {
    let providers = memory::with::<Providers, _>(ctx, |kept| kept.all(ctx));
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
    if let Some(interface) = served(ctx).interface.write().unwrap_or_else(PoisonError::into_inner).as_mut() {
        interface.push(FontInsert::new(&name, data.clone(), Vec::new()));
    }
    let families = families.into_iter().map(|family| InsertFontFamily { family, priority: FontPriority::Lowest }).collect();
    ctx.add_font(FontInsert { name, data, families });
}

/// Serves `data` as the family `family` (its face's name too): the face, then the faces of the served families of
/// `chain` that are served when egui first lays text out in the family, in order, then with `over_interface` the
/// proportional family's faces, so what the face lacks its chain and then the interface fonts draw. Returns whether
/// text laid out in the family in this pass finds it: false in the pass that first gives egui the served faces'
/// provider.
pub(crate) fn serve_face(ctx: &Context, family: &FontRef, data: FontData, over_interface: bool, chain: Vec<String>) -> bool {
    let FontFamily::Name(name) = family.family() else { return false };
    let (served, since) = memory::with::<Providers, _>(ctx, |providers| (Arc::clone(&providers.served), providers.served_since));
    let unset = served.interface.read().unwrap_or_else(PoisonError::into_inner).is_none();
    if over_interface && unset {
        ctx.fonts(|fonts| served.set_interface(fonts.definitions()));
    }
    served.families.write().unwrap_or_else(PoisonError::into_inner).insert(name.to_string(), ServedFace { data, over_interface, chain });
    match since {
        Some(since) => served_live(ctx, since),
        None => {
            reload_providers(ctx);
            !frame::in_pass(ctx)
        }
    }
}

/// Whether the served faces' provider, given to egui in `since`, is the one this pass lays text out with.
fn served_live(ctx: &Context, (frame_nr, in_pass): (u64, bool)) -> bool {
    !in_pass || frame_nr < pass::root_frame_nr(ctx)
}

/// Whether text laid out in this pass finds the served faces: false before any is served and in the pass that first
/// gives egui their provider.
pub(crate) fn is_served(ctx: &Context) -> bool {
    memory::with::<Providers, _>(ctx, |providers| providers.served_since).is_some_and(|since| served_live(ctx, since))
}

/// Stops serving the families `gone`. egui keeps the faces it parsed from them until its fonts are made again
/// (`reload_providers`, or a change of the definitions or providers).
pub(crate) fn withdraw_faces(ctx: &Context, gone: &[String]) {
    let served = served(ctx);
    let mut families = served.families.write().unwrap_or_else(PoisonError::into_inner);
    for name in gone {
        families.remove(name);
    }
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
