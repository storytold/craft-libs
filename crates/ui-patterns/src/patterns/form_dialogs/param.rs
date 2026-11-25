//! The parameter model of the form dialogs: [`Param`] and [`ParamKind`] describe one row of a parameter form,
//! [`Param::parse_list`] builds a list from a spec string, and the requests a row can make of its host
//! (`runtime::requests`) are plain values.

mod parse;

use super::fields::{DialogFields, FieldValue};
use crate::contract::Tooltip;
use crate::patterns::blend_keys::BlendKeyEntry;
use crate::runtime::env::{Label, LengthUnits, UnitKind};
use crate::runtime::requests::{ColorRequest, EyedropperRequest, ListingRequest, PathKind};
use crate::text::StepArrows;
use crate::value_drag::SliderScale;
use craft_behavior::{CancelReason, EditEvent, EditEvents, GestureId, GesturePhase, NumberCodec};
use craft_host::PickRequest;
use craft_theme::Srgba;
use egui::{Response, Ui};
use std::fmt;
use std::path::PathBuf;
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
    /// A group of radio buttons over `(id, label)` options; the field holds the id. The row is its own section, titled
    /// by the parameter's label, so it needs no `Param::section` before it.
    #[non_exhaustive]
    Radio {
        /// The `(id, label)` pairs in list order.
        options: Vec<(String, Label)>,
        /// The id selected when nothing is remembered; `None` selects the first option.
        default: Option<String>,
        /// The detail line under each option, in option order; empty for a group without details.
        details: Vec<Label>,
    },
    /// An angle dial with its field, in degrees.
    #[non_exhaustive]
    Angle {
        /// The angle in degrees shown when nothing is remembered.
        default: f64,
        /// The least angle; the field clamps below it and says so.
        min: f64,
        /// The greatest angle; the field clamps above it and says so.
        max: f64,
        /// The digits after the decimal point the field keeps; 0 takes whole degrees.
        decimals: u32,
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
    /// Read-only text the user can select and copy: see [`Param::read_only_text`]. It has no value.
    #[non_exhaustive]
    ReadOnlyText {
        /// The text shown; line breaks start new lines.
        text: Label,
    },
    /// A row the host draws: see [`Param::custom`]. Its value and its companions' are what `draw` keeps.
    Custom(CustomRow),
    /// A select of blend modes, each with the Shift+Alt key that picks it.
    #[non_exhaustive]
    Blend {
        /// The modes: their key entries (id and shortcut) and labels.
        modes: Vec<(BlendKeyEntry<String>, Label)>,
        /// The id of the mode shown when nothing is remembered; the first mode when `None`.
        default: Option<String>,
    },
    /// A pattern chosen from the dialog's pattern list (`ParamDialog::patterns`); its value is the pattern's id.
    #[non_exhaustive]
    Pattern {
        /// The id of the pattern shown when nothing is remembered.
        default: Option<String>,
    },
    /// A list of files or one folder (a `FileListPicker`); its value is a `{files, folder, useFolder}` object.
    #[non_exhaustive]
    Files {
        /// Whether the list picks a folder.
        folder_mode: bool,
        /// The file dialog request Browse makes.
        request: PickRequest,
        /// The paths of the documents already open, which the list offers to add.
        open_files: Vec<PathBuf>,
    },
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

    /// A group of radio buttons over `(id, label)` options, in a section of its own titled by the parameter's label.
    pub fn radio(options: Vec<(String, Label)>, default: Option<String>) -> Self {
        ParamKind::Radio { options, default, details: Vec::new() }
    }

    /// A group of radio buttons whose options each carry a detail line: `(id, label, detail)`.
    pub fn radio_with_details(options: Vec<(String, Label, Label)>, default: Option<String>) -> Self {
        let (options, details) = options.into_iter().map(|(id, label, detail)| ((id, label), detail)).unzip();
        ParamKind::Radio { options, default, details }
    }

    /// An angle dial with its field: one turn, -180 to 180, to a tenth of a degree.
    pub fn angle(default: f64) -> Self {
        ParamKind::Angle { default, min: -180.0, max: 180.0, decimals: 1 }
    }

    /// An angle dial with its field over `min` to `max` degrees at `decimals` digits: Global Light's -360 to 360 in
    /// whole degrees. A range wider than a turn does not wrap, so 180 stays 180.
    pub fn angle_in(min: f64, max: f64, default: f64, decimals: u32) -> Self {
        ParamKind::Angle { default, min, max, decimals }
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

    /// A select of blend `modes` with their keys; `default` is the id of the mode shown at first.
    pub fn blend(modes: Vec<(BlendKeyEntry<String>, Label)>, default: Option<String>) -> Self {
        ParamKind::Blend { modes, default }
    }

    /// A pattern chosen from the dialog's pattern list; `default` is the id shown at first.
    pub fn pattern(default: Option<String>) -> Self {
        ParamKind::Pattern { default }
    }

    /// Fields per row of a [`ParamKind::Grid`] of `cells`: `round(sqrt(cells))`, at least 1.
    pub fn grid_side(cells: usize) -> usize {
        ((cells as f32).sqrt().round() as usize).max(1)
    }

    /// The value a field of this kind starts with.
    pub fn default_value(&self) -> Option<FieldValue> {
        Some(match self {
            ParamKind::Range { default, .. } | ParamKind::Angle { default, .. } => FieldValue::Number(*default),
            ParamKind::Int { default, .. } => FieldValue::Int(*default),
            ParamKind::Number { default, .. } => FieldValue::Number(*default),
            ParamKind::Bool { default } => FieldValue::Bool(*default),
            ParamKind::Choice { options, default } | ParamKind::Radio { options, default, .. } => {
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
            ParamKind::Blend { modes, default } => {
                let initial = default.as_deref().or_else(|| modes.first().map(|(entry, _)| entry.key.as_str()));
                FieldValue::Text(initial.unwrap_or_default().to_owned())
            }
            ParamKind::Pattern { default } => FieldValue::Text(default.clone()?),
            ParamKind::Files { open_files, .. } => path_list(open_files),
            ParamKind::Custom(row) => row.opens_with.clone()?,
            ParamKind::ReadOnlyText { .. } | ParamKind::Section => return None,
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
            (ParamKind::Angle { min, max, .. }, v) => number(v).is_some_and(|n| n.is_finite() && n >= *min && n <= *max),
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
            // A custom row keeps whatever its `draw` writes.
            (ParamKind::Custom(_), _) => true,
            (ParamKind::Blend { modes, .. }, FieldValue::Text(id)) => modes.iter().any(|(entry, _)| entry.key == *id),
            (ParamKind::Pattern { .. }, FieldValue::Text(_)) => true,
            (ParamKind::Files { .. }, v) => is_path_list(v),
            _ => false,
        }
    }
}

type CustomDraw = dyn Fn(&mut Ui, &mut DialogFields, &mut CustomContext) -> Response + Send + Sync;

/// A [`Param::custom`] row's set-time rule ([`Param::on_set`]).
pub(crate) type SetRule = fn(&mut DialogFields, &str, serde_json::Value) -> Result<(), String>;

/// What draws a [`ParamKind::Custom`] row: it gets the row's control cell, the form's values and the row's
/// [`CustomContext`], and returns the response the row label names. Besides the row's own key, `draw` may keep values
/// under its companion keys ([`Param::companions`]). Equal only to a clone of itself with the same value and companions.
#[derive(Clone)]
pub struct CustomRow {
    draw: Arc<CustomDraw>,
    /// The value the row's key opens with ([`Param::opens_with`]).
    opens_with: Option<FieldValue>,
    /// The other keys `draw` keeps values under.
    companions: Vec<String>,
    /// The rule a value set from outside the row goes through ([`Param::on_set`]).
    on_set: Option<SetRule>,
}

impl CustomRow {
    /// Wraps `draw`, which draws the row and edits the values it owns through the [`DialogFields`].
    pub fn new(draw: impl Fn(&mut Ui, &mut DialogFields, &mut CustomContext) -> Response + Send + Sync + 'static) -> Self {
        Self { draw: Arc::new(draw), opens_with: None, companions: Vec::new(), on_set: None }
    }

    /// Draws the row.
    pub(crate) fn draw(&self, ui: &mut Ui, fields: &mut DialogFields, context: &mut CustomContext) -> Response {
        (self.draw)(ui, fields, context)
    }

    /// Whether `key` is one of the row's companion keys.
    pub(crate) fn has_companion(&self, key: &str) -> bool {
        self.companions.iter().any(|companion| companion == key)
    }

    /// The rule a value set from outside the row goes through, if the row has one.
    pub(crate) fn on_set(&self) -> Option<SetRule> {
        self.on_set
    }
}

impl fmt::Debug for CustomRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomRow").field("opens_with", &self.opens_with).field("companions", &self.companions).finish_non_exhaustive()
    }
}

impl PartialEq for CustomRow {
    fn eq(&self, other: &Self) -> bool {
        let same_rule = match (self.on_set, other.on_set) {
            (Some(a), Some(b)) => std::ptr::fn_addr_eq(a, b),
            (a, b) => a.is_none() && b.is_none(),
        };
        Arc::ptr_eq(&self.draw, &other.draw) && self.opens_with == other.opens_with && self.companions == other.companions && same_rule
    }
}

/// What a [`Param::custom`] row can ask of the host besides editing the values.
#[derive(Debug)]
#[non_exhaustive]
pub struct CustomContext {
    title: Label,
    request: Option<ColorRequest>,
    canvas: Option<egui::Id>,
    eyedropper: Option<EyedropperRequest>,
    sampling: bool,
    /// Whether an Escape ended sampling in the previous pass.
    sampling_ended: bool,
    /// The gesture steps the row's controls reported ([`CustomContext::gesture`]).
    gestures: Vec<Lifted>,
    /// The ids of the actions the row asked the host to run ([`CustomContext::request_action`]).
    actions: Vec<String>,
    action: Option<String>,
}

/// One edit event of a custom row's control with its value erased.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lifted {
    /// The gesture, `None` for a one-step set.
    pub(crate) id: Option<GestureId>,
    /// Where the gesture stands, `None` for a set.
    pub(crate) phase: Option<GesturePhase>,
    /// Why a cancel ended it.
    pub(crate) reason: Option<CancelReason>,
}

impl CustomContext {
    /// The context of a row labelled `title`; `sampling_ended` says an Escape ended sampling in the previous pass.
    pub(crate) fn new(title: Label, canvas: Option<egui::Id>, sampling_ended: bool) -> Self {
        CustomContext {
            title,
            request: None,
            canvas,
            eyedropper: None,
            sampling: false,
            sampling_ended,
            gestures: Vec::new(),
            actions: Vec::new(),
            action: None,
        }
    }

    /// The gesture steps the row reported, once.
    pub(crate) fn take_gestures(&mut self) -> Vec<Lifted> {
        std::mem::take(&mut self.gestures)
    }

    /// Asks the host to run the action `id`; the host takes this pass's requests once with
    /// `HostedParamDialog::take_actions` or `HostedParamDialog::take_action`.
    pub fn request_action(&mut self, id: impl Into<String>) {
        let id = id.into();
        self.actions.push(id.clone());
        self.action = Some(id);
    }

    /// The ids of the actions the row asked for, once.
    pub(crate) fn take_actions(&mut self) -> Vec<String> {
        std::mem::take(&mut self.actions)
    }

    /// Arms the host's eyedropper for the colour under `request.key`, as a colour row's dropper toggle does.
    pub fn arm_eyedropper(&mut self, request: EyedropperRequest) {
        self.eyedropper = Some(request);
    }

    /// Whether an Escape ended the row's sampling in the previous pass (the dialog claims Escape while a row samples).
    pub fn sampling_ended(&self) -> bool {
        self.sampling_ended
    }

    /// Whether the row's dropper samples now: `true` while the row wants canvas presses to pick colours.
    pub fn sampling(&mut self, on: bool) {
        self.sampling = on;
    }

    /// Hands a custom row's control events to the rows, which lift them as a value row's: each event's gesture id,
    /// phase and cancel reason, type-erased.
    pub fn gesture<T>(&mut self, events: &EditEvents<T>) {
        self.gestures.extend(events.iter().map(|event| Lifted {
            id: event.id(),
            phase: event.phase(),
            reason: match event {
                EditEvent::Cancel { reason, .. } => Some(*reason),
                _ => None,
            },
        }));
    }

    /// The canvas the dialog's rows work on (`ParamDialog::canvas`), when it has one.
    pub fn canvas(&self) -> Option<egui::Id> {
        self.canvas
    }

    /// Asks the host for its colour picker, open at `color` (the colour the row shows) and titled with the row's label;
    /// the host answers by writing the picked colour under `key`, which needs no row of its own. The request is
    /// reported once, as a colour row's press is (`ParamRowsOutput::pick_color`).
    pub fn pick_color(&mut self, key: &str, color: Srgba) {
        self.request = Some(ColorRequest::new(key, color).title(self.title.clone()));
    }

    /// The action the row asked for, once.
    pub(crate) fn take_action(&mut self) -> Option<String> {
        self.action.take()
    }

    /// The dropper the row armed, once.
    pub(crate) fn take_eyedropper(&mut self) -> Option<EyedropperRequest> {
        self.eyedropper.take()
    }

    /// Whether the row asked for canvas presses to pick colours.
    pub(crate) fn is_sampling(&self) -> bool {
        self.sampling
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
    pub(crate) popup_slider: bool,
    randomize: bool,
    eyedropper: bool,
    eyedropper_armed: bool,
    full_width: bool,
    /// The row or section stands beside the one before it ([`Param::beside`]).
    beside: bool,
    /// The field pairs with the next one in its section ([`Param::beside_next`]).
    beside_next: bool,
    /// A range row's slider is short and its number field content-wide ([`Param::short_slider`]).
    short_slider: bool,
    /// A radio row's options stand in one row with no caption ([`Param::options_in_row`]).
    options_in_row: bool,
    /// A radio row's options stand in a column with no caption, in a document dialog too ([`Param::stacked_options`]).
    stacked_options: bool,
    /// The option of the radio row before this one that the row stands under ([`Param::under_option`]).
    under_option: Option<String>,
    not_remembered: bool,
    /// The lines a multi-line text row shows.
    lines: Option<usize>,
    /// The most characters a text row takes ([`Param::max_chars`]).
    max_chars: Option<usize>,
    /// Whether a text row shows its text in the monospace face ([`Param::mono`]).
    mono: bool,
    /// Whether a text row selects its text when it takes the focus ([`Param::select_on_focus`]).
    select_on_focus: bool,
    /// The width a text row's field asks for, in characters ([`Param::field_chars`]).
    field_chars: Option<u16>,
    units: Option<UnitChoices>,
    /// The ids of the options a choice row shows disabled.
    disabled_options: Vec<String>,
    /// Why an option of `disabled_options` is disabled, by option id ([`Param::disabled_option`]).
    disabled_reasons: Vec<(String, Label)>,
    /// The description of an option of a choice or radio row, by option id ([`Param::option_description`]).
    option_descriptions: Vec<(String, Label)>,
    /// The row's select has a search field.
    searchable: bool,
    /// The indexes of the options a choice row draws a separator before ([`Param::separated_at`]).
    separated_at: Vec<usize>,
    /// The keys a change of the row's value removes ([`Param::resets`]).
    resets: Vec<String>,
    /// The row takes input only while the number under the key is above the bound ([`Param::enabled_above`]).
    enabled_above: Option<(String, f64)>,
    /// The row takes input only while the values under the two keys differ ([`Param::enabled_when_differs`]).
    enabled_when_differs: Option<(String, String)>,
    /// The value the row shows while it is disabled ([`Param::disabled_value`]).
    disabled_value: Option<FieldValue>,
    /// How a section head draws ([`Param::group`], [`Param::section_check`]).
    head: Head,
    /// The row's label shows a reset dot ([`Param::resettable`]).
    resettable: bool,
    /// The label a footer row shows while on ([`Param::in_footer`]).
    footer_on: Option<Label>,
    /// The lengths a number row keeps and shows ([`Param::length`]).
    length: Option<LengthUnits>,
    /// The codec a number row reads and writes with ([`Param::codec`]).
    codec: Option<NumberCodec>,
    /// The step arrows of a number row's field.
    step_arrows: Option<StepArrows>,
    /// The labelled marks of a range row's slider.
    marks: Vec<(f64, Label)>,
    /// The link the row's help button reports ([`Param::help_link`]).
    help_link: Option<String>,
}

/// How a section head draws: a fixed title, a disclosure over its rows, or a check box that switches its rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Head {
    /// A title over a rule.
    #[default]
    Fixed,
    /// A title that opens and closes the rows.
    Group {
        /// Whether the disclosure starts open.
        open: bool,
    },
    /// A check box over the rows, which take input only while it is checked.
    Check,
}

impl RowOptions {
    /// Whether the row or section stands beside the one before it.
    pub(crate) fn beside(&self) -> bool {
        self.beside
    }

    /// How a section head draws.
    pub(crate) fn head(&self) -> Head {
        self.head
    }

    /// Whether the row's label shows a reset dot.
    pub(crate) fn resettable(&self) -> bool {
        self.resettable
    }

    /// The lengths a number row keeps and shows.
    pub(crate) fn length(&self) -> Option<LengthUnits> {
        self.length
    }

    /// The codec a number row reads and writes with.
    pub(crate) fn codec(&self) -> Option<&NumberCodec> {
        self.codec.as_ref()
    }

    /// The step arrows of a number row's field.
    pub(crate) fn step_arrows(&self) -> Option<StepArrows> {
        self.step_arrows
    }

    /// The labelled marks of a range row's slider.
    pub(crate) fn marks(&self) -> &[(f64, Label)] {
        &self.marks
    }

    /// The link the row's help button reports.
    pub(crate) fn help_link(&self) -> Option<&str> {
        self.help_link.as_deref()
    }
}

/// The units of a number row ([`Param::number_with_units`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UnitChoices {
    units: Vec<NumberUnit>,
    /// The field key the chosen unit's id is stored under.
    key: String,
    initial: usize,
    /// The unit id is the value of another row (a choice row), not of a menu of this row.
    shared: bool,
}

/// The value that holds `paths`: a list of text, the shape a [`Param::files`] row keeps its files in.
pub(crate) fn path_list(paths: &[PathBuf]) -> FieldValue {
    FieldValue::List(paths.iter().map(|path| FieldValue::Text(path.to_string_lossy().into_owned())).collect())
}

/// Whether `value` is a list of paths. An empty list stored and read back is a `Grid` of no cells.
fn is_path_list(value: &FieldValue) -> bool {
    match value {
        FieldValue::List(items) => items.iter().all(|item| matches!(item, FieldValue::Text(_))),
        FieldValue::Grid(cells) => cells.is_empty(),
        _ => false,
    }
}

/// Texts a row may carry beyond its label.
#[derive(Clone, Debug, Default, PartialEq)]
struct RowTexts {
    hint: Option<Label>,
    tooltip: Option<Tooltip>,
}

/// When a row takes input.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum Enable {
    /// Always.
    #[default]
    Always,
    /// While `fields[key]` equals one of the values.
    When(String, Vec<FieldValue>),
    /// Never.
    Off,
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
    /// When the row takes input: always, while `fields[key]` equals one of the values, or never.
    pub enable: Enable,
    /// The row is shown only while every condition holds: `fields[key]` equals the value, or one of the values
    /// ([`Param::visible_when`]). A hidden row takes no space; its value stays in the fields and in the result.
    pub visible_when: Vec<(String, Vec<FieldValue>)>,
    /// Whether the row is shown at all, whatever the values ([`Param::shown`]): what decides it lies outside the
    /// values, such as what a host's job found. A hidden row takes no space; its value stays in the fields and in the
    /// result.
    pub shown: bool,
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
            enable: Enable::Always,
            visible_when: Vec::new(),
            shown: true,
            browse: None,
            texts: None,
            options: None,
        }
    }

    /// A row of files or one folder: a [`ParamKind::Files`] list the user fills with Browse.
    pub fn files(key: impl Into<String>, label: impl Into<Label>) -> Param {
        let key = key.into();
        let kind = ParamKind::Files { folder_mode: false, request: PickRequest::open_many(key.clone()), open_files: Vec::new() };
        Self::new(key, label, kind)
    }

    /// Whether a [`Param::files`] row picks one folder instead of files.
    #[must_use]
    pub fn folder_mode(mut self, on: bool) -> Self {
        if let ParamKind::Files { folder_mode, .. } = &mut self.kind {
            *folder_mode = on;
        }
        self
    }

    /// The paths of the documents already open, which a [`Param::files`] row offers to add.
    #[must_use]
    pub fn open_files(mut self, paths: Vec<PathBuf>) -> Self {
        if let ParamKind::Files { open_files, .. } = &mut self.kind {
            *open_files = paths;
        }
        self
    }

    /// Shows the options `ids` of a choice row disabled.
    #[must_use]
    pub fn disabled_options(mut self, ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.options.get_or_insert_default().disabled_options = ids.into_iter().map(Into::into).collect();
        self
    }

    /// Whether the option `id` of a choice row takes a pick.
    pub fn option_enabled(&self, id: &str) -> bool {
        self.options.as_deref().is_none_or(|options| !options.disabled_options.iter().any(|disabled| disabled == id))
    }

    /// Shows `text` as the description of the option `id` of a choice or radio row.
    #[must_use]
    pub fn option_description(mut self, id: impl Into<String>, text: impl Into<Label>) -> Self {
        keyed_text(&mut self.options.get_or_insert_default().option_descriptions, id.into(), text.into());
        self
    }

    /// Shows the option `id` disabled, with `reason` as its tooltip. It joins the options
    /// [`disabled_options`](Param::disabled_options) lists, so automation refuses it as it refuses those.
    #[must_use]
    pub fn disabled_option(mut self, id: impl Into<String>, reason: impl Into<Label>) -> Self {
        let options = self.options.get_or_insert_default();
        let id = id.into();
        if !options.disabled_options.contains(&id) {
            options.disabled_options.push(id.clone());
        }
        keyed_text(&mut options.disabled_reasons, id, reason.into());
        self
    }

    /// The description of the option `id` of a choice or radio row ([`option_description`](Param::option_description)).
    pub(crate) fn option_description_text(&self, id: &str) -> Option<&Label> {
        self.options.as_deref()?.option_descriptions.iter().find(|(held, _)| held == id).map(|(_, text)| text)
    }

    /// Why the option `id` is disabled, while it is ([`disabled_option`](Param::disabled_option)).
    pub(crate) fn disabled_reason(&self, id: &str) -> Option<&Label> {
        if self.option_enabled(id) {
            return None;
        }
        self.options.as_deref()?.disabled_reasons.iter().find(|(held, _)| held == id).map(|(_, reason)| reason)
    }

    /// The row never takes input: it is shown disabled.
    #[must_use]
    pub fn disabled(mut self) -> Self {
        self.enable = Enable::Off;
        self
    }

    /// Gives a choice row's select a search field.
    #[must_use]
    pub fn searchable(mut self) -> Self {
        self.options.get_or_insert_default().searchable = true;
        self
    }

    /// Draws a separator line before the option at `index` in a choice row's open list, between two groups of options
    /// (standard and custom paper sizes). Repeat the call for each group start. An `index` of 0, or past the last
    /// option, draws nothing: a separator sits between options.
    #[must_use]
    pub fn separated_at(mut self, index: usize) -> Self {
        let separated = &mut self.options.get_or_insert_default().separated_at;
        if !separated.contains(&index) {
            separated.push(index);
        }
        self
    }

    /// The keys of the rows that depend on this one (Match Color's source layer on its source document): changing this
    /// row's value to a different one, through the row, automation (`set_param_field`) or a host answer, removes them,
    /// so those rows start over from their default. Setting the value it holds keeps them.
    #[must_use]
    pub fn resets(mut self, keys: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.options.get_or_insert_default().resets.extend(keys.into_iter().map(Into::into));
        self
    }

    /// Removes the keys the row [`resets`](Param::resets) when its value in `fields` differs from `before`, the value
    /// it held before the write.
    pub(crate) fn drop_dependents(&self, before: Option<&FieldValue>, fields: &mut DialogFields) {
        let Some(options) = self.options.as_deref().filter(|options| !options.resets.is_empty()) else { return };
        if fields.get(&self.key) != before {
            for key in &options.resets {
                fields.remove(key);
            }
        }
    }

    /// The row takes input only while `fields[key]` equals one of `values`; otherwise it is shown disabled.
    #[must_use]
    pub fn enabled_when_in(mut self, key: impl Into<String>, values: impl IntoIterator<Item = impl Into<FieldValue>>) -> Self {
        self.enable = Enable::When(key.into(), values.into_iter().map(Into::into).collect());
        self
    }

    /// The row takes input only while the number `fields[key]` is above `bound` (an int reads as a number); otherwise
    /// it is shown disabled.
    #[must_use]
    pub fn enabled_above(mut self, key: impl Into<String>, bound: f64) -> Self {
        self.options.get_or_insert_default().enabled_above = Some((key.into(), bound));
        self
    }

    /// The row takes input only while `fields[a]` and `fields[b]` differ (for two switches: exactly one is on);
    /// otherwise it is shown disabled.
    #[must_use]
    pub fn enabled_when_differs(mut self, a: impl Into<String>, b: impl Into<String>) -> Self {
        self.options.get_or_insert_default().enabled_when_differs = Some((a.into(), b.into()));
        self
    }

    /// The value the row shows while it is disabled, in place of its own.
    #[must_use]
    pub fn disabled_value(mut self, value: impl Into<FieldValue>) -> Self {
        self.options.get_or_insert_default().disabled_value = Some(value.into());
        self
    }

    /// A [`Param::custom`] row's set-time rule: `set` runs for the row's key and its companions when a value is set
    /// from outside the row (automation), and may change other fields or refuse the value. Other rows ignore it.
    ///
    /// `set` gets the fields, the key being set and the incoming value, and writes in place of the row's default
    /// write; an `Err` refuses the value and is the refusal's message.
    #[must_use]
    pub fn on_set(mut self, set: fn(&mut DialogFields, &str, serde_json::Value) -> Result<(), String>) -> Param {
        if let ParamKind::Custom(row) = &mut self.kind {
            row.on_set = Some(set);
        }
        self
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
    /// for the host's colour picker); the row label names the response it returns. Nothing of the row is remembered;
    /// it opens with no value unless [`opens_with`](Param::opens_with) gives one. `draw` reads and writes the
    /// [`DialogFields`] under its key and the keys [`companions`](Param::companions) declares; automation
    /// (`set_json_field`) sets any of those keys to a JSON value, kept as [`FieldValue::Json`] (`null` removes it).
    pub fn custom(
        key: impl Into<String>,
        label: impl Into<Label>,
        draw: impl Fn(&mut Ui, &mut DialogFields, &mut CustomContext) -> Response + Send + Sync + 'static,
    ) -> Self {
        Self::new(key, label, ParamKind::Custom(CustomRow::new(draw)))
    }

    /// The value a [`Param::custom`] row's key opens with, as another row opens with its default. Other rows ignore it.
    #[must_use]
    pub fn opens_with(mut self, value: impl Into<FieldValue>) -> Self {
        if let ParamKind::Custom(row) = &mut self.kind {
            row.opens_with = Some(value.into());
        }
        self
    }

    /// The other keys a [`Param::custom`] row keeps values under (Color Lookup's `file` beside its `lut`), so automation
    /// sets them as it sets the row's own key. Other rows ignore it.
    #[must_use]
    pub fn companions(mut self, keys: impl IntoIterator<Item = impl Into<String>>) -> Self {
        if let ParamKind::Custom(row) = &mut self.kind {
            row.companions.extend(keys.into_iter().map(Into::into));
        }
        self
    }

    /// A row of read-only `text` beside the label, wrapped to the form's width: the user can select and copy it, and
    /// Tab passes over it. It has no value, so nothing is remembered or in the result. With
    /// [`full_width`](Param::full_width) it has no label and spans the form (a note over the rows).
    pub fn read_only_text(key: impl Into<String>, label: impl Into<Label>, text: impl Into<Label>) -> Self {
        Self::new(key, label, ParamKind::ReadOnlyText { text: text.into() })
    }

    /// A section heading: the label as the title of a section whose body is the rows that follow, up to the next
    /// section. It has no value and no control.
    pub fn section(label: impl Into<Label>) -> Self {
        let label = label.into();
        Self::new(format!("section:{}", label.as_str()), label, ParamKind::Section)
    }

    /// A section head that is a check box: it holds a `Bool` under `key`, and its rows take input only while it is
    /// checked.
    pub fn section_check(key: impl Into<String>, label: impl Into<Label>, default: bool) -> Self {
        let mut head = Self::new(key, label, ParamKind::Bool { default });
        head.options.get_or_insert_default().head = Head::Check;
        head
    }

    /// A section head whose title is a disclosure, open by default: its rows hide while it is closed and keep their
    /// values.
    pub fn group(label: impl Into<Label>) -> Self {
        let mut head = Self::section(label);
        head.options.get_or_insert_default().head = Head::Group { open: true };
        head
    }

    /// Binds the row's control to the value's reset (`Bind::given(value).reset(default)`) and shows the label's reset
    /// dot while the value differs from the default.
    #[must_use]
    pub fn resettable(mut self) -> Self {
        self.options.get_or_insert_default().resettable = true;
        self
    }

    /// On a [`Param::group`]: whether its disclosure starts open (the default is open).
    #[must_use]
    pub fn initially_open(mut self, on: bool) -> Self {
        if let Some(Head::Group { open }) = self.options.as_deref_mut().map(|options| &mut options.head) {
            *open = on;
        }
        self
    }

    /// Draws a `Bool` row as a footer button that shows the row's label while off and `on_label` while on; a press
    /// flips the value and the dialog stays open.
    #[must_use]
    pub fn in_footer(mut self, on_label: impl Into<Label>) -> Self {
        self.options.get_or_insert_default().footer_on = Some(on_label.into());
        self
    }

    /// The row or section stands beside the one before it; the pair falls under when the form lacks room. A custom
    /// row beside its section's rows spans its width, as [`full_width`](Param::full_width) does.
    #[must_use]
    pub fn beside(mut self) -> Self {
        let custom = matches!(self.kind, ParamKind::Custom(_));
        let options = self.options.get_or_insert_default();
        options.beside = true;
        options.full_width |= custom;
        self
    }

    /// Draws a `Number` row as a length: the stored unit in the binding, the shown unit in the field.
    #[must_use]
    pub fn length(mut self, lengths: LengthUnits) -> Self {
        self.options.get_or_insert_default().length = Some(lengths);
        self
    }

    /// Sets the `Number` row's codec.
    #[must_use]
    pub fn codec(mut self, codec: NumberCodec) -> Self {
        self.options.get_or_insert_default().codec = Some(codec);
        self
    }

    /// Passes `arrows` to the `Number` row's field.
    #[must_use]
    pub fn step_arrows(mut self, arrows: StepArrows) -> Self {
        self.options.get_or_insert_default().step_arrows = Some(arrows);
        self
    }

    /// Passes labelled `marks` to the `Range` row's slider.
    #[must_use]
    pub fn marks_labelled<L: Into<Label>>(mut self, marks: impl IntoIterator<Item = (f64, L)>) -> Self {
        self.options.get_or_insert_default().marks = marks.into_iter().map(|(at, label)| (at, label.into())).collect();
        self
    }

    /// Adds an icon button after the row's control that reports `link` as the help link; a press opens nothing.
    #[must_use]
    pub fn help_link(mut self, link: impl Into<String>) -> Self {
        self.options.get_or_insert_default().help_link = Some(link.into());
        self
    }

    /// Whether the row starts a section: a section head, or a check head ([`Param::section_check`]).
    pub(crate) fn is_head(&self) -> bool {
        // A section head and a check head are the two rows that open a section.
        matches!(self.kind, ParamKind::Section) || matches!(self.options.as_deref(), Some(options) if options.head == Head::Check)
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
        param.options.get_or_insert_default().units = Some(UnitChoices { units, key: format!("{key}.unit"), initial: 0, shared: false });
        param
    }

    /// A number row shown in the unit another row chooses: `units_key` is the key of a choice row whose options are the
    /// ids of `units`, so several number rows follow one menu. The value and its conversion are those of
    /// [`Param::number_with_units`]; the row draws no unit menu of its own and stores no unit key.
    pub fn number_in_units(
        key: impl Into<String>,
        label: impl Into<Label>,
        base: f64,
        step: f64,
        decimals: u32,
        units_key: impl Into<String>,
        units: Vec<NumberUnit>,
    ) -> Self {
        let mut param = Self::number(key, label, base, step, decimals);
        param.options.get_or_insert_default().units = Some(UnitChoices { units, key: units_key.into(), initial: 0, shared: true });
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
        self.options.as_deref()?.units.as_ref().filter(|units| !units.shared).map(|units| units.key.as_str())
    }

    /// The row is shown only while `fields[key]` equals `value`, as [`enabled_when`](Param::enabled_when) enables it;
    /// otherwise it takes no space. Its value stays in the fields and in the result.
    #[must_use]
    pub fn visible_when(self, key: impl Into<String>, value: impl Into<FieldValue>) -> Self {
        self.visible_when_in(key, [value])
    }

    /// The row is shown only while `fields[key]` is one of `values`, besides the conditions it already has; the
    /// one-value case is [`visible_when`](Param::visible_when).
    #[must_use]
    pub fn visible_when_in(mut self, key: impl Into<String>, values: impl IntoIterator<Item = impl Into<FieldValue>>) -> Self {
        self.visible_when.push((key.into(), values.into_iter().map(Into::into).collect()));
        self
    }

    /// Whether the row is shown at all; a hidden row takes no space, as one whose
    /// [`visible_when`](Param::visible_when) fails. For what the values do not decide; a
    /// [`HostedParamDialog`](super::HostedParamDialog) host sets it between passes with
    /// [`set_param_shown`](super::HostedParamDialog::set_param_shown).
    #[must_use]
    pub fn shown(mut self, shown: bool) -> Self {
        self.shown = shown;
        self
    }

    /// The row takes input only while `fields[key]` equals `value`; otherwise it is shown disabled. The one-value case
    /// of [`enabled_when_in`](Param::enabled_when_in).
    #[must_use]
    pub fn enabled_when(mut self, key: impl Into<String>, value: impl Into<FieldValue>) -> Self {
        self.enable = Enable::When(key.into(), vec![value.into()]);
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

    /// The most characters a [`ParamKind::Text`] row takes: the field limits its input and `accepts` refuses longer text.
    #[must_use]
    pub fn max_chars(mut self, chars: usize) -> Self {
        self.options.get_or_insert_default().max_chars = Some(chars);
        self
    }

    /// Shows a [`ParamKind::Text`] row's text in the monospace face.
    #[must_use]
    pub fn mono(mut self) -> Self {
        self.options.get_or_insert_default().mono = true;
        self
    }

    /// A [`ParamKind::Text`] row selects its whole text when it takes the focus, so typing replaces it (a Rename
    /// dialog opening on the name); by default the caret goes to the end. Other kinds are unchanged.
    #[must_use]
    pub fn select_on_focus(mut self) -> Self {
        self.options.get_or_insert_default().select_on_focus = true;
        self
    }

    /// The width of a [`ParamKind::Text`] row's field, in characters (`TextField::width_chars`): the field asks for
    /// `chars` times the width of a `0` in its face instead of the field length, so one row of a form can be wider or
    /// narrower than the others (a watermark's text against a short name). A row that lacks room still narrows it.
    /// `0` keeps the field length; other kinds, and a text row with a Browse button, are unchanged.
    #[must_use]
    pub fn field_chars(mut self, chars: u16) -> Self {
        self.options.get_or_insert_default().field_chars = (chars > 0).then_some(chars);
        self
    }

    /// Hover text for the whole row, label and control, of any kind.
    #[must_use]
    pub fn tooltip(mut self, text: impl Into<Tooltip>) -> Self {
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

    /// Shows a Range row's slider in a linear popup opened from the number field.
    #[must_use]
    pub fn popup_slider(mut self) -> Self {
        self.options.get_or_insert_default().popup_slider = true;
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

    /// For a [`Param::custom`] or [`Param::read_only_text`] row: no label, and the body spans the width of the form;
    /// other kinds are unchanged.
    #[must_use]
    pub fn full_width(mut self) -> Self {
        self.options.get_or_insert_default().full_width = true;
        self
    }

    /// For a [`ParamKind::Range`] row: the slider asks for a short field's length and the number field is as wide as
    /// its widest value, so the row takes its content's width rather than the form's (production's Opacity). Other
    /// kinds and a popup slider are unchanged.
    #[must_use]
    pub fn short_slider(mut self) -> Self {
        self.options.get_or_insert_default().short_slider = true;
        self
    }

    /// For a [`ParamKind::Radio`] row: the options stand side by side in one row, with no caption over them; the label
    /// still names the group for assistive technology. Other kinds are unchanged.
    #[must_use]
    pub fn options_in_row(mut self) -> Self {
        self.options.get_or_insert_default().options_in_row = true;
        self
    }

    /// For a [`ParamKind::Radio`] row: the options stand in a column with no caption over or beside them, in a document
    /// dialog too (production's Split); the label still names the group for assistive technology. Other kinds are
    /// unchanged.
    #[must_use]
    pub fn stacked_options(mut self) -> Self {
        self.options.get_or_insert_default().stacked_options = true;
        self
    }

    /// The row belongs to the option `id` of the radio row it follows: it stands under that option, one indent step in
    /// (production's Pages per file under Number of pages), or right after the option, with no caption of its own,
    /// when the radio row's options stand in a row ([`options_in_row`](Param::options_in_row); production's colour
    /// well after From colour). Only the rows that directly follow a radio row nest; elsewhere the mark is ignored.
    #[must_use]
    pub fn under_option(mut self, id: impl Into<String>) -> Self {
        self.options.get_or_insert_default().under_option = Some(id.into());
        self
    }

    /// The option of the radio row before this one that the row stands under ([`under_option`](Param::under_option)).
    pub(crate) fn nested_under(&self) -> Option<&str> {
        self.options.as_deref()?.under_option.as_deref()
    }

    /// The row stands on the same line as the row before it (production's Font, Size, Underline and Colour on one
    /// line), wrapping to the next line when the form is too narrow. The first row of a form or group ignores it.
    #[must_use]
    pub fn beside_previous(mut self) -> Self {
        self.options.get_or_insert_default().beside = true;
        self
    }

    /// A [`ParamKind::Text`] row without a Browse button edits multi-line text in a field `lines` lines tall: Enter
    /// inserts a line break and the dialog's default button leaves it to the field. `lines` below 2 keeps a
    /// single-line field, whose Enter answers the default button. Other kinds are unchanged.
    #[must_use]
    pub fn multiline(mut self, lines: usize) -> Self {
        self.options.get_or_insert_default().lines = Some(lines);
        self
    }

    /// Leaves the row out of the remembered values: a choice that refers to something in the document (a depth
    /// channel, a menu of open documents) opens from its default every time.
    #[must_use]
    pub fn not_remembered(mut self) -> Self {
        self.options.get_or_insert_default().not_remembered = true;
        self
    }

    /// Pairs this field with the next one in its section, wrapping when they cannot fit together.
    /// If only one is visible, it keeps the content width it takes in a pair.
    #[must_use]
    pub fn beside_next(mut self) -> Self {
        self.options.get_or_insert_default().beside_next = true;
        self
    }

    pub(super) fn pairs_with_next(&self) -> bool {
        self.option().beside_next
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

    /// The value the row shows while it is disabled ([`Param::disabled_value`]).
    pub(crate) fn shown_disabled(&self) -> Option<&FieldValue> {
        self.options.as_deref()?.disabled_value.as_ref()
    }

    /// The hint text.
    pub(crate) fn hint_text(&self) -> Option<&Label> {
        self.texts.as_deref()?.hint.as_ref()
    }

    /// The row's tooltip text.
    pub(crate) fn tooltip_text(&self) -> Option<&Tooltip> {
        self.texts.as_deref()?.tooltip.as_ref()
    }

    /// The label a [`Param::in_footer`] row shows while on; `None` for a row in the body.
    pub(crate) fn footer_on(&self) -> Option<Label> {
        self.options.as_deref()?.footer_on.clone()
    }

    /// Whether the row's value is saved with the form.
    pub fn is_remembered(&self) -> bool {
        !self.option().not_remembered
    }

    /// Whether `value` is one the field `key` of this row can hold: the row's own value, the unit id of a row with
    /// units, a Files row's companions, or a custom row's companion.
    pub fn accepts(&self, key: &str, value: &FieldValue) -> bool {
        if self.key == key {
            return self.kind.fits(value) && self.option_allowed(value);
        }
        if let ParamKind::Custom(row) = &self.kind {
            return row.has_companion(key);
        }
        if matches!(self.kind, ParamKind::Files { .. }) {
            return match value {
                FieldValue::Text(_) => key == self.folder_key(),
                FieldValue::Bool(_) => key == self.use_folder_key(),
                other => key == self.folder_files_key() && is_path_list(other),
            };
        }
        match (self.options.as_deref().and_then(|o| o.units.as_ref()), value) {
            (Some(units), FieldValue::Text(id)) => !units.shared && units.key == key && units.units.iter().any(|u| &u.id == id),
            _ => false,
        }
    }

    /// The key a [`Param::files`] row stores its folder under: `"{key}.folder"`.
    pub(crate) fn folder_key(&self) -> String {
        format!("{}.folder", self.key)
    }

    /// The key a [`Param::files`] row stores whether the folder is used under: `"{key}.useFolder"`.
    pub(crate) fn use_folder_key(&self) -> String {
        format!("{}.useFolder", self.key)
    }

    /// The listing a [`Param::files`] row asks for when `before` becomes `after`: one of the folder in `after` when
    /// that folder is in use and the folder or the use of it differs from `before`.
    pub(crate) fn listing_after(&self, before: &DialogFields, after: &DialogFields) -> Option<ListingRequest> {
        let (folder_key, use_key) = (self.folder_key(), self.use_folder_key());
        let uses = |fields: &DialogFields| fields.bool(&use_key).unwrap_or(false);
        let folder = after.text(&folder_key).filter(|_| uses(after))?;
        let changed = before.text(&folder_key) != Some(folder) || !uses(before);
        changed.then(|| ListingRequest::new(&self.key, PathBuf::from(folder)))
    }

    /// The key the host's listing of a [`Param::files`] row's folder is written under: `"{key}.folderFiles"`.
    pub(crate) fn folder_files_key(&self) -> String {
        format!("{}.folderFiles", self.key)
    }

    /// Whether `value` names no disabled option of a Radio, Choice or Blend row and is no longer than a text row's
    /// [`max_chars`](Param::max_chars), counted in characters.
    fn option_allowed(&self, value: &FieldValue) -> bool {
        match (&self.kind, value) {
            (ParamKind::Text { .. }, FieldValue::Text(text)) => {
                self.options.as_deref().and_then(|options| options.max_chars).is_none_or(|max| text.chars().count() <= max)
            }
            (ParamKind::Radio { .. } | ParamKind::Choice { .. } | ParamKind::Blend { .. }, FieldValue::Text(id)) => self.option_enabled(id),
            _ => true,
        }
    }

    /// Whether the row is shown with `fields`.
    pub fn visible(&self, fields: &DialogFields) -> bool {
        self.shown && self.visible_when.iter().all(|(key, values)| fields.get(key).is_some_and(|held| values.contains(held)))
    }

    /// Whether the row takes input with `fields`: every condition its `enabled_when`,
    /// [`enabled_above`](Param::enabled_above) and [`enabled_when_differs`](Param::enabled_when_differs) set holds.
    pub fn enabled(&self, fields: &DialogFields) -> bool {
        let options = self.options.as_deref();
        let enabled = match &self.enable {
            Enable::Always => true,
            Enable::When(key, values) => fields.get(key).is_some_and(|held| values.contains(held)),
            Enable::Off => false,
        };
        enabled
            && options.and_then(|options| options.enabled_above.as_ref()).is_none_or(|(key, bound)| fields.number(key).is_some_and(|n| n > *bound))
            && options.and_then(|options| options.enabled_when_differs.as_ref()).is_none_or(|(a, b)| fields.get(a) != fields.get(b))
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

    /// Whether a range row's slider is short.
    pub(crate) fn short_slider(&self) -> bool {
        self.short_slider
    }

    /// Whether a radio row's options stand in one row with no caption.
    pub(crate) fn options_in_row(&self) -> bool {
        self.options_in_row
    }

    /// Whether a radio row's options stand in a column with no caption.
    pub(crate) fn stacked_options(&self) -> bool {
        self.stacked_options
    }

    /// The lines of a multi-line text row; `None` for a single line.
    pub(crate) fn lines(&self) -> Option<usize> {
        self.lines
    }

    /// The most characters a text row takes; `None` for the field's own limit.
    pub(crate) fn max_chars(&self) -> Option<usize> {
        self.max_chars
    }

    /// Whether a text row shows its text in the monospace face.
    pub(crate) fn mono(&self) -> bool {
        self.mono
    }

    /// Whether a text row selects its text when it takes the focus.
    pub(crate) fn select_on_focus(&self) -> bool {
        self.select_on_focus
    }

    /// The width a text row's field asks for, in characters; `None` for the field length.
    pub(crate) fn field_chars(&self) -> Option<u16> {
        self.field_chars
    }

    /// Whether a choice row's select has a search field.
    pub(crate) fn searchable(&self) -> bool {
        self.searchable
    }

    /// Whether a choice row's list has a separator before the option at `index`.
    pub(crate) fn separated_before(&self, index: usize) -> bool {
        self.separated_at.contains(&index)
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

    /// Whether another row chooses the unit.
    pub(crate) fn is_shared(&self) -> bool {
        self.shared
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

/// The keys of the rows of `params` whose values differ between `before` and `after`: a row's own key, its unit key, a
/// custom row's companions or a Files row's folder and mode. Every record of the rows an edit changed reads this rule.
pub(crate) fn edited_rows(params: &[Param], before: &DialogFields, after: &DialogFields) -> Vec<String> {
    let differs = |key: &str| before.get(key) != after.get(key);
    params
        .iter()
        .filter(|param| {
            differs(&param.key)
                || param.unit_key().is_some_and(&differs)
                || matches!(&param.kind, ParamKind::Custom(row) if row.companions.iter().any(|key| differs(key)))
                || (matches!(param.kind, ParamKind::Files { .. }) && (differs(&param.folder_key()) || differs(&param.use_folder_key())))
        })
        .map(|param| param.key.clone())
        .collect()
}

/// Sets `text` as the one text of `id` in `texts`.
fn keyed_text(texts: &mut Vec<(String, Label)>, id: String, text: Label) {
    texts.retain(|(held, _)| *held != id);
    texts.push((id, text));
}

/// Fills the keys `params` have defaults for and `fields` lack, and takes the opening snapshot.
pub fn open_fields(params: &[Param], fields: &mut DialogFields) {
    fill_defaults(params, fields);
    fields.snapshot();
}

/// Fills the keys `params` have defaults for and `fields` lack, including the unit key of a row with units. A key
/// marked mixed stays absent: its row shows it blank.
pub(crate) fn fill_defaults(params: &[Param], fields: &mut DialogFields) {
    for param in params {
        if !fields.contains(&param.key)
            && !fields.is_mixed(&param.key)
            && let Some(value) = param.default_value()
        {
            fields.set(param.key.as_str(), value);
        }
        if let Some(units) = param.options.as_deref().and_then(|o| o.units()).filter(|units| !units.is_shared())
            && !fields.contains(units.key())
            && let Some(unit) = units.list().get(units.initial())
        {
            fields.set(units.key(), FieldValue::Text(unit.id.clone()));
        }
    }
}
