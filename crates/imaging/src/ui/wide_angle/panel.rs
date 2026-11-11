//! `WideAnglePanel`: Adaptive Wide Angle's settings column.

use std::sync::Arc;

use craft_behavior::{EditEvent, EditEvents, Mixed};
use craft_theme::{ControlSize, TypeRole};
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::form::SliderField;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::sections::Section;
use egui::{Response, Ui};

use super::session::{ConstraintOrientation, WideAngleConstraintEdit, WideAngleModel, WideAngleSession};
use super::words::WideAngleWords;

/// What [`WideAnglePanel::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct WideAnglePanelOutput {
    /// The panel's response.
    pub response: Response,
    /// The edits of the correction values (the lens model and the sliders) this pass, in order. A slider's gesture reports
    /// `Begin`, `Update`, then `Commit` or `Cancel`; the session in each event is the session with that value
    /// changed. The workspace writes it to its session.
    pub edit: EditEvents<WideAngleSession>,
    /// A row's Orientation or Delete; the workspace applies it.
    pub constraint: Option<WideAngleConstraintEdit>,
    /// The Add button was pressed: the workspace opens the Add Constraint form.
    pub add: bool,
}

impl Outcome for WideAnglePanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// One slider row of the correction values.
struct Slider {
    name: fn(&WideAngleWords) -> &Label,
    range: std::ops::RangeInclusive<f64>,
    unit: UnitKind,
    step: f64,
    get: fn(&WideAngleSession) -> f64,
    set: fn(&mut WideAngleSession, f64),
}

/// The settings column (inside a Standard side column), top to bottom: **Correction** (Auto, Fisheye, Perspective,
/// Full Spherical), Scale (50..=150 %), Focal Length (0..=200 mm), Crop Factor (0.1..=10); **Constraints (N)**: one
/// row per constraint with its number, an Orientation select and Delete, and the Add button; Preview (a check box: the
/// corrected preview or the source); the hint in the caption role.
#[derive(craft_ui::contract::Pattern)]
#[must_use = "show the panel with `show`"]
pub struct WideAnglePanel<'a> {
    session: &'a WideAngleSession,
    preview: Bind<'a, bool>,
}

impl<'a> WideAnglePanel<'a> {
    /// The panel over `session` (read; edits are reported) and the Preview check `preview`.
    pub fn new(session: &'a WideAngleSession, preview: Bind<'a, bool>) -> Self {
        Self { session, preview }
    }

    /// Draws the panel and returns its response and any session edit.
    pub fn show(self, ui: &mut Ui) -> WideAnglePanelOutput {
        let Self { session, mut preview } = self;
        let env = Env::of(ui);
        let words = env.words::<WideAngleWords>();
        let (mut edit, mut constraint, mut add) = (EditEvents::new(), None, false);

        let shown = Stack::new().label(words.settings.clone()).show(ui, |ui, _| {
            edit = EditEvents::new();
            Section::fixed(words.correction.clone()).show(ui, |ui, _| {
                let models = [
                    Entry::new(WideAngleModel::Auto, words.auto.clone()),
                    Entry::new(WideAngleModel::Fisheye, words.fisheye.clone()),
                    Entry::new(WideAngleModel::Perspective, words.perspective.clone()),
                    Entry::new(WideAngleModel::FullSpherical, words.full_spherical.clone()),
                ];
                if let Some(&picked) = Select::new(words.correction.clone(), Bind::given(session.model), &models).show(ui).value() {
                    let mut after = session.clone();
                    after.model = picked;
                    edit.push(EditEvent::set(Mixed::Same(session.clone()), after, Arc::from(words.correction.as_str()), None));
                }
                let rows = [
                    Slider { name: |w| &w.scale, range: 50.0..=150.0, unit: UnitKind::Percent, step: 1.0, get: |s| s.scale, set: |s, v| s.scale = v },
                    Slider {
                        name: |w| &w.focal_length,
                        range: 0.0..=200.0,
                        unit: UnitKind::Millimetres,
                        step: 1.0,
                        get: |s| s.focal_length,
                        set: |s, v| s.focal_length = v,
                    },
                    Slider {
                        name: |w| &w.crop_factor,
                        range: 0.1..=10.0,
                        unit: UnitKind::Times,
                        step: 0.1,
                        get: |s| s.crop_factor,
                        set: |s, v| s.crop_factor = v,
                    },
                ];
                for Slider { name, range, unit, step, get, set } in rows {
                    let field = SliderField::new(name(&words).clone(), Bind::given(get(session)), range)
                        .unit_kind(unit)
                        .step(step)
                        .id_salt(name(&words).as_str());
                    edit.extend(field.show(ui).events.map(|value| {
                        let mut shown = session.clone();
                        set(&mut shown, value);
                        shown
                    }));
                }
            });

            let count = session.constraints.len();
            let header = words.constraints.text(count as u64, &env.format().integer(count as u64));
            Section::fixed(Label::from(header)).show(ui, |ui, _| {
                for (index, row) in session.constraints.iter().enumerate() {
                    let number = env.format().integer(index as u64 + 1);
                    Row::new().show(ui, |ui, _| {
                        Text::new(number.clone()).role(TypeRole::Label).shade(Shade::Muted).show(ui);
                        let orientations = [
                            Entry::new(ConstraintOrientation::Free, words.free.clone()),
                            Entry::new(ConstraintOrientation::Horizontal, words.horizontal.clone()),
                            Entry::new(ConstraintOrientation::Vertical, words.vertical.clone()),
                        ];
                        let name = words.orientation_of.fill(&[("n", number.as_str())]);
                        let select =
                            Select::new(name, Bind::given(row.orientation), &orientations).size(ControlSize::Sm).id_salt(("orientation", index));
                        if let Some(&orientation) = select.show(ui).value() {
                            constraint = constraint.take().or(Some(WideAngleConstraintEdit::Orient { index, orientation }));
                        }
                        let delete = Button::new(words.delete.clone())
                            .size(ControlSize::Sm)
                            .description(words.delete_constraint.fill(&[("n", number.as_str())]))
                            .id_salt(("delete", index))
                            .show(ui);
                        if clicked(&delete) {
                            constraint = constraint.take().or(Some(WideAngleConstraintEdit::Delete(index)));
                        }
                    });
                }
                if clicked(&Button::new(words.add_constraint.clone()).size(ControlSize::Sm).id_salt("add").show(ui)) {
                    add = true;
                }
            });
            Checkbox::new(preview.reborrowed()).label(words.preview.clone()).id_salt("preview").show(ui);
            Text::new(words.hint.clone()).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
        });
        WideAnglePanelOutput { response: shown.output, edit, constraint, add }
    }
}
