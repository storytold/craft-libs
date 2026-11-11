//! `NewGuideDialog`: View > Guides > New Guide and Edit Selected Guides.

use super::layout::{GuideLength, GuideUnits};
use crate::runtime::requests::HostAnswer;

use super::{FormDialog, GUIDE_LIMIT, GuideDialogWords, HostRequests, NEW_GUIDE_COLOR_KEY, apply_color_answers, color_row};
use crate::canvas_data::{Guide, GuideColor, GuideKey};

use crate::choice::Segmented;
use crate::contract::{Bind, Entry, NoValue, Outcome, Show as _, ShowContainer as _};

use crate::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};

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
}

impl<'a> NewGuideDialog<'a> {
    /// New Guide over `values`, for a document measured in `units`.
    pub fn new(units: GuideUnits, values: Bind<'a, NewGuideValues>) -> Self {
        NewGuideDialog { units, values, edit: None, key: GuideKey::new(0), error: None, answers: Vec::new(), open: None }
    }

    /// Edit Selected Guides for `guide`, which is drawn in `preference` when it has no colour of its own: the
    /// `values` start as [`NewGuideValues::of_guide`].
    pub fn edit(units: GuideUnits, guide: Guide, preference: Rgba, values: Bind<'a, NewGuideValues>) -> Self {
        NewGuideDialog { edit: Some((guide, preference)), ..NewGuideDialog::new(units, values) }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> NewGuideOutput {
        let NewGuideDialog { units, values, edit, key, error, answers, open } = self;
        let env = Env::of(ui);
        let words = env.words::<GuideDialogWords>();
        let number_units = env.units();
        let title = if edit.is_some() { words.edit_title.clone() } else { words.new_guide_title.clone() };
        let mut dialog = FormDialog::new(title.clone(), values).outer_gesture(true).answers(answers);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        let orientations = [Entry::new(Axis::X, words.horizontal.clone()), Entry::new(Axis::Y, words.vertical.clone())];
        let shown = dialog.show(ui, |ui, form, values| {
            let first = form.first();
            if let (true, Some((guide, preference))) = (first, &edit) {
                *values = NewGuideValues::of_guide(guide, *preference);
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
            parsed
        });
        let result = shown.accepted.map(|values| guide_of(&values, edit.as_ref(), key));
        NewGuideOutput { response: shown.response, closed: shown.closed, result, requests: shown.requests }
    }
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

/// The Position row; `false` while its text does not parse.
fn position_row(ui: &mut Ui, words: &GuideDialogWords, numbers: &Units, units: &GuideUnits, values: &mut NewGuideValues, focus_here: bool) -> bool {
    let ruler = units.ruler;
    let extent = if values.along == Axis::Y { units.width_px } else { units.height_px };
    let shown = units.from_pixels(values.position_px, ruler, extent).unwrap_or(0.0);
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
                let changed = (typed * scale).round() != (shown * scale).round();
                if let (true, Some(px)) = (changed, units.to_pixels(GuideLength::new(typed, ruler), extent)) {
                    values.position_px = px.clamp(-GUIDE_LIMIT, GUIDE_LIMIT);
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
}

impl HostedNewGuideDialog {
    /// A dialog opening with `values`; `build` makes it over the values (`NewGuideDialog::new` or `edit`).
    pub fn new(values: NewGuideValues, build: impl for<'b> Fn(Bind<'b, NewGuideValues>) -> NewGuideDialog<'b> + Send + 'static) -> Self {
        HostedNewGuideDialog { id: dialog_id("guides.newGuide"), values, build: Box::new(build), requests: HostRequests::new() }
    }

    /// The values the dialog shows.
    pub fn values(&self) -> &NewGuideValues {
        &self.values
    }

    /// What the dialog asked of the host and the host's answers: take the colour request, answer it.
    pub fn requests(&mut self) -> &mut HostRequests {
        &mut self.requests
    }
}

impl StackDialog for HostedNewGuideDialog {
    type Result = Guide;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Guide>> {
        let shown = (self.build)(Bind::bound(&mut self.values)).answers(self.requests.take_answers()).show(ui);
        self.requests.extend(shown.requests);
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
