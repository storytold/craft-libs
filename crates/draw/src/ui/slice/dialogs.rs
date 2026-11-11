//! [`SliceOptionsDialog`] and [`DivideSlicesDialog`]. PhotoCraft and VectorCraft both have them with the same fields;
//! what the fields mean at export is the app's.

use super::SliceWords;
use craft_theme::Srgba;
use craft_ui::choice::Swatch;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::patterns::form_dialogs::{FormDialog, HostRequests};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, UnitKind};
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::runtime::requests::{ColorRequest, HostAnswer};
use craft_ui::text::{NumberField, TextField};
use egui::Ui;

/// What a slice holds at export.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SliceKind {
    /// The slice exports as an image.
    #[default]
    Image,
    /// The slice exports without an image.
    NoImage,
    /// Illustrator's HTML Text (Photoshop's Table).
    HtmlText,
}

/// A slice's options.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SliceOptions {
    /// What the slice holds at export.
    pub kind: SliceKind,
    /// The slice's name.
    pub name: String,
    /// The URL the slice links to.
    pub url: String,
    /// The link's target frame.
    pub target: String,
    /// The message shown for the slice.
    pub message: String,
    /// Image slices only.
    pub alt: String,
    /// No Image slices only.
    pub cell_text: String,
    /// Whether the Background row shows (PhotoCraft's; VectorCraft has none).
    pub background_row: bool,
    /// The cell background; `None` for none.
    pub background: Option<Srgba>,
}

/// The colour a background starts with when None is switched to Colour.
const BACKGROUND_START: Srgba = Srgba::rgb(1.0, 1.0, 1.0);

/// Slice Options for one slice: Slice Type (the [`kinds`](Self::kinds) offered), Name, URL, Target, Message Text, then
/// Alt Tag (Image) or Text (No Image, HTML Text), and the optional Background row (None or a colour, asked for with a
/// [`ColorRequest`]). OK answers the options; Cancel nothing. Enter is OK, Escape Cancel; the Name field has the focus
/// when the dialog opens.
///
/// The Background swatch asks for a colour: the app takes [`pick_color`](Self::pick_color) after each pass, opens its
/// picker and answers with [`set_color`](Self::set_color).
pub struct SliceOptionsDialog {
    id: DialogId,
    options: SliceOptions,
    kinds: Vec<SliceKind>,
    requests: HostRequests,
}

impl SliceOptionsDialog {
    /// The key of the [`ColorRequest`] the Background row reports.
    pub const BACKGROUND_KEY: &'static str = "slice.options.background";

    /// The dialog for `options`, offering Image and No Image (PhotoCraft's choices).
    pub fn new(options: SliceOptions) -> Self {
        SliceOptionsDialog {
            id: dialog_id("slice.options"),
            options,
            kinds: vec![SliceKind::Image, SliceKind::NoImage],
            requests: HostRequests::new(),
        }
    }

    /// The slice types offered (VectorCraft adds HTML Text).
    #[must_use]
    pub fn kinds(mut self, kinds: &[SliceKind]) -> Self {
        self.kinds = kinds.to_vec();
        self
    }

    /// The options as the fields show them now.
    pub fn options(&self) -> &SliceOptions {
        &self.options
    }

    /// The Background swatch was clicked in the pass since the last call: the request for the app's picker, keyed
    /// [`BACKGROUND_KEY`](Self::BACKGROUND_KEY). Returned once.
    pub fn pick_color(&mut self) -> Option<ColorRequest> {
        self.requests.take_color()
    }

    /// The colour the app's picker returned for `key`; other keys are ignored.
    pub fn set_color(&mut self, key: &str, color: Srgba) {
        if key == Self::BACKGROUND_KEY {
            self.requests.answer_color(key, color);
        }
    }
}

impl StackDialog for SliceOptionsDialog {
    type Result = SliceOptions;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<SliceOptions>> {
        let words = Env::of(ui).words::<SliceWords>();
        let kind_label = |kind: SliceKind| match kind {
            SliceKind::Image => words.image.clone(),
            SliceKind::NoImage => words.no_image.clone(),
            _ => words.html_text.clone(),
        };
        let kinds: Vec<Entry<SliceKind>> = self.kinds.iter().map(|&kind| Entry::new(kind, kind_label(kind))).collect();
        let backgrounds = [Entry::new(false, words.background_none.clone()), Entry::new(true, words.background_color_choice.clone())];
        let answers = self.requests.take_answers();
        let shown = FormDialog::new(words.options_title.clone(), Bind::bound(&mut self.options)).answers(answers).show(ui, |ui, form, options| {
            for answer in form.take_answers() {
                if let HostAnswer::Color { key, color, .. } = answer
                    && key == Self::BACKGROUND_KEY
                {
                    options.background = Some(color);
                }
            }
            let first = form.first();
            FieldGroup::new(words.options_title.clone()).show(ui, |ui, _| {
                Field::new(words.kind.clone()).show(ui, |ui, _| {
                    if let Some(kind) = Select::new(words.kind.clone(), Bind::given(options.kind), &kinds).show(ui).value().copied() {
                        options.kind = kind;
                    }
                });
                let name = Field::new(words.name.clone())
                    .show(ui, |ui, _| TextField::new(words.name.clone(), Bind::bound(&mut options.name)).show(ui).response.id);
                if first && let Some(id) = name.inner {
                    focus::request(ui.ctx(), FocusTarget::Id(id));
                }
                for (label, value) in [(&words.url, &mut options.url), (&words.target, &mut options.target), (&words.message, &mut options.message)] {
                    Field::new(label.clone()).show(ui, |ui, _| {
                        TextField::new(label.clone(), Bind::bound(value)).show(ui);
                    });
                }
                let (label, value) =
                    if options.kind == SliceKind::Image { (&words.alt, &mut options.alt) } else { (&words.cell_text, &mut options.cell_text) };
                Field::new(label.clone()).show(ui, |ui, _| {
                    TextField::new(label.clone(), Bind::bound(value)).show(ui);
                });
                if options.background_row {
                    Field::new(words.background.clone()).show(ui, |ui, _| {
                        Row::new().show(ui, |ui, _| {
                            let colour = options.background.is_some();
                            if let Some(on) = Select::new(words.background_kind.clone(), Bind::given(colour), &backgrounds).show(ui).value().copied()
                            {
                                options.background = on.then(|| options.background.unwrap_or(BACKGROUND_START));
                            }
                            if let Some(color) = options.background
                                && let Some(request) =
                                    Swatch::new(words.background_color.clone(), color).picker(Self::BACKGROUND_KEY).show(ui).pick_color
                            {
                                form.requests().request_color(request);
                            }
                        });
                    });
                }
            });
            true
        });
        self.requests.extend(shown.requests);
        match (shown.accepted, shown.closed) {
            (Some(options), _) => Some(DialogResult::Ok(options)),
            (None, Some(_)) => {
                self.requests.cancel();
                Some(DialogResult::Cancel)
            }
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<SliceOptions> {
        Some(self.options.clone())
    }

    fn cancelled(&mut self, _: &egui::Context) {
        self.requests.cancel();
    }
}

/// How one axis of a slice is divided.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum DivideBy {
    /// Into this many slices, evenly spaced.
    Count(u32),
    /// Into slices this many pixels long (the last takes what is left).
    Size(u32),
}

/// Divide Slice's answer: rows (divided horizontally) and columns (vertically); `None` leaves that axis.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DivideSlices {
    /// How to divide horizontally into rows, or `None` to leave rows.
    pub rows: Option<DivideBy>,
    /// How to divide vertically into columns, or `None` to leave columns.
    pub columns: Option<DivideBy>,
}

impl DivideSlices {
    /// Divide into `rows` and `columns`.
    pub fn new(rows: Option<DivideBy>, columns: Option<DivideBy>) -> Self {
        DivideSlices { rows, columns }
    }
}

impl Default for DivideSlices {
    /// Two rows, columns off (PhotoCraft's `horizontal` 2, `vertical` 1).
    fn default() -> Self {
        DivideSlices { rows: Some(DivideBy::Count(2)), columns: None }
    }
}

/// One axis of the dialog: its check box, whether the value is a size, and the value. An axis that is off keeps its
/// value for when it is switched on.
#[derive(Clone, Copy, Debug, PartialEq)]
struct AxisState {
    on: bool,
    by_size: bool,
    value: u32,
}

impl AxisState {
    /// The most slices one axis divides into (PhotoCraft's clamp).
    const MAX_COUNT: u32 = 1000;
    /// The largest slice size in pixels.
    const MAX_SIZE: u32 = 300_000;

    fn of(divide: Option<DivideBy>, off_value: u32) -> Self {
        match divide {
            Some(DivideBy::Count(n)) => AxisState { on: true, by_size: false, value: n.clamp(1, Self::MAX_COUNT) },
            Some(DivideBy::Size(n)) => AxisState { on: true, by_size: true, value: n.clamp(1, Self::MAX_SIZE) },
            None => AxisState { on: false, by_size: false, value: off_value },
        }
    }

    fn answer(self) -> Option<DivideBy> {
        self.on.then_some(if self.by_size { DivideBy::Size(self.value) } else { DivideBy::Count(self.value) })
    }

    fn max(self) -> u32 {
        if self.by_size { Self::MAX_SIZE } else { Self::MAX_COUNT }
    }
}

/// Both axes: rows, then columns.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Axes {
    rows: AxisState,
    columns: AxisState,
}

impl Default for Axes {
    fn default() -> Self {
        Axes::of(DivideSlices::default())
    }
}

impl Axes {
    fn of(divide: DivideSlices) -> Self {
        Axes { rows: AxisState::of(divide.rows, 2), columns: AxisState::of(divide.columns, 1) }
    }

    /// The answer the fields give now; `None` while both axes are off.
    fn answer(self) -> Option<DivideSlices> {
        let (rows, columns) = (self.rows.answer(), self.columns.answer());
        (rows.is_some() || columns.is_some()).then_some(DivideSlices { rows, columns })
    }
}

/// Divide Slice: "Divide Horizontally Into" and "Divide Vertically Into", each a check box with a choice of a count
/// ("slices, evenly spaced") or a size ("pixels per slice"), counts and sizes at least 1. OK answers
/// [`DivideSlices`] and is off while neither axis is on; Cancel answers nothing. Enter is OK, Escape Cancel.
pub struct DivideSlicesDialog {
    id: DialogId,
    axes: Axes,
}

impl DivideSlicesDialog {
    /// The dialog starting from `divide`.
    pub fn new(divide: DivideSlices) -> Self {
        DivideSlicesDialog { id: dialog_id("slice.divide"), axes: Axes::of(divide) }
    }
}

/// One axis: its check box, then (while it is on) what the value counts and the value.
fn axis_rows(ui: &mut Ui, axis: &mut AxisState, caption: &craft_ui::runtime::env::Label, words: &SliceWords) {
    Field::new(caption.clone()).show(ui, |ui, _| {
        Checkbox::new(Bind::bound(&mut axis.on)).label(caption.clone()).show(ui);
    });
    let unit = if axis.by_size { words.size.clone() } else { words.count.clone() };
    Field::new(words.divide_by.clone()).enabled(axis.on).show(ui, |ui, _| {
        RadioGroup::new(words.divide_by.clone(), Bind::bound(&mut axis.by_size)).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Radio::new(false).label(words.count.clone()).show(ui);
                Radio::new(true).label(words.size.clone()).show(ui);
            });
        });
    });
    Field::new(unit).enabled(axis.on).show(ui, |ui, _| {
        let max = axis.max();
        let field = NumberField::new(words.divide_value.clone(), Bind::given(Some(f64::from(axis.value))))
            .range(1.0..=f64::from(max))
            .commit_decimals(0)
            .unit_kind(UnitKind::Pixels)
            .show(ui);
        if let Some(Some(value)) = field.value().copied() {
            axis.value = value.round().clamp(1.0, f64::from(max)) as u32;
        }
        axis.value = axis.value.min(max);
    });
}

impl StackDialog for DivideSlicesDialog {
    type Result = DivideSlices;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<DivideSlices>> {
        let words = Env::of(ui).words::<SliceWords>();
        let shown = FormDialog::new(words.divide_title.clone(), Bind::bound(&mut self.axes)).show(ui, |ui, _, axes| {
            FieldGroup::new(words.divide_title.clone()).show(ui, |ui, _| {
                axis_rows(ui, &mut axes.rows, &words.horizontally, &words);
                axis_rows(ui, &mut axes.columns, &words.vertically, &words);
            });
            axes.answer().is_some()
        });
        match (shown.accepted, shown.closed) {
            (Some(axes), _) => axes.answer().map(DialogResult::Ok).or(Some(DialogResult::Cancel)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<DivideSlices> {
        self.axes.answer()
    }
}
