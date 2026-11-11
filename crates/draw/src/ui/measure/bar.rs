//! [`RulerBar`]: the Ruler tool's options as children of the app's options bar: the seven readouts, Use Measurement
//! Scale, Straighten Layer and Clear.

use super::model::RulerLine;
use super::words::RulerWords;
use craft_ui::buttons::Button;
use craft_ui::canvas::CanvasRulers;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, Sizing, clicked};
use craft_ui::display::{Divider, Text};
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::TypeRole;
use egui::{Response, Ui};

/// The digits of a readout.
const DECIMALS: usize = 2;

/// What the Ruler bar reported this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct RulerBarOutput {
    /// The strip's response.
    pub response: Response,
    /// Straighten Layer was clicked: the app runs its command.
    pub straighten_layer: bool,
    /// Clear was clicked: the app clears the line.
    pub clear: bool,
}

impl Outcome for RulerBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Ruler tool's options, shown as children of the app's options bar.
///
/// - Readouts X, Y (the start), W, H (end minus start), A, L1 and L2 (blank without a protractor; all blank without a
///   line), each a caption and a tabular value; live while the line is dragged (the app passes the line of this
///   pass). Values have two decimals. Lengths and positions are written in the canvas's ruler units
///   ([`rulers`](Self::rulers)) while Use Measurement Scale is off; the angle is a plain number of degrees.
/// - Use Measurement Scale multiplies X, Y, W, H, L1 and L2 by the scale factor, not the angle
///   ([`RulerReading::scaled`](super::RulerReading::scaled)); the values are then plain numbers in the scale's own
///   unit, which the bar does not know.
/// - One rule after the readouts, then the check box, Straighten Layer and Clear (both disabled with no line).
/// - **AccessKit:** each readout is a field named by the `*_name` words, valued with its text.
#[must_use = "show the bar's children with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct RulerBar<'a> {
    line: Option<RulerLine>,
    use_scale: Bind<'a, bool>,
    /// The measurement scale's factor (document units to the scale's units).
    #[prop(quantity = "scale factor")]
    factor: f64,
    /// The units of the canvas the line is on, which `CanvasSpace::rulers` gives: the bar sits outside the canvas, so
    /// it cannot read them itself. Default: document pixels.
    #[prop]
    rulers: CanvasRulers,
}

impl<'a> RulerBar<'a> {
    /// The bar for `line`, with the Use Measurement Scale check box editing `use_scale`.
    pub fn new(line: Option<RulerLine>, use_scale: Bind<'a, bool>) -> Self {
        Self { line, use_scale, factor: 1.0, rulers: CanvasRulers::default() }
    }

    /// Shows the bar's children.
    pub fn show(self, ui: &mut Ui) -> RulerBarOutput {
        let Self { line, mut use_scale, factor, rulers } = self;
        let env = Env::of(ui);
        let words = env.words::<RulerWords>();
        let units = env.units();
        let mut scaled = false;
        let mut straighten_layer = false;
        let mut clear = false;
        let shown = Row::new().show(ui, |ui, cx| {
            scaled = use_scale.reborrow().read(cx).is_some_and(|held| matches!(held, craft_behavior::Mixed::Same(true)));
            let reading = line.map(|line| {
                let reading = line.reading();
                if scaled { reading.scaled(factor) } else { reading }
            });
            let length = |axis_x: bool, value: Option<f64>| -> String {
                let Some(value) = value else { return String::new() };
                if scaled {
                    return units.number(value, DECIMALS..=DECIMALS);
                }
                let ruler = if axis_x { &rulers.x } else { &rulers.y };
                ruler.format(&units, value, 0.0, DECIMALS..=DECIMALS)
            };
            let angle = reading.map_or_else(String::new, |r| units.number(r.angle, DECIMALS..=DECIMALS));
            let readouts = [
                ("x", &words.x_name, length(true, reading.map(|r| r.x))),
                ("y", &words.y_name, length(false, reading.map(|r| r.y))),
                ("w", &words.w_name, length(true, reading.map(|r| r.w))),
                ("h", &words.h_name, length(false, reading.map(|r| r.h))),
                ("angle", &words.angle_name, angle),
                ("l1", &words.l1_name, length(true, reading.map(|r| r.l1))),
                ("l2", &words.l2_name, length(true, reading.and_then(|r| r.l2))),
            ];
            for (salt, name, value) in readouts {
                readout(ui, salt, name.clone(), value);
            }
            Divider::vertical().show(ui);
            Checkbox::new(use_scale.reborrowed()).label(words.use_scale.clone()).show(ui);
            let enabled = line.is_some();
            straighten_layer = clicked(&Button::new(words.straighten_layer.clone()).enabled(enabled).show(ui));
            clear = clicked(&Button::new(words.clear.clone()).enabled(enabled).show(ui));
        });
        RulerBarOutput { response: shown.output, straighten_layer, clear }
    }
}

/// One readout: a caption above a tabular value, named `name` for AccessKit.
fn readout(ui: &mut Ui, salt: &'static str, name: Label, value: String) {
    Field::new(name).id_salt(salt).sizing(Sizing::Content).value(value.clone()).show(ui, |ui, _| {
        Text::new(value.clone()).role(TypeRole::Mono).show(ui);
    });
}
