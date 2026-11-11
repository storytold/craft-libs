//! [`SelectMaskBar`]: Select and Mask's options bar.

use egui::{Response, Ui};

use std::sync::Arc;

use craft_behavior::{EditEvent, EditEvents, IconId, Mixed};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, NoValue, Outcome, Pattern, Props as _, Show as _, ShowContainer as _, TooltipSpec, clicked};
use craft_ui::form::Field;
use craft_ui::runtime::env::{Env, UnitKind};
use craft_ui::text::NumberField;

use super::tools::{BrushSizes, SelectMaskTool, SelectMaskToolOptions};
use super::words::SelectMaskWords;

/// The most a Brush hardness field takes.
const HARDNESS_MAX: f64 = 100.0;

/// What [`SelectMaskBar::show`] reports.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SelectMaskBarOutput {
    /// The bar's response.
    pub response: Response,
    /// The edit lifecycle of the options the user changed this pass, in order, with the options after it
    /// (already written to the options passed in).
    pub edit: EditEvents<SelectMaskToolOptions>,
    /// Sample All Layers changed: the preview is asked for again.
    pub sample_all_layers: bool,
    /// Select Subject pressed: the workspace records `SelectMaskEdit::Subject`.
    pub select_subject: bool,
}

impl Outcome for SelectMaskBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The options bar, full width under the title (`Bar`): for the editing tools + and − (Add, Subtract); for the brush
/// tools Size (1..=5000 px, a `NumberField`); for the Brush Hardness (0..=100 %); for Quick Selection, Refine Edge
/// Brush and Object Selection Sample All Layers; for the editing tools Select Subject and Refine Hair, disabled with
/// the reason "Refine Hair needs hair detection, which isn't available yet" (lead call 28). The current tool's icon
/// is the strip's, which marks it.
#[derive(Pattern)]
pub struct SelectMaskBar<'a> {
    tool: SelectMaskTool,
    options: &'a mut SelectMaskToolOptions,
}

impl<'a> SelectMaskBar<'a> {
    /// The bar for `tool`, editing `options` in place.
    pub fn new(tool: SelectMaskTool, options: &'a mut SelectMaskToolOptions) -> Self {
        Self { tool, options }
    }

    /// Draws the bar and returns its response and the edits it made.
    pub fn show(self, ui: &mut Ui) -> SelectMaskBarOutput {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:1497 `options_bar`.
        let Self { tool, options } = self;
        let words = Env::of(ui).words::<SelectMaskWords>();
        let mut edit = EditEvents::new();
        let mut sample_all_layers = false;
        let mut select_subject = false;
        let shown = Bar::new(words.name.clone(), BarKind::Options).show(ui, |ui, _| {
            edit = EditEvents::new();
            if tool.edits() {
                for (add, icon, name, salt) in [(true, "plus", &words.add, "add"), (false, "minus", &words.subtract, "subtract")] {
                    let button = IconButton::new(IconId::new(icon)).label(name.clone()).id_salt(salt).selected(options.add == add).show(ui);
                    if clicked(&button) && options.add != add {
                        let before = *options;
                        options.add = add;
                        let label: Arc<str> = Arc::from(name.as_str());
                        edit.push(EditEvent::set(Mixed::Same(before), *options, label, None));
                    }
                }
            }
            if tool.brushes() {
                let size = options.sizes.get(tool);
                let field = Field::new(words.size.clone()).show(ui, |ui, _| {
                    NumberField::new(words.size.clone(), Bind::given(Some(size))).range(BrushSizes::RANGE).unit_kind(UnitKind::Pixels).show(ui)
                });
                for event in field.inner.into_iter().flat_map(|number| number.edit.events) {
                    if let Some(Some(value)) = event.value()
                        && *value != size
                    {
                        options.sizes.set(tool, *value);
                    }
                    edit.push(event.map(|_| *options));
                }
            }
            if tool == SelectMaskTool::Brush {
                let field = Field::new(words.hardness.clone()).show(ui, |ui, _| {
                    NumberField::new(words.hardness.clone(), Bind::given(Some(options.hardness)))
                        .range(0.0..=HARDNESS_MAX)
                        .unit_kind(UnitKind::Percent)
                        .show(ui)
                });
                for event in field.inner.into_iter().flat_map(|number| number.edit.events) {
                    if let Some(Some(value)) = event.value() {
                        options.hardness = value.clamp(0.0, HARDNESS_MAX);
                    }
                    edit.push(event.map(|_| *options));
                }
            }
            if matches!(tool, SelectMaskTool::Quick | SelectMaskTool::RefineEdge | SelectMaskTool::Object) {
                let check = Checkbox::new(Bind::given(options.sample_all_layers)).label(words.sample_all_layers.clone()).show(ui);
                for event in check.events {
                    if let Some(&on) = event.value() {
                        options.sample_all_layers = on;
                    }
                    sample_all_layers = true;
                    edit.push(event.map(|_| *options));
                }
            }
            if tool.edits() {
                select_subject = clicked(&Button::new(words.select_subject.clone()).id_salt("select-subject").show(ui));
                // Hair detection is not available: the button says so.
                Button::new(words.refine_hair.clone())
                    .id_salt("refine-hair")
                    .enabled(false)
                    .tooltip(TooltipSpec::new(words.refine_hair.clone()).disabled_text(words.refine_hair_missing.clone()))
                    .show(ui);
            }
        });
        SelectMaskBarOutput { response: shown.output.response, edit, sample_all_layers, select_subject }
    }
}
