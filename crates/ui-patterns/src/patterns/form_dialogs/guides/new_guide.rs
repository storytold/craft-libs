//! `NewGuideDialog`: View > Guides > New Guide and Edit Selected Guides.

use super::layout::{GuideLength, GuideUnits};
use crate::runtime::requests::HostAnswer;

use super::{FormDialog, GUIDE_LIMIT, GuideDialogWords, HostRequests, NEW_GUIDE_COLOR_KEY, apply_color_answers, color_json, color_row, read_color};
use crate::canvas_data::{Guide, GuideColor, GuideKey};

use crate::choice::Segmented;
use crate::contract::{Bind, Entry, NoValue, Outcome, SaveKey, Show as _, ShowContainer as _};

use crate::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use crate::runtime::memory::{saved_value, set_saved_value};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::form::{Field, FieldGroup};

use crate::notices::{Notice, NoticeLayout};

use crate::runtime::env::{Env, Label, Units};

use crate::runtime::focus::{self, FocusTarget};

use crate::runtime::overlay::CloseReason;
use crate::text::NumberField;
use craft_behavior::{Axis, NumberCodec};

use craft_theme::Rgba;
use craft_theme::variant::Tone;
use egui::{Response, Ui};

/// The values of New Guide.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct NewGuideValues {
    /// The axis the guide runs along: `Axis::X` for a horizontal guide.
    pub along: Axis,
    /// The position in document pixels.
    pub position_px: f64,
    /// The guide's colour.
    pub color: GuideColor,
}

impl NewGuideValues {
    /// A guide running `along` at `position_px` in `color`.
    pub fn new(along: Axis, position_px: f64, color: GuideColor) -> Self {
        NewGuideValues { along, position_px, color }
    }

    /// The values that edit `guide`; a guide with no colour of its own is drawn in `preference`.
    pub fn of_guide(guide: &Guide, preference: Rgba) -> Self {
        NewGuideValues::new(guide.along, guide.position, GuideColor::from_rgba(guide.color.unwrap_or(preference)))
    }
}

impl Default for NewGuideValues {
    /// A horizontal guide at 0 in Cyan.
    fn default() -> Self {
        NewGuideValues::new(Axis::X, 0.0, GuideColor::default())
    }
}

/// What New Guide reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct NewGuideOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The guide, the pass the dialog closed as OK: the edited guide with the new values, or a new guide with the
    /// pattern's key.
    pub result: Option<Guide>,
    /// What the dialog asks of the host in this pass (the Custom colour).
    pub requests: HostRequests,
    /// OK takes the values: `false` while the Position text does not parse, which disables OK.
    pub ok_enabled: bool,
}

impl Outcome for NewGuideOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// New Guide, and Edit Selected Guides when it edits a guide: an orientation, a position with a unit and a colour.
///
/// - Keys: Tab walks the fields, Enter is OK, Escape cancels. Holding Alt turns Cancel into Reset, which restores the
///   values from when the dialog opened and keeps the dialog open. The Position field has the focus when the dialog
///   opens.
/// - Position: a number field in the ruler unit over the document's width (vertical guides) or height (horizontal),
///   so "50%", "2 cm" and "12 pt" work; text that is not a length is refused with the field's message.
/// - Colour: Photoshop's nine colours or Custom, which asks the host for a colour (`requests`); the host's answers
///   come back through [`answers`](NewGuideDialog::answers), and a cancelled pick leaves the colour as it was.
/// - Edit: the values start as [`NewGuideValues::of_guide`] whatever the binding held; the edited guide keeps its
///   other settings, and a guide with no colour of its own keeps using the app's preference unless the colour is
///   changed here.
/// - Values: the `values` binding holds them; the app keeps the last guide's for the next open.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct NewGuideDialog<'a> {
    units: GuideUnits,
    values: Bind<'a, NewGuideValues>,
    edit: Option<(Guide, Rgba)>,
    /// The key of the guide a new guide makes.
    #[prop]
    key: GuideKey,
    /// The app's message when it refused the position: shown above the fields.
    #[prop]
    error: Option<Label>,
    /// The host's answers to earlier requests.
    #[prop]
    answers: Vec<HostAnswer>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// Remembers a new guide's values under `key`: the remembered values replace the opening ones, and OK stores the
    /// confirmed ones for the next open. Edit Selected Guides neither recalls nor stores.
    #[prop]
    remember: Option<SaveKey>,
    /// Whether the values already start from the remembered ones, so the first pass recalls nothing: a hosted
    /// dialog recalls before any pass, and automation may have set a field since.
    recalled: bool,
}

impl<'a> NewGuideDialog<'a> {
    /// New Guide over `values`, for a document measured in `units`.
    pub fn new(units: GuideUnits, values: Bind<'a, NewGuideValues>) -> Self {
        NewGuideDialog {
            units,
            values,
            edit: None,
            key: GuideKey::new(0),
            error: None,
            answers: Vec::new(),
            open: None,
            remember: None,
            recalled: false,
        }
    }

    /// The key the values are remembered under: a new guide's `remember`, none for an edit.
    fn memory_key(&self) -> Option<&SaveKey> {
        self.remember.as_ref().filter(|_| self.edit.is_none())
    }

    /// Edit Selected Guides for `guide`, which is drawn in `preference` when it has no colour of its own: the
    /// `values` start as [`NewGuideValues::of_guide`].
    pub fn edit(units: GuideUnits, guide: Guide, preference: Rgba, values: Bind<'a, NewGuideValues>) -> Self {
        NewGuideDialog { edit: Some((guide, preference)), ..NewGuideDialog::new(units, values) }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> NewGuideOutput {
        let memory = self.memory_key().cloned();
        let NewGuideDialog { units, values, edit, key, error, answers, open, recalled, .. } = self;
        let env = Env::of(ui);
        let words = env.words::<GuideDialogWords>();
        let number_units = env.units();
        let title = if edit.is_some() { words.edit_title.clone() } else { words.new_guide_title.clone() };
        let mut dialog = FormDialog::new(title.clone(), values).outer_gesture(true).answers(answers);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        let orientations = [Entry::new(Axis::X, words.horizontal.clone()), Entry::new(Axis::Y, words.vertical.clone())];
        let mut ok_enabled = true;
        let shown = dialog.show(ui, |ui, form, values| {
            let first = form.first();
            if let (true, Some((guide, preference))) = (first, &edit) {
                *values = NewGuideValues::of_guide(guide, *preference);
            }
            if let (true, false, Some(remembered)) = (first, recalled, memory.as_ref().and_then(|key| recall(ui.ctx(), key))) {
                *values = remembered;
            }
            apply_color_answers(form.take_answers(), NEW_GUIDE_COLOR_KEY, &mut values.color);
            if let Some(message) = &error {
                Notice::new(Tone::Bad, message.clone()).layout(NoticeLayout::Inline).show(ui);
            }
            let mut parsed = true;
            FieldGroup::new(title.clone()).show(ui, |ui, _| {
                Field::new(words.orientation.clone()).show(ui, |ui, _| {
                    let along = values.along;
                    if let Some(picked) = Segmented::new(words.orientation.clone(), Bind::given(along), &orientations).show(ui).value().copied() {
                        values.along = picked;
                    }
                });
                parsed = position_row(ui, &words, &number_units, &units, values, first);
                color_row(ui, &words, &mut values.color, NEW_GUIDE_COLOR_KEY, form.requests());
            });
            // A measuring run lays the field out without its typing run, so it never says whether the text parses.
            if !crate::runtime::measure::active(ui.ctx()) {
                ok_enabled = parsed;
            }
            parsed
        });
        let result = shown.accepted.map(|values| {
            if let Some(memory) = &memory {
                store(ui.ctx(), memory, &values);
            }
            guide_of(&values, edit.as_ref(), key)
        });
        NewGuideOutput { response: shown.response, closed: shown.closed, result, requests: shown.requests, ok_enabled }
    }
}

/// What New Guide remembers under its `remember` key: the last confirmed new guide's values. Its own type because
/// [`GuideColor`] has no serde form; the colour is kept as its bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Remembered {
    along: Axis,
    position_px: f64,
    color: [u8; 4],
}

/// The values remembered under `key`, once a new guide was confirmed with it.
fn recall(ctx: &egui::Context, key: &SaveKey) -> Option<NewGuideValues> {
    let remembered = saved_value::<Option<Remembered>>(ctx, key)?;
    Some(NewGuideValues::new(remembered.along, remembered.position_px, GuideColor::from_rgba(Rgba(remembered.color))))
}

/// Stores `values` under `key` for the next open.
fn store(ctx: &egui::Context, key: &SaveKey, values: &NewGuideValues) {
    let remembered = Remembered { along: values.along, position_px: values.position_px, color: values.color.rgba().0 };
    set_saved_value(ctx, key, Some(remembered));
}

/// The guide the values make: the edited guide with the values applied, or a new guide with `key`. An edited guide
/// with no colour of its own stays so while the colour is still the preference it opened with.
fn guide_of(values: &NewGuideValues, edit: Option<&(Guide, Rgba)>, key: GuideKey) -> Guide {
    let (base, keeps_preference) = match edit {
        Some((guide, preference)) => (guide.clone(), guide.color.is_none() && values.color == GuideColor::from_rgba(*preference)),
        None => (Guide::new(key, values.along, values.position_px), false),
    };
    let color = if keeps_preference { None } else { Some(values.color.rgba()) };
    Guide { along: values.along, position: values.position_px, color, ..base }
}

/// The extent a position's percent is of: the document's width for a vertical guide, its height for a horizontal one.
fn position_extent(units: &GuideUnits, along: Axis) -> f64 {
    if along == Axis::Y { units.width_px } else { units.height_px }
}

/// The position in the ruler unit, as the Position field shows it.
fn ruler_position(units: &GuideUnits, values: &NewGuideValues) -> f64 {
    units.from_pixels(values.position_px, units.ruler, position_extent(units, values.along)).unwrap_or(0.0)
}

/// Sets the position from `typed` in the ruler unit, held to the guide limit, as a commit in the Position field
/// does; `false` and nothing changes when `typed` does not convert.
fn set_ruler_position(units: &GuideUnits, values: &mut NewGuideValues, typed: f64) -> bool {
    let extent = position_extent(units, values.along);
    let Some(px) = units.to_pixels(GuideLength::new(typed, units.ruler), extent) else { return false };
    values.position_px = px.clamp(-GUIDE_LIMIT, GUIDE_LIMIT);
    true
}

/// The Position row; `false` while its text does not parse.
fn position_row(ui: &mut Ui, words: &GuideDialogWords, numbers: &Units, units: &GuideUnits, values: &mut NewGuideValues, focus_here: bool) -> bool {
    let ruler = units.ruler;
    let extent = position_extent(units, values.along);
    let shown = ruler_position(units, values);
    let high = units.from_pixels(GUIDE_LIMIT, ruler, extent).filter(|v| v.is_finite()).unwrap_or(GUIDE_LIMIT).abs();
    let places = *ruler.decimals().end();
    Field::new(words.position.clone())
        .show(ui, |ui, _| {
            let codec = NumberCodec::new(units.reader(numbers, extent), places);
            let field = NumberField::new(words.position.clone(), Bind::given(Some(shown)))
                .codec(codec)
                .range(-high..=high)
                .step(ruler.step())
                .unit(numbers.symbol(ruler))
                .show(ui);
            if focus_here {
                focus::request(ui.ctx(), FocusTarget::Id(field.response.id));
            }
            if let Some(Some(typed)) = field.value().copied() {
                let scale = 10f64.powi(i32::try_from(places).unwrap_or(0));
                if (typed * scale).round() != (shown * scale).round() {
                    set_ruler_position(units, values, typed);
                }
            }
            !field.unparsed
        })
        .inner
        .unwrap_or(true)
}

/// Builds the dialog over the host-owned values of a [`HostedNewGuideDialog`] for one pass.
type Build = dyn for<'b> Fn(Bind<'b, NewGuideValues>) -> NewGuideDialog<'b> + Send;

/// A hosted New Guide: the `DialogHost` owns the values, and `build` makes the dialog over them each pass.
///
/// ```ignore
/// host.open(HostedNewGuideDialog::new(NewGuideValues::default(), move |values| NewGuideDialog::new(units, values).key(key)));
/// ```
pub struct HostedNewGuideDialog {
    id: DialogId,
    values: NewGuideValues,
    build: Box<Build>,
    requests: HostRequests,
    /// Whether the dialog is open: bound to the dialog's `opened` prop, so the host's dialog closes with it.
    open: bool,
    /// The app's message shown above the fields, set with [`set_error`](Self::set_error); it replaces the one
    /// `build` gives while set.
    error: Option<Label>,
    /// Whether the remembered values were recalled into `values`: once, before the first show, OK or set reads them.
    recalled: bool,
    /// Whether the last pass's OK took the values: `false` while the Position text did not parse.
    ok_enabled: bool,
}

impl HostedNewGuideDialog {
    /// A dialog opening with `values`; `build` makes it over the values (`NewGuideDialog::new` or `edit`).
    pub fn new(values: NewGuideValues, build: impl for<'b> Fn(Bind<'b, NewGuideValues>) -> NewGuideDialog<'b> + Send + 'static) -> Self {
        HostedNewGuideDialog {
            id: dialog_id("guides.newGuide"),
            values,
            build: Box::new(build),
            requests: HostRequests::new(),
            open: true,
            error: None,
            recalled: false,
            ok_enabled: true,
        }
    }

    /// Replaces the opening values with the remembered ones, once, as the dialog's first pass would: automation may
    /// set or confirm before any pass.
    fn recall(&mut self, ctx: &egui::Context) {
        if std::mem::replace(&mut self.recalled, true) {
            return;
        }
        let memory = (self.build)(Bind::bound(&mut self.values)).memory_key().cloned();
        if let Some(remembered) = memory.and_then(|key| recall(ctx, &key)) {
            self.values = remembered;
        }
    }

    /// The values the dialog shows.
    pub fn values(&self) -> &NewGuideValues {
        &self.values
    }

    /// Shows `error` above the fields from the next pass on: the app's message when it refused the position, such as
    /// Edit Selected Guides' check of each pass. `None` shows the one `build` gives, if any.
    pub fn set_error(&mut self, error: Option<Label>) {
        self.error = error;
    }

    /// What the dialog asked of the host and the host's answers: take the colour request, answer it.
    pub fn requests(&mut self) -> &mut HostRequests {
        &mut self.requests
    }
}

impl StackDialog for HostedNewGuideDialog {
    type Result = Guide;

    /// Answers the guide OK makes, and stores a new guide's values under its `remember` key as OK does. None while OK is
    /// disabled: the Position text the last pass showed does not parse, so the last good position is not the answer.
    fn confirm(&mut self, ctx: &egui::Context) -> Option<Self::Result> {
        if !self.ok_enabled {
            return None;
        }
        self.recall(ctx);
        let (memory, edit, key) = {
            let dialog = (self.build)(Bind::bound(&mut self.values));
            (dialog.memory_key().cloned(), dialog.edit.clone(), dialog.key)
        };
        if let Some(memory) = &memory {
            store(ctx, memory, &self.values);
        }
        Some(guide_of(&self.values, edit.as_ref(), key))
    }

    /// `orientation` (`horizontal` or `vertical`), `position` in the ruler unit as the Position field shows it, and
    /// `color` in the `Srgba` form.
    fn json_fields(&self) -> Option<Value> {
        let units = (self.build)(Bind::given(self.values)).units;
        let orientation = if self.values.along == Axis::Y { "vertical" } else { "horizontal" };
        Some(json!({
            "orientation": orientation,
            "position": ruler_position(&units, &self.values),
            "color": color_json(self.values.color),
        }))
    }

    /// Sets a field as its row does: `position` is in the ruler unit and held to the guide limit; `color` picks the
    /// named colour it matches, else Custom.
    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        self.recall(ctx);
        let units = (self.build)(Bind::bound(&mut self.values)).units;
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        let values = &mut self.values;
        match field {
            "orientation" => {
                values.along = match value.as_str() {
                    Some("horizontal") => Axis::X,
                    Some("vertical") => Axis::Y,
                    _ => return Err(rejects()),
                }
            }
            "position" => {
                let typed = value.as_f64().ok_or_else(rejects)?;
                if !set_ruler_position(&units, values, typed) {
                    return Err(rejects());
                }
                // The set position replaces the text the field held.
                self.ok_enabled = true;
            }
            "color" => values.color = read_color(&value).ok_or_else(rejects)?,
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Guide>> {
        self.recall(ui.ctx());
        let measuring = crate::runtime::measure::active(ui.ctx());
        let answers = if measuring { Vec::new() } else { self.requests.take_answers() };
        let mut dialog = (self.build)(Bind::bound(&mut self.values)).answers(answers).opened(Bind::bound(&mut self.open));
        dialog.recalled = true;
        if self.error.is_some() {
            dialog.error = self.error.clone();
        }
        let shown = dialog.show(ui);
        self.requests.extend(shown.requests);
        self.ok_enabled = shown.ok_enabled;
        match (shown.result, shown.closed) {
            (Some(guide), _) => Some(DialogResult::Ok(guide)),
            (None, Some(_)) => {
                self.requests.cancel();
                Some(DialogResult::Cancel)
            }
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some("guides.newGuide")
    }

    fn cancelled(&mut self, _: &egui::Context) {
        self.requests.cancel();
    }
}
