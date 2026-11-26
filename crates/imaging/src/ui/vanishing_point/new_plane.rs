//! The New Plane form: the four corners of a plane as numbers, the keyboard path of Create Plane and Edit Plane.

use egui::Ui;

use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::runtime::env::{Env, UnitKind};
use craft_ui::text::NumberField;

use super::words::VanishingPointWords;

/// The form's starting square, as a share of the document's shorter side.
const START_SQUARE: f64 = 0.5;

/// The furthest a corner may lie from the document's origin, in pixels.
const REACH: f64 = 1.0e6;

/// The four corners the form edits, in document pixels, in the order Create Plane's clicks come.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct NewPlaneCorners {
    pub corners: [[f64; 2]; 4],
}

impl NewPlaneCorners {
    /// The square at the document's centre.
    fn start(document: [f64; 2]) -> Self {
        let side = document[0].min(document[1]) * START_SQUARE;
        let (x0, y0) = ((document[0] - side) / 2.0, (document[1] - side) / 2.0);
        Self { corners: [[x0, y0], [x0 + side, y0], [x0 + side, y0 + side], [x0, y0 + side]] }
    }
}

/// Shows the form while `open`; the corners when OK was pressed.
pub(super) fn show(ui: &mut Ui, document: [f64; 2], values: Bind<'_, NewPlaneCorners>, open: Bind<'_, bool>) -> Option<[[f64; 2]; 4]> {
    let env = Env::of(ui);
    let words = env.words::<VanishingPointWords>();
    let shown = FormDialog::new(words.new_plane.clone(), values).opened(open).outer_gesture(true).show(ui, |ui, form, values| {
        if form.first() {
            *values = NewPlaneCorners::start(document);
        }
        for (n, corner) in values.corners.iter_mut().enumerate() {
            let caption = words.new_plane_corner.fill(&[("n", (n + 1).to_string().as_str())]);
            Field::new(caption.clone()).show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    for (axis, name) in [(0, &words.new_plane_x), (1, &words.new_plane_y)] {
                        let field = NumberField::new(name.clone(), Bind::given(Some(corner[axis])))
                            .range(-REACH..=REACH)
                            .unit_kind(UnitKind::Pixels)
                            .show(ui);
                        if let Some(typed) = field.value().copied().flatten() {
                            corner[axis] = typed;
                        }
                    }
                });
            });
        }
        true
    });
    shown.accepted.map(|values| values.corners)
}
