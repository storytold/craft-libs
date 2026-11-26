//! [`BoxConstraintsPopover`]: the Rectangle tool's gear.

use std::sync::Arc;

use super::ShapeWords;
use super::panel::{NumberCell, glyph_side};
use crate::ui::shape_drag::{BoxConstraint, BoxConstraints};
use craft_behavior::{EditEvent, EditEvents, IconId, Mixed};
use craft_ui::buttons::IconButton;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, LabelScheme, PopupKind, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::popover::Popover;
use craft_ui::runtime::env::{Env, Label, Units};
use craft_ui::text::RangeAlert;
use egui::{Response, Ui};

/// The Rectangle tool's gear: a button that opens the box constraints of a drag. The popover holds the radios
/// Unconstrained, Square, Fixed Size and Proportional, each size's W and H beside its radio and enabled only while it
/// is chosen, and the From Center check box. Fixed Size takes whole pixels (0 to 300,000), Proportional up to three
/// decimals (0 to 99,999); an emptied field is `None`, which draws as Unconstrained, and a value typed past its range
/// is clamped and reported once in [`BoxConstraintsOutput::out_of_range`]. Escape closes it and keeps the values. Its
/// `open` flag is read and written through the binding, so give it `Bind::bound`.
#[must_use = "show the popover with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct BoxConstraintsPopover<'a> {
    value: Bind<'a, BoxConstraints>,
    open: Bind<'a, bool>,
    pixels_per_inch: f64,
}

/// What [`BoxConstraintsPopover::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct BoxConstraintsOutput {
    /// The gear button's response.
    pub response: Response,
    /// The edits of this pass in order, each the whole value with one field replaced.
    pub edits: Vec<EditEvent<BoxConstraints>>,
    /// A size typed past its field's range this pass, which the field clamped: the app answers with its range alert.
    pub out_of_range: Option<RangeAlert>,
}

/// The most decimals a Proportional field shows.
const RATIO_DECIMALS: usize = 3;

impl<'a> BoxConstraintsPopover<'a> {
    /// The gear of `value`, open while `open` is set, in a document of `pixels_per_inch`, which sizes a Fixed Size typed
    /// in inches or points.
    pub fn new(value: Bind<'a, BoxConstraints>, open: Bind<'a, bool>, pixels_per_inch: f64) -> Self {
        BoxConstraintsPopover { value, open, pixels_per_inch }
    }

    /// Shows the gear button and, while open, the popover.
    pub fn show(self, ui: &mut Ui) -> BoxConstraintsOutput {
        let BoxConstraintsPopover { mut value, mut open, pixels_per_inch } = self;
        let env = Env::of(ui);
        let (words, units) = (env.words::<ShapeWords>(), env.units());
        let current = match value.reborrow().get() {
            Some(Mixed::Same(shown)) => shown,
            _ => BoxConstraints::default(),
        };
        let was = matches!(open.reborrow().get(), Some(Mixed::Same(true)));
        let gear = IconButton::new(IconId::new("settings"))
            .label(words.shape_options.clone())
            .tooltip(words.shape_options.clone())
            .popup(PopupKind::Dialog)
            .expanded(was)
            .show(ui)
            .response;
        if clicked(&gear) {
            // The popover's opener is manual: the gear's press is the pattern's to apply.
            let event = EditEvent::set(Mixed::Same(was), !was, Arc::from(words.shape_options.as_str()), None);
            let _ = open.reborrow().apply(&event);
        }
        let mut body = Body { words: &words, units: &units, pixels_per_inch, value: current, edits: Vec::new(), out_of_range: None };
        let _ = Popover::new(words.shape_options.clone(), open.reborrowed()).opened_by(&gear).show(ui, |ui, _| {
            FieldGroup::new(words.shape_options.clone()).labels(LabelScheme::Inspector).show(ui, |ui, _| body.show(ui));
        });
        BoxConstraintsOutput { response: gear, edits: body.edits, out_of_range: body.out_of_range }
    }
}

/// The popover's rows over one value, and what they report in one run.
struct Body<'p> {
    words: &'p ShapeWords,
    units: &'p Units,
    pixels_per_inch: f64,
    value: BoxConstraints,
    edits: Vec<EditEvent<BoxConstraints>>,
    out_of_range: Option<RangeAlert>,
}

impl Body<'_> {
    fn show(&mut self, ui: &mut Ui) {
        // The body may run more than once in a pass; each run starts clean.
        self.edits.clear();
        self.out_of_range = None;
        let (words, units, ppi, value) = (self.words, self.units, self.pixels_per_inch, self.value);
        let chosen = value.constraint;
        let fixed = [
            NumberCell::whole_pixels(units, ppi, "fixed-width", &words.fixed_width, value.fixed.0, BoxConstraints::FIXED),
            NumberCell::whole_pixels(units, ppi, "fixed-height", &words.fixed_height, value.fixed.1, BoxConstraints::FIXED),
        ];
        let ratio = [
            NumberCell::plain(units, None, RATIO_DECIMALS, "ratio-width", &words.ratio_width, value.ratio.0, BoxConstraints::RATIO),
            NumberCell::plain(units, None, RATIO_DECIMALS, "ratio-height", &words.ratio_height, value.ratio.1, BoxConstraints::RATIO),
        ];
        let (mut fixed_edits, mut ratio_edits) = (Pair::default(), Pair::default());
        let group = RadioGroup::new(words.shape_options.clone(), Bind::given(chosen)).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                Radio::new(BoxConstraint::Unconstrained).label(words.unconstrained.clone()).show(ui);
                Radio::new(BoxConstraint::Square).label(words.square.clone()).show(ui);
                Row::new().show(ui, |ui, _| {
                    Radio::new(BoxConstraint::FixedSize).label(words.fixed_size.clone()).show(ui);
                    fixed_edits = pair(ui, words, &fixed, chosen == BoxConstraint::FixedSize);
                });
                Row::new().show(ui, |ui, _| {
                    Radio::new(BoxConstraint::Proportional).label(words.proportional.clone()).show(ui);
                    ratio_edits = pair(ui, words, &ratio, chosen == BoxConstraint::Proportional);
                });
            });
        });
        self.edits.extend(group.output.events.map(|constraint| BoxConstraints { constraint, ..value }));
        let [width, height] = fixed_edits.edits;
        self.edits.extend(width.map(|width| BoxConstraints { fixed: (width, value.fixed.1), ..value }));
        self.edits.extend(height.map(|height| BoxConstraints { fixed: (value.fixed.0, height), ..value }));
        let [width, height] = ratio_edits.edits;
        self.edits.extend(width.map(|width| BoxConstraints { ratio: (width, value.ratio.1), ..value }));
        self.edits.extend(height.map(|height| BoxConstraints { ratio: (value.ratio.0, height), ..value }));
        let centre = Checkbox::new(Bind::given(value.from_center)).label(words.from_center.clone()).show(ui).events;
        self.edits.extend(centre.map(|from_center| BoxConstraints { from_center, ..value }));
        self.out_of_range = fixed_edits.alerts.into_iter().chain(ratio_edits.alerts).flatten().last();
    }
}

/// The edits and range alerts of a W and H pair.
#[derive(Default)]
struct Pair {
    edits: [EditEvents<Option<f64>>; 2],
    alerts: [Option<RangeAlert>; 2],
}

/// The W and H fields of `cells`, each under its short glyph and enabled while `on`.
fn pair(ui: &mut Ui, words: &ShapeWords, cells: &[NumberCell; 2], on: bool) -> Pair {
    let mut out = Pair::default();
    let glyphs: [&Label; 2] = [&words.width, &words.height];
    for (index, (cell, glyph)) in cells.iter().zip(glyphs).enumerate() {
        let shown = Field::new(cell.caption.clone()).id_salt(cell.key).side(glyph_side(glyph)).enabled(on).show(ui, |ui, _| cell.show_optional(ui));
        (out.edits[index], out.alerts[index]) = shown.inner.unwrap_or_default();
    }
    out
}
