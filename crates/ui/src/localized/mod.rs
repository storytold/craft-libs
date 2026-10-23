//! Number widgets that write numbers the way the user's region does (its digits, decimal mark and
//! grouping) and read back any digits with the region's separators.
//!
//! [`LocaleNumbers`] wraps a [`craft_i18n::Format`], which writes and reads the numbers.
//! [`LocalizedDragValue`] puts them on an [`egui::DragValue`]; [`NumberField`] is a text field for
//! apps that edit numbers in a [`egui::TextEdit`], with the apps' panel-field behaviour: a
//! [`Recessed`] box, scrubbing by the label, stepping to multiples of the step, a blank field for
//! [mixed](NumberField::mixed) values and [`focus_first_field`]. Text that isn't a number never
//! changes the value: it is outlined in the error color with a tooltip saying why, from the
//! catalog with [`LocaleNumbers::with_catalog`].

use std::cell::{Cell, RefCell};
use std::ops::RangeInclusive;
use std::rc::Rc;
use std::sync::Arc;

use craft_i18n::{Format, I18n, Lang, NumberError};
use egui::emath::{Numeric, round_to_decimals};
use egui::response::Flags;
use egui::text::{CCursor, CCursorRange};
use egui::{
    Align2, AsIdSalt, Color32, Context, CornerRadius, CursorIcon, DragValue, Event, Frame, Id, IdSalt, Key, Margin, Modifiers, MouseWheelUnit, Rect,
    Response, Sense, Stroke, StrokeKind, Style, TextEdit, Ui, Vec2, Widget, vec2,
};

#[cfg(test)]
mod tests;

/// How long a [`LocalizedDragValue`] keeps marking text it could not read after reverting, in
/// seconds.
const REJECTED_FOR: f64 = 2.5;

/// Points of horizontal drag on a [`NumberField`]'s label per step.
const SCRUB_POINTS_PER_STEP: f32 = 2.0;

type Messages = Arc<dyn Fn(&'static str) -> String + Send + Sync>;
type Fallback<'a> = Box<dyn 'a + Fn(&str) -> Option<f64>>;
type Formatter<'a> = Box<dyn 'a + Fn(f64, RangeInclusive<usize>) -> Option<String>>;
/// The last text egui asked the parser to read: whether it is still being typed, and the result.
type LastRead = Rc<Cell<Option<(bool, Result<f64, NumberError>)>>>;

/// How the user's region writes and reads numbers. Cheap to clone; build it from
/// [`I18n::format`] so widgets match the app's counts and sizes.
#[derive(Clone)]
pub struct LocaleNumbers {
    format: Arc<Format>,
    messages: Option<Messages>,
    grouping: bool,
}

impl std::fmt::Debug for LocaleNumbers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocaleNumbers").field("format", &self.format).finish_non_exhaustive()
    }
}

impl LocaleNumbers {
    pub fn new(format: Arc<Format>) -> LocaleNumbers {
        LocaleNumbers { format, messages: None, grouping: true }
    }

    /// The numbers of a locale tag (`de-CH`, `ar-EG`) with its own digits.
    pub fn from_tag(tag: &str) -> LocaleNumbers {
        LocaleNumbers::new(Arc::new(Format::from_tag(tag)))
    }

    /// The error tooltips from `i18n`'s catalog in `lang`: the plain rows keyed by the English
    /// text ([`message`]), English where a row is missing.
    pub fn with_catalog(self, i18n: &I18n, lang: Lang) -> LocaleNumbers {
        let rows: Vec<(&'static str, String)> = MESSAGES.iter().map(|&english| (english, i18n.t(lang, english).to_owned())).collect();
        self.with_messages(move |english| rows.iter().find(|(key, _)| *key == english).map_or_else(|| english.to_owned(), |(_, row)| row.clone()))
    }

    /// Translate the error tooltips with an app's own lookup: `translate` gets the English text
    /// ([`message`]) and returns it in the interface language.
    pub fn with_messages(mut self, translate: impl Fn(&'static str) -> String + Send + Sync + 'static) -> LocaleNumbers {
        self.messages = Some(Arc::new(translate));
        self
    }

    /// Write numbers without grouping separators, for pixel sizes and years: `1920`, not `1,920`.
    /// Grouped text still reads back.
    pub fn ungrouped(mut self) -> LocaleNumbers {
        self.grouping = false;
        self
    }

    /// `value` with between `decimals.start()` and `decimals.end()` digits after the decimal mark,
    /// trailing zeros dropped down to the minimum: `1,5`, `١٬٢٣٤٫٥`.
    pub fn format(&self, value: f64, decimals: RangeInclusive<usize>) -> String {
        let (min, max) = (*decimals.start(), (*decimals.end()).max(*decimals.start()));
        let full = round_to_decimals(value, max);
        let decimals = (min..max).find(|&d| round_to_decimals(value, d) == full).unwrap_or(max);
        // `-0.001` shown with two decimals is `0,00`, not `-0,00`.
        let value = if round_to_decimals(value, decimals) == 0.0 { 0.0 } else { value };
        if self.grouping { self.format.number(value, decimals) } else { self.format.number_ungrouped(value, decimals) }
    }

    /// The number in `text`, as [`Format::parse`] reads it: what [`LocaleNumbers::format`] writes,
    /// in any digits.
    pub fn parse(&self, text: &str) -> Result<f64, NumberError> {
        self.format.parse(text)
    }

    fn describe(&self, error: NumberError) -> String {
        let english = message(error);
        self.messages.as_ref().map_or_else(|| english.to_owned(), |translate| translate(english))
    }
}

/// Every text [`message`] returns: the catalog rows an app adds for [`LocaleNumbers::with_catalog`].
pub const MESSAGES: [&str; 5] =
    ["Enter a number", "Not a number", "Use one kind of digits", "Check the decimal and grouping marks", "Number too large"];

/// The English text the widgets show for `error`; a catalog key.
pub fn message(error: NumberError) -> &'static str {
    match error {
        NumberError::Empty => MESSAGES[0],
        NumberError::Unexpected(_) => MESSAGES[1],
        NumberError::MixedDigits => MESSAGES[2],
        NumberError::Separator => MESSAGES[3],
        NumberError::TooLarge => MESSAGES[4],
    }
}

/// A unit around a number, from a craft-i18n unit template with the number as `{n}`: `{n} px`,
/// `{n}%`, Turkish `%{n}`. Typed text may include it; it is removed before parsing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unit {
    prefix: String,
    suffix: String,
}

impl Unit {
    /// The unit `template` spells; a template without `{n}` is a suffix.
    pub fn new(template: &str) -> Unit {
        match template.split_once("{n}") {
            Some((prefix, suffix)) => Unit { prefix: prefix.to_owned(), suffix: suffix.to_owned() },
            None => Unit { prefix: String::new(), suffix: template.to_owned() },
        }
    }

    /// The unit row `template` (context `unit`) in `lang`, the English template if untranslated.
    pub fn translated(i18n: &I18n, lang: Lang, template: &str) -> Unit {
        Unit::new(i18n.get(lang, "unit", template).unwrap_or(template))
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub fn suffix(&self) -> &str {
        &self.suffix
    }

    fn strip<'t>(&self, text: &'t str) -> &'t str {
        let text = text.trim();
        let text = text.strip_prefix(self.prefix.trim()).unwrap_or(text);
        text.strip_suffix(self.suffix.trim()).unwrap_or(text).trim()
    }
}

/// Parses `text` with `numbers`, then with `fallback` (an app's expression or timecode reader).
fn read(numbers: &LocaleNumbers, unit: &Unit, fallback: Option<&Fallback<'_>>, text: &str) -> Result<f64, NumberError> {
    numbers.parse(unit.strip(text)).or_else(|error| fallback.and_then(|f| f(text)).ok_or(error))
}

/// Text that may still become a number as the user types (`-`, `1,`): not marked while editing.
fn incomplete(text: &str, error: NumberError) -> bool {
    match error {
        NumberError::Empty => true,
        NumberError::Separator => text.trim_end().ends_with(['.', ',', '٫', '٬', '\'', '’', '．', '，']),
        _ => false,
    }
}

/// Text the widget could not read. `until` is `None` while the user is still editing it.
#[derive(Clone)]
struct Rejected {
    error: NumberError,
    until: Option<f64>,
}

fn rejected_id(id: Id) -> Id {
    id.with("craft_ui::localized::rejected")
}

/// Outlines `response` in the error color and, with `tell`, shows `text` beside it.
fn mark_invalid(ui: &Ui, response: &Response, radius: CornerRadius, text: String, tell: bool) {
    ui.painter().rect_stroke(response.rect, radius, Stroke::new(1.5, ui.visuals().error_fg_color), StrokeKind::Inside);
    if tell {
        response.show_tooltip_text(text);
    }
}

/// An [`egui::DragValue`] that writes and reads numbers with [`LocaleNumbers`]. Set range, speed,
/// decimals and any prefix or suffix on the drag value; a [`Unit`] set here goes outside that
/// prefix and suffix (egui adds to them and can't take them off), so set one or the other. The
/// drag value's own `custom_formatter` and `custom_parser` are replaced: pass an app's formatter
/// and parser to [`LocalizedDragValue::or_format`] and [`LocalizedDragValue::or_parse`] instead.
///
/// Typed text that isn't a number is outlined while it is being edited. Committing it (Enter,
/// clicking away) keeps the old value, as egui does, and the field stays outlined with the reason
/// for a moment. Escape reverts without a mark. Committing the text as the drag value shows it
/// leaves the value as it is, with any decimals the drag value doesn't show.
///
/// ```
/// # egui::__run_test_ui(|ui| {
/// use craft_ui::localized::{LocaleNumbers, LocalizedDragValue, Unit};
/// let numbers = LocaleNumbers::from_tag("de");
/// let mut width = 1.5;
/// ui.add(LocalizedDragValue::new(egui::DragValue::new(&mut width).speed(0.1), &numbers).unit(Unit::new("{n} px")));
/// # });
/// ```
#[must_use = "You should put this widget in a ui with `ui.add(widget);`"]
pub struct LocalizedDragValue<'a> {
    drag: DragValue<'a>,
    numbers: LocaleNumbers,
    unit: Unit,
    formatter: Option<Formatter<'a>>,
    fallback: Option<Fallback<'a>>,
}

impl<'a> LocalizedDragValue<'a> {
    pub fn new(drag: DragValue<'a>, numbers: &LocaleNumbers) -> Self {
        LocalizedDragValue { drag, numbers: numbers.clone(), unit: Unit::default(), formatter: None, fallback: None }
    }

    /// The unit around the number, outside a prefix and suffix set on the drag value.
    pub fn unit(mut self, unit: Unit) -> Self {
        self.unit = unit;
        self
    }

    /// [`DragValue::range`] of the wrapped drag value.
    pub fn range<Num: Numeric>(mut self, range: RangeInclusive<Num>) -> Self {
        self.drag = self.drag.range(range);
        self
    }

    /// [`DragValue::speed`] of the wrapped drag value.
    pub fn speed(mut self, speed: impl Into<f64>) -> Self {
        self.drag = self.drag.speed(speed);
        self
    }

    /// Show the number without grouping separators ([`LocaleNumbers::ungrouped`]).
    pub fn ungrouped(mut self) -> Self {
        self.numbers.grouping = false;
        self
    }

    /// An app's own text for some values (a timecode, `Auto` for 0), tried before the locale's;
    /// `None` writes the number as the locale does. Pair it with [`LocalizedDragValue::or_parse`]
    /// so the text reads back.
    pub fn or_format(mut self, formatter: impl 'a + Fn(f64, RangeInclusive<usize>) -> Option<String>) -> Self {
        self.formatter = Some(Box::new(formatter));
        self
    }

    /// A parser for text that isn't a plain number (an expression, a timecode), tried after the
    /// locale's.
    pub fn or_parse(mut self, parser: impl 'a + Fn(&str) -> Option<f64>) -> Self {
        self.fallback = Some(Box::new(parser));
        self
    }
}

impl Widget for LocalizedDragValue<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let LocalizedDragValue { drag, numbers, unit, formatter, fallback } = self;
        let parsed: LastRead = Rc::new(Cell::new(None));
        // The value egui last had written and its text. egui reads the text back when the field
        // loses focus, edited or not; the unedited text gives the value itself, not the value
        // rounded to the decimals shown.
        let shown: Rc<RefCell<Option<(f64, String)>>> = Rc::new(RefCell::new(None));
        let mut drag = drag.custom_formatter({
            let (numbers, shown) = (numbers.clone(), Rc::clone(&shown));
            move |value, decimals| {
                let text = formatter.as_ref().and_then(|f| f(value, decimals.clone())).unwrap_or_else(|| numbers.format(value, decimals));
                *shown.borrow_mut() = Some((value, text.clone()));
                text
            }
        });
        if !unit.prefix.is_empty() {
            drag = drag.prefix(unit.prefix.clone());
        }
        if !unit.suffix.is_empty() {
            drag = drag.suffix(unit.suffix.clone());
        }
        drag = drag.custom_parser({
            let (numbers, unit, parsed) = (numbers.clone(), unit.clone(), Rc::clone(&parsed));
            let shown = Rc::clone(&shown);
            move |text| {
                let result = match &*shown.borrow() {
                    Some((value, written)) if written == text => Ok(*value),
                    _ => read(&numbers, &unit, fallback.as_ref(), text),
                };
                let still_typing = result.err().is_some_and(|error| incomplete(text, error));
                parsed.set(Some((still_typing, result)));
                result.ok()
            }
        });
        let response = ui.add(drag);

        let key = rejected_id(response.id);
        let now = ui.input(|i| i.time);
        let editing = response.has_focus();
        let mut rejected = ui.data(|d| d.get_temp::<Rejected>(key));
        match parsed.take() {
            Some((_, Ok(_))) => rejected = None,
            Some((still_typing, Err(error))) if editing => rejected = (!still_typing).then_some(Rejected { error, until: None }),
            // egui reads the text once more the frame after the edit loses focus.
            Some((_, Err(error))) => rejected = Some(Rejected { error, until: Some(now + REJECTED_FOR) }),
            // Escape, or focus left without a final read.
            None if !editing && rejected.as_ref().is_some_and(|r| r.until.is_none()) => rejected = None,
            None => {}
        }
        // A drag or arrow key after a rejection sets a new value: the mark is moot.
        if response.changed() || rejected.as_ref().is_some_and(|r| r.until.is_some_and(|t| t <= now)) {
            rejected = None;
        }
        match rejected {
            Some(r) => {
                if let Some(until) = r.until {
                    ui.ctx().request_repaint_after_secs((until - now) as f32);
                }
                mark_invalid(ui, &response, ui.visuals().widgets.inactive.corner_radius, numbers.describe(r.error), true);
                ui.data_mut(|d| d.insert_temp(key, r));
            }
            None => ui.data_mut(|d| d.remove::<Rejected>(key)),
        }
        response
    }
}

/// What a [`NumberField`] keeps between frames while its text differs from the value.
#[derive(Clone)]
struct FieldState {
    text: String,
    /// The value and mixed state when `text` was stored; a change means the app changed them, and
    /// the text is dropped.
    value: f64,
    mixed: bool,
    error: Option<NumberError>,
}

/// The sunken box of [`NumberField::recessed`], as the apps draw their own fields. Fill it from the
/// app's tokens, or start from [`Recessed::from_style`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recessed {
    pub fill: Color32,
    pub stroke: Stroke,
    /// The border while the field has the keyboard focus.
    pub focus_stroke: Stroke,
    pub corner_radius: CornerRadius,
    /// The box's height; the text is centred in it.
    pub height: f32,
    /// Space between the box's edge and the text.
    pub margin: Margin,
}

impl Recessed {
    /// egui's text-edit fill, the noninteractive border, the selection stroke while focused, and
    /// the interact height.
    pub fn from_style(style: &Style) -> Recessed {
        let visuals = &style.visuals;
        Recessed {
            fill: visuals.text_edit_bg_color(),
            stroke: visuals.widgets.noninteractive.bg_stroke,
            focus_stroke: visuals.selection.stroke,
            corner_radius: visuals.widgets.inactive.corner_radius,
            height: style.spacing.interact_size.y,
            margin: Margin::symmetric(6, 0),
        }
    }
}

fn first_field_id() -> Id {
    Id::unique("craft_ui::localized::focus_first_field")
}

/// While `on`, the next enabled [`NumberField`] drawn takes the keyboard focus with its text
/// selected, so typing replaces the value and Enter applies it: a dialog opening on its first
/// field. Raise it when the group opens; the field that takes it lowers it. A sizing pass (a new
/// window's first frame) leaves it for the next pass.
pub fn focus_first_field(ctx: &Context, on: bool) {
    ctx.data_mut(|d| {
        if on {
            d.insert_temp(first_field_id(), true);
        } else {
            d.remove::<bool>(first_field_id());
        }
    });
}

/// A drag on a [`NumberField`]'s label in progress.
#[derive(Clone, Copy)]
struct Scrub {
    /// The value before the first step, which Escape puts back.
    start: Option<f64>,
    /// Points dragged and not yet stepped.
    pending: f32,
}

/// A single-line text field for a number, read on commit (Enter or focus leaving) with
/// [`LocaleNumbers`]. Arrow up and down, and the mouse wheel over the focused field, step the
/// value; a drag on its [`NumberField::scrub_label`] scrubs it.
///
/// Text that isn't a number leaves the value unchanged and stays in the field, outlined, with the
/// reason in a tooltip, until it is fixed or Escape restores the value. A number is rounded to the
/// most decimals shown, half away from zero (`2,5` is 3 and `-0,5` is -1 in an integer field),
/// then clamped to the range. Committing the text as the field shows it leaves the value as it is,
/// with any decimals the field doesn't show. The response is `changed()` when the value changed.
///
/// ```
/// # egui::__run_test_ui(|ui| {
/// use craft_ui::localized::{LocaleNumbers, NumberField, Recessed, Unit};
/// let numbers = LocaleNumbers::from_tag("fr");
/// let mut gap = 12.0;
/// let label = ui.label("Gap");
/// let frame = Recessed::from_style(ui.style());
/// ui.add(NumberField::new(&mut gap, &numbers).range(0.0..=100.0).step(0.5).unit(Unit::new("{n} mm")).recessed(frame).scrub_label(&label));
/// # });
/// ```
#[must_use = "You should put this widget in a ui with `ui.add(widget);`"]
pub struct NumberField<'a> {
    get_set: Box<dyn 'a + FnMut(Option<f64>) -> f64>,
    numbers: LocaleNumbers,
    range: RangeInclusive<f64>,
    step: f64,
    decimals: RangeInclusive<usize>,
    unit: Unit,
    fallback: Option<Fallback<'a>>,
    width: Option<f32>,
    id_salt: Option<IdSalt>,
    mixed: bool,
    placeholder: Option<String>,
    recessed: Option<Recessed>,
    label: Option<(Id, Rect)>,
}

impl<'a> NumberField<'a> {
    /// A field for `value`. Integer types get no decimals and their full range.
    pub fn new<Num: Numeric>(value: &'a mut Num, numbers: &LocaleNumbers) -> Self {
        let field = NumberField {
            get_set: Box::new(move |v: Option<f64>| {
                if let Some(v) = v {
                    *value = Num::from_f64(v);
                }
                value.to_f64()
            }),
            numbers: numbers.clone(),
            range: f64::NEG_INFINITY..=f64::INFINITY,
            step: 1.0,
            decimals: 0..=6,
            unit: Unit::default(),
            fallback: None,
            width: None,
            id_salt: None,
            mixed: false,
            placeholder: None,
            recessed: None,
            label: None,
        };
        if Num::INTEGRAL { field.decimals(0..=0).range(Num::MIN.to_f64()..=Num::MAX.to_f64()) } else { field }
    }

    pub fn range(mut self, range: RangeInclusive<f64>) -> Self {
        self.range = range;
        self
    }

    /// The change per arrow key press, wheel notch or scrub step; 0 turns stepping off. Default 1.
    /// Shift steps ten, and Ctrl/Cmd a tenth where the decimals show it. A plain or Ctrl/Cmd step
    /// goes to the next multiple of its size (by 1, `5,1` goes up to 6 and down to 5); a Shift step
    /// rounds to a multiple of `step` first.
    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    /// The fewest and most digits shown after the decimal mark. Default `0..=6`.
    pub fn decimals(mut self, decimals: RangeInclusive<usize>) -> Self {
        self.decimals = decimals;
        self
    }

    pub fn unit(mut self, unit: Unit) -> Self {
        self.unit = unit;
        self
    }

    /// A parser for text that isn't a plain number (an expression, a timecode), tried after the
    /// locale's.
    pub fn or_parse(mut self, parser: impl 'a + Fn(&str) -> Option<f64>) -> Self {
        self.fallback = Some(Box::new(parser));
        self
    }

    pub fn desired_width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn id_salt(mut self, id_salt: impl AsIdSalt) -> Self {
        self.id_salt = Some(IdSalt::new(id_salt));
        self
    }

    /// The value differs between the selected objects: the field is blank (showing the
    /// [`NumberField::placeholder`]) and the value stays as it is until the user enters a number.
    /// That number is `changed()` even when it equals the value, so the app sets it on every
    /// object. Committing the field blank changes nothing, and the label doesn't scrub.
    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    /// Text shown in the field while it is empty, as a [`NumberField::mixed`] field is.
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// Draw the field in a sunken box of `frame`'s fill, border and height.
    pub fn recessed(mut self, frame: Recessed) -> Self {
        self.recessed = Some(frame);
        self
    }

    /// The field's label, drawn before it. It names the field for accessibility, and dragging it
    /// sideways scrubs the value: a step every two points, with the arrow keys' Shift and Ctrl/Cmd
    /// steps. A click on it focuses the field; Escape during the drag puts the value back. The
    /// response reports the label's drag as its own (`drag_started`, `dragged`, `drag_stopped`),
    /// so the app can make the whole drag one undo step.
    pub fn scrub_label(mut self, label: &Response) -> Self {
        self.label = Some((label.id, label.rect));
        self
    }
}

impl Widget for NumberField<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let NumberField { mut get_set, numbers, range, step, decimals, unit, fallback, width, id_salt, mixed, placeholder, recessed, label } = self;
        let id = id_salt.map_or_else(|| ui.next_auto_id(), |salt| ui.make_persistent_id(salt));
        let old = get_set(None);
        let places = *decimals.end();
        let clamp = |v: f64| if range.start() <= range.end() { v.clamp(*range.start(), *range.end()) } else { v };
        let settle = |v: f64| clamp(round_half_away(v, places));
        let shown = if mixed { String::new() } else { numbers.format(old, decimals.clone()) };

        let mut state = ui.data(|d| d.get_temp::<FieldState>(id)).filter(|s| s.value.to_bits() == old.to_bits() && s.mixed == mixed);
        let mut select = false;
        if ui.is_enabled() && !ui.is_sizing_pass() && ui.data_mut(|d| d.remove_temp::<bool>(first_field_id())).is_some() {
            ui.memory_mut(|m| m.request_focus(id));
            select = true;
        }
        // The shown text stands for `old` itself: reading it back would round the value to the
        // shown decimals, so leaving an untouched field would change it.
        let read_text = |text: &str| if !mixed && text == shown { Ok(old) } else { read(&numbers, &unit, fallback.as_ref(), text) };
        let mut value = old;
        // A number the user entered; in a mixed field it counts as a change even if it equals `old`.
        let mut entered = false;
        if step != 0.0 && ui.memory(|m| m.has_focus(id)) {
            let typed = state.as_ref().map_or(shown.as_str(), |s| s.text.as_str());
            // Text that isn't a number takes no steps: the wheel scrolls the panel instead.
            if let Ok(base) = read_text(typed) {
                let over = ui.ctx().read_response(id).is_some_and(|r| r.contains_pointer());
                let steps = take_steps(ui, id, over);
                if !steps.is_empty() {
                    value = settle(steps.iter().fold(base, |v, &(modifiers, n)| stepped(v, step, places, modifiers, n)));
                    (entered, select, state) = (true, true, None);
                }
            }
        }
        let mut scrubbed = None;
        if let Some((_, rect)) = label.filter(|_| !mixed && step != 0.0) {
            let (drag, to) = scrub(ui, id, rect, value, |v, modifiers, n| settle(stepped(v, step, places, modifiers, n)));
            if let Some(to) = to {
                (value, state) = (to, None);
            }
            scrubbed = Some(drag);
        }
        let mut text = match &state {
            Some(s) => s.text.clone(),
            None if value.to_bits() == old.to_bits() && !entered => shown.clone(),
            None => numbers.format(value, decimals.clone()),
        };
        if select {
            select_all(ui.ctx(), id, &text);
        }

        let mut edit = TextEdit::singleline(&mut text).id(id);
        if let Some(placeholder) = placeholder {
            edit = edit.hint_text(placeholder);
        }
        if !unit.prefix.is_empty() {
            edit = edit.prefix(unit.prefix.clone());
        }
        if !unit.suffix.is_empty() {
            edit = edit.suffix(unit.suffix.clone());
        }
        if let Some(width) = width {
            edit = edit.desired_width(width);
        }
        let radius = match recessed {
            Some(frame) => {
                let stroke = if ui.memory(|m| m.has_focus(id)) { frame.focus_stroke } else { frame.stroke };
                let sunk = Frame::NONE.fill(frame.fill).stroke(stroke).corner_radius(frame.corner_radius).inner_margin(frame.margin);
                edit = edit.frame(sunk).min_size(vec2(0.0, frame.height)).align(Align2::LEFT_CENTER);
                frame.corner_radius
            }
            None => ui.visuals().widgets.inactive.corner_radius,
        };
        let mut response = ui.add(edit);
        // After the text edit, which gives up the focus on a click elsewhere.
        if scrubbed.as_ref().is_some_and(Response::clicked) {
            ui.memory_mut(|m| m.request_focus(id));
            select_all(ui.ctx(), id, &text);
        }
        let editing = response.has_focus();

        let escape = ui.input(|i| i.key_pressed(Key::Escape));
        let mut error = state.as_ref().and_then(|s| s.error);
        if response.lost_focus() && (escape || mixed && unit.strip(&text).is_empty()) {
            state = None;
            error = None;
        } else if response.lost_focus() {
            match read_text(&text) {
                Ok(parsed) => {
                    value = if !mixed && text == shown { parsed } else { settle(parsed) };
                    entered = true;
                    state = None;
                    error = None;
                }
                Err(e) => {
                    error = Some(e);
                    state = Some(FieldState { text: text.clone(), value, mixed, error });
                }
            }
        } else if response.changed() {
            error = read(&numbers, &unit, fallback.as_ref(), &text).err().filter(|e| !incomplete(&text, *e));
            state = Some(FieldState { text: text.clone(), value, mixed, error });
        } else if response.has_focus() && state.is_none() {
            state = Some(FieldState { text: text.clone(), value, mixed, error: None });
        }

        // By bits, so a NaN value equals itself and isn't written back every frame.
        if value.to_bits() != old.to_bits() || mixed && entered {
            get_set(Some(value));
            response.mark_changed();
        } else {
            response.flags.remove(Flags::CHANGED);
        }
        if let Some(drag) = scrubbed {
            response.flags.insert(drag.flags & (Flags::DRAG_STARTED | Flags::DRAGGED | Flags::DRAG_STOPPED));
        }
        if let Some((label, _)) = label {
            response = response.labelled_by(label);
        }
        let value = get_set(None);
        match state {
            Some(mut s) => {
                s.value = value;
                ui.data_mut(|d| d.insert_temp(id, s));
            }
            None => ui.data_mut(|d| d.remove::<FieldState>(id)),
        }
        if let Some(error) = error {
            mark_invalid(ui, &response, radius, numbers.describe(error), editing);
            if !editing {
                response = response.on_hover_text(numbers.describe(error));
            }
        }
        response
    }
}

/// `value` moved `n` steps of `step` (down when `n` is negative) with `modifiers` held: ten steps
/// with Shift, a tenth of one with Ctrl/Cmd when `decimals` show it. A plain or Ctrl/Cmd step
/// goes to the next multiple of its size in its direction, then on by whole steps; a Shift step
/// rounds to a multiple of `step` first.
fn stepped(value: f64, step: f64, decimals: usize, modifiers: Modifiers, n: f64) -> f64 {
    let fine = step / 10.0;
    if modifiers.command && (round_half_away(fine, decimals) - fine).abs() <= fine.abs() * 1e-9 {
        to_multiple(value, fine, n)
    } else if modifiers.shift {
        (value / step).round() * step + 10.0 * step * n
    } else {
        to_multiple(value, step, n)
    }
}

/// `value` moved to the `n`th multiple of `size` past it: by 1, 5.1 goes up to 6 and down to 5,
/// and 5 to 6 and 4. A value within a millionth of a step of a multiple counts as on it.
fn to_multiple(value: f64, size: f64, n: f64) -> f64 {
    let q = value / size;
    let from = if n > 0.0 { (q + 1e-6).floor() } else { (q - 1e-6).ceil() };
    (from + n) * size
}

/// The steps asked for this frame by the arrow keys in a focused field and, with `wheel`, by the
/// mouse wheel over it, in order: their modifiers and how many steps up (down when negative).
/// They are taken from the input, so the text edit doesn't move its cursor and the panel under the
/// field doesn't scroll.
fn take_steps(ui: &Ui, id: Id, wheel: bool) -> Vec<(Modifiers, f64)> {
    // The wheel turns in notches: a line each, or `line` points of a trackpad. What is left of a
    // notch carries over to the next turn.
    let line = ui.ctx().options(|o| o.input_options.line_scroll_speed).max(1.0);
    let notch_id = id.with("craft_ui::localized::notches");
    let mut notches: f32 = if wheel { ui.data(|d| d.get_temp(notch_id)).unwrap_or(0.0) } else { 0.0 };
    let mut steps = Vec::new();
    ui.input_mut(|i| {
        i.events.retain(|event| {
            let (modifiers, n) = match event {
                Event::Key { key: Key::ArrowUp, pressed: true, modifiers, .. } => (*modifiers, 1.0),
                Event::Key { key: Key::ArrowDown, pressed: true, modifiers, .. } => (*modifiers, -1.0),
                Event::MouseWheel { unit, delta, modifiers, .. } if wheel => {
                    // Shift may turn the wheel sideways (macOS does).
                    let turn = if modifiers.shift { delta.x + delta.y } else { delta.y };
                    notches += match unit {
                        MouseWheelUnit::Point => turn / line,
                        MouseWheelUnit::Line | MouseWheelUnit::Page => turn,
                    };
                    let whole = notches.trunc();
                    notches -= whole;
                    (*modifiers, f64::from(whole))
                }
                _ => return true,
            };
            if n != 0.0 {
                steps.push((modifiers, n));
            }
            false
        });
        if wheel {
            i.smooth_scroll_delta = Vec2::ZERO;
        }
    });
    if wheel {
        ui.data_mut(|d| d.insert_temp(notch_id, notches));
    }
    steps
}

/// A drag this frame on `label`, the label of field `id`: its response, and the value it scrubs
/// `value` to with `step` (value, modifiers, steps).
fn scrub(ui: &Ui, id: Id, label: Rect, value: f64, step: impl Fn(f64, Modifiers, f64) -> f64) -> (Response, Option<f64>) {
    // On top of the label, which may sense clicks itself (selectable text).
    let drag = ui.interact(label, id.with("craft_ui::localized::scrub"), Sense::click_and_drag());
    if drag.hovered() || drag.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    let key = drag.id;
    if drag.drag_started() {
        // Typing in the field ends, and its text is committed, before the first step.
        ui.memory_mut(|m| m.surrender_focus(id));
        // The drag starts once the pointer has moved a few points: those count too.
        let moved = match (ui.input(|i| i.pointer.press_origin()), drag.interact_pointer_pos()) {
            (Some(from), Some(to)) => to.x - from.x,
            _ => drag.drag_delta().x,
        };
        ui.data_mut(|d| d.insert_temp(key, Scrub { start: None, pending: moved }));
        return (drag, None);
    }
    let Some(mut s) = ui.data(|d| d.get_temp::<Scrub>(key)) else { return (drag, None) };
    if !drag.dragged() {
        ui.data_mut(|d| d.remove::<Scrub>(key));
        // egui ends a drag on Escape: the value goes back.
        let cancelled = drag.drag_stopped() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        return (drag, s.start.filter(|_| cancelled));
    }
    s.pending += drag.drag_delta().x;
    let n = (s.pending / SCRUB_POINTS_PER_STEP).trunc();
    let mut to = None;
    if n != 0.0 {
        s.pending -= n * SCRUB_POINTS_PER_STEP;
        s.start.get_or_insert(value);
        to = Some(step(value, ui.input(|i| i.modifiers), f64::from(n)));
    }
    ui.data_mut(|d| d.insert_temp(key, s));
    (drag, to)
}

/// Selects all of `text` in text edit `id`, so typing replaces it.
fn select_all(ctx: &Context, id: Id, text: &str) {
    let mut state = TextEdit::load_state(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(text.chars().count()))));
    state.store(ctx, id);
}

/// `value` rounded to `decimals` digits after the point, ties away from zero; never `-0`.
fn round_half_away(value: f64, decimals: usize) -> f64 {
    let scale = 10_f64.powi(i32::try_from(decimals).unwrap_or(i32::MAX));
    let rounded = if scale.is_finite() { (value * scale).round() / scale } else { value };
    if rounded.is_finite() { rounded + 0.0 } else { value }
}
