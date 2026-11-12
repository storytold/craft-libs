//! `NumberField`: a text part over [`NumberCodec`] with a range, a step, a unit, presets and a popup slider.
//!
//! Typing, expressions, unit spellings and locale digits are the codec's (`quantity::Reader`); the field adds the
//! range every written value stays in and the step that arrows, the wheel and a scrub move by. A drag on a field that
//! is not in edit scrubs it; a click enters edit, where a drag selects text.

#[cfg(feature = "testing")]
mod stories;

use super::adorn::{Adornment, Placed, place, reserve};
use super::words::TextWords;
use super::{FieldStyle, text_part};
use crate::buttons::IconButton;
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, Entry, Focus as _, Input, MeasureCx, Numeric, OverlayUi, Overlays as _, PaintUi,
    PartsCx, Pointer as _, Props as _, Role, Scrub, Sizing, Styled as _, WordSetCx as _,
};
use crate::primitives::{TextOptions, TextRun};
use crate::runtime::env::{Label, LengthUnits, UnitKind, Units};
use crate::style::{ComponentStyle, StyleColor, StyleCx, SurfaceLook, TextLook, TypeRole};
use craft_behavior::nav::{LARGE_STEPS, StepResult, ValueAxes, ValueSteps};
use craft_behavior::{
    Codec, HitLevel, HitPriority, Metrics, NavMove, NumberCodec, PartKey, PressPhase, Sense, ValueDrag, decimals_for_step, wheel_step,
};
use craft_theme::{Stateful, States};
use egui::{Rect, Vec2};
use std::borrow::Cow;
use std::marker::PhantomData;
use std::ops::RangeInclusive;

mod popups;
mod rule;
mod value;

pub use rule::NumberStepper;
use rule::{Rule, Ruled, key_scale, round_to};
pub use value::{NamedOr, NumberName, NumberOutput, NumberPick, NumberValue};

/// The step of an arrow press, the wheel and a scrub when the field sets none.
const DEFAULT_STEP: f64 = 1.0;
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
    /// What one arrow press, one wheel notch and one point of scrub change; the default is 1.
    #[prop(quantity = "step")]
    step: Option<f64>,
    /// Where an arrow press, a Page key and a wheel notch go from the current value, for values on a ladder (the zoom
    /// ladder); the step is ignored for them. The range still holds the result.
    #[prop(func = "Fn(f64, i32) -> f64")]
    stepper: Option<NumberStepper<'a>>,
    /// A unit shown after the number.
    #[prop(into)]
    unit: Option<Label>,
    /// The rows of the list a chevron after the field opens; Alt+Down opens it too.
    #[prop]
    presets: Option<&'a [Entry<NumberPick>]>,
    /// A chevron after the field opens a slider over `range` below it; pressing the chevron and dragging moves the
    /// value at once, as far as the slider would, and the release closes the slider.
    #[prop]
    popup_slider: Option<RangeInclusive<f64>>,
    /// A typed value goes to the nearest multiple of the step when it commits.
    #[prop]
    snap_typed: bool,
    /// The decimals a committed value is rounded to; the display keeps the codec's.
    #[prop]
    commit_decimals: Option<usize>,
    /// The field is as wide as the widest value of its range, when that is wider than a number cell.
    #[prop]
    fit_range: bool,
    /// An empty field means automatic: it shows the "Auto" word as its placeholder.
    #[prop]
    automatic: bool,
    /// The field writes exactly this many decimals, zeros included ("1.00"), when it has no codec of its own.
    #[prop]
    fixed_decimals: Option<usize>,
    /// The lengths a length field keeps and shows (`NumberField::length`).
    lengths: Option<LengthUnits>,
    /// How many shown units make one stored unit, read from the env in measure and parts; 1 for a plain field.
    scale: f64,
    options: TextOptions,
    scrub: bool,
    /// The popup slider's track length, read from the tokens in parts: a drag on its chevron moves the value as far
    /// as a drag on the slider would.
    track: f32,
    picked: Option<NumberPick>,
    /// The inside label's glyph before the text (`EnvView::inside_label`), measured in measure and parts for paint.
    leading: Vec<Adornment>,
    /// The unit and the chevrons after the text, measured in measure and parts for paint.
    trailing: Vec<Adornment>,
}

impl<'a> NumberField<'a, Option<f64>> {
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
        let mut field = Self::of(label, value);
        field.lengths = Some(lengths);
        field.unit_kind = Some(lengths.shown);
        field.step = Some(lengths.shown.step());
        field
    }
}

impl<'a, V: NumberValue + 'static> NumberField<'a, V> {
    /// A number field named `label` over a value of another type, such as [`NamedOr`].
    pub fn named(label: impl Into<Label>, value: Bind<'a, V>) -> Self {
        Self::of(label, value)
    }

    fn of(label: impl Into<Label>, value: Bind<'a, V>) -> Self {
        NumberField {
            common: Common::new(),
            value,
            codec: None,
            unit_kind: None,
            range: None,
            step: None,
            stepper: None,
            unit: None,
            presets: None,
            popup_slider: None,
            snap_typed: false,
            fit_range: false,
            automatic: false,
            commit_decimals: None,
            fixed_decimals: None,
            lengths: None,
            scale: 1.0,
            options: TextOptions::default(),
            scrub: false,
            track: 0.0,
            picked: None,
            leading: Vec::new(),
            trailing: Vec::new(),
        }
        .label(label)
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

    /// Reads what the field keeps from the env: a length field's scale from stored to shown, and its unit kind's suffix
    /// (the symbol with the space the language puts before it) after the number unless the field sets a unit.
    fn settle_lengths(&mut self, units: &Units) {
        if let Some(lengths) = self.lengths {
            self.scale = units.length_scale(lengths).unwrap_or(1.0);
        }
        if self.unit.is_none()
            && let Some(kind) = self.unit_kind
        {
            self.unit = Some(units.suffix(kind).into());
        }
    }

    /// The unit's rect: against the end of the shown number, which sits centred in `text`, and no further out than
    /// `reserved`, the room `place` keeps for it at the end of the field.
    fn unit_at(reserved: Rect, text: Rect, number_width: f32, rtl: bool) -> Rect {
        let width = reserved.width();
        if rtl {
            let end = (text.center().x - number_width / 2.0).max(reserved.max.x);
            Rect::from_min_max(egui::pos2(end - width, reserved.min.y), egui::pos2(end, reserved.max.y))
        } else {
            let start = (text.center().x + number_width / 2.0).min(reserved.min.x);
            Rect::from_min_max(egui::pos2(start, reserved.min.y), egui::pos2(start + width, reserved.max.y))
        }
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

    /// The field's codec: the one set, else the env's locale and the spellings of the unit kind (a length field
    /// reads every length spelling).
    fn number_codec(&self, units: &Units) -> NumberCodec {
        self.codec.clone().unwrap_or_else(|| {
            let reader = match self.lengths {
                Some(lengths) => units.length_reader(lengths),
                None => units.reader(self.unit_kind),
            };
            let codec = NumberCodec::new(reader, self.decimals());
            if self.fixed_decimals.is_some() { codec.fixed() } else { codec }
        })
    }

    /// The most decimals the field writes.
    fn decimals(&self) -> usize {
        self.codec.as_ref().map_or_else(
            || self.fixed_decimals.unwrap_or_else(|| self.unit_kind.map_or(DEFAULT_DECIMALS, |kind| *kind.decimals().end())),
            NumberCodec::decimals,
        )
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

    fn ruled(&self, units: &Units, mixed: bool, names: Vec<(u32, String)>) -> Ruled<V> {
        Ruled {
            inner: self.number_codec(units),
            rule: self.rule(),
            snap: self.snap_typed,
            decimals: self.commit_decimals,
            mixed,
            names,
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

    /// The unit, measured, then the chevrons: what sits after the text.
    fn after_text(&self, cx: &MeasureCx<'_, Self>, chevrons: &[(PartKey, Label)]) -> Vec<Adornment> {
        let unit = self.unit.iter().map(|unit| Adornment::text(unit.clone(), cx.style.unit.role).measured(cx));
        let chevrons =
            chevrons.iter().map(|(key, name)| Adornment::button(*key, crate::controls::disclosure::DisclosureGlyph::dropdown(), name.clone()));
        unit.chain(chevrons).collect()
    }

    /// The inside label's glyph, measured: what sits before the text.
    fn before_text(cx: &MeasureCx<'_, Self>) -> Vec<Adornment> {
        let Some(inside) = cx.env.inside_label() else { return Vec::new() };
        let glyph = match (inside.icon(), inside.text()) {
            (Some(icon), _) => Adornment::glyph(icon.clone()),
            (None, Some(text)) => Adornment::text(text.clone(), TypeRole::Body).apart().measured(cx),
            (None, None) => return Vec::new(),
        };
        vec![glyph]
    }

    /// The rect of the inside label's glyph, the text, the unit and the chevrons inside `rect`.
    fn laid_out(&self, style: &NumberStyle, rect: Rect, rtl: bool) -> Placed {
        place(rect, &style.field, &self.leading, &self.trailing, rtl)
    }
}
impl<V: NumberValue + 'static> Component for NumberField<'_, V> {
    const ROLE: Role = Role::SPIN_BUTTON;
    const INPUT: Input = Input::Text;
    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(crate::contract::Input::Text).with_moves();
    type Style = NumberStyle;
    type Output = NumberOutput<V>;

    fn style(cx: &StyleCx<'_>, _: NumberFieldStyleKey<V>) -> NumberStyle {
        let field = FieldStyle::at(cx);
        let faint = cx.color(cx.tokens.color.text.faint);
        let muted = cx.color(cx.tokens.color.text.muted);
        NumberStyle {
            surface: field.surface.clone(),
            // The width is a number cell, or the range's widest value with `fit_range` or `Sizing::Content` (`measure`);
            // the style's minimum is only the height.
            metrics: field.clone().sized_by_content().metrics,
            unit: TextLook::new(TypeRole::Body, Stateful::new(faint).off(States::new(faint, faint, faint, cx.color(cx.tokens.color.text.disabled)))),
            handle: Stateful::new(muted).off(States::new(muted, muted, muted, cx.color(cx.tokens.color.text.disabled))),
            cell: cx.control().number,
            field,
        }
    }

    /// A drag on the body scrubs a field that is not in edit.
    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::DRAG | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        self.settle_lengths(&cx.env.units());
        let codec = self.ruled(&cx.env.units(), false, self.names(|row| cx.env.resolve(&row.label().clone())));
        // The number's room is one digit and the caret; a field sized to its range (`fit_range`) or to its content
        // (`Sizing::Content`) takes the widest value and the caret.
        let content = cx.resolved.sizing() == Some(Sizing::Content);
        let text = if self.fit_range || content {
            let sample = cx.value().map_or(Cow::Borrowed(""), |value| codec.sample(value));
            cx.field_text(&TextRun::new(sample.as_ref()))
        } else {
            cx.field_text(&TextRun::new("0"))
        };
        self.leading = Self::before_text(cx);
        self.trailing = self.after_text(cx, &self.chevrons(&cx.words::<TextWords>()));
        // The adornments take their own room and the change of the edge inset a button makes.
        let width = text.x + reserve(&cx.style.field, &self.leading, &self.trailing);
        // A field is one number cell wide, the unit and the chevrons inside it, so a column of fields ends at one edge;
        // it grows only when the room left for the number drops below one digit and the caret. A content-sized field
        // is as wide as its widest value and adornments.
        if content {
            return Vec2::new(width, text.y);
        }
        let field = cx.style.field.metrics();
        let width = width.max(cx.style.cell - field.padding.start - field.padding.end);
        Vec2::new(width, text.y)
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        self.settle_lengths(&cx.env.units());
        self.track = cx.tokens.size.length.track;
        let rtl = cx.flow.is_rtl();
        let chevrons = self.chevrons(&cx.words::<TextWords>());
        self.leading = Self::before_text(cx);
        self.trailing = self.after_text(cx, &chevrons);
        let placed = self.laid_out(cx.style, cx.content_rect, rtl);
        if let Some(slot) = placed.leading.first() {
            cx.label_part(*slot);
        }
        let text = placed.text;
        self.options.centred = true;
        self.options.steps_value = self.presets.is_none() && self.popup_slider.is_none();
        let shown = cx.value();
        if self.options.placeholder.is_none() {
            if shown.is_none() || non_finite(shown) {
                self.options.placeholder = Some(BLANK.into());
            } else if self.automatic {
                self.options.placeholder = Some(cx.words::<TextWords>().automatic.clone());
            }
        }
        let names = self.names(|row| cx.env.resolve(&row.label().clone()));
        let blank = V::blank().unwrap_or_else(|| V::of_number(self.rule().blank()));
        cx.text(text_part::<V>(), text, &self.ruled(&cx.env.units(), shown.is_none(), names), shown.unwrap_or(&blank), &self.options);
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

    /// A drag on the body of a field not in edit, or on its side label with `scrub_by_label`, moves a step a point;
    /// a drag on the popup slider's chevron moves the value as far as the same drag on the slider.
    fn scrub(&self) -> Option<Scrub<V>> {
        let rule = self.rule();
        let drag = ValueDrag::relative(rule.step, rule.bounds()).warp(true).hide_cursor(true).coarse(LARGE_STEPS).fine(1.0 / LARGE_STEPS);
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
    /// fine step. A step starts from the applied value, or from 0 kept in the
    /// range while the field is blank.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        // A drag from the slider's chevron is one gesture with the slider it opened: its release closes the slider.
        if cx.part_if_declared(SLIDER).is_some_and(|part| matches!(part.press.phase, PressPhase::Release { click: false, .. })) {
            cx.popup("slider").close(cx);
        }
        let rule = self.rule();
        let fine_shows = self.decimals() >= decimals_for_step(rule.step / LARGE_STEPS);
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
            let from = self.shown_number(cx.value()).unwrap_or_else(|| rule.blank());
            let next = match &self.stepper {
                Some(stepper) => rule.settle(stepper.next(from, notches)),
                None => rule.stepped(from, notches, scale),
            };
            cx.set(V::of_number(self.to_stored(next)));
        }
        if let Some(wheel) = cx.take_wheel() {
            let from = self.shown_number(cx.value()).unwrap_or_else(|| rule.blank());
            let steps = ValueSteps::new(from, rule.low, rule.high, rule.step, rule.step * LARGE_STEPS).fine(rule.step / LARGE_STEPS);
            if let StepResult::Set(next) = wheel_step(&steps, ValueAxes::Both, &wheel, cx.env.direction().is_rtl()) {
                let next = match &self.stepper {
                    Some(stepper) if next != from => stepper.next(from, if next > from { 1 } else { -1 }),
                    _ => round_to(next, decimals_for_step(rule.step / LARGE_STEPS)),
                };
                cx.set(V::of_number(self.to_stored(rule.settle(next))));
            }
        }
        if let Some(next) = cx.intents().take_set_value() {
            cx.set(V::of_number(self.to_stored(rule.settle(next))));
        }
    }

    /// The presets list and the popup slider.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        if let Some(rows) = self.presets {
            self.show_presets(ui, cx, rows);
        }
        if let Some(range) = self.popup_slider.clone() {
            self.show_slider(ui, cx, &range);
        }
    }

    /// The numeric value, and a button node per chevron named by its word (the painted face). The text part adds the
    /// text value, the selection and the text actions to the same node.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let rule = self.rule();
        let numeric = match self.shown_number(cx.value()) {
            Some(value) => Numeric::new(value, rule.bounds()),
            None => Numeric::empty(rule.bounds()),
        };
        access.numeric(numeric.step(rule.step));
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
        // A mixed value shows the dash alone: a unit beside it would claim a measure the selection does not have.
        if self.unit.is_some()
            && cx.value().is_some_and(|value| !non_finite(Some(value)))
            && let (Some(unit), Some(at)) = (self.trailing.first(), placed.trailing.first())
        {
            // The unit reads as part of the number ("100%", "12 px"), so it follows the number's shown text.
            let number =
                cx.text(&text_part::<V>()).and_then(|outcome| outcome.layout.as_ref()).map_or(placed.text.width(), |layout| layout.galley.size().x);
            unit.paint(ui, Self::unit_at(*at, placed.text, number, rtl), cx.resolve(&cx.style.unit.color));
        }
        if let (Some(glyph), Some(at)) = (self.leading.first(), placed.leading.first()) {
            glyph.paint(ui, *at, cx.resolve(&cx.style.handle));
        }
        for (key, name) in chevrons {
            ui.face(key, IconButton::new(crate::controls::disclosure::DisclosureGlyph::dropdown()).label(name));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> NumberOutput<V> {
        NumberOutput {
            edit: cx.edit_response(),
            preset: self.picked,
            typed_unit: cx.text(&text_part::<V>()).and_then(|outcome| outcome.committed_tag).and_then(UnitKind::from_tag),
        }
    }
}
