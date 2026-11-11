//! [`SelectMaskPanel`]: Select and Mask's Properties panel.

use std::ops::RangeInclusive;

use egui::{Response, Ui};

use craft_behavior::EditEvents;
use craft_theme::TypeRole;
use craft_theme::variant::Tone;
use craft_ui::buttons::{Button, ToggleButton};
use craft_ui::choice::{Checkbox, Swatch};
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Pattern, Props as _, SaveKey, Show as _, ShowContainer as _, TooltipSpec, clicked};
use craft_ui::display::Text;
use craft_ui::form::{Field, SliderField};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::sections::Section;

use crate::selection::ColorIndicates;
use crate::ui::mask_view::{MaskViewControls, MaskViewMode, MaskViewState};

use super::edits::SelectMaskEdit;
use super::session::{OutputTo, RefineSettings, SelectMaskSettings};
use super::words::SelectMaskWords;

/// The [`ColorRequest::key`] of the Overlay colour swatch; the app answers with
/// `SelectMaskWorkspace::set_color`.
pub const OVERLAY_COLOR_KEY: &str = "selectMask.overlayColor";

/// What [`SelectMaskPanel::show`] reports.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SelectMaskPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// The edit lifecycle of the refinement values the user changed this pass, in order, with the values
    /// after it (already written to the refinement passed in): a slider's `Begin`, `Update` and
    /// `Commit`, a check box's or the Output To choice's `Set`. A refinement change is a session
    /// change: the history settles it and a preview is asked for.
    pub edit: EditEvents<RefineSettings>,
    /// The view or a view option changed (no undo step; a preview is asked for).
    pub view_changed: bool,
    /// Clear Selection or Invert pressed.
    pub selection_edit: Option<SelectMaskEdit>,
    /// The Overlay colour swatch was pressed: the app opens its colour picker with this request
    /// (keyed [`OVERLAY_COLOR_KEY`]) and answers with `SelectMaskWorkspace::set_color`. Once.
    pub pick_color: Option<ColorRequest>,
}

impl Outcome for SelectMaskPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Properties panel in the side column, top to bottom:
///
/// - Remember Settings.
/// - **View Mode**: [`MaskViewControls`] (the view, its opacity, Show Edge, Show Original), then Real-Time
///   Refinement and High Quality Preview; with Overlay, Color (a swatch that asks the app through `ColorRequest`)
///   and Indicates (Masked Areas, Selected Areas).
/// - **Refine Mode**: Color Aware (on) and Object Aware, disabled with the reason "Object Aware needs a segmentation
///   model, which isn't available yet".
/// - **Edge Detection**: Radius (0..=250 px), Smart Radius.
/// - **Global Refinements**: Smooth (0..=100), Feather (0..=1000 px), Contrast (0..=100 %), Shift Edge (−100..=100 %);
///   buttons Clear Selection and Invert.
/// - **Output Settings**: Decontaminate Colors, Amount (0..=100 %, enabled with it), Output To (the targets
///   [`OutputTo::allowed`]).
/// - The error line (the limit message, or the preview's failure) in the bad tone, caption role.
///
/// Each section remembers whether it is open. The footer (Reset at the leading end, OK and Cancel) is the dialog's.
#[derive(Pattern)]
pub struct SelectMaskPanel<'a> {
    settings: &'a mut SelectMaskSettings,
    refine: &'a mut RefineSettings,
    view: &'a mut MaskViewState,
    error: Option<&'a str>,
}

/// Where a control writes its value in the refinement.
type Setter<T> = fn(&mut RefineSettings, T);

/// A control's edit of one refinement field as an edit of the refinement: the refinement at the start of the pass
/// with that field replaced.
fn lift<T: Clone + 'static>(events: EditEvents<T>, base: &RefineSettings, set: Setter<T>) -> EditEvents<RefineSettings> {
    events.map(|value| {
        let mut refine = base.clone();
        set(&mut refine, value);
        refine
    })
}

impl<'a> SelectMaskPanel<'a> {
    /// The panel over `settings`, `refine` and `view`, which it edits in place.
    pub fn new(settings: &'a mut SelectMaskSettings, refine: &'a mut RefineSettings, view: &'a mut MaskViewState) -> Self {
        Self { settings, refine, view, error: None }
    }

    /// The error line under the panel.
    #[must_use]
    pub fn error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
        self
    }

    /// Draws the panel and returns its response and any edits.
    pub fn show(self, ui: &mut Ui) -> SelectMaskPanelOutput {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:1540 `properties`.
        let Self { settings, refine, view, error } = self;
        let words = Env::of(ui).words::<SelectMaskWords>();
        let base = refine.clone();
        let mut rows = Rows { base: &base, edit: EditEvents::new() };
        let (mut view_changed, mut selection_edit, mut pick_color) = (false, None, None);
        let shown = Stack::new().show(ui, |ui, _| {
            rows.edit = EditEvents::new();
            let remember = Checkbox::new(Bind::given(settings.remember)).label(words.remember.clone()).show(ui);
            if let Some(&on) = remember.events.value() {
                settings.remember = on;
            }

            Section::new(words.view_mode.clone()).remember(SaveKey::new("craft_imaging.select_mask.view")).show(ui, |ui, _| {
                view_changed |= MaskViewControls::new(view).show(ui).changed.is_some();
                let real_time = Checkbox::new(Bind::given(settings.real_time)).label(words.real_time.clone()).show(ui);
                if let Some(&on) = real_time.events.value() {
                    settings.real_time = on;
                }
                let high_quality = Checkbox::new(Bind::given(settings.high_quality)).label(words.high_quality.clone()).show(ui);
                if let Some(&on) = high_quality.events.value() {
                    settings.high_quality = on;
                    view_changed = true;
                }
                if view.mode == MaskViewMode::Overlay {
                    Field::new(words.overlay_color.clone()).show(ui, |ui, _| {
                        let swatch = Swatch::new(words.overlay_color.clone(), view.overlay_color).picker(OVERLAY_COLOR_KEY).show(ui);
                        pick_color = swatch.pick_color;
                    });
                    let entries = [
                        Entry::new(ColorIndicates::MaskedAreas, words.masked_areas.clone()),
                        Entry::new(ColorIndicates::SelectedAreas, words.selected_areas.clone()),
                    ];
                    let indicates = Select::new(words.indicates.clone(), Bind::given(view.overlay_indicates), &entries).show(ui);
                    if let Some(&picked) = indicates.events.value()
                        && picked != view.overlay_indicates
                    {
                        view.overlay_indicates = picked;
                        view_changed = true;
                    }
                }
            });

            Section::new(words.refine_mode.clone()).remember(SaveKey::new("craft_imaging.select_mask.refine-mode")).show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    // Color Aware is the one mode there is: pressing it changes nothing.
                    ToggleButton::new(words.color_aware.clone(), Bind::given(true)).id_salt("color-aware").show(ui);
                    ToggleButton::new(words.object_aware.clone(), Bind::given(false))
                        .id_salt("object-aware")
                        .enabled(false)
                        .tooltip(TooltipSpec::new(words.object_aware.clone()).disabled_text(words.object_aware_missing.clone()))
                        .show(ui);
                });
            });

            Section::new(words.edge_detection.clone()).remember(SaveKey::new("craft_imaging.select_mask.edge")).show(ui, |ui, _| {
                rows.slider(ui, Track::new(&words.radius, base.radius, 0.0..=250.0).unit(UnitKind::Pixels), |s, v| s.radius = v);
                rows.check(ui, &words.smart_radius, base.smart_radius, |s, v| s.smart_radius = v);
            });

            Section::new(words.global_refinements.clone()).remember(SaveKey::new("craft_imaging.select_mask.global")).show(ui, |ui, _| {
                rows.slider(ui, Track::new(&words.smooth, base.smooth, 0.0..=100.0), |s, v| s.smooth = v);
                rows.slider(ui, Track::new(&words.feather, base.feather, 0.0..=1000.0).unit(UnitKind::Pixels), |s, v| s.feather = v);
                rows.slider(ui, Track::new(&words.contrast, base.contrast, 0.0..=100.0).unit(UnitKind::Percent), |s, v| s.contrast = v);
                rows.slider(ui, Track::new(&words.shift_edge, base.shift_edge, -100.0..=100.0).unit(UnitKind::Percent), |s, v| s.shift_edge = v);
                Row::new().show(ui, |ui, _| {
                    if clicked(&Button::new(words.clear_selection.clone()).id_salt("clear").show(ui)) {
                        selection_edit = Some(SelectMaskEdit::Clear);
                    }
                    if clicked(&Button::new(words.invert.clone()).id_salt("invert").show(ui)) {
                        selection_edit = Some(SelectMaskEdit::Invert);
                    }
                });
            });

            Section::new(words.output_settings.clone()).remember(SaveKey::new("craft_imaging.select_mask.output")).show(ui, |ui, _| {
                rows.check(ui, &words.decontaminate, base.decontaminate, |s, v| {
                    s.decontaminate = v;
                    s.follow_decontaminate();
                });
                rows.slider(ui, Track::new(&words.amount, base.amount, 0.0..=100.0).unit(UnitKind::Percent).enabled(base.decontaminate), |s, v| {
                    s.amount = v
                });
                let decontaminate = rows.base.decontaminate;
                let targets: Vec<(OutputTo, &Label)> = OutputTo::ALL
                    .into_iter()
                    .filter(|target| target.allowed(decontaminate))
                    .map(|target| (target, output_label(&words, target)))
                    .collect();
                rows.choose(ui, &words.output_to, base.output, &targets, |s, v| s.output = v);
            });

            if let Some(error) = error {
                Text::new(error.to_owned()).role(TypeRole::Caption).tone(Tone::Bad).show(ui);
            }
        });
        let edit = rows.edit;
        if let Some(value) = edit.value() {
            refine.clone_from(value);
        }
        SelectMaskPanelOutput { response: shown.output, edit, view_changed, selection_edit, pick_color }
    }
}

/// One slider row: its label, value, range, unit and whether it is enabled.
struct Track<'a> {
    label: &'a Label,
    value: f64,
    range: RangeInclusive<f64>,
    unit: Option<UnitKind>,
    enabled: bool,
}

impl<'a> Track<'a> {
    fn new(label: &'a Label, value: f64, range: RangeInclusive<f64>) -> Self {
        Track { label, value, range, unit: None, enabled: true }
    }

    fn unit(mut self, unit: UnitKind) -> Self {
        self.unit = Some(unit);
        self
    }

    fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
}

/// The controls of one pass: the refinement they started from and the edit they add up to.
struct Rows<'p> {
    base: &'p RefineSettings,
    edit: EditEvents<RefineSettings>,
}

impl Rows<'_> {
    /// A slider with a number field over `track`; its edit is the pass's edit.
    fn slider(&mut self, ui: &mut Ui, track: Track<'_>, set: Setter<f64>) {
        let mut field = SliderField::new(track.label.clone(), Bind::given(track.value), track.range).step(1.0).enabled(track.enabled);
        if let Some(kind) = track.unit {
            field = field.unit_kind(kind);
        }
        self.edit.extend(lift(field.show(ui).events, self.base, set));
    }

    /// A check box over `value`; its edit is the pass's edit.
    fn check(&mut self, ui: &mut Ui, label: &Label, value: bool, set: Setter<bool>) {
        let events = Checkbox::new(Bind::given(value)).label(label.clone()).show(ui).events;
        self.edit.extend(lift(events, self.base, set));
    }

    /// A select over `options` named `label`; its edit is the pass's edit.
    fn choose<T: Clone + PartialEq + Send + Sync + 'static>(
        &mut self,
        ui: &mut Ui,
        label: &Label,
        current: T,
        options: &[(T, &Label)],
        set: Setter<T>,
    ) {
        let entries: Vec<Entry<T>> = options.iter().map(|(value, text)| Entry::new(value.clone(), (*text).clone())).collect();
        let events = Select::new(label.clone(), Bind::given(current), &entries).show(ui).events;
        self.edit.extend(lift(events, self.base, set));
    }
}

fn output_label(words: &SelectMaskWords, target: OutputTo) -> &Label {
    match target {
        OutputTo::Selection => &words.output_selection,
        OutputTo::LayerMask => &words.output_layer_mask,
        OutputTo::NewLayer => &words.output_new_layer,
        OutputTo::NewLayerWithMask => &words.output_new_layer_with_mask,
        OutputTo::NewDocument => &words.output_new_document,
        OutputTo::NewDocumentWithMask => &words.output_new_document_over_mask,
    }
}
