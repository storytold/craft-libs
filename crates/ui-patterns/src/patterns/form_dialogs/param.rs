//! The parameter model of the form dialogs: [`Param`] and [`ParamKind`] describe one row of a parameter form,
//! [`Param::parse_list`] builds a list from a spec string, and the requests a row can make of its host
//! (`runtime::requests`) are plain values.

mod parse;

use super::fields::{DialogFields, FieldValue};
use crate::runtime::env::{Label, UnitKind};
use crate::runtime::requests::{ColorRequest, PathKind};
use crate::value_drag::SliderScale;
use craft_theme::Srgba;
use egui::{Response, Ui};
use std::fmt;
use std::sync::Arc;

/// What a parameter is and which control edits it. Defaults are the values a form opens with when nothing is
/// remembered and the host gave no values. Build a kind with its constructor (`ParamKind::range`, ...).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ParamKind {
    /// A slider with a number field. [`SliderScale::Log`] for ranges over three decades.
    #[non_exhaustive]
    Range {
        /// Lowest value.
        min: f64,
        /// Highest value.
        max: f64,
        /// The value shown when nothing is remembered.
        default: f64,
        /// How the slider maps position to value.
        scale: SliderScale,
    },
    /// A whole-number field; `None` bounds are unbounded.
    #[non_exhaustive]
    Int {
        /// Lowest value, or `None` for no lower bound.
        min: Option<i64>,
        /// Highest value, or `None` for no upper bound.
        max: Option<i64>,
        /// The value shown when nothing is remembered.
        default: i64,
    },
    /// A number field without a slider. A `None` bound is open on that side. Arrow keys and scrubs move by `step`;
    /// `decimals` is the most digits shown after the decimal mark, so an integer is `decimals: 0`. The field holds a
    /// [`FieldValue::Number`].
    #[non_exhaustive]
    Number {
        /// Lowest value, or `None` for no lower bound.
        min: Option<f64>,
        /// Highest value, or `None` for no upper bound.
        max: Option<f64>,
        /// The value shown when nothing is remembered.
        default: f64,
        /// How far a scrub step or an arrow key moves the value.
        step: f64,
        /// The most digits shown after the decimal mark; 0 for an integer.
        decimals: u32,
    },
    /// A check box.
    #[non_exhaustive]
    Bool {
        /// Whether the box starts checked.
        default: bool,
    },
    /// A select of `(id, label)` options; the field holds the id.
    #[non_exhaustive]
    Choice {
        /// The `(id, label)` pairs in list order.
        options: Vec<(String, Label)>,
        /// The id selected when nothing is remembered; `None` selects the first option.
        default: Option<String>,
    },
    /// A group of radio buttons over `(id, label)` options; the field holds the id.
    #[non_exhaustive]
    Radio {
        /// The `(id, label)` pairs in list order.
        options: Vec<(String, Label)>,
        /// The id selected when nothing is remembered; `None` selects the first option.
        default: Option<String>,
    },
    /// An angle dial with its field, in degrees.
    #[non_exhaustive]
    Angle {
        /// The angle in degrees shown when nothing is remembered.
        default: f64,
    },
    /// A colour the host's picker chooses; `None`: unset until picked.
    #[non_exhaustive]
    Color {
        /// The colour as RGB in 0 to 1; `None` leaves it unset.
        default: Option<[f32; 3]>,
    },
    /// A grid of `cells` integer fields, [`grid_side`](ParamKind::grid_side) per row (`int[25]` is 5×5). A `cells`
    /// that is not square wraps every `round(sqrt(cells))` fields and the last row is short.
    #[non_exhaustive]
    Grid {
        /// The number of fields.
        cells: usize,
        /// The value every cell starts with.
        default: i64,
    },
    /// A point picked over the preview image plus X and Y fields, in percent of the image.
    #[non_exhaustive]
    Point {
        /// `[x, y]` in percent of the image.
        default: [f64; 2],
    },
    /// One of the document's channels by index, shown by name.
    #[non_exhaustive]
    Channel {
        /// The channel names in index order.
        names: Vec<String>,
        /// The index selected when nothing is remembered.
        default: usize,
    },
    /// A text field.
    #[non_exhaustive]
    Text {
        /// The text shown when nothing is remembered.
        default: String,
    },
    /// A number that may be unset: a "Custom" check box switches the field on. Unset the key is absent from the
    /// [`DialogFields`]. `initial` is what the field shows the first time the box is checked; `decimals` is the most
    /// digits shown after the decimal mark (`0` for an integer, as in [`ParamKind::Number`]).
    #[non_exhaustive]
    OptionalNumber {
        /// Lowest value.
        min: f64,
        /// Highest value.
        max: f64,
        /// The value when nothing is remembered; `None` leaves the number unset.
        default: Option<f64>,
        /// The value the field shows the first time the box is checked.
        initial: f64,
        /// The most digits shown after the decimal mark; 0 for an integer.
        decimals: u32,
    },
    /// A row the host draws: see [`Param::custom`]. It has no value of its own.
    Custom(CustomRow),
    /// A section heading over the rows that follow it: see [`Param::section`]. It has no value.
    Section,
}

impl ParamKind {
    /// A slider and field over `min..=max`.
    pub fn range(min: f64, max: f64, default: f64, scale: SliderScale) -> Self {
        ParamKind::Range { min, max, default, scale }
    }

    /// A whole-number field.
    pub fn int(min: Option<i64>, max: Option<i64>, default: i64) -> Self {
        ParamKind::Int { min, max, default }
    }

    /// A number field.
    pub fn number(min: Option<f64>, max: Option<f64>, default: f64, step: f64, decimals: u32) -> Self {
        ParamKind::Number { min, max, default, step, decimals }
    }

    /// A check box.
    pub fn bool(default: bool) -> Self {
        ParamKind::Bool { default }
    }

    /// A select of `(id, label)` options.
    pub fn choice(options: Vec<(String, Label)>, default: Option<String>) -> Self {
        ParamKind::Choice { options, default }
    }

    /// A group of radio buttons over `(id, label)` options.
    pub fn radio(options: Vec<(String, Label)>, default: Option<String>) -> Self {
        ParamKind::Radio { options, default }
    }

    /// An angle dial with its field.
    pub fn angle(default: f64) -> Self {
        ParamKind::Angle { default }
    }

    /// A colour the host's picker chooses.
    pub fn color(default: Option<[f32; 3]>) -> Self {
        ParamKind::Color { default }
    }

    /// A grid of `cells` integer fields.
    pub fn grid(cells: usize, default: i64) -> Self {
        ParamKind::Grid { cells, default }
    }

    /// A point in percent of the image.
    pub fn point(default: [f64; 2]) -> Self {
        ParamKind::Point { default }
    }

    /// A channel by index, shown by name.
    pub fn channel(names: Vec<String>, default: usize) -> Self {
        ParamKind::Channel { names, default }
    }

    /// A text field.
    pub fn text(default: impl Into<String>) -> Self {
        ParamKind::Text { default: default.into() }
    }

    /// A number that a check box switches on.
    pub fn optional_number(min: f64, max: f64, default: Option<f64>, initial: f64, decimals: u32) -> Self {
        ParamKind::OptionalNumber { min, max, default, initial, decimals }
    }

    /// Fields per row of a [`ParamKind::Grid`] of `cells`: `round(sqrt(cells))`, at least 1.
    pub fn grid_side(cells: usize) -> usize {
        ((cells as f32).sqrt().round() as usize).max(1)
    }

    /// The value a field of this kind starts with.
    pub fn default_value(&self) -> Option<FieldValue> {
        Some(match self {
            ParamKind::Range { default, .. } | ParamKind::Angle { default } => FieldValue::Number(*default),
            ParamKind::Int { default, .. } => FieldValue::Int(*default),
            ParamKind::Number { default, .. } => FieldValue::Number(*default),
            ParamKind::Bool { default } => FieldValue::Bool(*default),
            ParamKind::Choice { options, default } | ParamKind::Radio { options, default } => {
                let initial = default.as_deref().or_else(|| options.first().map(|(id, _)| id.as_str()));
                FieldValue::Text(initial.unwrap_or_default().to_owned())
            }
            ParamKind::Color { default } => FieldValue::Color((*default)?),
            ParamKind::Grid { cells: count, default } => {
                let mut cells = vec![*default; *count];
                // A grid that starts at 0 is a kernel: the identity, 1 in the centre.
                if *default == 0
                    && let Some(centre) = cells.get_mut(count / 2)
                {
                    *centre = 1;
                }
                FieldValue::Grid(cells)
            }
            ParamKind::Point { default } => FieldValue::Point(*default),
            ParamKind::Channel { default, .. } => FieldValue::Int(i64::try_from(*default).unwrap_or(0)),
            ParamKind::Text { default } => FieldValue::Text(default.clone()),
            ParamKind::OptionalNumber { default, .. } => FieldValue::Number((*default)?),
            ParamKind::Custom(_) | ParamKind::Section => return None,
        })
    }

    /// Whether `value` is one this kind can hold (a remembered value must, or it is dropped).
    pub fn fits(&self, value: &FieldValue) -> bool {
        let number = |value: &FieldValue| match value {
            FieldValue::Number(n) => Some(*n),
            FieldValue::Int(i) => Some(*i as f64),
            _ => None,
        };
        match (self, value) {
            (ParamKind::Range { min, max, .. }, v) => number(v).is_some_and(|n| n.is_finite() && n >= *min && n <= *max),
            (ParamKind::Angle { .. }, v) => number(v).is_some_and(f64::is_finite),
            (ParamKind::Int { min, max, .. }, FieldValue::Int(i)) => min.is_none_or(|m| *i >= m) && max.is_none_or(|m| *i <= m),
            (ParamKind::Number { min, max, .. }, v) => {
                number(v).is_some_and(|n| n.is_finite() && min.is_none_or(|m| n >= m) && max.is_none_or(|m| n <= m))
            }
            (ParamKind::Bool { .. }, FieldValue::Bool(_)) => true,
            (ParamKind::Choice { options, .. } | ParamKind::Radio { options, .. }, FieldValue::Text(id)) => {
                options.iter().any(|(option, _)| option == id)
            }
            (ParamKind::Color { .. }, FieldValue::Color(c)) => c.iter().all(|v| v.is_finite()),
            (ParamKind::Grid { cells: count, .. }, FieldValue::Grid(cells)) => cells.len() == *count,
            (ParamKind::Point { .. }, FieldValue::Point(p)) => p.iter().all(|v| v.is_finite()),
            (ParamKind::Channel { names, .. }, FieldValue::Int(i)) => usize::try_from(*i).is_ok_and(|i| i < names.len()),
            (ParamKind::Text { .. }, FieldValue::Text(_)) => true,
            (ParamKind::OptionalNumber { min, max, .. }, v) => number(v).is_some_and(|n| n.is_finite() && n >= *min && n <= *max),
            _ => false,
        }
    }
}

type CustomDraw = dyn Fn(&mut Ui, &mut DialogFields, &mut CustomContext) -> Response + Send + Sync;

/// What draws a [`ParamKind::Custom`] row: it gets the row's control cell, the form's values and the row's
/// [`CustomContext`], and returns the response the row label names. Equal only to itself.
#[derive(Clone)]
pub struct CustomRow(Arc<CustomDraw>);

impl CustomRow {
    /// Wraps `draw`, which draws the row and edits the values it owns through the [`DialogFields`].
    pub fn new(draw: impl Fn(&mut Ui, &mut DialogFields, &mut CustomContext) -> Response + Send + Sync + 'static) -> Self {
        Self(Arc::new(draw))
    }

    /// Draws the row.
    pub(crate) fn draw(&self, ui: &mut Ui, fields: &mut DialogFields, context: &mut CustomContext) -> Response {
        (self.0)(ui, fields, context)
    }
}

impl fmt::Debug for CustomRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CustomRow")
    }
}

impl PartialEq for CustomRow {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// What a [`Param::custom`] row can ask of the host besides editing the values.
#[derive(Debug)]
#[non_exhaustive]
pub struct CustomContext {
    title: Label,
    colors: DialogFields,
    request: Option<ColorRequest>,
}

impl CustomContext {
    /// The context of a row labelled `title` over `colors`, the values at the start of the pass.
    pub(crate) fn new(title: Label, colors: DialogFields) -> Self {
        CustomContext { title, colors, request: None }
    }

    /// Asks the host for its colour picker for the value under `key`, which needs no row of its own. The picker opens
    /// at the colour the form holds under `key` (black without one), titled with the row's label. The request is
    /// reported once, as a colour row's press is (`ParamRowsOutput::pick_color`); the host answers by writing the
    /// colour under `key`.
    pub fn pick_color(&mut self, key: &str) {
        let color = Srgba::from_rgb_array(self.colors.color(key).unwrap_or([0.0; 3]));
        self.request = Some(ColorRequest::new(key, color).title(self.title.clone()));
    }

    /// The colour request the row made, once.
    pub(crate) fn take_request(&mut self) -> Option<ColorRequest> {
        self.request.take()
    }
}

/// One unit of a number row with units ([`Param::number_with_units`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NumberUnit {
    /// What the form result carries as the chosen unit (`"ppi"`).
    pub id: String,
    /// The kind of unit the number is read and shown in. The units of one row have different kinds.
    pub kind: UnitKind,
    /// The name in the unit menu (`"pixels/inch"`).
    pub label: Label,
    /// How many base units one of this unit is: the base value is the shown value times `factor` (pixels/cm is `2.54`
    /// over a base of pixels/inch). Must be positive and finite.
    pub factor: f64,
}

impl NumberUnit {
    /// A unit `id` of `kind` named `label` that is `factor` base units.
    pub fn new(id: impl Into<String>, kind: UnitKind, label: impl Into<Label>, factor: f64) -> Self {
        Self { id: id.into(), kind, label: label.into(), factor }
    }
}

/// Options a row may carry beyond its kind.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RowOptions {
    range_decimals: Option<u32>,
    randomize: bool,
    eyedropper: bool,
    eyedropper_armed: bool,
    full_width: bool,
    not_remembered: bool,
    units: Option<UnitChoices>,
}

/// The units of a number row ([`Param::number_with_units`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UnitChoices {
    units: Vec<NumberUnit>,
    /// The field key the chosen unit's id is stored under.
    key: String,
    initial: usize,
}

/// Texts a row may carry beyond its label.
#[derive(Clone, Debug, Default, PartialEq)]
struct RowTexts {
    hint: Option<Label>,
    tooltip: Option<Label>,
}

/// One row of a parameter form.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Param {
    /// The key in [`DialogFields`] and in the command the app runs (`"radius"`).
    pub key: String,
    /// The label without its colon (`"Radius"`).
    pub label: Label,
    /// Which control edits the value, and its default.
    pub kind: ParamKind,
    /// Shown after the number (`px`, `%`, `°`); formats through the locale.
    pub unit: Option<UnitKind>,
    /// The row takes input only while `fields[key]` equals the value.
    pub enabled_when: Option<(String, FieldValue)>,
    /// The row is shown only while `fields[key]` equals the value ([`Param::visible_when`]). A hidden row takes no
    /// space; its value stays in the fields and in the result.
    pub visible_when: Option<(String, FieldValue)>,
    /// A Browse button after a [`ParamKind::Text`] field.
    pub browse: Option<PathKind>,
    /// The hint and tooltip, boxed so rows without them stay small.
    texts: Option<Box<RowTexts>>,
    /// The named options, boxed for the same reason.
    options: Option<Box<RowOptions>>,
}

impl Param {
    /// A row with the field `key`, the `label` and the control `kind`.
    pub fn new(key: impl Into<String>, label: impl Into<Label>, kind: ParamKind) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind,
            unit: None,
            enabled_when: None,
            visible_when: None,
            browse: None,
            texts: None,
            options: None,
        }
    }

    /// A bare number row: a [`ParamKind::Number`] open on both sides. Bound it with [`Param::bounds`].
    pub fn number(key: impl Into<String>, label: impl Into<Label>, default: f64, step: f64, decimals: u32) -> Self {
        Self::new(key, label, ParamKind::number(None, None, default, step, decimals))
    }

    /// Bounds a [`ParamKind::Number`] row (`None`: open on that side); other kinds are unchanged.
    #[must_use]
    pub fn bounds(mut self, lo: Option<f64>, hi: Option<f64>) -> Self {
        if let ParamKind::Number { min, max, .. } = &mut self.kind {
            (*min, *max) = (lo, hi);
        }
        self
    }

    /// A row the host draws. `draw` gets the control cell, the form's values and the row's [`CustomContext`] (to ask
    /// for the host's colour picker); the row label names the response it returns. The row has no default and
    /// nothing remembered; `draw` reads and writes [`DialogFields`] under whatever keys it likes.
    pub fn custom(
        key: impl Into<String>,
        label: impl Into<Label>,
        draw: impl Fn(&mut Ui, &mut DialogFields, &mut CustomContext) -> Response + Send + Sync + 'static,
    ) -> Self {
        Self::new(key, label, ParamKind::Custom(CustomRow::new(draw)))
    }

    /// A section heading: the label as the title of a section whose body is the rows that follow, up to the next
    /// section. It has no value and no control.
    pub fn section(label: impl Into<Label>) -> Self {
        let label = label.into();
        Self::new(format!("section:{}", label.as_str()), label, ParamKind::Section)
    }

    /// A number row with a unit menu after the field. `base` is the value in base units; the field shows
    /// `base / factor` of the chosen unit, and an edit stores `shown * factor`. The first of `units` is chosen at
    /// first (see [`Param::default_unit`]).
    ///
    /// The result holds the base-unit value under the row's key and the chosen unit's [`id`](NumberUnit::id) as text
    /// under [`Param::unit_key`] (`"{key}.unit"`). [`Param::bounds`] are in base units. Both keys are remembered with
    /// the form.
    pub fn number_with_units(key: impl Into<String>, label: impl Into<Label>, base: f64, step: f64, decimals: u32, units: Vec<NumberUnit>) -> Self {
        let key = key.into();
        let mut param = Self::number(key.clone(), label, base, step, decimals);
        param.options.get_or_insert_default().units = Some(UnitChoices { units, key: format!("{key}.unit"), initial: 0 });
        param
    }

    /// The unit of a [`Param::number_with_units`] row that is chosen when the form opens; an id the row does not have
    /// leaves the first.
    #[must_use]
    pub fn default_unit(mut self, id: &str) -> Self {
        if let Some(choices) = self.options.get_or_insert_default().units.as_mut()
            && let Some(at) = choices.units.iter().position(|unit| unit.id == id)
        {
            choices.initial = at;
        }
        self
    }

    /// The key a [`Param::number_with_units`] row stores its chosen unit's id under: `"{key}.unit"`.
    pub fn unit_key(&self) -> Option<&str> {
        self.options.as_deref()?.units.as_ref().map(|units| units.key.as_str())
    }

    /// The row is shown only while `fields[key]` equals `value`, as [`enabled_when`](Param::enabled_when) enables it;
    /// otherwise it takes no space. Its value stays in the fields and in the result.
    #[must_use]
    pub fn visible_when(mut self, key: impl Into<String>, value: impl Into<FieldValue>) -> Self {
        self.visible_when = Some((key.into(), value.into()));
        self
    }

    /// The row takes input only while `fields[key]` equals `value`; otherwise it is shown disabled.
    #[must_use]
    pub fn enabled_when(mut self, key: impl Into<String>, value: impl Into<FieldValue>) -> Self {
        self.enabled_when = Some((key.into(), value.into()));
        self
    }

    /// A Browse button after the field of a [`ParamKind::Text`] row. The press comes back as
    /// `ParamRowsOutput::path`.
    #[must_use]
    pub fn browse(mut self, kind: PathKind) -> Self {
        self.browse = Some(kind);
        self
    }

    /// The text shown while a [`ParamKind::Text`] field is empty; other kinds are unchanged.
    #[must_use]
    pub fn hint(mut self, text: impl Into<Label>) -> Self {
        self.texts.get_or_insert_default().hint = Some(text.into());
        self
    }

    /// Hover text for the whole row, label and control, of any kind.
    #[must_use]
    pub fn tooltip(mut self, text: impl Into<Label>) -> Self {
        self.texts.get_or_insert_default().tooltip = Some(text.into());
        self
    }

    /// The most digits shown after the decimal mark on a [`ParamKind::OptionalNumber`] or [`ParamKind::Range`] row
    /// (`0` for an integer); other kinds are unchanged. A Range row stores its value rounded to these digits, and `0`
    /// stores a [`FieldValue::Int`].
    #[must_use]
    pub fn decimals(mut self, places: u32) -> Self {
        match &mut self.kind {
            ParamKind::OptionalNumber { decimals, .. } => *decimals = places,
            ParamKind::Range { .. } => self.options.get_or_insert_default().range_decimals = Some(places),
            _ => {}
        }
        self
    }

    /// A Random button after the field of a [`ParamKind::Int`] row (a seed): it sets a random value within the row's
    /// bounds (an open side is `0` or `i32::MAX`); other kinds are unchanged.
    #[must_use]
    pub fn randomize(mut self) -> Self {
        self.options.get_or_insert_default().randomize = true;
        self
    }

    /// A pipette toggle after the colour control of a [`ParamKind::Color`] row. A press comes back as
    /// `ParamRowsOutput::eyedropper`, and the host arms or disarms its own dropper from it; other kinds are
    /// unchanged.
    #[must_use]
    pub fn eyedropper(mut self) -> Self {
        self.options.get_or_insert_default().eyedropper = true;
        self
    }

    /// Arms the [`eyedropper`](Param::eyedropper) toggle of a colour row when `armed` becomes true (it opens armed if
    /// it starts true) and disarms it when it becomes false. Only one row is armed at a time. Rows without an
    /// eyedropper are unchanged.
    #[must_use]
    pub fn armed(mut self, armed: bool) -> Self {
        self.options.get_or_insert_default().eyedropper_armed = armed;
        self
    }

    /// For a [`Param::custom`] row: no label, and the body spans the width of the form; other kinds are unchanged.
    #[must_use]
    pub fn full_width(mut self) -> Self {
        self.options.get_or_insert_default().full_width = true;
        self
    }

    /// Leaves the row out of the remembered values: a choice that refers to something in the document (a depth
    /// channel, a menu of open documents) opens from its default every time.
    #[must_use]
    pub fn not_remembered(mut self) -> Self {
        self.options.get_or_insert_default().not_remembered = true;
        self
    }

    /// Sets the unit shown after the number.
    #[must_use]
    pub fn unit(mut self, unit: UnitKind) -> Self {
        self.unit = Some(unit);
        self
    }

    /// Parses the registry notation into a list, labels from the keys in Title Case with two fixed names (`cellSize`
    /// is `Cell Size`, `bpc` is `Black Point Compensation`;
    /// `{"radius":0.1..1000=1,"method":"spin|zoom","monochromatic":bool,"seed":u32=0}`). Apps with their own command
    /// registry build [`Param`]s directly.
    pub fn parse_list(spec: &str) -> Vec<Param> {
        parse::list(spec)
    }

    /// The value the row opens with: its kind's default, stored by the same rule as an edit (a Range row's
    /// [`decimals`](Param::decimals)).
    pub(crate) fn default_value(&self) -> Option<FieldValue> {
        match (&self.kind, self.option().range_decimals) {
            (ParamKind::Range { default, .. }, Some(places)) => Some(rounded(*default, Some(places))),
            _ => self.kind.default_value(),
        }
    }

    /// The row's options, or the defaults.
    pub(crate) fn option(&self) -> RowOptions {
        self.options.as_deref().cloned().unwrap_or_default()
    }

    /// The hint text.
    pub(crate) fn hint_text(&self) -> Option<&Label> {
        self.texts.as_deref()?.hint.as_ref()
    }

    /// The row's tooltip text.
    pub(crate) fn tooltip_text(&self) -> Option<&Label> {
        self.texts.as_deref()?.tooltip.as_ref()
    }

    /// Whether the row's value is saved with the form.
    pub fn is_remembered(&self) -> bool {
        !self.option().not_remembered
    }

    /// Whether `value` is one the field `key` of this row can hold: the row's own value, or the unit id of a row with
    /// units.
    pub fn accepts(&self, key: &str, value: &FieldValue) -> bool {
        if self.key == key {
            return self.kind.fits(value);
        }
        match (self.options.as_deref().and_then(|o| o.units.as_ref()), value) {
            (Some(units), FieldValue::Text(id)) => units.key == key && units.units.iter().any(|u| &u.id == id),
            _ => false,
        }
    }

    /// Whether the row is shown with `fields`.
    pub fn visible(&self, fields: &DialogFields) -> bool {
        self.visible_when.as_ref().is_none_or(|(key, value)| fields.get(key) == Some(value))
    }

    /// Whether the row takes input with `fields`.
    pub fn enabled(&self, fields: &DialogFields) -> bool {
        self.enabled_when.as_ref().is_none_or(|(key, value)| fields.get(key) == Some(value))
    }
}

impl RowOptions {
    /// The decimals a range row stores, or `None` for as typed.
    pub(crate) fn range_places(&self) -> Option<u32> {
        self.range_decimals
    }

    /// Whether an int row has a Random button.
    pub(crate) fn randomize(&self) -> bool {
        self.randomize
    }

    /// Whether a colour row has an eyedropper toggle.
    pub(crate) fn eyedropper(&self) -> bool {
        self.eyedropper
    }

    /// Whether the eyedropper toggle opens armed.
    pub(crate) fn armed(&self) -> bool {
        self.eyedropper_armed
    }

    /// Whether a custom row spans the width of the form.
    pub(crate) fn full_width(&self) -> bool {
        self.full_width
    }

    /// The units of a number row.
    pub(crate) fn units(&self) -> Option<&UnitChoices> {
        self.units.as_ref()
    }
}

impl UnitChoices {
    /// The units in menu order.
    pub(crate) fn list(&self) -> &[NumberUnit] {
        &self.units
    }

    /// The key the chosen unit's id is stored under.
    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    /// The index of the unit chosen when nothing is stored.
    pub(crate) fn initial(&self) -> usize {
        self.initial
    }
}

/// `value` rounded to `places` digits (`None`: as is); no digits stores an integer.
pub(crate) fn rounded(value: f64, places: Option<u32>) -> FieldValue {
    match places {
        None => FieldValue::Number(value),
        Some(0) => FieldValue::Int(value.round() as i64),
        Some(n) => {
            let scale = 10f64.powi(n as i32);
            FieldValue::Number((value * scale).round() / scale)
        }
    }
}

/// An sRGB component in 0..=1 as a byte.
pub fn encode(component: f32) -> u8 {
    (component.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Writes `color` into the colour parameter `key` of `params`, or into a key no parameter has when a
/// [`Param::custom`] row can have asked for it; other keys are ignored.
pub fn set_color_field(params: &[Param], fields: &mut DialogFields, key: &str, color: Srgba) {
    let colour_row = params.iter().any(|param| param.key == key && matches!(param.kind, ParamKind::Color { .. }));
    let custom_request = params.iter().any(|param| matches!(param.kind, ParamKind::Custom(_))) && params.iter().all(|param| param.key != key);
    if colour_row || custom_request {
        fields.set(key, FieldValue::Color(color.to_rgb_array()));
    }
}

/// Fills the keys `params` have defaults for and `fields` lack, and takes the opening snapshot.
pub fn open_fields(params: &[Param], fields: &mut DialogFields) {
    fill_defaults(params, fields);
    fields.snapshot();
}

/// Fills the keys `params` have defaults for and `fields` lack, including the unit key of a row with units.
pub(crate) fn fill_defaults(params: &[Param], fields: &mut DialogFields) {
    for param in params {
        if !fields.contains(&param.key)
            && let Some(value) = param.default_value()
        {
            fields.set(param.key.as_str(), value);
        }
        if let Some(units) = param.options.as_deref().and_then(|o| o.units())
            && !fields.contains(units.key())
            && let Some(unit) = units.list().get(units.initial())
        {
            fields.set(units.key(), FieldValue::Text(unit.id.clone()));
        }
    }
}
