//! `NumberField`: a text part over [`NumberCodec`] with a range, a step, a unit, presets and a popup slider.
//!
//! Typing, expressions, unit spellings and locale digits are the codec's (`quantity::Reader`); the field adds the
//! range every written value stays in and the step that arrows, the wheel and a scrub move by. A drag on a field that
//! is not in edit scrubs it; a click enters edit, where a drag selects text.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::adorn::{Adornment, Placed, place, reserve};
use super::words::TextWords;
use super::{FieldStyle, text_part};
use crate::buttons::IconButton;
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, Entry, Focus as _, Input, MeasureCx, Numeric, OverlayUi, Overlays as _, PaintUi,
    PartsCx, Pointer as _, Props as _, Role, Scrub, Sizing, Styled as _, WordSetCx as _,
};
use crate::primitives::{TextOptions, TextRun};
use crate::runtime::env::{EnvView, Label, LengthUnits, UnitKind, Units};
use crate::runtime::frame;
use crate::style::{ComponentStyle, StyleColor, StyleCx, SurfaceLook, TextLook, TypeRole};
use craft_behavior::nav::{LARGE_STEPS, StepResult, ValueAxes, ValueSteps};
use craft_behavior::{
    Codec, ColumnKey, Granularity, HitLevel, HitPriority, HoldRepeat, Metrics, MoveMods, NavMove, NumberCodec, PartKey, PressPhase, Sense, ValueDrag,
    decimals_for_step, wheel_step,
};
use craft_theme::{Rgba, Stateful, States};
use egui::{CornerRadius, Rect, Vec2};
use std::borrow::Cow;
use std::marker::PhantomData;
use std::ops::RangeInclusive;

mod popups;
mod rule;
mod value;

pub use rule::NumberStepper;
use rule::{Rule, Ruled, key_scale, round_to};
pub use value::{NamedOr, NumberName, NumberOutput, NumberPick, NumberValue, RangeAlert};

/// The step of an arrow press, the wheel and a scrub when the field sets none.
const DEFAULT_STEP: f64 = 1.0;

/// The columns content-sized fields share in a column group (a `FieldGroup`): the widest value and the widest unit,
/// so their values and units line up across the group's fields.
const VALUES: ColumnKey = ColumnKey::new("number_values");
const UNITS: ColumnKey = ColumnKey::new("number_units");

/// Where a number field shows its unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnitPlacement {
    /// Inside the field, after the number.
    #[default]
    Inside,
    /// After the field, outside its box: the field draws no unit and keeps the unit's spellings for typed text and its
    /// decimals, and the host draws the unit text beside it (a column of units in a grid of fields).
    After,
}

/// Whether a number field shows step arrows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum StepArrows {
    /// No arrows.
    #[default]
    Hidden,
    /// Up and down halves at the start edge. A press steps once as ArrowUp or ArrowDown, one edit step each; a held
    /// press repeats after the hold delay at the hold interval; disabled or read-only arrows take no press.
    Leading,
}

/// How a number field sets its value and unit. An app sets it once at its root (`RootEnv::values`); the default is
/// the centred number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::runtime::env::EnvValue, crate::contract::MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum NumberLook {
    /// The number centred in the mono face, the unit apart at the end of the value area in caption text, so a column
    /// of fields lines its units up; a percentage stands against its sign at the end.
    #[default]
    Centred,
    /// The number and its unit as one run ("720 pt", "0°", "100%") at the field's start, in the interface face and
    /// the value's colour: editing shows the same text, and the caret and the selection go through the whole run.
    Inline,
}

/// How many decimals a number field without a codec, fixed or maximum decimals of its own shows. An app sets it once
/// at its root (`RootEnv::values`); the default is the standard precision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::runtime::env::EnvValue, crate::contract::MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum NumberPrecision {
    /// A percentage reads one decimal, written in full ("100.0%"); other numbers their unit kind's decimals.
    #[default]
    Standard,
    /// A percentage reads whole ("60%"), and a length field to a thousandth of points, pixels, picas, Q and Ha and a
    /// ten-thousandth of the larger units, without trailing zeros ("357.114 pt", "8.2677 in"): a drawing app's
    /// positions and sizes.
    Measured,
}

/// The placeholder of a mixed field.
const BLANK: &str = "\u{2014}";

/// The decimals a field writes when neither a codec nor a unit kind says.
const DEFAULT_DECIMALS: usize = 3;
/// How far a value may sit off a multiple of the step and still count as on it.
const ON_GRID: f64 = 1e-9;

/// Whether the value is a number that is not finite; the field shows the dash for it, as for a mixed value.
fn non_finite<V: NumberValue>(value: Option<&V>) -> bool {
    value.is_some_and(|v| v.named().is_none() && v.number().is_some_and(|number| !number.is_finite()))
}

/// Whether the host sized the field to its content (`Sizing::Content`).
fn content_sized<C: Component>(cx: &MeasureCx<'_, C>) -> bool {
    cx.resolved.sizing() == Some(Sizing::Content)
}

/// The chevron that opens the presets list.
const PRESETS: PartKey = PartKey::new("presets");
/// The chevron that opens the popup slider.
const SLIDER: PartKey = PartKey::new("slider");

/// The style of a number field: the field look, which the chevrons are laid out by, and the unit's text.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct NumberStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    /// The unit shown after the number.
    #[style(text)]
    unit: TextLook,
    /// The field look the adornments are placed by; `surface` and `metrics` are its own.
    field: FieldStyle,
    /// The colour of the inside label's glyph, a scrub handle.
    handle: Stateful<StyleColor>,
    /// The width of one number cell at the control's size (`size.length.number`), padding included.
    cell: f32,
}

/// A number field: a text part over [`NumberCodec`]; empty text is `None`.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(overlay, probe(NumberField<'static, Option<f64>>))]
pub struct NumberField<'a, V: NumberValue + 'static = Option<f64>> {
    #[common]
    common: Common,
    value: Bind<'a, V>,
    /// The codec, to read another locale's separators or units, or to read words such as "Auto" (`or_parse`).
    /// Without one the field reads and writes with the env's locale and the spellings of `unit_kind`.
    #[prop]
    codec: Option<NumberCodec>,
    /// What the number counts: typed text may carry this kind's unit spellings ("12 mm") in the user's language or
    /// English, and the field shows at most the kind's decimals.
    #[prop]
    unit_kind: Option<UnitKind>,
    /// The range typed, stepped and scrubbed values stay in.
    #[prop]
    range: Option<RangeInclusive<f64>>,
    /// What one arrow press, one wheel notch and one point of scrub change unless overridden. The default is 1.
    #[prop(quantity = "step")]
    step: Option<f64>,
    /// The change per point of scrub, in shown units. An omitted, non-positive or non-finite rate uses the field step.
    #[prop(quantity = "step")]
    scrub_step: Option<f64>,
    /// Where an arrow press, a Page key and a wheel notch go from the current value, for values on a ladder (the zoom
    /// ladder); the step is ignored for them. The range still holds the result.
    #[prop(func = "Fn(f64, i32, f64) -> f64")]
    stepper: Option<NumberStepper<'a>>,
    /// A unit shown after the number.
    #[prop(into)]
    unit: Option<Label>,
    /// Whether the unit is drawn inside the field or left to the host to draw after it.
    #[prop]
    unit_placement: UnitPlacement,
    /// Up and down step halves at the start edge: a press steps once, a held press repeats; no Tab stop.
    #[prop]
    step_arrows: StepArrows,
    #[style_key]
    status_zoom: bool,
    /// The rows of the list a chevron after the field opens; Alt+Down opens it too.
    #[prop]
    presets: Option<&'a [Entry<NumberPick>]>,
    /// A chevron after the field opens a slider over `range` below it; pressing the chevron and dragging moves the
    /// value at once, as far as the slider would, and the release closes the slider.
    #[prop]
    popup_slider: Option<RangeInclusive<f64>>,
    /// Whether the popup slider snaps to the configured step; defaults to true.
    #[prop]
    snap_popup_slider: bool,
    /// A typed value goes to the nearest multiple of the step when it commits.
    #[prop]
    snap_typed: bool,
    /// The decimals a committed value is rounded to; the display keeps the codec's, except that a percent field with no
    /// decimals of its own shows none when this is 0 ("10%").
    #[prop]
    commit_decimals: Option<usize>,
    /// An empty field means automatic: it shows the "Auto" word as its placeholder.
    #[prop]
    automatic: bool,
    /// A mixed field shows the word "Mixed" as its placeholder, not the dash (a size field over a selection of
    /// several sizes).
    #[prop]
    mixed_word: bool,
    /// The value is derived, not chosen (an automatic leading that follows the font size). While the field is not in
    /// edit it shows the value in parentheses, with its unit inside them ("(87.6 pt)"), and that text is its
    /// accessible value; the field draws no separate unit then, as the unit is in the text. Focus starts an edit from
    /// the plain number, without parentheses or unit. A committed edit reports the value as usual: it is the host's
    /// job to turn this off, as the value is no longer derived. Steppers, presets and the popup slider work as usual.
    /// A blank, mixed or named value is shown as it would be without this.
    #[prop]
    derived: bool,
    /// The field writes exactly this many decimals, zeros included ("1.00"), when it has no codec of its own.
    #[prop]
    fixed_decimals: Option<usize>,
    /// The number the held name stands for: while the value is a name, the field shows this number in parentheses in
    /// place of the name's label; the presets row keeps the label.
    #[prop(quantity = "field value")]
    #[allow(dead_code)]
    // PLAN: `Ruled::format`'s named arm (rule.rs, appearance-w3) must show this number; the field passes it on once `Ruled` carries it
    name_number: Option<f64>,
    /// The most decimals shown without trailing zeroes when the field has no codec or fixed precision, capped at 12.
    #[prop]
    max_decimals: Option<usize>,
    /// The lengths a length field keeps and shows (`NumberField::length`).
    lengths: Option<LengthUnits>,
    /// How many shown units make one stored unit, read from the env in measure and parts; 1 for a plain field.
    scale: f64,
    /// The scope's look, read from the env in measure and parts.
    look: NumberLook,
    /// The scope's precision, read from the env in measure and parts.
    precision: NumberPrecision,
    options: TextOptions,
    positive_only: bool,
    scrub: bool,
    /// The popup slider's track length, read from the tokens in parts: a drag on its chevron moves the value as far
    /// as a drag on the slider would.
    track: f32,
    picked: Option<NumberPick>,
    /// The inside label's glyph before the text (`EnvView::inside_label`), measured in measure and parts for paint.
    leading: Vec<Adornment>,
    /// The unit and the chevrons after the text, measured in measure and parts for paint.
    trailing: Vec<Adornment>,
    /// Width of the current value in this pass, including its caret.
    value_width: f32,
}

impl<'a> NumberField<'a, Option<f64>> {
    /// The up half of the step arrows ([`step_arrows`](Self::step_arrows)): a press steps the value up once.
    pub const STEP_UP: PartKey = PartKey::new("step_up");
    /// The down half of the step arrows: a press steps the value down once.
    pub const STEP_DOWN: PartKey = PartKey::new("step_down");

    /// A number field named `label`; empty text is `None`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Option<f64>>) -> Self {
        Self::of(label, value)
    }

    /// A number field over a length the binding keeps in `lengths.stored` (the document's unit) and shows in
    /// `lengths.shown` (the user's unit), after the shown unit's symbol. Typed text reads any pixel or length unit
    /// spelling ("10 mm" in a field that shows inches) and a bare number in the shown unit; the binding, its check
    /// and its edit events are in the stored unit. `range` is in the stored unit and `popup_slider` and presets
    /// numbers too; the step is in the shown unit and defaults to the shown unit's own.
    pub fn length(label: impl Into<Label>, value: Bind<'a, Option<f64>>, lengths: LengthUnits) -> Self {
        Self::of(label, value).lengths(lengths)
    }
}

impl<'a, V: NumberValue + 'static> NumberField<'a, V> {
    /// A number field named `label` over a value of another type, such as [`NamedOr`].
    pub fn named(label: impl Into<Label>, value: Bind<'a, V>) -> Self {
        Self::of(label, value)
    }

    /// The lengths the field keeps and shows, as [`NumberField::length`] gives them: a named-number field reads and
    /// shows lengths through the one units path.
    #[must_use]
    pub fn lengths(mut self, lengths: LengthUnits) -> Self {
        self.lengths = Some(lengths);
        self.unit_kind = Some(lengths.shown);
        self.step = Some(lengths.shown.step());
        self
    }

    fn of(label: impl Into<Label>, value: Bind<'a, V>) -> Self {
        NumberField {
            common: Common::new(),
            value,
            codec: None,
            unit_kind: None,
            range: None,
            step: None,
            scrub_step: None,
            stepper: None,
            unit: None,
            unit_placement: UnitPlacement::Inside,
            step_arrows: StepArrows::Hidden,
            status_zoom: false,
            presets: None,
            popup_slider: None,
            snap_popup_slider: true,
            snap_typed: false,
            automatic: false,
            mixed_word: false,
            derived: false,
            commit_decimals: None,
            fixed_decimals: None,
            name_number: None,
            max_decimals: None,
            lengths: None,
            scale: 1.0,
            look: NumberLook::Centred,
            precision: NumberPrecision::Standard,
            options: TextOptions::default(),
            positive_only: false,
            scrub: false,
            track: 0.0,
            picked: None,
            leading: Vec::new(),
            trailing: Vec::new(),
            value_width: 0.0,
        }
        .label(label)
    }

    /// The caller hangs a context menu of its own on the field (`Menu::context` over its response, a unit menu): a
    /// right click opens that menu instead of the field's edit menu.
    #[must_use]
    pub fn own_context_menu(mut self) -> Self {
        self.options.own_menu = true;
        self
    }

    /// Enter acts in the field itself (it runs a search): the field takes it, and an enclosing dialog's default button
    /// does not. `EditResponse::submitted` and `submit_mods` tell the submitting Enter, as for
    /// [`TextField::submits`](crate::text::TextField::submits).
    #[must_use]
    pub fn submits(mut self) -> Self {
        self.options.submits = true;
        self
    }

    /// The whole value is selected when the field takes focus, so typing replaces it, as a value field in a photo
    /// editor's sliders behaves. Focus by key or request selects it at once; a pointer press keeps the caret or range
    /// it makes, and a plain click selects the whole value once it is released, as an address bar does.
    #[must_use]
    pub fn select_on_focus(mut self) -> Self {
        self.options.select_on_focus = true;
        self
    }

    /// Identifies the target of this field's edit, for a control reused across drawings or selections.
    /// Changing the key discards the previous draft even when the two bound values are equal.
    /// The field keeps its identity and focus.
    #[must_use]
    pub fn edit_target(mut self, key: impl egui::AsIdSalt) -> Self {
        self.options.edit_target = Some(egui::IdSalt::new(key));
        self
    }

    /// Zoom typing: a Tab rewrites the draft (with no draft it moves focus) and a press on the field's other parts
    /// keeps it unapplied.
    #[must_use]
    pub(super) fn zoom_typing(mut self) -> Self {
        self.options.tab = crate::primitives::TabKey::Rewrites;
        self.options.holds_drafts = true;
        self.positive_only = true;
        self
    }

    /// Keeps the entire zoom percentage together in an unframed well in the status bar.
    #[must_use]
    pub(super) fn status_zoom(mut self) -> Self {
        self.status_zoom = true;
        self.sizing(Sizing::Content)
    }

    /// A drag on the field's side label scrubs the value too, with or without the focus in the field.
    #[must_use]
    pub fn scrub_by_label(mut self) -> Self {
        self.scrub = true;
        self
    }

    /// The range and step in the shown unit.
    fn rule(&self) -> Rule {
        let range = self.range.as_ref().map(|range| range.start() * self.scale..=range.end() * self.scale);
        Rule::new(range.as_ref(), self.step)
    }

    /// Reads the scope's look and precision and a length field's scale from stored to shown from the env. The status bar's zoom
    /// well keeps its own look.
    fn settle(&mut self, env: &EnvView<'_>) {
        self.look = if self.status_zoom { NumberLook::Centred } else { env.get() };
        self.precision = env.get();
        if let Some(lengths) = self.lengths {
            self.scale = env.units().length_scale(lengths).unwrap_or(1.0);
        }
    }

    /// Whether the field draws its unit inside the box, not the host after the field.
    fn unit_inside(&self) -> bool {
        !self.status_zoom && self.unit_placement == UnitPlacement::Inside
    }

    /// The unit the field shows with its number: the field's own, else the symbol of its unit kind; none when the host
    /// draws it after the field.
    fn unit_label(&self, units: &Units) -> Option<Label> {
        if !self.unit_inside() {
            return None;
        }
        self.unit.clone().or_else(|| self.unit_kind.map(|kind| units.symbol(kind).into()))
    }

    /// The unit drawn apart inside the field after the number: [`Self::unit_label`], except where the number's run holds
    /// it (the inline look) and for a derived value, whose unit is in its parenthesised text.
    fn shown_unit(&self, units: &Units, value: Option<&V>) -> Option<Label> {
        if self.look == NumberLook::Inline || self.derives(value) { None } else { self.unit_label(units) }
    }

    /// Whether `value` is shown as a derived value: the field is `derived` and the value is a number it shows, not a
    /// blank, a mixed value or a name.
    fn derives(&self, value: Option<&V>) -> bool {
        self.derived && !self.shows_blank(value) && value.is_some_and(|value| value.named().is_none() && value.number().is_some())
    }

    /// `number`, as the field writes it, as a derived value: in parentheses with its unit. The inline look's number
    /// already carries its unit.
    fn derived_text(&self, number: &str, units: &Units) -> String {
        match self.unit_label(units).filter(|_| self.look != NumberLook::Inline) {
            Some(unit) => format!("({number} {})", unit.as_str()),
            None => format!("({number})"),
        }
    }

    /// The text the number's run holds around the number: the status zoom's sign; in the inline look the unit kind's
    /// unit word, else the field's own unit, against the number when it is one sign ("%", "°") and after a space
    /// otherwise.
    fn affixes(&self, env: &EnvView<'_>) -> Option<(String, String)> {
        if self.status_zoom {
            return Some((String::new(), env.units().symbol(UnitKind::Percent)));
        }
        if !self.unit_inside() || self.look != NumberLook::Inline {
            return None;
        }
        match (&self.unit, self.unit_kind) {
            (Some(unit), _) => {
                let unit = env.resolve(unit);
                let mut chars = unit.as_str().chars();
                let sign = chars.next().is_some_and(|c| !c.is_alphanumeric()) && chars.next().is_none();
                Some((String::new(), if sign { unit.as_str().to_owned() } else { format!(" {}", unit.as_str()) }))
            }
            (None, Some(kind)) => Some(env.units().affixes(kind)),
            (None, None) => None,
        }
    }

    /// The face the number is set in: the mono face, or the interface's label face in the inline look.
    fn value_role(&self) -> TypeRole {
        match self.look {
            NumberLook::Centred => TypeRole::Mono,
            NumberLook::Inline => TypeRole::Label,
        }
    }

    /// The unit drawn this pass. A mixed value shows the dash alone: a unit beside it would claim a measure the
    /// selection does not have.
    fn painted_unit(&self, cx: &Cx<'_, Self>) -> Option<Label> {
        self.shown_unit(&cx.env.units(), cx.value()).filter(|_| cx.value().is_some_and(|value| !non_finite(Some(value))))
    }

    /// `stored` in the shown unit.
    fn to_shown(&self, stored: f64) -> f64 {
        stored * self.scale
    }

    /// `shown` in the stored unit.
    fn to_stored(&self, shown: f64) -> f64 {
        shown / self.scale
    }

    /// `stored` kept in the range.
    fn settle_stored(&self, stored: f64) -> f64 {
        self.to_stored(self.rule().settle(self.to_shown(stored)))
    }

    /// The number the field holds, in the shown unit.
    fn shown_number(&self, value: Option<&V>) -> Option<f64> {
        value.and_then(NumberValue::number).map(|stored| self.to_shown(stored))
    }

    /// Whether the field counts percent: it shows its sign right after the number.
    fn is_percent(&self) -> bool {
        self.codec.is_none() && self.unit_kind == Some(UnitKind::Percent)
    }

    /// The decimals a percent field writes in full when the caller sets none of its own: one at the standard precision
    /// ("100.0%"), none at the measured precision ("100%") or for a field whose committed values carry none
    /// (`commit_decimals(0)`).
    fn percent_decimals(&self) -> Option<usize> {
        (self.is_percent() && self.fixed_decimals.is_none() && self.max_decimals.is_none()).then(|| match self.precision {
            NumberPrecision::Standard => self.commit_decimals.unwrap_or(1).min(1),
            NumberPrecision::Measured => 0,
        })
    }

    /// Whether the field uses the dash for a missing or invalid value.
    fn shows_blank(&self, shown: Option<&V>) -> bool {
        shown.is_none() || non_finite(shown) || (self.status_zoom && shown.and_then(NumberValue::number).is_none())
    }

    /// The field's codec: the one set, else the env's locale and the spellings of the unit kind (a length field
    /// reads every length spelling).
    fn number_codec(&self, units: &Units) -> NumberCodec {
        self.codec.clone().unwrap_or_else(|| {
            let reader = match self.lengths {
                Some(lengths) => units.length_reader(lengths),
                None => units.reader(self.unit_kind),
            };
            let codec = NumberCodec::new(reader, self.decimals());
            if self.fixed_decimals.is_some() || self.percent_decimals().is_some() { codec.fixed() } else { codec }
        })
    }

    /// The decimals the field shows: its codec's, fixed or maximum precision, a percent field's own, else its unit
    /// kind's.
    fn shown_decimals(&self) -> RangeInclusive<usize> {
        if let Some(percent) = self.percent_decimals() {
            return percent..=percent;
        }
        match (&self.codec, self.fixed_decimals, self.max_decimals, self.unit_kind) {
            (Some(codec), _, _, _) => 0..=codec.decimals(),
            (None, Some(fixed), _, _) => fixed..=fixed,
            (None, None, Some(max), _) => 0..=max.min(12),
            (None, None, None, Some(kind)) => match (self.precision, self.lengths) {
                (NumberPrecision::Measured, Some(_)) => {
                    0..=if matches!(kind, UnitKind::Points | UnitKind::Pixels | UnitKind::Picas | UnitKind::Q | UnitKind::Ha) { 3 } else { 4 }
                }
                _ => kind.decimals(),
            },
            (None, None, None, None) => 0..=DEFAULT_DECIMALS,
        }
    }

    /// The most decimals the field writes.
    fn decimals(&self) -> usize {
        *self.shown_decimals().end()
    }

    /// The names the presets list holds that a value of this type can be: the labels of its `Choice` rows.
    fn names(&self, label: impl Fn(&Entry<NumberPick>) -> Label) -> Vec<(u32, String)> {
        let rows = self.presets.unwrap_or_default();
        rows.iter()
            .filter_map(|row| match row.value() {
                Some(NumberPick::Choice(id)) if V::from_name(*id).is_some() => Some((*id, label(row).as_str().to_owned())),
                _ => None,
            })
            .collect()
    }

    fn ruled(&self, env: &EnvView<'_>, mixed: bool, names: Vec<(u32, String)>) -> Ruled<V> {
        Ruled {
            inner: self.number_codec(&env.units()),
            rule: self.rule(),
            snap: self.snap_typed,
            decimals: self.commit_decimals,
            mixed,
            positive_only: self.positive_only,
            names,
            affixes: self.affixes(env),
            scale: self.scale,
            value: PhantomData,
        }
    }

    /// The chevrons after the text, in order, with their parts and accessible names.
    fn chevrons(&self, words: &TextWords) -> Vec<(PartKey, Label)> {
        let mut chevrons = Vec::new();
        if self.presets.is_some() {
            chevrons.push((PRESETS, words.presets.clone()));
        }
        if self.popup_slider.is_some() {
            chevrons.push((SLIDER, words.slider.clone()));
        }
        chevrons
    }

    /// The unit, measured, then the chevrons: what sits after the text. The unit stands apart at the end of the value
    /// area, so units line up in a column of fields and the number centres in the room before its unit. A
    /// content-sized field's unit is as wide as the widest of its column group's.
    fn after_text(&self, cx: &MeasureCx<'_, Self>, chevrons: &[(PartKey, Label)]) -> Vec<Adornment> {
        let shared = content_sized(cx);
        // A mixed field that reads "Mixed" has no number for a unit to follow: the word takes the room.
        let worded = self.mixed_word && cx.value().is_none();
        let unit = self.shown_unit(&cx.env.units(), cx.value()).filter(|_| !worded).map(|unit| {
            let unit = Adornment::text(unit, cx.style.unit.role).apart().measured(cx);
            if shared { unit.in_column(cx.ui.ctx(), &UNITS) } else { unit }
        });
        let arrow = crate::controls::disclosure::DisclosureGlyph::dropdown_of(cx.env.get());
        let chevrons = chevrons.iter().map(|(key, name)| Adornment::button(*key, arrow.clone(), name.clone()));
        unit.into_iter().chain(chevrons).collect()
    }

    /// The step strip, then the inside label's glyph, measured: what sits before the text. The glyph stays last, next
    /// to the text.
    fn before_text(cx: &MeasureCx<'_, Self>, arrows: StepArrows) -> Vec<Adornment> {
        let mut before = Vec::new();
        if arrows == StepArrows::Leading {
            before.push(Adornment::steps(NumberField::STEP_UP, NumberField::STEP_DOWN));
        }
        if let Some(inside) = cx.env.inside_label() {
            match (inside.icon(), inside.text()) {
                (Some(icon), _) => before.push(Adornment::glyph(icon.clone())),
                (None, Some(text)) => before.push(Adornment::text(text.clone(), TypeRole::Body).apart().measured(cx)),
                (None, None) => {}
            }
        }
        before
    }

    /// The leading adornment that is the step strip (`steps`) or the inside label's glyph, with its rect.
    fn leading_at(&self, placed: &Placed, steps: bool) -> Option<(&Adornment, Rect)> {
        self.leading.iter().zip(&placed.leading).find(|(adornment, _)| adornment.is_steps() == steps).map(|(adornment, at)| (adornment, *at))
    }

    /// One step of `notches` (up positive) scaled by `scale`, from the applied value or from blank, by the stepper when
    /// the field has one and by the rule otherwise. A wheel run commits first and the step starts from the value it
    /// showed.
    fn step_by(&self, cx: &mut Cx<'_, Self>, notches: i32, scale: f64) {
        if cx.in_wheel_run() {
            cx.commit();
        }
        let rule = self.rule();
        let from = self.shown_number(cx.value()).unwrap_or_else(|| rule.blank());
        let next = match &self.stepper {
            Some(stepper) => rule.settle(stepper.next(from, notches, scale)),
            None => rule.stepped(from, notches, scale),
        };
        cx.set(V::of_number(self.to_stored(next)));
    }

    /// The rect of the inside label's glyph, the text, the unit and the chevrons inside `rect`.
    fn laid_out(&self, style: &NumberStyle, rect: Rect, rtl: bool) -> Placed {
        let mut field = style.field.clone();
        let mut content = rect;
        let mut placed = place(content, &field, &self.leading, &self.trailing, rtl);
        if placed.text.width() < self.value_width {
            field.metrics.gap = 0.0;
            placed = place(content, &field, &self.leading, &self.trailing, rtl);
            let missing = (self.value_width - placed.text.width()).max(0.0);
            let padding = field.metrics.padding;
            let available = padding.start + padding.end;
            if available > 0.0 {
                let fraction = (missing / available).min(1.0);
                let start = padding.start * fraction;
                let end = padding.end * fraction;
                field.metrics.padding.start -= start;
                field.metrics.padding.end -= end;
                let (left, right) = if rtl { (end, start) } else { (start, end) };
                content.min.x -= left;
                content.max.x += right;
                placed = place(content, &field, &self.leading, &self.trailing, rtl);
            }
        }
        placed
    }

    /// The alert for `typed`, a typed number the rule held to its range, in shown units; `kept` is the committed
    /// value, in shown units too.
    fn range_alert(&self, typed: f64, kept: Option<f64>) -> RangeAlert {
        let rule = self.rule();
        RangeAlert {
            typed,
            kept: kept.unwrap_or_else(|| rule.settle(typed)),
            range: rule.bounds(),
            whole: self.commit_decimals == Some(0) || self.decimals() == 0,
            unit: self.unit.clone(),
            unit_kind: self.unit_kind,
            decimals: self.shown_decimals(),
        }
    }
}
impl<V: NumberValue + 'static> Component for NumberField<'_, V> {
    const ROLE: Role = Role::SPIN_BUTTON;
    const INPUT: Input = Input::Text;
    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(crate::contract::Input::Text).with_moves();
    type Style = NumberStyle;
    type Output = NumberOutput<V>;

    fn style(cx: &StyleCx<'_>, key: NumberFieldStyleKey<V>) -> NumberStyle {
        let field = FieldStyle::at(cx);
        let mut surface = field.surface.clone();
        if key.status_zoom {
            surface.kind = None;
            surface.radius = CornerRadius::ZERO;
            // Use the theme's recessed field color; focus and invalid states still use the usual borders.
            surface.fill = Stateful::new(cx.color(cx.tokens.color.surface.sunken));
            let clear = cx.color(Rgba::TRANSPARENT);
            if let Some(border) = &mut surface.border {
                border.off = States::new(clear, clear, clear, clear);
            }
        }
        let faint = cx.color(cx.tokens.color.text.faint);
        let muted = cx.color(cx.tokens.color.text.muted);
        NumberStyle {
            surface,
            // The width is the value area, at most a number cell, and the adornments beside it, at least a cell
            // (`measure`); the style's minimum is only the height.
            metrics: field.clone().sized_by_content().metrics,
            unit: TextLook::new(
                TypeRole::Caption,
                Stateful::new(faint).off(States::new(faint, faint, faint, cx.color(cx.tokens.color.text.disabled))),
            ),
            handle: Stateful::new(muted).off(States::new(muted, muted, muted, cx.color(cx.tokens.color.text.disabled))),
            cell: cx.control().number,
            field,
        }
    }

    /// A drag on the body scrubs a field that is not in edit; the wheel steps the value.
    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::DRAG | Sense::FOCUS | Sense::TAKE_FOCUS | Sense::WHEEL_Y
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        self.settle(&cx.env);
        let codec = self.ruled(&cx.env, false, self.names(|row| cx.env.resolve(&row.label().clone())));
        // The value area holds the widest value the field shows (the range's ends at the codec's precision, or at the
        // places it commits when it rounds what it commits, or the value when the range is open) with the caret and the unit, at most one number cell; a longer value scrolls
        // inside it. The inside label and the chevrons take their own room beside it, each as `place` gives it. The
        // field is never narrower than a cell, so a column of short fields ends at one edge. A content-sized field
        // takes its whole value and has no floor; in a column group its value area is the group's widest.
        let content = content_sized(cx);
        // A mixed field is sized as it is with a value, so it keeps its width when the selection changes.
        let blank = V::blank().unwrap_or_else(|| V::of_number(self.rule().blank()));
        let shown = cx.value();
        let no_zoom_value = self.status_zoom && self.shows_blank(shown);
        let (sample, role) = if no_zoom_value {
            // Text editors draw their hint in the proportional face.
            (Cow::Borrowed(BLANK), self.options.role)
        } else if self.derives(shown) {
            // A derived value is sized as it shows at rest, in parentheses with its unit.
            (Cow::Owned(self.derived_text(&codec.sample(shown.unwrap_or(&blank)), &cx.env.units())), self.value_role())
        } else {
            (codec.sample(shown.unwrap_or(&blank)), self.value_role())
        };
        let mut text = cx.field_text(&TextRun::new(sample.as_ref()).role(role));
        if content {
            text.x = crate::layout::ambient_width(cx.ui.ctx(), &VALUES, text.x).map_or(text.x, |width| width.max(text.x));
        }
        let chevrons = self.chevrons(&cx.words::<TextWords>());
        self.leading = Self::before_text(cx, self.step_arrows);
        self.trailing = self.after_text(cx, &chevrons);
        let unit = &self.trailing[..self.trailing.len() - chevrons.len()];
        let value = text.x + reserve(&cx.style.field, &[], unit);
        let beside = reserve(&cx.style.field, &self.leading, &self.trailing) - reserve(&cx.style.field, &[], unit);
        if content {
            return Vec2::new(value + beside, text.y);
        }
        let field = cx.style.field.metrics();
        let cell = cx.style.cell - field.padding.start - field.padding.end;
        Vec2::new((value.min(cell) + beside).max(cell), text.y)
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        self.settle(&cx.env);
        self.track = cx.tokens.size.length.track;
        let rtl = cx.flow.is_rtl();
        let chevrons = self.chevrons(&cx.words::<TextWords>());
        self.leading = Self::before_text(cx, self.step_arrows);
        self.trailing = self.after_text(cx, &chevrons);
        let names = self.names(|row| cx.env.resolve(&row.label().clone()));
        // A derived value is measured as it shows at rest, in parentheses with its unit, so its reading takes the
        // padding it needs as a longer number does.
        self.value_width = cx.value().map_or(0.0, |value| {
            let mut text = self.ruled(&cx.env, false, names).format(value);
            if self.derives(Some(value)) {
                text = self.derived_text(&text, &cx.env.units());
            }
            cx.field_text(&TextRun::new(&text).role(self.value_role())).x
        });
        let placed = self.laid_out(cx.style, cx.content_rect, rtl);
        if let Some((_, slot)) = self.leading_at(&placed, false) {
            cx.label_part(slot);
        }
        // The halves take a press without the focus: ArrowUp and ArrowDown in the field step the same way.
        let halves = self.leading_at(&placed, true).and_then(|(strip, at)| strip.halves(at));
        for (key, half) in halves.into_iter().flatten() {
            cx.part(key, half).sense(Sense::CLICK | Sense::KEEP_FOCUS).pointer_only("ArrowUp and ArrowDown step the value");
        }
        let text = placed.text;
        // A percentage reads "100.0%": the number stands against its sign, not in the middle of the room. The inline
        // run starts at the field's start.
        let centred = self.look == NumberLook::Centred;
        self.options.centred = centred && !self.is_percent();
        self.options.end_aligned = centred && self.is_percent();
        self.options.mono = centred;
        if !centred {
            self.options.role = self.value_role();
        }
        self.options.steps_value = self.presets.is_none() && self.popup_slider.is_none();
        let shown = cx.value();
        if self.options.placeholder.is_none() {
            if self.mixed_word && shown.is_none() {
                self.options.placeholder = Some(cx.words::<TextWords>().mixed.clone());
            } else if self.shows_blank(shown) {
                self.options.placeholder = Some(BLANK.into());
            } else if self.automatic {
                self.options.placeholder = Some(cx.words::<TextWords>().automatic.clone());
            }
        }
        let names = self.names(|row| cx.env.resolve(&row.label().clone()));
        let blank = V::blank().unwrap_or_else(|| V::of_number(self.rule().blank()));
        let ruled = self.ruled(&cx.env, shown.is_none(), names);
        // A derived value shows in parentheses until the field is edited; the text part keeps the plain number for the
        // edit to start from.
        self.options.resting = shown.filter(|value| self.derives(Some(*value))).map(|value| self.derived_text(&ruled.format(value), &cx.env.units()));
        cx.text(text_part::<V>(), text, &ruled, shown.unwrap_or(&blank), &self.options);
        let slider_reason =
            if self.presets.is_some() { "Alt+Down opens the presets list; the slider opens by pointer" } else { "Alt+Down opens the slider" };
        let chevron_rects = &placed.trailing[placed.trailing.len() - chevrons.len()..];
        for ((key, _), at) in chevrons.iter().zip(chevron_rects) {
            let reason = if *key == PRESETS { "Alt+Down opens the list" } else { slider_reason };
            let popup = if *key == PRESETS { "presets" } else { "slider" };
            cx.part(*key, *at)
                .sense(Sense::CLICK | Sense::DRAG | Sense::KEEP_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .pointer_only(reason)
                .toggles(popup);
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, V>> {
        Some(self.value.reborrow())
    }

    /// A write-back equal to the proposal at the field's decimals is the gesture's own write.
    fn same_value(&self, proposed: &V, host: &V) -> bool {
        let decimals = self.decimals();
        let at_precision = |value: &V| value.number().map(|number| round_to(number * self.scale, decimals));
        proposed == host || (proposed.named() == host.named() && at_precision(proposed) == at_precision(host))
    }

    /// A drag on the body of a field not in edit, or on its side label with `scrub_by_label`, moves by the scrub step per point;
    /// a drag on the popup slider's chevron moves the value as far as the same drag on the slider.
    fn scrub(&self) -> Option<Scrub<V>> {
        let rule = self.rule();
        let step = self.scrub_step.filter(|step| step.is_finite() && *step > 0.0).unwrap_or(rule.step);
        let drag = ValueDrag::relative(step, rule.bounds()).warp(true).hide_cursor(true).coarse(LARGE_STEPS).fine(1.0 / LARGE_STEPS);
        // Rounded to the field's decimals, so a scrubbed value carries no float noise. The step is not a grid
        // here: an Alt drag moves by a share of it.
        let decimals = self.decimals();
        let scale = self.scale;
        let mut scrub =
            Scrub::new(drag, move |value: &V| value.number().unwrap_or(0.0) * scale, move |number| V::of_number(round_to(number, decimals) / scale))
                .on_field();
        if self.scrub {
            scrub = scrub.by_label();
        }
        if let Some(range) = self.popup_slider.as_ref().filter(|_| self.track > 0.0) {
            let shown = range.start() * scale..=range.end() * scale;
            let rate = (shown.end() - shown.start()) / f64::from(self.track);
            scrub = scrub.part(SLIDER, ValueDrag::relative(rate, shown));
        }
        Some(scrub)
    }

    /// Arrows, Page keys, the wheel and AccessKit Increment and Decrement step the value, AccessKit sets it, and the
    /// chevrons open the popups. Up and Down step by the step, with Shift by `LARGE_STEPS` steps and, on a field
    /// without a dropdown, with Alt by a tenth where the decimals show it (else by the step); on a field with presets
    /// or a popup slider Alt+Down opens the dropdown instead, so it has no fine key step, and the wheel keeps its Alt
    /// fine step. A step starts from the applied value (the text part applies a typed draft first), or from 0 kept in
    /// the range while the field is blank. A wheel run is one edit gesture that the wheel gesture's end commits; a key
    /// or an AccessKit set during the run commits it first and steps from the value it showed.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        // A drag from the slider's chevron is one gesture with the slider it opened: its release closes the slider.
        if cx.part_if_declared(SLIDER).is_some_and(|part| matches!(part.press.phase, PressPhase::Release { click: false, .. })) {
            cx.popup("slider").close(cx);
        }
        let rule = self.rule();
        let fine_shows = self.decimals() >= decimals_for_step(rule.step / LARGE_STEPS);
        // A press on a half steps in the pass it goes down; while it is held the steps due since the last pass
        // repeat, from the press's own start time and the pass's time, so a rerun pass (no time elapsed) steps none.
        for (key, notches) in [(NumberField::STEP_UP, 1), (NumberField::STEP_DOWN, -1)] {
            let Some(press) = cx.part_if_declared(key).map(|part| part.press) else { continue };
            let (start, first) = match press.phase {
                PressPhase::Down { start, .. } => (start, true),
                PressPhase::Held { start, .. } => (start, false),
                _ => continue,
            };
            if cx.resolved.read_only || !cx.resolved.enabled {
                continue;
            }
            let ctx = cx.response.ctx.clone();
            let hold = HoldRepeat::new(start.time);
            let (now, elapsed) = frame::running(&ctx).map_or((start.time, 0.0), |facts| (facts.now, facts.elapsed));
            let scale = key_scale(MoveMods::new(press.mods.shift, false, Granularity::Unit).fine(press.mods.alt), fine_shows);
            for _ in 0..if first { 1 } else { hold.due(now, elapsed) } {
                self.step_by(cx, notches, scale);
            }
            frame::request_repaint_after(&ctx, hold.next_repeat(now) - now);
        }
        while let Some((mv, mods)) = cx.intents().take_move() {
            let (notches, scale) = match mv {
                NavMove::Up => (1, key_scale(mods, fine_shows)),
                NavMove::Down => (-1, key_scale(mods, fine_shows)),
                NavMove::PageUp => (1, LARGE_STEPS),
                NavMove::PageDown => (-1, LARGE_STEPS),
                // AccessKit Increment and Decrement: the text keeps the horizontal arrows, so only those arrive.
                NavMove::Forward => (1, 1.0),
                NavMove::Back => (-1, 1.0),
                _ => continue,
            };
            self.step_by(cx, notches, scale);
        }
        if let Some(wheel) = cx.take_wheel() {
            let from = self.shown_number(cx.value()).unwrap_or_else(|| rule.blank());
            let steps = ValueSteps::new(from, rule.low, rule.high, rule.step, rule.step * LARGE_STEPS).fine(rule.step / LARGE_STEPS);
            // A notch that changes the value proposes it; the runtime's zero-scroll end share changes none and ends
            // the run.
            let next = match wheel_step(&steps, ValueAxes::Both, &wheel, cx.env.direction().is_rtl()) {
                StepResult::Set(next) => {
                    let next = match &self.stepper {
                        Some(stepper) if next != from => stepper.next(from, if next > from { 1 } else { -1 }, 1.0),
                        _ => round_to(next, decimals_for_step(rule.step / LARGE_STEPS)),
                    };
                    let next = rule.settle(next);
                    (next != from).then(|| V::of_number(self.to_stored(next)))
                }
                _ => None,
            };
            cx.wheel_run(wheel.phase, next);
        }
        if let Some(next) = cx.intents().take_set_value() {
            if cx.in_wheel_run() {
                cx.commit();
            }
            cx.set(V::of_number(self.to_stored(rule.settle(next))));
        }
    }

    /// The edit menu, the presets list and the popup slider.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        crate::menu::text_menu(ui, cx, &self.options);
        if let Some(rows) = self.presets {
            self.show_presets(ui, cx, rows);
        }
        if let Some(range) = self.popup_slider.clone() {
            self.show_slider(ui, cx, &range);
        }
    }

    /// The numeric value, the painted unit, and a button node per chevron named by its word (the painted face). The
    /// text part adds the text value, read with the unit, the selection and the text actions to the same node.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let rule = self.rule();
        // A value that is not finite shows the dash, as a mixed one does, and has no number to read.
        let numeric = match self.shown_number(cx.value()).filter(|value| value.is_finite()) {
            Some(value) => Numeric::new(value, rule.bounds()),
            None => Numeric::empty(rule.bounds()),
        };
        access.numeric(numeric.step(rule.step));
        if let Some(unit) = self.painted_unit(cx) {
            access.unit(cx.env.resolve(&unit).as_str());
        }
        for (key, shown) in [(PRESETS, self.presets.is_some()), (SLIDER, self.popup_slider.is_some())] {
            if shown {
                access.node(key, Role::BUTTON);
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rtl = cx.flow.is_rtl();
        let chevrons = self.chevrons(&cx.words::<TextWords>());
        let placed = self.laid_out(cx.style, cx.content_rect, rtl);
        if self.painted_unit(cx).is_some()
            && let (Some(unit), Some(at)) = (self.trailing.first(), placed.trailing.first())
        {
            unit.paint(ui, *at, cx.resolve(&cx.style.unit.color), self.options.role);
        }
        for steps in [true, false] {
            if let Some((adornment, at)) = self.leading_at(&placed, steps) {
                adornment.paint(ui, at, cx.resolve(&cx.style.handle), self.options.role);
            }
        }
        for (key, name) in chevrons {
            ui.face(key, IconButton::new(crate::controls::disclosure::DisclosureGlyph::dropdown_of(cx.env.get())).label(name));
        }
    }

    /// The value as the field shows it: its ruled codec's text, in the shown unit, a name as its presets row's label.
    fn word_value(&self, value: &V, cx: &Cx<'_, Self>) -> Option<String> {
        let names = self.names(|row| cx.env.resolve(&row.label().clone()));
        Some(self.ruled(&cx.env, false, names).format(value))
    }

    /// The edit, the preset picked, and what the committed text's parse learned: its unit, whether the range held it
    /// (only a typed commit carries a note, so steps, the wheel and a scrub never report out of range) and whether
    /// the text was cut.
    fn finish(self, cx: Cx<'_, Self>) -> NumberOutput<V> {
        let committed = cx.text(&text_part::<V>()).and_then(|outcome| outcome.committed);
        let out_of_range = committed.and_then(|note| note.held).map(|typed| self.range_alert(typed, self.shown_number(cx.value())));
        NumberOutput {
            edit: cx.edit_response(),
            preset: self.picked,
            typed_unit: committed.and_then(|note| note.unit).and_then(UnitKind::from_tag),
            out_of_range,
            cut: committed.is_some_and(|note| note.cut),
            link: cx.link_activated(),
        }
    }
}
