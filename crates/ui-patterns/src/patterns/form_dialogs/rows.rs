//! `ParamRows`: a form with one `Field` per visible parameter of a [`Param`] list, over a [`DialogFields`] value.

use super::fields::{DialogFields, FieldValue};
use crate::runtime::requests::{ColorRequest, EyedropperRequest, PathRequest};

use super::param::{CustomContext, Param, ParamKind, fill_defaults, rounded};
use super::words::ParamWords;
use crate::buttons::{Button, ToggleButton};
use crate::choice::{Checkbox, Radio, RadioGroup};
use crate::contract::{Bind, Entry, NoValue, Outcome, Props as _, Remember as _, Sampler as _, Show as _, ShowContainer as _, clicked};
use crate::form::{Field, FieldGroup, SliderField, SliderFieldLayout};

use crate::layout_components::{Row, Stack};

use crate::pickers::{Select, UnitSelect};

use crate::runtime::env::{Env, Label, UnitKind, Units, with_env};

use crate::sections::Section;
use crate::text::{NumberField, TextField};

use crate::value_drag::{AngleDial, PointPicker, SliderScale};

use craft_behavior::{EditEvent, IconId, Mixed, NumberCodec};

use craft_theme::Srgba;
use egui::{Id, Response, Ui};
use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::sync::Arc;

#[cfg(feature = "testing")]
mod stories;

/// Where a grid or point row's numbers go: whole numbers have no decimals.
const WHOLE: u32 = 0;
/// A point row's numbers run over the whole image, in percent.
const PERCENT: RangeInclusive<f64> = 0.0..=100.0;
/// The most digits an angle shows.
const ANGLE_DECIMALS: u32 = 1;
/// The largest value of a Random button over an int row with no upper bound.
const RANDOM_HIGH: i64 = i32::MAX as i64;
/// The first state of the Random button's sequence when the caller gives no seed.
const SEED_START: u64 = 0x9E37_79B9_7F4A_7C15;

/// What a [`ParamRows`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ParamRowsOutput {
    /// The rows' group.
    pub response: Response,
    /// A row changed the values this pass (each pass of a slider drag, a keystroke that parsed).
    pub changed: bool,
    /// The values after the pass when they differ from the binding's (a row changed one or a default was filled in).
    /// A binding that writes through has them already; a given one applies them.
    pub values: Option<DialogFields>,
    /// The colour parameter whose button was pressed this pass: the host opens its colour picker for the request and
    /// writes the answer into the request's key as a [`FieldValue::Color`].
    pub pick_color: Option<ColorRequest>,
    /// The Browse button pressed this pass: the host picks a path and writes it into the request's key as a
    /// [`FieldValue::Text`].
    pub path: Option<PathRequest>,
    /// The eyedropper toggle turned on this pass: the host answers with the colour the sampler picks for the request's
    /// key. While a toggle is on the rows arm the sampler for `ParamRows::canvas`; turning it off files nothing.
    pub eyedropper: Option<EyedropperRequest>,
}

impl Outcome for ParamRowsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The rows of a parameter list: a group of one labelled field per visible [`Param`], the control chosen by its
/// [`ParamKind`], in a panel the host lays out or inside a dialog.
///
/// - **Values.** They live in the caller's [`DialogFields`] binding; a missing key shows its parameter's default,
///   which the pass writes. The host keeps its own history of the values and its own Reset.
/// - **Rows.** A row is shown while its `visible_when` holds and takes input while its `enabled_when` holds. A
///   [`Param::section`] opens a section whose body is the rows up to the next section.
/// - **Requests.** A colour press, a Browse press and the eyedropper toggle come back in [`ParamRowsOutput`].
/// - **Random.** The Random button of an int row draws from a sequence seeded by `seed`'s binding; without one the
///   sequence starts the same in every run.
/// - **Read only.** With `read_only(true)` every row shows its value and refuses edits, as a read-only text field
///   does: the rows keep focus, selection and their accessible values, and raise no host request.
#[must_use = "show the rows with `show`"]
#[derive(crate::contract::Pattern)]
pub struct ParamRows<'a> {
    label: Label,
    params: &'a [Param],
    fields: Bind<'a, DialogFields>,
    seed: Option<Bind<'a, u64>>,
    read_only: bool,
    canvas: Option<Id>,
}

impl<'a> ParamRows<'a> {
    /// Rows for `params` over `fields`, the group named `label`.
    pub fn new(label: impl Into<Label>, params: &'a [Param], fields: Bind<'a, DialogFields>) -> Self {
        ParamRows { label: label.into(), params, fields, seed: None, read_only: false, canvas: None }
    }

    /// The state of the Random button's sequence: each press reads it and writes the next.
    pub fn seed(mut self, seed: Bind<'a, u64>) -> Self {
        self.seed = Some(seed);
        self
    }

    /// The canvas (the canvas component's id) the eyedropper rows sample while the rows are inside a modal dialog: an
    /// armed eyedropper registers it with `Sampler::arm_sampler`, so primary presses on it pass through the dialog's
    /// modal layer. The dialog body keeps the view usable itself with `Sampler::keep_view_open`.
    pub fn canvas(mut self, canvas: Id) -> Self {
        self.canvas = Some(canvas);
        self
    }

    /// Whether the rows only show their values.
    pub fn read_only(mut self, on: bool) -> Self {
        self.read_only = on;
        self
    }

    /// Shows the rows across the width offered.
    pub fn show(self, ui: &mut Ui) -> ParamRowsOutput {
        let ParamRows { label, params, mut fields, mut seed, read_only, canvas } = self;
        let env = Env::of(ui);
        let words = env.words::<ParamWords>();
        let units = env.units();
        let name: Arc<str> = label.as_str().into();
        let mut pass = Pass::new(&words, &units);
        let mut changed = false;
        let mut values = None;
        let mut group = |ui: &mut Ui| {
            FieldGroup::new(label.clone()).show(ui, |ui, cx| {
                let before = match fields.reborrow().read(cx) {
                    Some(Mixed::Same(held)) => held,
                    _ => DialogFields::new(),
                };
                let mut filled = before.clone();
                fill_defaults(params, &mut filled);
                let state = match seed.as_mut().and_then(|seed| seed.reborrow().read(cx)) {
                    Some(Mixed::Same(state)) => Some(state),
                    _ => None,
                };
                pass.begin(&filled, state, cx.keep::<Memo>().clone());
                pass.rows(ui, params);
                if let Some(canvas) = canvas.filter(|_| pass.sampling && !read_only) {
                    cx.arm_sampler(canvas);
                }
                *cx.keep::<Memo>() = pass.memo.clone();
                changed = pass.work != filled;
                values = (pass.work != before).then(|| pass.work.clone());
                if let Some(after) = &values {
                    let event = EditEvent::set(Mixed::Same(before), after.clone(), name.clone(), None);
                    let _ = fields.reborrow().write(cx, &event);
                }
                if let (Some(bind), Some(next)) = (seed.as_mut(), pass.seed) {
                    let held = Mixed::Same(state.unwrap_or(SEED_START));
                    let _ = bind.reborrow().write(cx, &EditEvent::set(held, next, name.clone(), None));
                }
            })
        };
        let shown = if read_only { with_env(ui, |scope| scope.read_only = Some(true), group) } else { group(ui) };
        let (pick_color, path, eyedropper) = if read_only { Default::default() } else { (pass.color, pass.path, pass.eyedropper) };
        ParamRowsOutput { response: shown.output, changed, values, pick_color, path, eyedropper }
    }
}

/// What the rows keep between passes in the group's session slot: the armed eyedropper row, the last number of each
/// optional row while it is switched off, and the Random sequence when the caller gives no seed.
#[derive(Clone, Default)]
struct Memo {
    /// The one armed eyedropper row: turning a toggle on replaces it, as Photoshop's droppers exclude each other.
    armed: Option<String>,
    /// What each eyedropper row's `Param::armed` said last pass; a change reaches `armed`, so the host can arm and
    /// disarm a row.
    declared: BTreeMap<String, bool>,
    last: BTreeMap<String, f64>,
    sequence: Option<u64>,
}

/// One pass over the rows: the working values, what the rows ask of the host, and what the pass reads.
struct Pass<'p> {
    words: &'p ParamWords,
    units: &'p Units,
    work: DialogFields,
    memo: Memo,
    /// The Random sequence's state after this pass, when a button advanced it.
    seed: Option<u64>,
    /// The state the pass starts the sequence from.
    start: u64,
    color: Option<ColorRequest>,
    path: Option<PathRequest>,
    eyedropper: Option<EyedropperRequest>,
    /// An eyedropper row is armed.
    sampling: bool,
}

/// A parameter as the pass shows it: its key, its label without the colon, and whether it takes input.
struct Line<'r> {
    param: &'r Param,
    key: &'r str,
    name: Label,
    enabled: bool,
}

impl Line<'_> {
    /// Shows `body` in a field named by the row: its label beside the control, its tooltip over both.
    fn show(&self, ui: &mut Ui, mut body: impl FnMut(&mut Ui)) {
        let mut field = Field::new(self.name.clone()).id_salt(self.key).enabled(self.enabled);
        if let Some(tip) = self.param.tooltip_text() {
            field = field.tooltip(tip.clone());
        }
        field.show(ui, |ui, _| body(ui));
    }
}

impl<'p> Pass<'p> {
    fn new(words: &'p ParamWords, units: &'p Units) -> Self {
        Pass {
            words,
            units,
            work: DialogFields::new(),
            memo: Memo::default(),
            seed: None,
            start: SEED_START,
            color: None,
            path: None,
            eyedropper: None,
            sampling: false,
        }
    }

    /// Starts a body run over `work`: the body may run more than once in a pass, and each run starts clean.
    fn begin(&mut self, work: &DialogFields, seed: Option<u64>, memo: Memo) {
        self.work = work.clone();
        self.start = seed.or(memo.sequence).unwrap_or(SEED_START);
        self.seed = None;
        self.memo = memo;
        self.color = None;
        self.path = None;
        self.eyedropper = None;
        self.sampling = false;
    }

    /// The rows of `params`: those before the first section, then each section with the rows up to the next.
    fn rows(&mut self, ui: &mut Ui, params: &[Param]) {
        let is_section = |param: &Param| matches!(param.kind, ParamKind::Section);
        let first = params.iter().position(is_section).unwrap_or(params.len());
        for param in &params[..first] {
            self.row(ui, param);
        }
        let mut rest = &params[first..];
        while let Some((head, tail)) = rest.split_first() {
            let end = tail.iter().position(is_section).unwrap_or(tail.len());
            let (body, after) = tail.split_at(end);
            if head.visible(&self.work) {
                Section::fixed(plain(&head.label)).id_salt(head.key.as_str()).show(ui, |ui, _| {
                    for param in body {
                        self.row(ui, param);
                    }
                });
            }
            rest = after;
        }
    }

    fn row(&mut self, ui: &mut Ui, param: &Param) {
        if !param.visible(&self.work) {
            return;
        }
        let line = Line { param, key: param.key.as_str(), name: plain(&param.label), enabled: param.enabled(&self.work) };
        match &param.kind {
            ParamKind::Range { min, max, scale, .. } => self.range(ui, &line, *min..=*max, *scale),
            ParamKind::Number { .. } if param.option().units().is_some() => self.with_units(ui, &line),
            ParamKind::Number { min, max, step, decimals, .. } => self.number(ui, &line, bounds(*min, *max), *step, *decimals),
            ParamKind::Int { min, max, .. } => self.int(ui, &line, *min, *max),
            ParamKind::Bool { default } => self.check(ui, &line, *default),
            ParamKind::Choice { options, .. } => self.choice(ui, &line, options),
            ParamKind::Radio { options, .. } => self.radio_options(ui, &line, options),
            ParamKind::Angle { default } => self.angle(ui, &line, *default),
            ParamKind::Color { .. } => self.color(ui, &line),
            ParamKind::Grid { cells, .. } => self.grid(ui, &line, *cells),
            ParamKind::Point { default } => self.point(ui, &line, *default),
            ParamKind::Channel { names, default } => self.channel(ui, &line, names, *default),
            ParamKind::Text { default } => self.text(ui, &line, default),
            ParamKind::OptionalNumber { min, max, initial, decimals, .. } => self.optional(ui, &line, *min..=*max, *initial, *decimals),
            ParamKind::Custom(_) => self.custom(ui, &line),
            ParamKind::Section => {}
        }
    }

    fn range(&mut self, ui: &mut Ui, line: &Line<'_>, range: RangeInclusive<f64>, scale: SliderScale) {
        let value = self.work.number(line.key).unwrap_or(*range.start());
        let places = line.param.option().range_places();
        line.show(ui, |ui| {
            let mut slider = SliderField::new(line.name.clone(), Bind::given(value), range.clone()).scale(scale).layout(SliderFieldLayout::Inline);
            if let Some(places) = places {
                slider = slider.step(10f64.powi(-(places as i32)));
            }
            if let Some(kind) = line.param.unit {
                slider = slider.unit_kind(kind);
            }
            if let Some(v) = slider.show(ui).value().cloned() {
                self.work.set(line.key, rounded(v, places));
            }
        });
    }

    fn number(&mut self, ui: &mut Ui, line: &Line<'_>, range: Option<RangeInclusive<f64>>, step: f64, decimals: u32) {
        let value = self.work.number(line.key).unwrap_or(0.0);
        line.show(ui, |ui| {
            let mut field = number_field(self.units, line.name.clone(), Some(value), decimals, line.param.unit).step(step);
            if let Some(range) = range.clone() {
                field = field.range(range);
            }
            if let Some(v) = field.show(ui).edit.value().cloned().flatten() {
                self.work.set(line.key, FieldValue::Number(v));
            }
        });
    }

    /// A number row with a unit menu: the field shows the base value in the chosen unit.
    fn with_units(&mut self, ui: &mut Ui, line: &Line<'_>) {
        let ParamKind::Number { min, max, .. } = &line.param.kind else { return };
        let option = line.param.option();
        let Some(choices) = option.units() else { return };
        let base = self.work.number(line.key).unwrap_or(0.0);
        let at = self.work.text(choices.key()).and_then(|id| choices.list().iter().position(|unit| unit.id == id)).unwrap_or(choices.initial());
        let Some(chosen) = choices.list().get(at) else { return };
        let factor = chosen.factor;
        let range = bounds(min.map(|m| m / factor), max.map(|m| m / factor));
        let entries: Vec<Entry<UnitKind>> = choices.list().iter().map(|unit| Entry::new(unit.kind, unit.label.clone())).collect();
        let unit_name = self.words.unit.clone();
        line.show(ui, |ui| {
            let mut field = UnitSelect::new(line.name.clone(), Bind::given(Some(base / factor)), Bind::given(chosen.kind), &entries)
                .unit_label(unit_name.clone())
                .id_salt("field");
            if let Some(range) = range.clone() {
                field = field.range(range);
            }
            let shown = field.show(ui);
            if let Some(v) = shown.edit.value().cloned().flatten() {
                self.work.set(line.key, FieldValue::Number(v * factor));
            }
            let picked = shown.unit.value().copied();
            if let Some(unit) = picked.and_then(|kind| choices.list().iter().find(|unit| unit.kind == kind)) {
                self.work.set(choices.key(), FieldValue::Text(unit.id.clone()));
            }
        });
    }

    fn int(&mut self, ui: &mut Ui, line: &Line<'_>, min: Option<i64>, max: Option<i64>) {
        let value = self.work.int(line.key).unwrap_or(0);
        let range = bounds(min.map(|m| m as f64), max.map(|m| m as f64));
        let random = line.param.option().randomize();
        line.show(ui, |ui| {
            beside(ui, |ui| {
                let mut field = number_field(self.units, line.name.clone(), Some(value as f64), WHOLE, line.param.unit).id_salt("field");
                if let Some(range) = range.clone() {
                    field = field.range(range);
                }
                if let Some(v) = field.show(ui).edit.value().cloned().flatten() {
                    self.work.set(line.key, FieldValue::Int(v.round() as i64));
                }
                if random && clicked(&Button::new(self.words.randomize.clone()).id_salt("random").show(ui)) {
                    let mut state = self.seed.unwrap_or(self.start);
                    let drawn = random_in(&mut state, min.unwrap_or(0), max.unwrap_or(RANDOM_HIGH));
                    self.seed = Some(state);
                    self.memo.sequence = Some(state);
                    self.work.set(line.key, FieldValue::Int(drawn));
                }
            });
        });
    }

    fn check(&mut self, ui: &mut Ui, line: &Line<'_>, default: bool) {
        let value = self.work.bool(line.key).unwrap_or(default);
        let mut check = Checkbox::new(Bind::given(value)).label(line.name.clone()).id_salt(line.key).enabled(line.enabled);
        if let Some(tip) = line.param.tooltip_text() {
            check = check.tooltip(tip.clone());
        }
        if let Some(v) = check.show(ui).value().cloned() {
            self.work.set(line.key, FieldValue::Bool(v));
        }
    }

    fn choice(&mut self, ui: &mut Ui, line: &Line<'_>, options: &[(String, Label)]) {
        let value = self.work.text(line.key).map(str::to_owned).or_else(|| line.param.kind.default_value().and_then(text_of)).unwrap_or_default();
        let entries: Vec<Entry<String>> = options.iter().map(|(id, label)| Entry::new(id.clone(), label.clone())).collect();
        line.show(ui, |ui| {
            if let Some(v) = Select::new(line.name.clone(), Bind::given(value.clone()), &entries).show(ui).value().cloned() {
                self.work.set(line.key, FieldValue::Text(v));
            }
        });
    }

    fn radio_options(&mut self, ui: &mut Ui, line: &Line<'_>, options: &[(String, Label)]) {
        let value = self.work.text(line.key).map(str::to_owned).or_else(|| line.param.kind.default_value().and_then(text_of)).unwrap_or_default();
        line.show(ui, |ui| {
            let group = RadioGroup::new(line.name.clone(), Bind::given(value.clone())).enabled(line.enabled).show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    for (id, label) in options {
                        Radio::new(id.clone()).label(label.clone()).show(ui);
                    }
                });
            });
            if let Some(v) = group.output.value().cloned() {
                self.work.set(line.key, FieldValue::Text(v));
            }
        });
    }

    fn angle(&mut self, ui: &mut Ui, line: &Line<'_>, default: f64) {
        let value = self.work.number(line.key).unwrap_or(default);
        let field_name = Label::from(self.words.angle_field.fill(&[("name", line.name.as_str())]));
        line.show(ui, |ui| {
            beside(ui, |ui| {
                let dial = AngleDial::new(Bind::given(value)).label(line.name.clone()).id_salt("dial").show(ui);
                let field = number_field(self.units, field_name.clone(), Some(value), ANGLE_DECIMALS, Some(UnitKind::Degrees)).id_salt("field");
                let typed = field.show(ui).edit.value().cloned().flatten();
                if let Some(v) = dial.value().cloned().or(typed) {
                    self.work.set(line.key, FieldValue::Number(v));
                }
            });
        });
    }

    /// A colour row: a button that asks the host's picker, and the eyedropper toggle when the row has one.
    fn color(&mut self, ui: &mut Ui, line: &Line<'_>) {
        let color = self.work.color(line.key);
        let option = line.param.option();
        if option.eyedropper() && self.memo.declared.insert(line.key.to_owned(), option.armed()) != Some(option.armed()) {
            if option.armed() {
                self.memo.armed = Some(line.key.to_owned());
            } else if self.memo.armed.as_deref() == Some(line.key) {
                self.memo.armed = None;
            }
        }
        let armed = self.memo.armed.as_deref() == Some(line.key);
        let shown = color.map_or_else(|| self.words.color_unset.clone(), |rgb| Label::from(Srgba::from_rgb_array(rgb).to_hex()));
        line.show(ui, |ui| {
            beside(ui, |ui| {
                if clicked(&Button::new(shown.clone()).id_salt("choose").show(ui)) {
                    let current = Srgba::from_rgb_array(color.unwrap_or([0.0; 3]));
                    self.color = Some(ColorRequest::new(line.key, current).title(line.name.clone()));
                }
                if option.eyedropper() {
                    let toggle = ToggleButton::new(self.words.eyedropper.clone(), Bind::given(armed))
                        .icon(IconId::new("eyedropper"))
                        .icon_only()
                        .id_salt("eyedropper")
                        .show(ui);
                    if let Some(on) = toggle.value().cloned() {
                        if on {
                            self.memo.armed = Some(line.key.to_owned());
                            self.eyedropper = Some(EyedropperRequest::new(line.key));
                        } else if self.memo.armed.as_deref() == Some(line.key) {
                            self.memo.armed = None;
                        }
                    }
                    self.sampling |= self.memo.armed.as_deref() == Some(line.key);
                }
            });
        });
    }

    fn grid(&mut self, ui: &mut Ui, line: &Line<'_>, count: usize) {
        let side = ParamKind::grid_side(count);
        let cells = match self.work.grid(line.key) {
            Some(cells) if cells.len() == count => cells.to_vec(),
            _ => vec![0; count],
        };
        let mut next = cells.clone();
        line.show(ui, |ui| {
            Stack::new().show(ui, |ui, _| {
                for row in 0..count.div_ceil(side) {
                    Row::new().id_salt(row).show(ui, |ui, _| {
                        for column in 0..side.min(count - row * side) {
                            let at = row * side + column;
                            let name = self.words.grid_cell.fill(&[
                                ("name", line.name.as_str()),
                                ("row", (row + 1).to_string().as_str()),
                                ("column", (column + 1).to_string().as_str()),
                            ]);
                            let cell = number_field(self.units, Label::from(name), Some(cells[at] as f64), WHOLE, None).id_salt(at);
                            if let Some(v) = cell.show(ui).edit.value().cloned().flatten() {
                                next[at] = v.round() as i64;
                            }
                        }
                    });
                }
            });
        });
        if next != cells {
            self.work.set(line.key, FieldValue::Grid(next));
        }
    }

    fn point(&mut self, ui: &mut Ui, line: &Line<'_>, default: [f64; 2]) {
        let point = self.work.point(line.key).unwrap_or(default);
        let names = [
            Label::from(self.words.point_x.fill(&[("name", line.name.as_str())])),
            Label::from(self.words.point_y.fill(&[("name", line.name.as_str())])),
        ];
        line.show(ui, |ui| {
            beside(ui, |ui| {
                let shown = [Mixed::Same(point[0]), Mixed::Same(point[1])];
                let picker = PointPicker::new(Bind::given(shown), PERCENT, PERCENT).label(line.name.clone()).id_salt("point").show(ui);
                let mut next = match picker.value().cloned() {
                    Some([Mixed::Same(x), Mixed::Same(y)]) => [x, y],
                    _ => point,
                };
                for (axis, name) in names.iter().enumerate() {
                    let field = number_field(self.units, name.clone(), Some(point[axis]), 1, Some(UnitKind::Percent)).range(PERCENT).id_salt(axis);
                    if let Some(v) = field.show(ui).edit.value().cloned().flatten() {
                        next[axis] = v;
                    }
                }
                if next != point {
                    self.work.set(line.key, FieldValue::Point(next));
                }
            });
        });
    }

    fn channel(&mut self, ui: &mut Ui, line: &Line<'_>, names: &[String], default: usize) {
        let value = self.work.int(line.key).and_then(|i| usize::try_from(i).ok()).unwrap_or(default);
        let entries: Vec<Entry<usize>> = names.iter().enumerate().map(|(i, name)| Entry::new(i, Label::from(name.clone()))).collect();
        line.show(ui, |ui| {
            if let Some(v) = Select::new(line.name.clone(), Bind::given(value), &entries).show(ui).value().cloned() {
                self.work.set(line.key, FieldValue::Int(i64::try_from(v).unwrap_or(0)));
            }
        });
    }

    fn text(&mut self, ui: &mut Ui, line: &Line<'_>, default: &str) {
        let value = self.work.text(line.key).unwrap_or(default).to_owned();
        let browse = line.param.browse.clone();
        line.show(ui, |ui| {
            beside(ui, |ui| {
                let mut field = TextField::new(line.name.clone(), Bind::given(value.clone())).id_salt("text");
                if let Some(hint) = line.param.hint_text() {
                    field = field.placeholder(hint.clone());
                }
                if let Some(v) = field.show(ui).value().cloned() {
                    self.work.set(line.key, FieldValue::Text(v));
                }
                if let Some(kind) = &browse
                    && clicked(&Button::new(self.words.browse.clone()).id_salt("browse").show(ui))
                {
                    self.path = Some(PathRequest::new(line.key, kind.clone()));
                }
            });
        });
    }

    /// A number a check box switches on; switched off, the key leaves the values and the number is kept for the
    /// next time the box is checked.
    fn optional(&mut self, ui: &mut Ui, line: &Line<'_>, range: RangeInclusive<f64>, initial: f64, decimals: u32) {
        let stored = self.work.number(line.key);
        let last = stored.or_else(|| self.memo.last.get(line.key).copied()).unwrap_or(initial);
        line.show(ui, |ui| {
            beside(ui, |ui| {
                let check = Checkbox::new(Bind::given(stored.is_some())).label(self.words.custom.clone()).id_salt("custom").show(ui);
                match check.value().cloned() {
                    Some(true) => self.work.set(line.key, FieldValue::Number(last)),
                    Some(false) => {
                        self.memo.last.insert(line.key.to_owned(), last);
                        self.work.remove(line.key);
                    }
                    None => {}
                }
                let field = number_field(self.units, line.name.clone(), Some(last), decimals, line.param.unit).range(range.clone());
                let field = field.id_salt("field").enabled(self.work.number(line.key).is_some());
                if let Some(v) = field.show(ui).edit.value().cloned().flatten() {
                    self.work.set(line.key, FieldValue::Number(v));
                }
            });
        });
    }

    fn custom(&mut self, ui: &mut Ui, line: &Line<'_>) {
        let ParamKind::Custom(row) = &line.param.kind else { return };
        let mut context = CustomContext::new(line.name.clone(), self.work.clone());
        if line.param.option().full_width() {
            Row::new().id_salt(line.key).enabled(line.enabled).fill().show(ui, |ui, _| {
                let _ = row.draw(ui, &mut self.work, &mut context);
            });
        } else {
            line.show(ui, |ui| {
                let _ = row.draw(ui, &mut self.work, &mut context);
            });
        }
        if let Some(request) = context.take_request() {
            self.color = Some(request);
        }
    }
}

/// Shows `body` as one row of controls side by side.
fn beside(ui: &mut Ui, mut body: impl FnMut(&mut Ui)) {
    Row::new().show(ui, |ui, _| body(ui));
}

/// A number field named `name` that reads and writes `decimals` digits and the spellings of `unit`.
fn number_field(units: &Units, name: Label, value: Option<f64>, decimals: u32, unit: Option<UnitKind>) -> NumberField<'static> {
    NumberField::new(name, Bind::given(value)).codec(NumberCodec::new(units.reader(unit), decimals as usize))
}

/// The range that has the bounds that are set, open on the other side; `None` when neither is.
fn bounds(min: Option<f64>, max: Option<f64>) -> Option<RangeInclusive<f64>> {
    (min.is_some() || max.is_some()).then(|| min.unwrap_or(f64::NEG_INFINITY)..=max.unwrap_or(f64::INFINITY))
}

fn text_of(value: FieldValue) -> Option<String> {
    match value {
        FieldValue::Text(text) => Some(text),
        _ => None,
    }
}

/// A row's label without a trailing colon: the form puts its own beside the control.
fn plain(label: &Label) -> Label {
    match label {
        Label::Word(_) | Label::Empty => label.clone(),
        _ => {
            let text = label.as_str();
            let trimmed = text.trim_end_matches([':', '：']);
            if trimmed.len() == text.len() { label.clone() } else { Label::from(trimmed.to_owned()) }
        }
    }
}

/// A pseudo-random integer in `lo..=hi` from the sequence at `state` (SplitMix64), which advances; a seed needs no
/// better source.
fn random_in(state: &mut u64, lo: i64, hi: i64) -> i64 {
    *state = state.wrapping_add(SEED_START);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    let span = (i128::from(hi) - i128::from(lo) + 1) as u128;
    (i128::from(lo) + (u128::from(z) % span) as i128) as i64
}
