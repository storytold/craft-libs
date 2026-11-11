//! [`FillBar`]: Content-Aware Fill's options bar.

use egui::{Response, Ui};

use craft_behavior::IconId;
use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::choice::Segmented;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Pattern, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::Divider;
use craft_ui::patterns::form_dialogs::ParamDialogWords;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, UnitKind};
use craft_ui::text::NumberField;

use super::lasso::FillAreaEdit;
use super::tool::{FillTool, FillToolOptions};
use super::words::FillWords;

/// What [`FillBar::show`] reports.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FillBarOutput {
    /// The bar's response.
    pub response: Response,
    /// Expand or Contract pressed: the edit by the bar's amount.
    pub edit: Option<FillAreaEdit>,
    /// The Lasso's Reset pressed: the fill-area edits go.
    pub reset_fill_area: bool,
}

impl Outcome for FillBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The options bar across the top, per tool:
///
/// - **Sampling Brush**: Add and Subtract (a segmented choice of icons) and Size (px, 1..=5000).
/// - **Lasso**: the four selection operations (New, Add, Subtract, Intersect), Expand and Contract with
///   their amount (px, 1..=100) and a Reset icon button for the fill area.
/// - **Hand**, **Zoom**: no options.
#[derive(Pattern)]
pub struct FillBar<'a> {
    tool: FillTool,
    options: &'a mut FillToolOptions,
}

impl<'a> FillBar<'a> {
    /// The bar of `tool` over `options`, which it edits in place.
    pub fn new(tool: FillTool, options: &'a mut FillToolOptions) -> Self {
        Self { tool, options }
    }

    /// Draws the bar and returns what the user asked for.
    pub fn show(self, ui: &mut Ui) -> FillBarOutput {
        // pc: crates/ui-egui/src/dialogs/caf_ui/view.rs:62-69 (the buttons and the field), controls.rs:72-75.
        let Self { tool, options } = self;
        let env = Env::of(ui);
        let words = env.words::<FillWords>();
        let reset = env.words::<ParamDialogWords>().reset.clone();
        let mut edit = None;
        let mut reset_fill_area = false;
        let shown = Bar::new(words.options.clone(), BarKind::Options).show(ui, |ui, _| match tool {
            FillTool::SamplingBrush => {
                let modes = [
                    Entry::new(true, words.add.clone()).icon(IconSpec::svg(IconId::new("plus"))),
                    Entry::new(false, words.subtract.clone()).icon(IconSpec::svg(IconId::new("minus"))),
                ];
                if let Some(add) = Segmented::new(words.add.clone(), Bind::given(options.brush_add), &modes).show(ui).value().copied() {
                    options.brush_add = add;
                }
                Divider::vertical().show(ui);
                if let Some(size) = number(ui, &words.size, options.brush_size, 1.0..=FillToolOptions::SIZE_MAX) {
                    options.brush_size = size;
                }
            }
            FillTool::Lasso => {
                let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
                let operations = [
                    Entry::new(SelectionOperation::New, words.new_selection.clone()).icon(icon("square")),
                    Entry::new(SelectionOperation::Add, words.add_to_selection.clone()).icon(icon("plus")),
                    Entry::new(SelectionOperation::Subtract, words.subtract_from_selection.clone()).icon(icon("minus")),
                    Entry::new(SelectionOperation::Intersect, words.intersect_with_selection.clone()).icon(icon("squares_subtract")),
                ];
                let picked = Segmented::new(words.options.clone(), Bind::given(options.lasso), &operations).show(ui).value().copied();
                if let Some(operation) = picked {
                    options.lasso = operation;
                }
                Divider::vertical().show(ui);
                for (expand, name) in [(true, &words.expand), (false, &words.contract)] {
                    if clicked(&Button::new(name.clone()).id_salt(if expand { "expand" } else { "contract" }).show(ui)) {
                        edit = Some(FillAreaEdit::grow(expand, options.grow_by));
                    }
                }
                let (min, max) = FillAreaEdit::GROW_RANGE;
                if let Some(by) = number(ui, &words.grow_by, options.grow_by, min..=max) {
                    options.grow_by = by;
                }
                if clicked(&IconButton::new(icon("undo")).label(reset.clone()).id_salt("reset").show(ui)) {
                    reset_fill_area = true;
                }
            }
            FillTool::Hand | FillTool::Zoom => {}
        });
        FillBarOutput { response: shown.output.response, edit, reset_fill_area }
    }
}

/// A whole-pixel number field over `value`; the value the user committed this pass, kept inside `range`.
fn number(ui: &mut Ui, label: &craft_ui::runtime::env::Label, value: f64, range: std::ops::RangeInclusive<f64>) -> Option<f64> {
    let (min, max) = (*range.start(), *range.end());
    let field = NumberField::new(label.clone(), Bind::given(Some(value))).range(range).unit_kind(UnitKind::Pixels).commit_decimals(0).show(ui);
    field.edit.events.value().copied().flatten().map(|typed| typed.clamp(min, max))
}
