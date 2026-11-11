//! [`LiquifyPanel`]: Liquify's Properties panel.

use std::ops::RangeInclusive;

use egui::{Response, Ui};

use craft_behavior::EditEvents;
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Pattern, Props as _, SaveKey, Show as _, ShowContainer as _, clicked};
use craft_ui::form::SliderField;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::sections::Section;

use super::session::{BackdropMode, BackdropUse, LiquifyGlobal, LiquifySettings, MeshSize};
use super::tools::LiquifyBrush;
use super::words::LiquifyWords;

/// What [`LiquifyPanel::show`] reports.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct LiquifyPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// The pass's edits of the settings, in order, which already hold the new value: a slider's gesture (`Begin`,
    /// `Update`, `Commit`) or a one-shot `Set` for a check box or a select.
    pub edit: EditEvents<LiquifySettings>,
    /// Reconstruct (with the Amount), None, Mask All or Invert All pressed.
    pub global: Option<LiquifyGlobal>,
    /// Restore All pressed.
    pub restore_all: bool,
    /// Show Backdrop turned on, or Use changed while it is on: the backdrop to ask the app for
    /// (`None`: All Layers).
    pub backdrop_wanted: Option<Option<u64>>,
}

/// The Properties panel in the side column, top to bottom, as PhotoCraft (and Photoshop) order it:
///
/// - **Brush Tool Options**: sliders Size (1..=3000, the
///   [`LiquifyBrush::SLIDER_MAX`](super::LiquifyBrush::SLIDER_MAX)), Density, Pressure, Rate (0..=100).
/// - **Brush Reconstruct Options**: Amount (0..=100 %); buttons Reconstruct and Restore All.
/// - **Mask Options**: buttons None, Mask All, Invert All.
/// - **View Options**: Show Mesh; Mesh Size (Small, Medium, Large); Show Mask; Show Backdrop and,
///   while it is on, Use (All Layers, then the source's layers bottom to top), Mode (In Front,
///   Behind, Blend), Opacity (0..=100).
///
/// PhotoCraft's stats line ("N stroke(s) · field … · dab … ms · preview … ms") is not drawn (lead
/// call 14); [`LiquifyOutput::timing`](super::LiquifyOutput::timing) carries the timing the library
/// can measure. Every control is a Tab stop in that order. The panel writes the new values to the
/// bound settings and reports the pass's change as [`LiquifyPanelOutput::edit`]. The sections remember their
/// open state.
#[derive(Pattern)]
pub struct LiquifyPanel<'a> {
    settings: Bind<'a, LiquifySettings>,
    layers: &'a [(u64, String)],
}

/// What the controls of one pass add up to.
#[derive(Default)]
struct Pass {
    edit: EditEvents<LiquifySettings>,
    global: Option<LiquifyGlobal>,
    restore_all: bool,
    backdrop_wanted: Option<Option<u64>>,
}

/// Where a control writes its value in the settings.
type Field<T> = fn(&mut LiquifySettings, T);

/// A control's edit of one field as an edit of the settings: the settings at the start of the pass with that field
/// replaced.
fn lift<T: Clone + 'static>(events: EditEvents<T>, base: &LiquifySettings, set: Field<T>) -> EditEvents<LiquifySettings> {
    events.map(|value| {
        let mut settings = base.clone();
        set(&mut settings, value);
        settings
    })
}

impl Outcome for LiquifyPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl<'a> LiquifyPanel<'a> {
    /// A panel over the settings `settings`, which it edits in place.
    pub fn new(settings: Bind<'a, LiquifySettings>) -> Self {
        Self { settings, layers: &[] }
    }

    /// Show Backdrop › Use's layers, bottom to top ([`LiquifySource::layers`](super::LiquifySource::layers)).
    #[must_use]
    pub fn layers(mut self, layers: &'a [(u64, String)]) -> Self {
        self.layers = layers;
        self
    }

    /// Draws the panel and returns the pass's changes.
    pub fn show(self, ui: &mut Ui) -> LiquifyPanelOutput {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:995-1059 (the four sections).
        let Self { mut settings, layers } = self;
        let env = Env::of(ui);
        let words = env.words::<LiquifyWords>();
        let mut done = Pass::default();
        let shown = Stack::new().show(ui, |ui, root| {
            let Some(base) = settings.reborrow().read(root).and_then(|held| held.same().cloned()) else { return };
            let mut rows = Rows { base: &base, pass: Pass::default() };
            Section::new(words.brush_options.clone()).remember(SaveKey::new("craft_imaging.liquify.brush")).show(ui, |ui, _| {
                let size = 1.0..=LiquifyBrush::SLIDER_MAX;
                rows.slider(ui, &words.size, base.brush.size, size, None, |s, v| s.brush.size = v);
                rows.slider(ui, &words.density, base.brush.density, 0.0..=100.0, None, |s, v| s.brush.density = v);
                rows.slider(ui, &words.pressure, base.brush.pressure, 0.0..=100.0, None, |s, v| s.brush.pressure = v);
                rows.slider(ui, &words.rate, base.brush.rate, 0.0..=100.0, None, |s, v| s.brush.rate = v);
            });
            Section::new(words.reconstruct_options.clone()).remember(SaveKey::new("craft_imaging.liquify.reconstruct")).show(ui, |ui, _| {
                let amount = base.reconstruct_amount;
                rows.slider(ui, &words.amount, amount, 0.0..=100.0, Some(UnitKind::Percent), |s, v| s.reconstruct_amount = v);
                Row::new().show(ui, |ui, _| {
                    if clicked(&Button::new(words.reconstruct.clone()).id_salt("reconstruct").show(ui)) {
                        rows.pass.global = Some(LiquifyGlobal::Reconstruct { amount });
                    }
                    if clicked(&Button::new(words.restore_all.clone()).id_salt("restore-all").show(ui)) {
                        rows.pass.restore_all = true;
                    }
                });
            });
            Section::new(words.mask_options.clone()).remember(SaveKey::new("craft_imaging.liquify.mask")).show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    if clicked(&Button::new(words.mask_none.clone()).id_salt("mask-none").show(ui)) {
                        rows.pass.global = Some(LiquifyGlobal::MaskNone);
                    }
                    if clicked(&Button::new(words.mask_all.clone()).id_salt("mask-all").show(ui)) {
                        rows.pass.global = Some(LiquifyGlobal::MaskAll);
                    }
                    if clicked(&Button::new(words.invert_all.clone()).id_salt("invert-all").show(ui)) {
                        rows.pass.global = Some(LiquifyGlobal::InvertAll);
                    }
                });
            });
            Section::new(words.view_options.clone()).remember(SaveKey::new("craft_imaging.liquify.view")).show(ui, |ui, _| {
                rows.view_options(ui, &words, layers);
            });
            for event in &rows.pass.edit {
                settings.reborrow().write(root, event);
            }
            done = rows.pass;
        });
        LiquifyPanelOutput {
            response: shown.output,
            edit: done.edit,
            global: done.global,
            restore_all: done.restore_all,
            backdrop_wanted: done.backdrop_wanted,
        }
    }
}

/// The controls of one pass: the settings they started from and what they add up to.
struct Rows<'p> {
    base: &'p LiquifySettings,
    pass: Pass,
}

impl Rows<'_> {
    /// Show Mesh, Mesh Size, Show Mask and the Show Backdrop group.
    fn view_options(&mut self, ui: &mut Ui, words: &LiquifyWords, layers: &[(u64, String)]) {
        let base = self.base;
        self.check(ui, &words.show_mesh, base.show_mesh, |s, v| s.show_mesh = v);
        let sizes = [(MeshSize::Small, &words.small), (MeshSize::Medium, &words.medium), (MeshSize::Large, &words.large)];
        self.choose(ui, &words.mesh_size, base.mesh_size, &sizes, |s, v| s.mesh_size = v);
        self.check(ui, &words.show_mask, base.show_mask, |s, v| s.show_mask = v);
        if self.check(ui, &words.show_backdrop, base.show_backdrop, |s, v| s.show_backdrop = v) == Some(true) {
            self.pass.backdrop_wanted = Some(backdrop_id(base.backdrop));
        }
        if !base.show_backdrop {
            return;
        }
        // A layer that is gone shows All Layers.
        let current = match base.backdrop {
            BackdropUse::Layer(layer) if layers.iter().any(|(known, _)| *known == layer) => BackdropUse::Layer(layer),
            _ => BackdropUse::AllLayers,
        };
        let all = words.all_layers.clone();
        let named: Vec<(BackdropUse, Label)> = std::iter::once((BackdropUse::AllLayers, all))
            .chain(layers.iter().map(|(layer, name)| (BackdropUse::Layer(*layer), Label::from(name.clone()))))
            .collect();
        let uses: Vec<(BackdropUse, &Label)> = named.iter().map(|(value, label)| (*value, label)).collect();
        if let Some(picked) = self.choose(ui, &words.backdrop_use, current, &uses, |s, v| s.backdrop = v) {
            self.pass.backdrop_wanted = Some(backdrop_id(picked));
        }
        let modes = [(BackdropMode::InFront, &words.in_front), (BackdropMode::Behind, &words.behind), (BackdropMode::Blend, &words.blend)];
        self.choose(ui, &words.backdrop_mode, base.backdrop_mode, &modes, |s, v| s.backdrop_mode = v);
        self.slider(ui, &words.opacity, base.backdrop_opacity, 0.0..=100.0, None, |s, v| s.backdrop_opacity = v);
    }

    /// A slider with a number field over the number `value`; its edit is the pass's edit.
    fn slider(&mut self, ui: &mut Ui, label: &Label, value: f64, range: RangeInclusive<f64>, unit: Option<UnitKind>, set: Field<f64>) {
        let mut field = SliderField::new(label.clone(), Bind::given(value), range).step(1.0);
        if let Some(kind) = unit {
            field = field.unit_kind(kind);
        }
        self.pass.edit.extend(lift(field.show(ui).events, self.base, set));
    }

    /// A check box over `value`; the new value when the user toggled it, whose edit is the pass's edit.
    fn check(&mut self, ui: &mut Ui, label: &Label, value: bool, set: Field<bool>) -> Option<bool> {
        let events = Checkbox::new(Bind::given(value)).label(label.clone()).show(ui).events;
        let toggled = events.value().copied();
        self.pass.edit.extend(lift(events, self.base, set));
        toggled
    }

    /// A select over `options` named `label`; the value the user picked this pass, whose edit is the pass's edit.
    fn choose<T: Clone + PartialEq + Send + Sync + 'static>(
        &mut self,
        ui: &mut Ui,
        label: &Label,
        current: T,
        options: &[(T, &Label)],
        set: Field<T>,
    ) -> Option<T> {
        let entries: Vec<Entry<T>> = options.iter().map(|(value, text)| Entry::new(value.clone(), (*text).clone())).collect();
        let events = Select::new(label.clone(), Bind::given(current), &entries).show(ui).events;
        let picked = events.value().cloned();
        self.pass.edit.extend(lift(events, self.base, set));
        picked
    }
}

/// The layer id the app is asked for: `None` for All Layers.
pub(super) fn backdrop_id(of: BackdropUse) -> Option<u64> {
    match of {
        BackdropUse::AllLayers => None,
        BackdropUse::Layer(id) => Some(id),
    }
}
