//! `ParamRows`: a form with one `Field` per visible parameter of a [`Param`] list, over a [`DialogFields`] value.

use super::fields::{DialogFields, FieldValue};
use super::file_list::{FileList, FileListMode, FileListPicker};
use crate::patterns::blend_keys::{BlendKeyEntry, BlendKeys, BlendShortcut, BlendStep, blend_entries};
use crate::patterns::presets::{PresetFns, PresetKind, PresetPicker, PresetSource};
use crate::runtime::commands::{CommandState, Commands as _};
use crate::runtime::requests::{ColorRequest, EyedropperRequest, ListingRequest, PathRequest};

use super::param::{CustomContext, Head, Lifted, Param, ParamKind, fill_defaults, path_list, rounded};
use super::path_row::PathRow;
use super::words::ParamWords;
use crate::buttons::{Button, IconButton, ToggleButton};
use crate::choice::{Checkbox, Radio, RadioGroup, Swatch};
use crate::contract::{
    Bind, BodyCx, CanvasGesture, Entry, FieldLayout, NoValue, Outcome, Props as _, Remember as _, Sampler as _, Show as _, ShowContainer as _,
    SideLabel, Sizing, clicked,
};
use crate::display::Text;
use crate::form::{Field, FieldGroup, SliderField, SliderFieldLayout};

use crate::layout_components::{Row, Stack};

use crate::pickers::{Select, UnitSelect};

use crate::runtime::env::{Env, Label, UnitKind, Units, with_env};

use crate::sections::{HeaderStrength, Section};
use crate::text::{NumberField, RangeAlert, SelectableText, TextField, UnitPlacement};

use crate::value_drag::{AngleDial, PointPicker, SliderScale};

use craft_behavior::{CancelReason, EditEvent, EditEvents, GesturePhase, IconId, Mixed, NumberCodec, decimals_for_step};
use craft_host::PickRequest;
use craft_keymap::{Chord, Key};

use craft_theme::Srgba;
use egui::{Id, Response, Ui};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// Where a grid or point row's numbers go: whole numbers have no decimals.
const WHOLE: u32 = 0;
/// A point row's numbers run over the whole image, in percent.
const PERCENT: RangeInclusive<f64> = 0.0..=100.0;
/// The largest value of a Random button over an int row with no upper bound.
const RANDOM_HIGH: i64 = i32::MAX as i64;
/// The first state of the Random button's sequence when the caller gives no seed.
const SEED_START: u64 = 0x9E37_79B9_7F4A_7C15;
/// The chord that ends a row's sampling: held by the rows while one samples, so the dialog does not cancel.
const ESCAPE: Chord = Chord::plain(Key::Escape);

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
    /// The folder a files row asked the host to list this pass; the host answers with `HostAnswer::Listing`.
    pub listing: Option<ListingRequest>,
    /// A typed value a row clamped to its range, in the pass it committed: the host shows the range alert.
    pub out_of_range: Option<RangeAlert>,
    /// The pass's edits of the whole values, lifted from the rows' controls: a gesture as begin, update, commit or
    /// cancel with the control's gesture id, a one-step change as a set; empty when nothing changed.
    pub events: EditEvents<DialogFields>,
    /// A row's text does not parse: OK and Enter wait while it stays so.
    pub unparsed: bool,
    /// The help link a row's button (`Param::help_link`) reported in this pass.
    pub help_link: Option<String>,
    /// An Escape ended a row's sampling in this pass: the dialog stays open, and the next Escape cancels as usual.
    pub sampling_ended: bool,
    /// The ids of the actions custom rows asked the host to run in this pass (`CustomContext::request_action`), in
    /// the order asked.
    pub actions: Vec<String>,
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
///   [`Param::section`] opens a section, a bold title under a rule, whose body is the rows up to the next section.
///   A range row is a slider and its number field beside the row's label, or stacked under the label
///   ([`sliders`](ParamRows::sliders)).
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
    patterns: Option<&'a PresetSource<'a, String>>,
    preview_colors: &'a [(String, Srgba)],
    sliders: SliderFieldLayout,
}

impl<'a> ParamRows<'a> {
    /// Rows for `params` over `fields`, the group named `label`.
    pub fn new(label: impl Into<Label>, params: &'a [Param], fields: Bind<'a, DialogFields>) -> Self {
        ParamRows {
            label: label.into(),
            params,
            fields,
            seed: None,
            read_only: false,
            canvas: None,
            patterns: None,
            preview_colors: &[],
            sliders: SliderFieldLayout::Inline,
        }
    }

    /// How the range rows arrange their slider and number field: `Inline` (the default) puts them on one line beside
    /// the row's label; `Stacked` puts the label and the number field on one line and the slider across the width
    /// under them.
    pub fn sliders(mut self, layout: SliderFieldLayout) -> Self {
        self.sliders = layout;
        self
    }

    /// The colours child pickers preview, by colour row key: each such row draws its colour for the pass in place of
    /// its value and writes nothing, so the values stay as they were when the picker closes.
    pub fn preview_colors(mut self, colors: &'a [(String, Srgba)]) -> Self {
        self.preview_colors = colors;
        self
    }

    /// The state of the Random button's sequence: each press reads it and writes the next.
    pub fn seed(mut self, seed: Bind<'a, u64>) -> Self {
        self.seed = Some(seed);
        self
    }

    /// The canvas (the canvas component's id) the eyedropper rows sample while the rows are inside a modal dialog: an
    /// armed eyedropper lets its press through the dialog's modal layer (`Sampler::pass_canvas_gesture`). The dialog
    /// body keeps the view usable itself with `Sampler::keep_view_open`.
    pub fn canvas(mut self, canvas: Id) -> Self {
        self.canvas = Some(canvas);
        self
    }

    /// The patterns a [`ParamKind::Pattern`] row offers, by id.
    pub fn patterns(mut self, source: &'a PresetSource<'a, String>) -> Self {
        self.patterns = Some(source);
        self
    }

    /// Whether the rows only show their values.
    pub fn read_only(mut self, on: bool) -> Self {
        self.read_only = on;
        self
    }

    /// Shows the rows across the width offered.
    pub fn show(self, ui: &mut Ui) -> ParamRowsOutput {
        let ParamRows { label, params, mut fields, mut seed, read_only, canvas, patterns, preview_colors, sliders } = self;
        let env = Env::of(ui);
        let words = env.words::<ParamWords>();
        let units = env.units();
        let name: Arc<str> = label.as_str().into();
        let mut pass = Pass::new(&words, &units, patterns, canvas);
        pass.previews = preview_colors;
        pass.sliders = sliders;
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
                if !read_only {
                    pass.blend_keys(cx, params);
                }
                pass.rows(ui, params);
                // While a row samples the rows hold Escape from every component, renewed each pass; the held press is
                // consumed here, so a second pass of the frame ends nothing new. The capture lapses once sampling stops.
                let escaped = pass.sampling && !read_only && !ui.is_sizing_pass() && {
                    crate::runtime::keys::capture(ui.ctx(), cx.id, ESCAPE);
                    crate::runtime::keys::take_claimed(ui.ctx(), cx.id, ESCAPE)
                };
                if escaped {
                    pass.memo.armed = None;
                    pass.memo.ended = true;
                    pass.sampling_ended = true;
                }
                if let Some(canvas) = canvas.filter(|_| pass.sampling && !escaped && !read_only) {
                    cx.pass_canvas_gesture(canvas, CanvasGesture::PRIMARY);
                }
                *cx.keep::<Memo>() = pass.memo.clone();
                changed = pass.work != filled;
                values = (pass.work != before).then(|| pass.work.clone());
                if let Some(after) = &values {
                    let event = EditEvent::set(Mixed::Same(before.clone()), after.clone(), name.clone(), None);
                    let _ = fields.reborrow().write(cx, &event);
                    // A gesture's own events stand for the edit; a change no control lifted is one set.
                    if pass.events.is_empty() {
                        pass.events.push(event);
                    }
                }
                if let (Some(bind), Some(next)) = (seed.as_mut(), pass.seed) {
                    let held = Mixed::Same(state.unwrap_or(SEED_START));
                    let _ = bind.reborrow().write(cx, &EditEvent::set(held, next, name.clone(), None));
                }
            })
        };
        let shown = if read_only { with_env(ui, |scope| scope.read_only = Some(true), group) } else { group(ui) };
        let (pick_color, path, eyedropper, listing, out_of_range, help_link, actions) = if read_only {
            Default::default()
        } else {
            (pass.color, pass.path, pass.eyedropper, pass.listing, pass.out_of_range, pass.help_link, pass.actions)
        };
        ParamRowsOutput {
            response: shown.output,
            changed,
            values,
            pick_color,
            path,
            eyedropper,
            listing,
            out_of_range,
            events: pass.events,
            unparsed: pass.unparsed,
            help_link,
            sampling_ended: pass.sampling_ended,
            actions,
        }
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
    /// The selected file of each files row: the selection belongs to the list's view, not to the row's values.
    selected: BTreeMap<String, PathBuf>,
    /// An Escape ended sampling in the pass that kept this memo; the next pass tells its custom rows.
    ended: bool,
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
    /// The folder a files row asked the host to list.
    listing: Option<ListingRequest>,
    /// The range alert of the number row that clamped a typed value; one a pass is the most a user can cause.
    out_of_range: Option<RangeAlert>,
    /// An eyedropper row is armed.
    sampling: bool,
    /// The patterns a pattern row offers.
    patterns: Option<&'p PresetSource<'p, String>>,
    /// The canvas a custom row can ask about.
    canvas: Option<Id>,
    /// The colours shown in colour rows in place of their values (`ParamRows::preview_colors`).
    previews: &'p [(String, Srgba)],
    /// How range rows arrange their slider and number field (`ParamRows::sliders`).
    sliders: SliderFieldLayout,
    /// The pass's edits of the whole values, lifted from the rows' controls.
    events: EditEvents<DialogFields>,
    /// A number or length field holds text that does not parse.
    unparsed: bool,
    /// The help link a row's button reported.
    help_link: Option<String>,
    /// The actions custom rows asked the host to run.
    actions: Vec<String>,
    /// An Escape ended sampling in this pass.
    sampling_ended: bool,
    /// An Escape ended sampling in the previous pass.
    ended_prev: bool,
    /// The edits of the row being shown, until `lift` makes them edits of the whole values.
    pending: EditEvents<FieldValue>,
    /// The rows take no input: they stand under a check head that is off.
    locked: bool,
}

/// A parameter as the pass shows it: its key, its label without the colon, and whether it takes input.
struct Line<'r> {
    param: &'r Param,
    key: &'r str,
    name: Label,
    enabled: bool,
    /// The key is marked mixed: the row shows it blank until an edit.
    mixed: bool,
    /// The row's help button was pressed in this pass.
    helped: Cell<bool>,
    /// The row stands in a pair (`Param::beside_next`): its field takes the width of its content.
    compact: bool,
}

impl Line<'_> {
    /// Shows `body` in a field named by the row: its label beside the control, its tooltip over both, the reset dot
    /// after the label of a resettable row.
    fn show(&self, ui: &mut Ui, mut body: impl FnMut(&mut Ui)) {
        let mut field = Field::new(self.name.clone()).id_salt(self.key).enabled(self.enabled);
        if self.compact {
            field = field.sizing(Sizing::Content);
        }
        if let Some(tip) = self.param.tooltip_text() {
            field = field.tooltip(tip.clone());
        }
        if self.param.option().resettable() {
            field = field.side(SideLabel::new().reset_dot(true));
        }
        field.show(ui, |ui, _| match self.param.option().help_link() {
            None => body(ui),
            Some(_) => beside(ui, |ui| {
                body(ui);
                self.help_button(ui);
            }),
        });
    }

    /// The help button after a row's control: a press is noted for the pass, which reports the row's link.
    fn help_button(&self, ui: &mut Ui) {
        let help = Env::of(ui).words::<ParamWords>().help.clone();
        if clicked(&IconButton::new(IconId::new("help")).label(help).id_salt("help").show(ui)) {
            self.helped.set(true);
        }
    }

    /// The binding of a control that shows `value` (blank while the key is mixed); a resettable row's carries the
    /// row's default, which its reset dot restores.
    fn bind<T: Clone + PartialEq + Send + Sync + 'static>(&self, value: T, default: Option<T>) -> Bind<'static, T> {
        let bind = if self.mixed { Bind::given(Mixed::<T>::Mixed) } else { Bind::given(value) };
        match default {
            Some(default) if self.param.option().resettable() => bind.reset(default),
            _ => bind,
        }
    }

    /// A scalar field over `value` with the parameter's declared unit drawn beside the editable text, read and written
    /// with `codec` when the row has one.
    fn scalar_field(&self, units: &Units, value: Bind<'static, Option<f64>>, decimals: u32, codec: Option<&NumberCodec>) -> NumberField<'static> {
        let unit = self.param.unit;
        let field = bound_field(units, self.name.clone(), value, decimals, unit, codec);
        if let Some(unit) = unit { field.unit_kind(unit) } else { field }
    }

    /// Shows `body` across the form for a full-width row, without a label, or else beside the row's label.
    fn show_body(&self, ui: &mut Ui, mut body: impl FnMut(&mut Ui)) {
        if self.param.option().full_width() {
            Row::new().id_salt(self.key).enabled(self.enabled).fill().show(ui, |ui, _| body(ui));
        } else {
            self.show(ui, body);
        }
    }
}

impl<'p> Pass<'p> {
    fn new(words: &'p ParamWords, units: &'p Units, patterns: Option<&'p PresetSource<'p, String>>, canvas: Option<Id>) -> Self {
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
            listing: None,
            out_of_range: None,
            sampling: false,
            patterns,
            canvas,
            previews: &[],
            sliders: SliderFieldLayout::Inline,
            events: EditEvents::new(),
            unparsed: false,
            help_link: None,
            actions: Vec::new(),
            sampling_ended: false,
            ended_prev: false,
            pending: EditEvents::new(),
            locked: false,
        }
    }

    /// Starts a body run over `work`: the body may run more than once in a pass, and each run starts clean.
    fn begin(&mut self, work: &DialogFields, seed: Option<u64>, mut memo: Memo) {
        self.work = work.clone();
        self.start = seed.or(memo.sequence).unwrap_or(SEED_START);
        self.seed = None;
        self.ended_prev = std::mem::take(&mut memo.ended);
        self.memo = memo;
        self.events = EditEvents::new();
        self.unparsed = false;
        self.help_link = None;
        self.actions.clear();
        self.sampling_ended = false;
        self.pending = EditEvents::new();
        self.locked = false;
        self.color = None;
        self.path = None;
        self.eyedropper = None;
        self.listing = None;
        self.out_of_range = None;
        self.sampling = false;
    }

    /// The blend keys of the first visible Blend row: Shift+= and Shift+- step its mode, Shift+Alt and a letter pick
    /// one. The commands are taken from the board once, so a second pass in the same frame applies none: a pass
    /// applies only its own input. Later Blend rows publish nothing, so one key press never moves two rows.
    fn blend_keys(&mut self, cx: &mut BodyCx<'_>, params: &[Param]) {
        let Some(param) = params.iter().find(|param| matches!(param.kind, ParamKind::Blend { .. }) && param.visible(&self.work)) else {
            return;
        };
        let ParamKind::Blend { modes, .. } = &param.kind else { return };
        let enabled = param.enabled(&self.work);
        let state = || CommandState::new().enabled(enabled);
        let entries = blend_key_entries(modes);
        for step in [BlendStep::Next, BlendStep::Previous] {
            cx.publish(step, state());
        }
        for shortcut in entries.iter().filter_map(|entry| entry.shortcut) {
            cx.publish(shortcut, state());
        }
        let mut fired = Vec::new();
        while let Some((_, step)) = cx.take_command::<BlendStep>() {
            fired.push(step);
        }
        while let Some((_, letter)) = cx.take_command::<BlendShortcut>() {
            fired.push(letter);
        }
        let mut current = self.work.text(&param.key).map(str::to_owned).or_else(|| param.kind.default_value().and_then(text_of)).unwrap_or_default();
        let offered = |id: &String| param.option_enabled(id);
        for fired in fired {
            if let Some(pick) = BlendKeys::new(&entries).current(&current).enabled(&offered).fired(&fired) {
                let id = pick.into_key();
                self.work.set(param.key.as_str(), FieldValue::Text(id.clone()));
                current = id;
            }
        }
    }

    /// The rows of `params`: those before the first head, then each section with the rows up to the next head.
    fn rows(&mut self, ui: &mut Ui, params: &[Param]) {
        let first = params.iter().position(Param::is_head).unwrap_or(params.len());
        self.body(ui, &params[..first]);
        let mut sections = Vec::new();
        let mut rest = &params[first..];
        while let Some((head, tail)) = rest.split_first() {
            let end = tail.iter().position(Param::is_head).unwrap_or(tail.len());
            let (body, after) = tail.split_at(end);
            if head.visible(&self.work) {
                sections.push((head, body));
            }
            rest = after;
        }
        self.sides(ui, &sections, |(head, _)| head.option().beside(), |pass, ui, (head, body)| pass.section(ui, head, body));
    }

    /// The units of `rows` that the body shows: a field and the one it pairs with while either is visible, any other
    /// row while it is visible. A footer row is drawn by the dialog's footer.
    fn shown<'a>(&self, rows: &'a [Param]) -> Vec<Unit<'a>> {
        let mut rows = rows.iter().filter(|param| param.footer_on().is_none());
        let mut units = Vec::new();
        while let Some(param) = rows.next() {
            let unit = match param.pairs_with_next().then(|| rows.next()).flatten() {
                Some(next) => Unit::Pair(param, next),
                None => Unit::One(param),
            };
            // A unit with no visible row must not add a gap to the form.
            if unit.params().any(|param| param.visible(&self.work)) {
                units.push(unit);
            }
        }
        units
    }

    /// The rows of one section.
    fn body(&mut self, ui: &mut Ui, rows: &[Param]) {
        let shown = self.shown(rows);
        self.sides(
            ui,
            &shown,
            |unit| unit.first().option().beside(),
            |pass, ui, unit| match unit {
                Unit::One(param) => pass.row(ui, param, false),
                Unit::Pair(param, next) => {
                    Row::new().id_salt(param.key.as_str()).wrap(true).fill_width().show(ui, |ui, _| {
                        pass.row(ui, param, true);
                        pass.row(ui, next, true);
                    });
                }
            },
        );
    }

    /// Draws `items` in order with `draw`; an item that stands `beside` the one before it shares a wrapping row with
    /// it, so the pair falls under when the room is short. Values and answers keep the order of `items`.
    fn sides<T: Copy>(&mut self, ui: &mut Ui, items: &[T], beside: impl Fn(&T) -> bool, mut draw: impl FnMut(&mut Self, &mut Ui, T)) {
        let mut at = 0;
        while at < items.len() {
            let end = at + 1 + items[at + 1..].iter().take_while(|item| beside(item)).count();
            match &items[at..end] {
                [one] => draw(self, ui, *one),
                run => {
                    Row::new().wrap(true).id_salt(at).show(ui, |ui, _| {
                        for (place, item) in run.iter().enumerate() {
                            Stack::new().id_salt(place).show(ui, |ui, _| draw(self, ui, *item));
                        }
                    });
                }
            }
            at = end;
        }
    }

    /// One section: its head, then its rows.
    fn section(&mut self, ui: &mut Ui, head: &Param, body: &[Param]) {
        let title = plain(&head.label);
        match head.option().head() {
            Head::Fixed => {
                Section::fixed(title).id_salt(head.key.as_str()).show(ui, |ui, _| self.body(ui, body));
            }
            Head::Group { open } => {
                Section::new(title).initially_open(open).id_salt(head.key.as_str()).show(ui, |ui, _| self.body(ui, body));
            }
            Head::Check => {
                Stack::new().id_salt(head.key.as_str()).show(ui, |ui, _| {
                    self.row(ui, head, false);
                    // The rows take input only while the head is checked.
                    let locked = self.locked;
                    self.locked |= !self.work.bool(&head.key).unwrap_or(false);
                    self.body(ui, body);
                    self.locked = locked;
                });
            }
        }
    }

    /// `param` as the pass shows it, `compact` in a pair.
    fn line<'r>(&self, param: &'r Param, compact: bool) -> Line<'r> {
        let key = param.key.as_str();
        Line {
            param,
            key,
            name: plain(&param.label),
            enabled: param.enabled(&self.work) && !self.locked,
            mixed: self.work.is_mixed(key),
            helped: Cell::new(false),
            compact,
        }
    }

    fn row(&mut self, ui: &mut Ui, param: &Param, compact: bool) {
        if !param.visible(&self.work) {
            return;
        }
        let mut line = self.line(param, compact);
        let held = self.work.get(line.key).cloned();
        // A disabled row shows its `disabled_value` over the values, which keep its own.
        let own = param.shown_disabled().filter(|_| !line.enabled).map(|shown| {
            line.mixed = false;
            let mut work = self.work.clone();
            work.set(line.key, shown.clone());
            std::mem::replace(&mut self.work, work)
        });
        self.show_row(ui, &line);
        if let Some(own) = own {
            self.work = own;
        }
        param.drop_dependents(held.as_ref(), &mut self.work);
        self.lift(line.key, held);
        if let Some(link) = param.option().help_link().filter(|_| line.helped.get()) {
            self.help_link = Some(link.to_owned());
        }
    }

    /// Notes the events of a control that edits one value, as edits of the whole values once the row has written it.
    fn note<T: Clone>(&mut self, events: &EditEvents<T>, to: impl Fn(T) -> Option<FieldValue>) {
        self.pending.extend(events.clone().filter_map(to));
    }

    /// Lifts the events the row's control noted to edits of the whole values: each carries the values around the
    /// row's own, which stood at `held` before the row.
    fn lift(&mut self, key: &str, held: Option<FieldValue>) {
        let events = std::mem::take(&mut self.pending);
        if events.is_empty() {
            return;
        }
        let mut base = self.work.clone();
        match held {
            Some(held) => base.set(key, held),
            None => {
                base.remove(key);
            }
        }
        self.events.extend(events.map(|value| {
            let mut fields = base.clone();
            fields.set(key, value);
            fields
        }));
    }

    /// Shows `line`'s control, chosen by its kind.
    fn show_row(&mut self, ui: &mut Ui, line: &Line<'_>) {
        let param = line.param;
        match &param.kind {
            ParamKind::Range { min, max, scale, .. } => self.range(ui, line, *min..=*max, *scale),
            ParamKind::Number { .. } if param.option().units().is_some() => self.with_units(ui, line),
            ParamKind::Number { min, max, step, decimals, .. } => self.number(ui, line, bounds(*min, *max), *step, *decimals),
            ParamKind::Int { min, max, .. } => self.int(ui, line, *min, *max),
            ParamKind::Bool { default } => self.check(ui, line, *default),
            ParamKind::Choice { options, .. } => self.choice(ui, line, options),
            ParamKind::Radio { options, details, .. } => self.radio_options(ui, line, options, details),
            ParamKind::Angle { default, min, max, decimals } => self.angle(ui, line, *default, *min..=*max, *decimals),
            ParamKind::Color { .. } => self.color(ui, line),
            ParamKind::Grid { cells, .. } => self.grid(ui, line, *cells),
            ParamKind::Point { default } => self.point(ui, line, *default),
            ParamKind::Channel { names, default } => self.channel(ui, line, names, *default),
            ParamKind::Text { default } => self.text(ui, line, default),
            ParamKind::OptionalNumber { min, max, initial, decimals, .. } => self.optional(ui, line, *min..=*max, *initial, *decimals),
            ParamKind::ReadOnlyText { text } => read_only_text(ui, line, text),
            ParamKind::Custom(_) => self.custom(ui, line),
            ParamKind::Blend { modes, .. } => self.blend(ui, line, modes),
            ParamKind::Pattern { .. } => self.pattern(ui, line),
            ParamKind::Files { folder_mode, request, open_files } => self.files(ui, line, *folder_mode, request, open_files),
            ParamKind::Section => {}
        }
    }

    /// Keeps a row's range alert for the output.
    fn alert(&mut self, alert: Option<RangeAlert>) {
        if alert.is_some() {
            self.out_of_range = alert;
        }
    }

    /// A range row: inline beside its label in the row's field, or stacked under its own caption across the form.
    fn range(&mut self, ui: &mut Ui, line: &Line<'_>, range: RangeInclusive<f64>, scale: SliderScale) {
        let value = self.work.number(line.key).unwrap_or(*range.start());
        let option = line.param.option();
        let places = option.range_places();
        if option.popup_slider {
            self.popup_range(ui, line, range, value, places);
            return;
        }
        let layout = self.sliders;
        let default = line.param.default_value().and_then(number_of);
        let mut show = |ui: &mut Ui| {
            let mut slider = SliderField::new(line.name.clone(), line.bind(value, default), range.clone()).scale(scale).layout(layout);
            if let Some(places) = places {
                slider = slider.step(10f64.powi(-(places as i32)));
            }
            if !option.marks().is_empty() {
                slider = slider.marks_labelled(option.marks().iter().cloned());
            }
            if let Some(kind) = line.param.unit {
                slider = slider.unit_kind(kind);
            }
            if layout == SliderFieldLayout::Stacked {
                slider = slider.id_salt(line.key).enabled(line.enabled);
                if let Some(tip) = line.param.tooltip_text() {
                    slider = slider.tooltip(tip.clone());
                }
            }
            let shown = slider.show(ui);
            let value = shown.value().cloned();
            self.unparsed |= shown.edit.unparsed;
            self.note(&shown.edit.events, |v| Some(rounded(v, places)));
            self.alert(shown.out_of_range);
            if let Some(v) = value {
                self.work.set(line.key, rounded(v, places));
            }
        };
        match layout {
            SliderFieldLayout::Stacked => show(ui),
            _ => line.show(ui, show),
        }
    }

    fn popup_range(&mut self, ui: &mut Ui, line: &Line<'_>, range: RangeInclusive<f64>, value: f64, places: Option<u32>) {
        // Match the ordinary range row's number field precision when the caller sets no step.
        let default_step = if range.end() - range.start() <= 10.0 { 0.01 } else { 1.0 };
        let step = places.map_or(default_step, |places| 10f64.powi(-(places as i32)));
        let unit = line.param.unit.map(|kind| (kind, self.units.symbol(kind)));
        let default = line.param.default_value().and_then(number_of).map(Some);
        line.show(ui, |ui| {
            Row::new().show(ui, |ui, _| {
                let mut field = NumberField::new(line.name.clone(), line.bind(Some(value), default))
                    .range(range.clone())
                    .popup_slider(range.clone())
                    .snap_popup_slider(places.is_some())
                    .commit_decimals(decimals_for_step(step))
                    .scrub_by_label();
                if places.is_some() || default_step != 1.0 {
                    field = field.step(step).snap_typed(places.is_some());
                }
                if let Some((kind, _)) = &unit {
                    field = field.unit_kind(*kind).unit_placement(UnitPlacement::After);
                }
                let shown = field.show(ui);
                if let Some((_, symbol)) = &unit {
                    Text::new(symbol.clone()).show(ui);
                }
                self.alert(shown.out_of_range);
                self.unparsed |= shown.edit.unparsed;
                self.note(&shown.edit.events, |v| v.map(|v| rounded(v, places)));
                if let Some(value) = shown.edit.value().copied().flatten() {
                    self.work.set(line.key, rounded(value, places));
                }
            });
        });
    }

    fn number(&mut self, ui: &mut Ui, line: &Line<'_>, range: Option<RangeInclusive<f64>>, step: f64, decimals: u32) {
        let value = self.work.number(line.key).unwrap_or(0.0);
        let option = line.param.option();
        let default = line.param.default_value().and_then(number_of).map(Some);
        line.show(ui, |ui| {
            let mut field = match (option.length(), option.codec()) {
                (Some(lengths), Some(codec)) => NumberField::length(line.name.clone(), line.bind(Some(value), default), lengths).codec(codec.clone()),
                (Some(lengths), None) => NumberField::length(line.name.clone(), line.bind(Some(value), default), lengths),
                (None, codec) => line.scalar_field(self.units, line.bind(Some(value), default), decimals, codec),
            }
            .step(step);
            if let Some(range) = range.clone() {
                field = field.range(range);
            }
            if let Some(arrows) = option.step_arrows() {
                field = field.step_arrows(arrows);
            }
            let shown = field.show(ui);
            self.alert(shown.out_of_range);
            self.unparsed |= shown.edit.unparsed;
            self.note(&shown.edit.events, |v| v.map(FieldValue::Number));
            if let Some(v) = shown.edit.value().cloned().flatten() {
                self.work.set(line.key, FieldValue::Number(v));
            }
        });
    }

    /// A number row with a unit menu: the field shows the base value in the chosen unit.
    fn with_units(&mut self, ui: &mut Ui, line: &Line<'_>) {
        let ParamKind::Number { min, max, step, decimals, .. } = &line.param.kind else { return };
        let option = line.param.option();
        let Some(choices) = option.units() else { return };
        let base = self.work.number(line.key).unwrap_or(0.0);
        let at = self.work.text(choices.key()).and_then(|id| choices.list().iter().position(|unit| unit.id == id)).unwrap_or(choices.initial());
        let Some(chosen) = choices.list().get(at) else { return };
        let factor = chosen.factor;
        let range = bounds(min.map(|m| m / factor), max.map(|m| m / factor));
        let entries: Vec<Entry<UnitKind>> = choices.list().iter().map(|unit| Entry::new(unit.kind, unit.label.clone())).collect();
        let unit_name = self.words.unit.clone();
        if choices.is_shared() {
            line.show(ui, |ui| {
                let mut field = bound_field(self.units, line.name.clone(), line.bind(Some(base / factor), None), *decimals, Some(chosen.kind), None)
                    .step(*step / factor)
                    .id_salt("field");
                if let Some(range) = range.clone() {
                    field = field.range(range);
                }
                let shown = field.show(ui);
                self.alert(shown.out_of_range);
                self.unparsed |= shown.edit.unparsed;
                self.note(&shown.edit.events, |v| v.map(|v| FieldValue::Number(v * factor)));
                if let Some(v) = shown.edit.value().cloned().flatten() {
                    self.work.set(line.key, FieldValue::Number(v * factor));
                }
            });
            return;
        }
        line.show(ui, |ui| {
            let mut field = UnitSelect::new(line.name.clone(), line.bind(Some(base / factor), None), Bind::given(chosen.kind), &entries)
                .unit_label(unit_name.clone())
                .id_salt("field");
            if let Some(range) = range.clone() {
                field = field.range(range);
            }
            let shown = field.show(ui);
            self.alert(shown.out_of_range);
            self.unparsed |= shown.edit.unparsed;
            self.note(&shown.edit.events, |v| v.map(|v| FieldValue::Number(v * factor)));
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
        let default = line.param.default_value().and_then(number_of).map(Some);
        line.show(ui, |ui| {
            beside(ui, |ui| {
                let mut field = line.scalar_field(self.units, line.bind(Some(value as f64), default), WHOLE, None).id_salt("field");
                if let Some(range) = range.clone() {
                    field = field.range(range);
                }
                let shown = field.show(ui);
                self.alert(shown.out_of_range);
                self.unparsed |= shown.edit.unparsed;
                self.note(&shown.edit.events, |v| v.map(|v| FieldValue::Int(v.round() as i64)));
                if let Some(v) = shown.edit.value().cloned().flatten() {
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
        let mut check = Checkbox::new(line.bind(value, Some(default))).label(line.name.clone()).id_salt(line.key).enabled(line.enabled);
        if let Some(tip) = line.param.tooltip_text() {
            check = check.tooltip(tip.clone());
        }
        let shown = check.show(ui);
        self.note(&shown.events, |v| Some(FieldValue::Bool(v)));
        if let Some(v) = shown.value().cloned() {
            self.work.set(line.key, FieldValue::Bool(v));
        }
    }

    fn choice(&mut self, ui: &mut Ui, line: &Line<'_>, options: &[(String, Label)]) {
        let value = self.work.text(line.key).map(str::to_owned).or_else(|| line.param.kind.default_value().and_then(text_of)).unwrap_or_default();
        let entries: Vec<Entry<String>> =
            options.iter().map(|(id, label)| Entry::new(id.clone(), label.clone()).enabled(line.param.option_enabled(id))).collect();
        let searchable = line.param.option().searchable();
        let default = line.param.default_value().and_then(text_of);
        line.show(ui, |ui| {
            let mut select = Select::new(line.name.clone(), line.bind(value.clone(), default.clone()), &entries);
            if searchable {
                select = select.searchable();
            }
            let shown = select.show(ui);
            self.note(&shown.events, |v| Some(FieldValue::Text(v)));
            if let Some(v) = shown.value().cloned() {
                self.work.set(line.key, FieldValue::Text(v));
            }
        });
    }

    /// A select of blend modes. The separators between the mode groups come from the entries' keys, not from the caller.
    fn blend(&mut self, ui: &mut Ui, line: &Line<'_>, modes: &[(BlendKeyEntry<String>, Label)]) {
        let value = self.work.text(line.key).map(str::to_owned).or_else(|| line.param.kind.default_value().and_then(text_of)).unwrap_or_default();
        let keys = blend_key_entries(modes);
        let entries: Vec<Entry<String>> =
            blend_entries(&keys, |entry| modes.iter().find(|(mode, _)| mode.key == entry.key).map_or(Label::Empty, |(_, label)| label.clone()))
                .into_iter()
                .map(|entry| {
                    let enabled = entry.value().is_none_or(|id| line.param.option_enabled(id));
                    entry.enabled(enabled)
                })
                .collect();
        line.show(ui, |ui| {
            let shown = Select::new(line.name.clone(), line.bind(value.clone(), None), &entries).show(ui);
            self.note(&shown.events, |v| Some(FieldValue::Text(v)));
            if let Some(v) = shown.value().cloned() {
                self.work.set(line.key, FieldValue::Text(v));
            }
        });
    }

    /// A pattern row: the stored id when the rows have it as a pattern, else the first pattern is shown. A pick writes
    /// the pattern's id; the group headers of the list take no pick.
    fn pattern(&mut self, ui: &mut Ui, line: &Line<'_>) {
        let none = PresetSource::new((), PresetFns::new(|_, _| Vec::new()));
        let source = self.patterns.unwrap_or(&none);
        let rows = source.rows(ui.ctx(), &Env::of(ui));
        let mut patterns = rows.iter().filter(|row| matches!(row.kind, PresetKind::Item)).map(|row| &row.key);
        let stored = self.work.text(line.key);
        let first = patterns.clone().next();
        let current = patterns.find(|key| Some(key.as_str()) == stored).or(first).cloned();
        // With no pattern to offer the row is disabled and writes nothing.
        let enabled = line.enabled && current.is_some();
        let line = Line { enabled, name: line.name.clone(), helped: Cell::new(false), ..*line };
        line.show(ui, |ui| {
            let picked = PresetPicker::pattern(line.name.clone(), Bind::given(current.clone()), source, Bind::kept()).show(ui).picked;
            if let Some(id) = picked.filter(|id| rows.iter().any(|row| matches!(row.kind, PresetKind::Item) && row.key == *id)) {
                self.work.set(line.key, FieldValue::Text(id));
            }
        });
    }

    /// A files row: a `FileListPicker` over the row's keys. A change of folder, or Folder mode turned on with a folder
    /// set, drops the old listing and files a request for the new one in the same edit, so the rows keep no memo of it.
    fn files(&mut self, ui: &mut Ui, line: &Line<'_>, folder_mode: bool, request: &PickRequest, open_files: &[PathBuf]) {
        let param = line.param;
        let (folder_key, use_key, listed_key) = (param.folder_key(), param.use_folder_key(), param.folder_files_key());
        let files = paths_at(&self.work, line.key);
        let folder = self.work.text(&folder_key).map(PathBuf::from);
        let use_folder = folder_mode && self.work.bool(&use_key).unwrap_or(false);
        let listed = paths_at(&self.work, &listed_key);
        let before_work = self.work.clone();
        let mut list = FileList {
            mode: if use_folder { FileListMode::Folder } else { FileListMode::Files },
            files: files.clone(),
            folder: folder.clone(),
            selected: self.memo.selected.get(line.key).filter(|path| files.contains(path)).cloned(),
        };
        line.show(ui, |ui| {
            let shown = FileListPicker::new(line.name.clone(), Bind::bound(&mut list))
                .folder_mode(folder_mode)
                .open_files(open_files)
                .folder_files(&listed)
                .request(request.clone())
                .show(ui);
            if !shown.changed {
                return;
            }
            if list.files != files {
                self.work.set(line.key, path_list(&list.files));
            }
            let now_folder = list.mode == FileListMode::Folder;
            if now_folder != use_folder {
                self.work.set(use_key.as_str(), FieldValue::Bool(now_folder));
            }
            if list.folder != folder
                && let Some(path) = &list.folder
            {
                self.work.set(folder_key.as_str(), FieldValue::Text(path.to_string_lossy().into_owned()));
            }
            if (list.folder != folder || now_folder != use_folder) && !listed.is_empty() {
                self.work.remove(&listed_key);
            }
            if let Some(request) = param.listing_after(&before_work, &self.work) {
                self.listing = Some(request);
            }
        });
        match list.selected {
            Some(path) => self.memo.selected.insert(line.key.to_owned(), path),
            None => self.memo.selected.remove(line.key),
        };
    }

    /// A radio row is its own section: the title shows the row's name once, and the group it names follows. A row with
    /// [`options_in_row`](Param::options_in_row) has no title and stands its options side by side.
    fn radio_options(&mut self, ui: &mut Ui, line: &Line<'_>, options: &[(String, Label)], details: &[Label]) {
        if line.param.option().options_in_row() {
            self.radio_group(ui, line, options, details, true);
            return;
        }
        Section::fixed(line.name.clone()).strength(HeaderStrength::Caption).id_salt(line.key).show(ui, |ui, _| {
            self.radio_group(ui, line, options, details, false);
        });
    }

    /// The radio group of `line`, its options in a column, or side by side when `in_row`.
    fn radio_group(&mut self, ui: &mut Ui, line: &Line<'_>, options: &[(String, Label)], details: &[Label], in_row: bool) {
        let default = line.param.kind.default_value().and_then(text_of);
        let value = self.work.text(line.key).map(str::to_owned).or_else(|| default.clone()).unwrap_or_default();
        let mut group = RadioGroup::new(line.name.clone(), line.bind(value, default)).enabled(line.enabled);
        if let Some(tip) = line.param.tooltip_text() {
            group = group.tooltip(tip.clone());
        }
        let group = group.show(ui, |ui, cx| {
            let mut radios = |ui: &mut Ui| {
                for (at, (id, label)) in options.iter().enumerate() {
                    let radio = Radio::new(id.clone()).enabled(line.param.option_enabled(id) && line.enabled);
                    match details.get(at) {
                        // An option with a detail line draws it under its label through the body's label step.
                        Some(detail) => {
                            let side = SideLabel::new().layout(FieldLayout::After).detail(detail.clone());
                            cx.labelled(ui, id, label.clone(), side, radio);
                        }
                        None => {
                            radio.label(label.clone()).show(ui);
                        }
                    }
                }
            };
            if in_row {
                Row::new().show(ui, |ui, _| radios(ui));
            } else {
                Stack::new().show(ui, |ui, _| radios(ui));
            }
        });
        self.note(&group.output.events, |v| Some(FieldValue::Text(v)));
        if let Some(v) = group.output.value().cloned() {
            self.work.set(line.key, FieldValue::Text(v));
        }
    }

    /// An angle row: the dial and the field over `range` at `decimals` digits. The field clamps a typed angle into
    /// the range and raises the range alert.
    fn angle(&mut self, ui: &mut Ui, line: &Line<'_>, default: f64, range: std::ops::RangeInclusive<f64>, decimals: u32) {
        let value = self.work.number(line.key).unwrap_or(default);
        let field_name = Label::from(self.words.angle_field.fill(&[("name", line.name.as_str())]));
        let default = Some(default);
        line.show(ui, |ui| {
            beside(ui, |ui| {
                let dial = AngleDial::new(line.bind(value, default)).label(line.name.clone()).range(range.clone()).id_salt("dial").show(ui);
                let field =
                    bound_field(self.units, field_name.clone(), line.bind(Some(value), default.map(Some)), decimals, Some(UnitKind::Degrees), None)
                        .range(range.clone())
                        .commit_decimals(decimals as usize)
                        .id_salt("field");
                let shown = field.show(ui);
                self.alert(shown.out_of_range);
                self.unparsed |= shown.edit.unparsed;
                self.note(&dial.events, |v| Some(FieldValue::Number(v)));
                self.note(&shown.edit.events, |v| v.map(FieldValue::Number));
                let typed = shown.edit.value().cloned().flatten();
                if let Some(v) = dial.value().cloned().or(typed) {
                    self.work.set(line.key, FieldValue::Number(v));
                }
            });
        });
    }

    /// A colour row: a swatch that asks the host's picker (mixed while the row has no colour), and the eyedropper
    /// toggle when the row has one.
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
        let previewed = self.previews.iter().find(|(key, _)| key == line.key).map(|(_, color)| *color);
        let shown = match previewed {
            Some(color) => Mixed::Same(color),
            None => color.map_or(Mixed::Mixed, |rgb| Mixed::Same(Srgba::from_rgb_array(rgb))),
        };
        line.show(ui, |ui| {
            beside(ui, |ui| {
                let swatch = Swatch::new(line.name.clone(), shown).picker(line.key).id_salt("choose").show(ui);
                if let Some(request) = swatch.pick_color {
                    self.color = Some(request);
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
                            let shown = cell.show(ui);
                            self.unparsed |= shown.edit.unparsed;
                            if let Some(v) = shown.edit.value().cloned().flatten() {
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
                    let shown = field.show(ui);
                    self.alert(shown.out_of_range);
                    self.unparsed |= shown.edit.unparsed;
                    if let Some(v) = shown.edit.value().cloned().flatten() {
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
            let shown = Select::new(line.name.clone(), line.bind(value, None), &entries).show(ui);
            if let Some(v) = shown.value().cloned() {
                self.work.set(line.key, FieldValue::Int(i64::try_from(v).unwrap_or(0)));
            }
        });
    }

    fn text(&mut self, ui: &mut Ui, line: &Line<'_>, default: &str) {
        let value = self.work.text(line.key).unwrap_or(default).to_owned();
        let browse = line.param.browse.clone();
        line.show(ui, |ui| {
            let typed = match &browse {
                Some(kind) => {
                    let mut row = PathRow::new(line.name.clone(), Bind::given(value.clone()));
                    if let Some(hint) = line.param.hint_text() {
                        row = row.placeholder(hint.clone());
                    }
                    let shown = row.show(ui);
                    if shown.browse {
                        self.path = Some(PathRequest::new(line.key, kind.clone()));
                    }
                    shown.edit.value().cloned()
                }
                None => {
                    let mut field = TextField::new(line.name.clone(), line.bind(value.clone(), Some(default.to_owned()))).id_salt("text");
                    let options = line.param.option();
                    if let Some(lines) = options.lines() {
                        field = field.lines(lines);
                    }
                    if let Some(max) = options.max_chars() {
                        field = field.limit(Some(max));
                    }
                    if options.mono() {
                        field = field.mono();
                    }
                    if let Some(chars) = options.field_chars() {
                        field = field.width_chars(chars);
                    }
                    if let Some(hint) = line.param.hint_text() {
                        field = field.placeholder(hint.clone());
                    }
                    field.show(ui).value().cloned()
                }
            };
            if let Some(v) = typed {
                self.work.set(line.key, FieldValue::Text(v));
            }
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
                let field = line.scalar_field(self.units, Bind::given(Some(last)), decimals, None).range(range.clone());
                let field = field.id_salt("field").enabled(self.work.number(line.key).is_some());
                let shown = field.show(ui);
                self.alert(shown.out_of_range);
                self.unparsed |= shown.edit.unparsed;
                if let Some(v) = shown.edit.value().cloned().flatten() {
                    self.work.set(line.key, FieldValue::Number(v));
                }
            });
        });
    }

    fn custom(&mut self, ui: &mut Ui, line: &Line<'_>) {
        let ParamKind::Custom(row) = &line.param.kind else { return };
        let mut context = CustomContext::new(line.name.clone(), self.canvas, self.ended_prev);
        let before = self.work.clone();
        line.show_body(ui, |ui| {
            let _ = row.draw(ui, &mut self.work, &mut context);
        });
        for step in context.take_gestures() {
            self.events.push(lifted_edit(step, &before, &self.work, &line.name));
        }
        if let Some(request) = context.take_request() {
            self.color = Some(request);
        }
        if let Some(request) = context.take_eyedropper() {
            self.memo.armed = Some(request.key.clone());
            self.eyedropper = Some(request);
        }
        self.sampling |= context.is_sampling();
        self.actions.extend(context.take_actions());
    }
}

/// Read-only text beside the row's label, or across the form for a full-width row, inset like a field's text.
fn read_only_text(ui: &mut Ui, line: &Line<'_>, text: &Label) {
    let shown = Env::of(ui).view().resolve(text);
    line.show_body(ui, |ui| {
        SelectableText::new(line.name.clone(), shown.as_str()).multiline().field_inset().show(ui);
    });
}

/// A custom row's control event as an edit of the whole values: they stood at `before` until the row drew and at
/// `after` since.
fn lifted_edit(step: Lifted, before: &DialogFields, after: &DialogFields, label: &Label) -> EditEvent<DialogFields> {
    let was = Mixed::Same(before.clone());
    match (step.id, step.phase) {
        (Some(id), Some(GesturePhase::Begin)) => EditEvent::begin(id, was, label.as_str().into(), None),
        (Some(id), Some(GesturePhase::Update)) => EditEvent::update(id, after.clone()),
        (Some(id), Some(GesturePhase::Commit)) => EditEvent::commit(id, was, after.clone()),
        (Some(id), Some(GesturePhase::Cancel)) => EditEvent::cancel(id, step.reason.unwrap_or(CancelReason::Dismissed), Some(was)),
        _ => EditEvent::set(was, after.clone(), label.as_str().into(), None),
    }
}

/// The mode entries of a Blend row's `modes`, in menu order.
fn blend_key_entries(modes: &[(BlendKeyEntry<String>, Label)]) -> Vec<BlendKeyEntry<String>> {
    modes.iter().map(|(entry, _)| entry.clone()).collect()
}

/// The paths a files row keeps under `key`.
fn paths_at(fields: &DialogFields, key: &str) -> Vec<PathBuf> {
    let items = fields.list(key);
    items
        .iter()
        .flat_map(|items| items.iter())
        .filter_map(|item| match item {
            FieldValue::Text(text) => Some(PathBuf::from(text)),
            _ => None,
        })
        .collect()
}

/// Shows `body` as one row of controls side by side.
fn beside(ui: &mut Ui, mut body: impl FnMut(&mut Ui)) {
    Row::new().show(ui, |ui, _| body(ui));
}

/// A unit of a section's rows: one row, or a field and the one it pairs with ([`Param::beside_next`]).
#[derive(Clone, Copy)]
enum Unit<'a> {
    One(&'a Param),
    Pair(&'a Param, &'a Param),
}

impl<'a> Unit<'a> {
    /// The unit's first row: the one that may stand beside the unit before it ([`Param::beside`]).
    fn first(self) -> &'a Param {
        match self {
            Unit::One(param) | Unit::Pair(param, _) => param,
        }
    }

    /// The unit's rows.
    fn params(self) -> impl Iterator<Item = &'a Param> {
        let (first, second) = match self {
            Unit::One(param) => (param, None),
            Unit::Pair(param, next) => (param, Some(next)),
        };
        std::iter::once(first).chain(second)
    }
}

/// A number field named `name` that reads and writes `decimals` digits and the spellings of `unit`.
fn number_field(units: &Units, name: Label, value: Option<f64>, decimals: u32, unit: Option<UnitKind>) -> NumberField<'static> {
    bound_field(units, name, Bind::given(value), decimals, unit, None)
}

/// [`number_field`] over `value`, read and written with `codec` when the row has one.
fn bound_field(
    units: &Units,
    name: Label,
    value: Bind<'static, Option<f64>>,
    decimals: u32,
    unit: Option<UnitKind>,
    codec: Option<&NumberCodec>,
) -> NumberField<'static> {
    let codec = codec.cloned().unwrap_or_else(|| NumberCodec::new(units.reader(unit), decimals as usize));
    NumberField::new(name, value).codec(codec)
}

/// The number `value` holds, an int as a number.
fn number_of(value: FieldValue) -> Option<f64> {
    match value {
        FieldValue::Number(n) => Some(n),
        FieldValue::Int(i) => Some(i as f64),
        _ => None,
    }
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
