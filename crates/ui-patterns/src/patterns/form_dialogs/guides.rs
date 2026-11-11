//! View > Guides > New Guide, Edit Selected Guides and New Guide Layout: [`NewGuideDialog`] and [`GuideLayoutDialog`]
//! on `FormDialog`, and their `DialogHost` forms.
//!
//! Positions and sizes are lengths: the fields open in the ruler unit ("0 cm") and take any unit Photoshop's fields
//! take ("50%", "2.5 cm", "1in", "12 pt", "3 pc", "100 px"); a bare number is in the ruler unit. Percent is of the
//! target's width for vertical guides, column widths and the left and right margins, and of its height for the rest.
//! [`GuideLayoutOptions::lines`] is the layout calculation, so the dialog's live preview and the app's command make
//! the same `canvas::Guide`s.

mod layout;
mod new_guide;

#[cfg(feature = "testing")]
mod stories;

pub use layout::{GuideGroup, GuideLayoutError, GuideLayoutOptions, GuideLayoutPreset, GuideLength, GuideRect, GuideTarget, GuideUnits, MAX_COUNT};

pub use new_guide::{HostedNewGuideDialog, NewGuideDialog, NewGuideOutput, NewGuideValues};

use super::color_choice::{self, ColorChoice, ColorRowTexts, apply_color_answers};
use crate::runtime::requests::HostAnswer;

use super::{FormDialog, HostRequests, ParamDialogWords};
use crate::canvas_data::{Guide, GuideColor};

use crate::choice::Checkbox;
use crate::contract::{Bind, EditResponse, Entry, NoValue, Outcome, Props as _, Remember as _, Show as _, ShowContainer as _};

use crate::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};

use crate::display::Text;
use crate::form::{Field, FieldGroup};

use crate::notices::{Notice, NoticeLayout};

use crate::pickers::Select;
use crate::runtime::env::{Env, Label, Units};

use crate::runtime::overlay::CloseReason;
use crate::sections::GroupBox;
use crate::text::NumberField;
use craft_behavior::NumberCodec;
use craft_theme::variant::Tone;
use craft_theme::{Rgba, Srgba};

use egui::{Response, Ui};

use std::cell::{Cell, RefCell};

/// The [`ColorRequest::key`](crate::runtime::requests::ColorRequest::key) of New Guide's Custom colour.
pub const NEW_GUIDE_COLOR_KEY: &str = "guides.newGuide.color";
/// The [`ColorRequest::key`](crate::runtime::requests::ColorRequest::key) of New Guide Layout's Custom colour.
pub const GUIDE_LAYOUT_COLOR_KEY: &str = "guides.layout.color";

/// How far from the origin a guide may be, in pixels.
const MAX_GUIDE_PX: f64 = 1_000_000.0;
/// The largest length a layout field takes, in pixels.
const MAX_LAYOUT_PX: f64 = 300_000.0;

craft_ui::words! {
    /// The words of the guide dialogs (`newGuideDialog`, `guideLayoutDialog`) and of the guide colour list (`guideColor`), English by default (Photoshop's). OK, Cancel and Reset are the common words.
    pub struct GuideDialogWords {
        new_guide_title: "newGuideDialog.title" = "New Guide";
        edit_title: "newGuideDialog.editTitle" = "Edit Selected Guides";
        orientation: "newGuideDialog.orientation" = "Orientation";
        horizontal: "newGuideDialog.horizontal" = "Horizontal";
        vertical: "newGuideDialog.vertical" = "Vertical";
        position: "newGuideDialog.position" = "Position";
        color: "guideColor.label" = "Color";
        custom_color: "guideColor.custom" = "Custom…";
        light_blue: "guideColor.lightBlue" = "Light Blue";
        light_red: "guideColor.lightRed" = "Light Red";
        green: "guideColor.green" = "Green";
        medium_blue: "guideColor.mediumBlue" = "Medium Blue";
        yellow: "guideColor.yellow" = "Yellow";
        magenta: "guideColor.magenta" = "Magenta";
        cyan: "guideColor.cyan" = "Cyan";
        light_gray: "guideColor.lightGray" = "Light Gray";
        black: "guideColor.black" = "Black";
        picker_title: "guideColor.pickerTitle" = "Color Picker (Guides)";
        layout_title: "guideLayoutDialog.title" = "New Guide Layout";
        preset: "guideLayoutDialog.preset" = "Preset";
        preset_custom: "guideLayoutDialog.presetCustom" = "Custom";
        preset_five: "guideLayoutDialog.presetFive" = "5 Columns, 3 Rows";
        preset_eight: "guideLayoutDialog.presetEight" = "8 Columns";
        preset_twelve: "guideLayoutDialog.presetTwelve" = "12 Columns";
        target: "guideLayoutDialog.target" = "Target";
        document: "guideLayoutDialog.document" = "Document";
        selected_artboards: "guideLayoutDialog.selectedArtboards" = "Selected Artboards";
        columns: "guideLayoutDialog.columns" = "Columns";
        rows: "guideLayoutDialog.rows" = "Rows";
        number: "guideLayoutDialog.number" = "Number";
        width: "guideLayoutDialog.width" = "Width";
        height: "guideLayoutDialog.height" = "Height";
        gutter: "guideLayoutDialog.gutter" = "Gutter";
        automatic_size: "guideLayoutDialog.automaticSize" = "Leave empty for automatic size";
        margin: "guideLayoutDialog.margin" = "Margin";
        top: "guideLayoutDialog.top" = "Top";
        left: "guideLayoutDialog.left" = "Left";
        bottom: "guideLayoutDialog.bottom" = "Bottom";
        right: "guideLayoutDialog.right" = "Right";
        center_columns: "guideLayoutDialog.centerColumns" = "Center Columns";
        clear_existing: "guideLayoutDialog.clearExisting" = "Clear Existing Guides";
        invalid: "guideLayoutDialog.invalid" = "Invalid guide layout";
        does_not_fit: "guideLayoutDialog.doesNotFit" = "The columns or rows, their size and gutter must fit inside the margins.";
        margins_too_large: "guideLayoutDialog.marginsTooLarge" = "The margins must leave space inside the target.";
        nothing: "guideLayoutDialog.nothing" = "Turn on Columns, Rows or Margin.";
        too_many: "guideLayoutDialog.tooMany" = "The layout would make too many guides.";
        no_artboards: "guideLayoutDialog.noArtboards" = "Select an artboard or a layer inside one.";
    }
}

/// The name of `color` in the list.
fn color_label(color: GuideColor, words: &GuideDialogWords) -> Label {
    match color {
        GuideColor::LightBlue => words.light_blue.clone(),
        GuideColor::LightRed => words.light_red.clone(),
        GuideColor::Green => words.green.clone(),
        GuideColor::MediumBlue => words.medium_blue.clone(),
        GuideColor::Yellow => words.yellow.clone(),
        GuideColor::Magenta => words.magenta.clone(),
        GuideColor::Cyan => words.cyan.clone(),
        GuideColor::LightGray => words.light_gray.clone(),
        GuideColor::Black => words.black.clone(),
        GuideColor::Custom(_) => words.custom_color.clone(),
    }
}

impl ColorChoice for GuideColor {
    const NAMED: &'static [Self] = &GuideColor::NAMED;
    const CUSTOM_FIRST: bool = true;

    fn from_picked(color: Srgba) -> Self {
        GuideColor::Custom(color.to_rgba())
    }

    fn is_custom(self) -> bool {
        matches!(self, GuideColor::Custom(_))
    }

    fn srgba(self) -> Srgba {
        Srgba::from(self.rgba())
    }
}

/// The guide colour row: Custom first, then Photoshop's nine.
fn color_row(ui: &mut Ui, words: &GuideDialogWords, color: &mut GuideColor, key: &str, requests: &mut HostRequests) {
    let texts = ColorRowTexts { label: words.color.clone(), picker_title: words.picker_title.clone() };
    color_choice::color_row(ui, &texts, color, key, requests, |value| color_label(value, words));
}

/// `value` rounded to `places` decimals, for telling a typed value from the one the field already showed.
fn rounded(value: f64, places: usize) -> f64 {
    let scale = 10f64.powi(i32::try_from(places).unwrap_or(0));
    (value * scale).round() / scale
}

/// What the length fields of a guide dialog read with, and whether one of them shows text that does not parse in
/// this pass.
struct Measure<'a> {
    words: &'a GuideDialogWords,
    numbers: &'a Units,
    units: &'a GuideUnits,
    unparsed: Cell<bool>,
}

impl Measure<'_> {
    /// Notes a field's response: OK waits while its text does not parse.
    fn note<T>(&self, field: &EditResponse<T>) {
        if field.unparsed {
            self.unparsed.set(true);
        }
    }

    /// A number field `label` over the ruler unit showing `shown`, for lengths across `extent` pixels: the value the
    /// user typed if it differs from the one shown (`Some(None)` is an emptied field).
    fn input(&self, ui: &mut Ui, label: Label, shown: Option<f64>, extent: f64) -> Option<Option<f64>> {
        let ruler = self.units.ruler;
        let places = *ruler.decimals().end();
        let high = self.units.from_pixels(MAX_LAYOUT_PX, ruler, extent).filter(|v| v.is_finite()).unwrap_or(MAX_LAYOUT_PX).max(0.0);
        let codec = NumberCodec::new(self.units.reader(self.numbers, extent), places);
        let field = NumberField::new(label, Bind::given(shown)).codec(codec).range(0.0..=high).step(ruler.step()).unit(self.numbers.symbol(ruler));
        let field = field.show(ui);
        self.note(&field);
        let typed = field.value().copied()?;
        match (typed, shown) {
            (Some(new), Some(old)) if rounded(new, places) == rounded(old, places) => None,
            (None, None) => None,
            (typed, _) => Some(typed),
        }
    }

    /// A required length row in the ruler unit.
    fn length(&self, ui: &mut Ui, label: Label, length: &mut GuideLength, extent: f64) {
        let ruler = self.units.ruler;
        let shown = self.units.to_pixels(*length, extent).and_then(|px| self.units.from_pixels(px, ruler, extent)).unwrap_or(length.value);
        Field::new(label.clone()).show(ui, |ui, _| {
            if let Some(Some(value)) = self.input(ui, label.clone(), Some(shown), extent) {
                *length = GuideLength::new(value, ruler);
            }
        });
    }
}

/// The values of New Guide Layout.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct GuideLayoutValues {
    /// The layout's settings.
    pub options: GuideLayoutOptions,
    /// The guides' colour.
    pub color: GuideColor,
    /// Whether the app draws the layout while the dialog is open.
    pub preview: bool,
}

impl GuideLayoutValues {
    /// The values for `options` and `color`, with Preview on.
    pub fn new(options: GuideLayoutOptions, color: GuideColor) -> Self {
        GuideLayoutValues { options, color, preview: true }
    }
}

impl Default for GuideLayoutValues {
    fn default() -> Self {
        GuideLayoutValues::new(GuideLayoutOptions::default(), GuideColor::default())
    }
}

/// What New Guide Layout answers.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct GuideLayoutResult {
    /// The settings the user confirmed.
    pub options: GuideLayoutOptions,
    /// The guides OK commits: the existing guides (unless Clear Existing Guides) and the layout's.
    pub guides: Vec<Guide>,
    /// The colour chosen; the app may keep it as the guide colour preference.
    pub color: Rgba,
}

/// What New Guide Layout reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct GuideLayoutOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The answer, the pass the dialog closed as OK.
    pub result: Option<GuideLayoutResult>,
    /// The guides the app shows on the canvas in place of the document's while the dialog is open: the full set OK
    /// would commit. `None` with Preview off; the last valid set while a field is invalid.
    pub preview: Option<Vec<Guide>>,
    /// What the dialog asks of the host in this pass (the Custom colour).
    pub requests: HostRequests,
}

impl Outcome for GuideLayoutOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The last layout that was valid, for the preview while a field is invalid.
#[derive(Clone, Default)]
struct LastValid(Option<Vec<Guide>>);

/// New Guide Layout, Photoshop's dialog: Preset and Target lists, Color, the Columns and Rows groups (each a check
/// box with Number, Width or Height, and Gutter), the Margin group (a check box and Top, Left, Bottom, Right), Center
/// Columns and Clear Existing Guides; OK, Cancel and Preview.
///
/// - Keys: Tab walks the fields, Enter is OK when the layout is valid, Escape cancels. Holding Alt turns Cancel
///   into Reset, which restores the values from when the dialog opened and keeps the dialog open.
/// - Fields: every length takes any unit and shows the ruler unit; Width and Height left empty mean automatic size.
///   Typing in a layout field turns the preset to Custom; changing target, colour or Preview does not. A group whose
///   box is off greys its fields and keeps their values.
/// - Preview: while Preview is on and the layout is valid, [`GuideLayoutOutput::preview`] is the set the app draws.
///   An invalid layout disables OK and shows the reason under the fields.
/// - Colour: Custom asks the host for a colour (`requests`); the host's answers come back through
///   [`answers`](GuideLayoutDialog::answers).
/// - Values: the `values` binding holds them; the app keeps them for the next open.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct GuideLayoutDialog<'a> {
    units: GuideUnits,
    document: GuideRect,
    values: Bind<'a, GuideLayoutValues>,
    /// The selected artboards the Target list offers.
    #[prop]
    artboards: Vec<GuideRect>,
    /// The guides the document has; the layout is added to them.
    #[prop]
    existing: Vec<Guide>,
    /// The key of the first guide the layout makes.
    #[prop]
    first_key: u64,
    /// The host's answers to earlier requests.
    #[prop]
    answers: Vec<HostAnswer>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
}

impl<'a> GuideLayoutDialog<'a> {
    /// A dialog over `values` for the `document` measured in `units`.
    pub fn new(units: GuideUnits, document: GuideRect, values: Bind<'a, GuideLayoutValues>) -> Self {
        GuideLayoutDialog { units, document, values, artboards: Vec::new(), existing: Vec::new(), first_key: 0, answers: Vec::new(), open: None }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> GuideLayoutOutput {
        let GuideLayoutDialog { units, document, values, artboards, existing, first_key, answers, open } = self;
        let env = Env::of(ui);
        let words = env.words::<GuideDialogWords>();
        let preview_word = env.words::<ParamDialogWords>().preview.clone();
        let number_units = env.units();
        let mut dialog = FormDialog::new(words.layout_title.clone(), values).outer_gesture(true).answers(answers);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        let valid = RefCell::new(None::<Vec<Guide>>);
        let last = RefCell::new(None::<Vec<Guide>>);
        let previewing = Cell::new(false);
        let measure = Measure { words: &words, numbers: &number_units, units: &units, unparsed: Cell::new(false) };
        let rows = Form { document: &document, artboards: &artboards, preview_word: &preview_word };
        let shown = dialog.show(ui, |ui, form, values| {
            apply_color_answers(form.take_answers(), GUIDE_LAYOUT_COLOR_KEY, &mut values.color);
            measure.unparsed.set(false);
            layout_rows(ui, &measure, &rows, values, form.requests());
            previewing.set(values.preview);
            let rects = targets(&values.options, &document, &artboards);
            let made = values.options.lines(&rects, &existing, &units, first_key, Some(values.color.rgba()));
            let remembered = form.memory::<LastValid>();
            let ok = match made {
                Ok(guides) => {
                    remembered.0 = Some(guides.clone());
                    *valid.borrow_mut() = Some(guides);
                    true
                }
                Err(error) => {
                    *valid.borrow_mut() = None;
                    Notice::new(Tone::Bad, words.invalid.clone()).layout(NoticeLayout::Inline).show(ui);
                    Text::new(error.message(&words)).show(ui);
                    false
                }
            };
            *last.borrow_mut() = remembered.0.clone();
            ok && !measure.unparsed.get()
        });
        let result = shown.accepted.and_then(|values| {
            valid.borrow_mut().take().map(|guides| GuideLayoutResult { options: values.options, guides, color: values.color.rgba() })
        });
        let preview = previewing.get().then(|| last.into_inner()).flatten();
        GuideLayoutOutput { response: shown.response, closed: shown.closed, result, preview, requests: shown.requests }
    }
}

/// The rectangles the layout fills for its target.
fn targets(options: &GuideLayoutOptions, document: &GuideRect, artboards: &[GuideRect]) -> Vec<GuideRect> {
    match options.target {
        GuideTarget::Document => vec![*document],
        GuideTarget::SelectedArtboards => artboards.to_vec(),
    }
}

/// The widths and heights percentages are of: the first target's.
fn extents(options: &GuideLayoutOptions, document: &GuideRect, artboards: &[GuideRect]) -> (f64, f64) {
    let rect = targets(options, document, artboards).first().copied().unwrap_or(*document);
    (rect.width(), rect.height())
}

/// The fields of New Guide Layout.
fn layout_rows(ui: &mut Ui, measure: &Measure<'_>, form: &Form<'_>, values: &mut GuideLayoutValues, requests: &mut HostRequests) {
    let words = measure.words;
    let before = values.options;
    let (across, down) = extents(&values.options, form.document, form.artboards);
    FieldGroup::new(words.layout_title.clone()).show(ui, |ui, _| {
        preset_row(ui, words, &mut values.options);
        target_row(ui, words, &mut values.options, !form.artboards.is_empty());
        color_row(ui, words, &mut values.color, GUIDE_LAYOUT_COLOR_KEY, requests);
        group_box(ui, measure, &mut values.options.columns, GroupKind::Columns, across);
        group_box(ui, measure, &mut values.options.rows, GroupKind::Rows, down);
        margin_box(ui, measure, &mut values.options, across, down);
        let center = values.options.center_columns;
        if let Some(on) = Checkbox::new(Bind::given(center)).label(words.center_columns.clone()).show(ui).value().copied() {
            values.options.center_columns = on;
        }
        let clear = values.options.clear_existing;
        if let Some(on) = Checkbox::new(Bind::given(clear)).label(words.clear_existing.clone()).show(ui).value().copied() {
            values.options.clear_existing = on;
        }
        if let Some(on) = Checkbox::new(Bind::given(values.preview)).label(form.preview_word.clone()).show(ui).value().copied() {
            values.preview = on;
        }
    });
    if !values.options.same_fields(&before) && values.options.preset == before.preset {
        values.options.preset = GuideLayoutPreset::Custom;
    }
}

/// What the layout rows read besides the values.
struct Form<'a> {
    document: &'a GuideRect,
    artboards: &'a [GuideRect],
    preview_word: &'a Label,
}

fn preset_row(ui: &mut Ui, words: &GuideDialogWords, options: &mut GuideLayoutOptions) {
    let entries: Vec<Entry<GuideLayoutPreset>> = GuideLayoutPreset::ALL.into_iter().map(|preset| Entry::new(preset, preset.label(words))).collect();
    let current = options.preset;
    Field::new(words.preset.clone()).show(ui, |ui, _| {
        if let Some(picked) = Select::new(words.preset.clone(), Bind::given(current), &entries).show(ui).value().copied() {
            picked.apply(options);
            options.preset = picked;
        }
    });
}

fn target_row(ui: &mut Ui, words: &GuideDialogWords, options: &mut GuideLayoutOptions, artboards: bool) {
    let mut entries = vec![Entry::new(GuideTarget::Document, words.document.clone())];
    if artboards {
        entries.push(Entry::new(GuideTarget::SelectedArtboards, words.selected_artboards.clone()));
    }
    let current = options.target;
    Field::new(words.target.clone()).show(ui, |ui, _| {
        if let Some(picked) = Select::new(words.target.clone(), Bind::given(current), &entries).show(ui).value().copied() {
            options.target = picked;
        }
    });
}

#[derive(Clone, Copy, PartialEq)]
enum GroupKind {
    Columns,
    Rows,
}

fn group_box(ui: &mut Ui, measure: &Measure<'_>, group: &mut GuideGroup, kind: GroupKind, extent: f64) {
    let words = measure.words;
    let (title, size_label) = match kind {
        GroupKind::Columns => (words.columns.clone(), words.width.clone()),
        GroupKind::Rows => (words.rows.clone(), words.height.clone()),
    };
    let mut enabled = group.enabled;
    let on = group.enabled;
    GroupBox::new(title, Some(Bind::bound(&mut enabled))).fill(false).show(ui, |ui, _| {
        Field::new(words.number.clone()).enabled(on).show(ui, |ui, _| {
            let number = Some(f64::from(group.number));
            let field = NumberField::new(words.number.clone(), Bind::given(number)).range(1.0..=f64::from(MAX_COUNT)).step(1.0);
            let field = field.show(ui);
            measure.note(&field);
            if let Some(Some(value)) = field.value().copied() {
                group.number = (value.round() as u32).clamp(1, MAX_COUNT);
            }
        });
        let ruler = measure.units.ruler;
        let shown = group.size.and_then(|size| measure.units.to_pixels(size, extent)).and_then(|px| measure.units.from_pixels(px, ruler, extent));
        Field::new(size_label.clone()).enabled(on).tooltip(words.automatic_size.clone()).show(ui, |ui, _| {
            if let Some(typed) = measure.input(ui, size_label.clone(), shown, extent) {
                group.size = typed.map(|value| GuideLength::new(value, ruler));
            }
        });
        let mut gutter = group.gutter;
        measure.length(ui, words.gutter.clone(), &mut gutter, extent);
        group.gutter = gutter;
    });
    group.enabled = enabled;
}

fn margin_box(ui: &mut Ui, measure: &Measure<'_>, options: &mut GuideLayoutOptions, across: f64, down: f64) {
    let words = measure.words;
    let sides = [words.top.clone(), words.left.clone(), words.bottom.clone(), words.right.clone()];
    let mut enabled = options.margin_enabled;
    GroupBox::new(words.margin.clone(), Some(Bind::bound(&mut enabled))).fill(false).show(ui, |ui, _| {
        for (index, label) in sides.iter().enumerate() {
            let extent = if index % 2 == 0 { down } else { across };
            let mut length = options.margin[index];
            measure.length(ui, label.clone(), &mut length, extent);
            options.margin[index] = length;
        }
    });
    options.margin_enabled = enabled;
}

/// Builds the dialog over the host-owned values of a [`HostedGuideLayoutDialog`] for one pass.
type Build = dyn for<'b> Fn(Bind<'b, GuideLayoutValues>) -> GuideLayoutDialog<'b> + Send;

/// A hosted New Guide Layout: the `DialogHost` owns the values, and `build` makes the dialog over them each pass.
///
/// ```ignore
/// host.open(HostedGuideLayoutDialog::new(GuideLayoutValues::default(), move |values| {
///     GuideLayoutDialog::new(units, document, values).existing(guides.clone())
/// }));
/// ```
pub struct HostedGuideLayoutDialog {
    id: DialogId,
    values: GuideLayoutValues,
    build: Box<Build>,
    preview: Option<Vec<Guide>>,
    requests: HostRequests,
}

impl HostedGuideLayoutDialog {
    /// A dialog opening with `values`; `build` makes it over the values.
    pub fn new(values: GuideLayoutValues, build: impl for<'b> Fn(Bind<'b, GuideLayoutValues>) -> GuideLayoutDialog<'b> + Send + 'static) -> Self {
        HostedGuideLayoutDialog { id: dialog_id("guides.layout"), values, build: Box::new(build), preview: None, requests: HostRequests::new() }
    }

    /// The values the dialog shows.
    pub fn values(&self) -> &GuideLayoutValues {
        &self.values
    }

    /// The guides to draw on the canvas while the dialog is open, as of the last pass.
    pub fn preview(&self) -> Option<&[Guide]> {
        self.preview.as_deref()
    }

    /// What the dialog asked of the host and the host's answers: take the colour request, answer it.
    pub fn requests(&mut self) -> &mut HostRequests {
        &mut self.requests
    }
}

impl StackDialog for HostedGuideLayoutDialog {
    type Result = GuideLayoutResult;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<GuideLayoutResult>> {
        let shown = (self.build)(Bind::bound(&mut self.values)).answers(self.requests.take_answers()).show(ui);
        self.preview = shown.preview;
        self.requests.extend(shown.requests);
        match (shown.result, shown.closed) {
            (Some(result), _) => Some(DialogResult::Ok(result)),
            (None, Some(_)) => {
                self.requests.cancel();
                Some(DialogResult::Cancel)
            }
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some("guides.layout")
    }

    fn cancelled(&mut self, _: &egui::Context) {
        self.requests.cancel();
    }
}

/// The most a guide's position can be from the origin, in pixels.
pub(super) const GUIDE_LIMIT: f64 = MAX_GUIDE_PX;
