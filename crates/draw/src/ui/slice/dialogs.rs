//! [`SliceOptionsDialog`] and [`DivideSlicesDialog`]. PhotoCraft and VectorCraft both have them with the same fields;
//! what the fields mean at export is the app's.

use super::SliceWords;
use craft_behavior::Value;
use craft_color_ui::ui::{ChosenColor, ColorChoice, NamedColor};
use craft_theme::Srgba;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, Entry, FieldLabelRole, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, HeldAnswer, HeldOk, HoldsOk, StackDialog, dialog_id};
use craft_ui::display::{Shade, Text};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::patterns::form_dialogs::{FormDialog, HostRequests};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, LengthUnits, UnitKind};
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

/// The horizontal alignment of a No Image or HTML Text cell at export.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SliceCellAlign {
    /// The export's own alignment.
    #[default]
    Default,
    /// Aligned to the left.
    Left,
    /// Centred.
    Center,
    /// Aligned to the right.
    Right,
}

/// The vertical alignment of a No Image or HTML Text cell at export.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SliceCellVAlign {
    /// The export's own alignment.
    #[default]
    Default,
    /// Aligned to the top.
    Top,
    /// Aligned to the middle.
    Middle,
    /// Aligned to the text's baseline.
    Baseline,
    /// Aligned to the bottom.
    Bottom,
}

/// What a slice cell shows behind its content at export.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum SliceBackground {
    /// No background.
    #[default]
    None,
    /// The matte colour of the export.
    Matte,
    /// This colour.
    Color(Srgba),
}

/// The opaque white and black the Background row names; a colour equal to one shows as that row.
const WHITE: Srgba = Srgba::rgb(1.0, 1.0, 1.0);
const BLACK: Srgba = Srgba::rgb(0.0, 0.0, 0.0);

impl SliceBackground {
    /// The backgrounds the Background row names, by the id its `ColorChoice` stores.
    const NAMED: [(&'static str, SliceBackground); 4] = [
        ("none", SliceBackground::None),
        ("matte", SliceBackground::Matte),
        ("white", SliceBackground::Color(WHITE)),
        ("black", SliceBackground::Color(BLACK)),
    ];

    /// The row this background shows as: its named row, else the custom colour.
    fn chosen(self) -> ChosenColor {
        Self::NAMED.iter().find(|(_, listed)| *listed == self).map_or(ChosenColor::Custom, |(id, _)| ChosenColor::Named((*id).to_owned()))
    }

    /// The background the named row `id` stands for.
    fn named(id: &str) -> Option<Self> {
        Self::NAMED.iter().find(|(listed, _)| *listed == id).map(|(_, background)| *background)
    }
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
    /// The cell background.
    pub background: SliceBackground,
    /// The alignment of a No Image or HTML Text cell; JSON `hAlign` takes `default`, `left`, `center` or `right`.
    pub h_align: SliceCellAlign,
    /// The vertical alignment of a No Image or HTML Text cell; JSON `vAlign` takes `default`, `top`, `middle`,
    /// `baseline` or `bottom`.
    pub v_align: SliceCellVAlign,
}

/// The name of `kind` in the dialog's JSON fields.
fn kind_name(kind: SliceKind) -> &'static str {
    match kind {
        SliceKind::Image => "image",
        SliceKind::NoImage => "noImage",
        SliceKind::HtmlText => "htmlText",
    }
}

/// The JSON names of the horizontal alignments.
const H_ALIGNS: [(SliceCellAlign, &str); 4] =
    [(SliceCellAlign::Default, "default"), (SliceCellAlign::Left, "left"), (SliceCellAlign::Center, "center"), (SliceCellAlign::Right, "right")];

/// The JSON names of the vertical alignments.
const V_ALIGNS: [(SliceCellVAlign, &str); 5] = [
    (SliceCellVAlign::Default, "default"),
    (SliceCellVAlign::Top, "top"),
    (SliceCellVAlign::Middle, "middle"),
    (SliceCellVAlign::Baseline, "baseline"),
    (SliceCellVAlign::Bottom, "bottom"),
];

/// The JSON name of `value` in `table`.
fn name_of<T: Copy + PartialEq>(table: &[(T, &'static str)], value: T) -> &'static str {
    table.iter().find(|(listed, _)| *listed == value).map_or("default", |(_, name)| *name)
}

/// The value `table` names by the JSON string `value` of `field`.
fn pick<T: Copy>(table: &[(T, &'static str)], field: &str, value: &serde_json::Value) -> Result<T, String> {
    let name = value.as_str().ok_or_else(|| format!("field `{field}` takes a string, got {value}"))?;
    table.iter().find(|(_, listed)| *listed == name).map(|(listed, _)| *listed).ok_or_else(|| {
        let names: Vec<&str> = table.iter().map(|(_, listed)| *listed).collect();
        format!("field `{field}` takes one of {}, got `{name}`", names.join(", "))
    })
}

/// A labelled text field.
fn text_row(ui: &mut Ui, label: &Label, value: &mut String) {
    Field::new(label.clone()).show(ui, |ui, _| {
        TextField::new(label.clone(), Bind::bound(value)).show(ui);
    });
}

/// A labelled `Select` over `entries`.
fn select_row<T: Copy + Value>(ui: &mut Ui, label: &Label, value: &mut T, entries: &[Entry<T>]) {
    Field::new(label.clone()).show(ui, |ui, _| {
        if let Some(picked) = Select::new(label.clone(), Bind::given(*value), entries).show(ui).value().copied() {
            *value = picked;
        }
    });
}

/// Slice Options for one slice: Slice Type (the [`kinds`](Self::kinds) offered), then by type: Name, URL, Target,
/// Message Text and Alt Tag for an image; the cell's Text for No Image, a note for HTML Text, and for both the
/// Horizontal and Vertical alignment; then Background (None, Matte, White, Black or a custom colour, asked for with a
/// [`ColorRequest`]). OK answers the options; Cancel nothing. Enter is OK, Escape Cancel; the first field has the focus
/// when the dialog opens.
///
/// The custom colour asks for a colour: the dialog host takes
/// [`take_color_request`](StackDialog::take_color_request) after each pass, opens its picker and answers with
/// [`answer_color`](StackDialog::answer_color).
#[derive(craft_ui::contract::Pattern)]
pub struct SliceOptionsDialog {
    id: DialogId,
    options: SliceOptions,
    kinds: Vec<SliceKind>,
    requests: HostRequests,
    held: Option<HeldAnswer<SliceOptions>>,
}

impl SliceOptionsDialog {
    /// The dialog for `options`, offering Image and No Image (PhotoCraft's choices).
    pub fn new(options: SliceOptions) -> Self {
        SliceOptionsDialog {
            id: dialog_id("slice.options"),
            options,
            kinds: vec![SliceKind::Image, SliceKind::NoImage],
            requests: HostRequests::new(),
            held: None,
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
}

impl HoldsOk for SliceOptionsDialog {
    type Answer = SliceOptions;

    fn held_ok(mut self) -> Self {
        self.held = Some(HeldAnswer::new());
        self
    }

    fn take_ok(&mut self) -> Option<SliceOptions> {
        self.held.as_mut()?.take()
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        if let Some(held) = &mut self.held {
            held.answer(answer);
        }
    }
}

impl StackDialog for SliceOptionsDialog {
    type Result = SliceOptions;

    fn json_fields(&self) -> Option<serde_json::Value> {
        let options = &self.options;
        let background = match options.background {
            SliceBackground::None => serde_json::Value::Null,
            SliceBackground::Matte => serde_json::json!("matte"),
            SliceBackground::Color(color) => serde_json::Value::String(Srgba::rgb(color.r, color.g, color.b).to_hex()),
        };
        Some(serde_json::json!({
            "kind": kind_name(options.kind),
            "name": options.name,
            "url": options.url,
            "target": options.target,
            "message": options.message,
            "alt": options.alt,
            "cellText": options.cell_text,
            "background": background,
            "hAlign": name_of(&H_ALIGNS, options.h_align),
            "vAlign": name_of(&V_ALIGNS, options.v_align),
        }))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        let text =
            |value: &serde_json::Value| value.as_str().map(str::to_owned).ok_or_else(|| format!("field `{field}` takes a string, got {value}"));
        match field {
            "kind" => {
                let name = text(&value)?;
                let kind = self
                    .kinds
                    .iter()
                    .copied()
                    .find(|&kind| kind_name(kind) == name)
                    .ok_or_else(|| format!("field `kind` takes one of the offered types, got `{name}`"))?;
                self.options.kind = kind;
            }
            "name" => self.options.name = text(&value)?,
            "url" => self.options.url = text(&value)?,
            "target" => self.options.target = text(&value)?,
            "message" => self.options.message = text(&value)?,
            "alt" => self.options.alt = text(&value)?,
            "cellText" => self.options.cell_text = text(&value)?,
            "hAlign" => self.options.h_align = pick(&H_ALIGNS, field, &value)?,
            "vAlign" => self.options.v_align = pick(&V_ALIGNS, field, &value)?,
            "background" => {
                self.options.background = match &value {
                    serde_json::Value::Null => SliceBackground::None,
                    serde_json::Value::String(name) if name == "matte" => SliceBackground::Matte,
                    serde_json::Value::String(hex) => {
                        let digits = hex.strip_prefix('#').unwrap_or(hex);
                        if digits.len() != 6 {
                            return Err(format!("field `background` takes six hex digits, \"matte\" or null, got `{hex}`"));
                        }
                        SliceBackground::Color(
                            Srgba::from_hex(digits)
                                .ok_or_else(|| format!("field `background` takes six hex digits, \"matte\" or null, got `{hex}`"))?,
                        )
                    }
                    other => return Err(format!("field `background` takes six hex digits, \"matte\" or null, got {other}")),
                };
            }
            _ => return Err(format!("unknown field `{field}`")),
        }
        ctx.request_repaint();
        Ok(())
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn take_color_request(&mut self) -> Option<ColorRequest> {
        self.requests.take_color()
    }

    fn answer_color(&mut self, key: &str, color: Srgba) {
        if key == ColorChoice::CUSTOM_KEY {
            self.requests.answer_color(key, color);
        }
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<SliceOptions>> {
        let words = Env::of(ui).words::<SliceWords>();
        let kind_label = |kind: SliceKind| match kind {
            SliceKind::Image => words.image.clone(),
            SliceKind::NoImage => words.no_image.clone(),
            _ => words.html_text.clone(),
        };
        let kinds: Vec<Entry<SliceKind>> = self.kinds.iter().map(|&kind| Entry::new(kind, kind_label(kind))).collect();
        let h_aligns: Vec<Entry<SliceCellAlign>> = [
            (SliceCellAlign::Default, &words.align_default),
            (SliceCellAlign::Left, &words.align_left),
            (SliceCellAlign::Center, &words.align_center),
            (SliceCellAlign::Right, &words.align_right),
        ]
        .into_iter()
        .map(|(value, label)| Entry::new(value, label.clone()))
        .collect();
        let v_aligns: Vec<Entry<SliceCellVAlign>> = [
            (SliceCellVAlign::Default, &words.valign_default),
            (SliceCellVAlign::Top, &words.valign_top),
            (SliceCellVAlign::Middle, &words.valign_middle),
            (SliceCellVAlign::Baseline, &words.valign_baseline),
            (SliceCellVAlign::Bottom, &words.valign_bottom),
        ]
        .into_iter()
        .map(|(value, label)| Entry::new(value, label.clone()))
        .collect();
        let background_labels = [&words.background_none, &words.background_matte, &words.background_white, &words.background_black];
        let measuring = craft_ui::runtime::measure::active(ui.ctx());
        let answers = if measuring { Vec::new() } else { self.requests.take_answers() };
        let mut dialog = FormDialog::new(words.options_title.clone(), Bind::bound(&mut self.options)).answers(answers);
        if let Some(held) = &mut self.held {
            dialog = dialog.held(held);
        }
        let shown = dialog.show(ui, |ui, form, options| {
            for answer in form.take_answers() {
                if let HostAnswer::Color { key, color, .. } = answer
                    && key == ColorChoice::CUSTOM_KEY
                {
                    options.background = SliceBackground::Color(color);
                }
            }
            FieldGroup::new(words.options_title.clone()).show(ui, |ui, _| {
                Field::new(words.kind.clone()).show(ui, |ui, _| {
                    if let Some(kind) = Select::new(words.kind.clone(), Bind::given(options.kind), &kinds).show(ui).value().copied() {
                        options.kind = kind;
                    }
                });
                if options.kind == SliceKind::Image {
                    for (label, value) in [
                        (&words.name, &mut options.name),
                        (&words.url, &mut options.url),
                        (&words.target, &mut options.target),
                        (&words.message, &mut options.message),
                        (&words.alt, &mut options.alt),
                    ] {
                        text_row(ui, label, value);
                    }
                } else {
                    if options.kind == SliceKind::NoImage {
                        Field::new(words.cell_text.clone()).show(ui, |ui, _| {
                            TextField::new(words.cell_text.clone(), Bind::bound(&mut options.cell_text)).lines(4).show(ui);
                        });
                    } else {
                        Text::new(words.html_note.clone()).shade(Shade::Muted).show(ui);
                    }
                    select_row(ui, &words.h_align, &mut options.h_align, &h_aligns);
                    select_row(ui, &words.v_align, &mut options.v_align, &v_aligns);
                }
                let chosen = options.background.chosen();
                let custom = match options.background {
                    SliceBackground::Color(color) if chosen == ChosenColor::Custom => Some(color),
                    _ => None,
                };
                Field::new(words.background.clone()).show(ui, |ui, _| {
                    let named = SliceBackground::NAMED.iter().zip(background_labels).map(|((id, background), label)| {
                        let swatch = match background {
                            SliceBackground::Color(color) => Some(*color),
                            _ => None,
                        };
                        NamedColor::new(*id, label.clone(), swatch)
                    });
                    let picked = ColorChoice::new(words.background.clone(), &chosen).named(named).custom(custom).show(ui);
                    if let Some(ChosenColor::Named(id)) = &picked.chosen
                        && let Some(background) = SliceBackground::named(id)
                    {
                        options.background = background;
                    }
                    if let Some(request) = picked.pick_color {
                        form.requests().request_color(request);
                    }
                });
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
        let Some(held) = &mut self.held else { return Some(self.options.clone()) };
        if let Some(answer) = held.finish() {
            return Some(answer);
        }
        if !held.is_waiting() {
            held.file(self.options.clone());
        }
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.as_ref().and_then(HeldAnswer::state)
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
    /// Into slices this long, a length in the dialog's stored unit above 0 (the last takes what is left).
    Size(f64),
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

/// One axis of the dialog: its check box, whether the value is a size, and a count and a size. Both values are kept, so
/// a switch between Count and Size, or an axis switched off, loses neither.
#[derive(Clone, Copy, Debug, PartialEq)]
struct AxisState {
    on: bool,
    by_size: bool,
    count: u32,
    size: f64,
}

impl AxisState {
    /// The most slices one axis divides into (PhotoCraft's clamp).
    const MAX_COUNT: u32 = 1000;
    /// The largest slice size, in the dialog's stored unit.
    const MAX_SIZE: f64 = 300_000.0;

    /// The axis for `divide`; the value `divide` does not give starts at `off_value`.
    fn of(divide: Option<DivideBy>, off_value: u32) -> Self {
        let off_size = f64::from(off_value);
        match divide {
            Some(DivideBy::Count(n)) => AxisState { on: true, by_size: false, count: n.clamp(1, Self::MAX_COUNT), size: off_size },
            Some(DivideBy::Size(size)) => AxisState { on: true, by_size: true, count: off_value, size: Self::size_of(size, off_size) },
            None => AxisState { on: false, by_size: false, count: off_value, size: off_size },
        }
    }

    /// `size` within the size range; `fallback` when it is not a number.
    fn size_of(size: f64, fallback: f64) -> f64 {
        if size.is_finite() { size.clamp(1.0, Self::MAX_SIZE) } else { fallback }
    }

    fn answer(self) -> Option<DivideBy> {
        self.on.then_some(if self.by_size { DivideBy::Size(self.size) } else { DivideBy::Count(self.count) })
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
/// ("slices, evenly spaced") or a size ("per slice", a length in the [`lengths`](Self::lengths) units), counts and
/// sizes at least 1. OK answers [`DivideSlices`] and is off while neither axis is on; Cancel answers nothing. Enter
/// is OK, Escape Cancel.
#[derive(craft_ui::contract::Pattern)]
pub struct DivideSlicesDialog {
    id: DialogId,
    axes: Axes,
    lengths: LengthUnits,
    held: Option<HeldAnswer<Axes>>,
}

impl DivideSlicesDialog {
    /// The dialog starting from `divide`.
    pub fn new(divide: DivideSlices) -> Self {
        DivideSlicesDialog {
            id: dialog_id("slice.divide"),
            axes: Axes::of(divide),
            lengths: LengthUnits::new(UnitKind::Pixels, UnitKind::Pixels, 72.0),
            held: None,
        }
    }

    /// The units the size field keeps and shows its length in; pixels in both, at 72 per inch, by default.
    #[must_use]
    pub fn lengths(mut self, lengths: LengthUnits) -> Self {
        self.lengths = lengths;
        self
    }

    /// The size an axis that opened on a count switches to, for rows and columns, in the stored unit.
    #[must_use]
    pub fn start_sizes(mut self, rows: f64, columns: f64) -> Self {
        for (axis, size) in [(&mut self.axes.rows, rows), (&mut self.axes.columns, columns)] {
            if !axis.by_size {
                axis.size = AxisState::size_of(size, axis.size);
            }
        }
        self
    }
}

/// One axis: its check box, then (while it is on) what the value counts and the value. Both axes show the same Divide
/// By and value labels, so those rows describe their controls, and each control is named by its label and the axis's
/// caption.
fn axis_rows(ui: &mut Ui, axis: &mut AxisState, caption: &Label, words: &SliceWords, lengths: LengthUnits) {
    Field::new(caption.clone()).show(ui, |ui, _| {
        Checkbox::new(Bind::bound(&mut axis.on)).label(caption.clone()).show(ui);
    });
    let in_axis = |control: &Label| Label::from(words.axis_control.fill(&[("control", control.as_str()), ("axis", caption.as_str())]));
    let unit = if axis.by_size { words.size.clone() } else { words.count.clone() };
    let describes = || SideLabel::new().role(FieldLabelRole::Describes);
    Field::new(words.divide_by.clone()).side(describes()).enabled(axis.on).show(ui, |ui, _| {
        RadioGroup::new(in_axis(&words.divide_by), Bind::bound(&mut axis.by_size)).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Radio::new(false).label(words.count.clone()).show(ui);
                Radio::new(true).label(words.size.clone()).show(ui);
            });
        });
    });
    let value_name = in_axis(&unit);
    Field::new(unit).side(describes()).enabled(axis.on).show(ui, |ui, _| {
        if axis.by_size {
            let field = NumberField::length(value_name.clone(), Bind::given(Some(axis.size)), lengths).range(1.0..=AxisState::MAX_SIZE).show(ui);
            if let Some(Some(size)) = field.value().copied() {
                axis.size = AxisState::size_of(size, axis.size);
            }
        } else {
            let max = f64::from(AxisState::MAX_COUNT);
            let field = NumberField::new(value_name.clone(), Bind::given(Some(f64::from(axis.count)))).range(1.0..=max).commit_decimals(0).show(ui);
            if let Some(Some(count)) = field.value().copied() {
                axis.count = count.round().clamp(1.0, max) as u32;
            }
        }
    });
}

impl HoldsOk for DivideSlicesDialog {
    type Answer = DivideSlices;

    fn held_ok(mut self) -> Self {
        self.held = Some(HeldAnswer::new());
        self
    }

    fn take_ok(&mut self) -> Option<DivideSlices> {
        self.held.as_mut()?.take()?.answer()
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        if let Some(held) = &mut self.held {
            held.answer(answer);
        }
    }
}

impl StackDialog for DivideSlicesDialog {
    type Result = DivideSlices;

    fn json_fields(&self) -> Option<serde_json::Value> {
        let Axes { rows, columns } = self.axes;
        let axis = |axis: AxisState| match (axis.on, axis.by_size) {
            (false, _) => serde_json::Value::Null,
            (true, true) => serde_json::json!(axis.size),
            (true, false) => serde_json::json!(axis.count),
        };
        let by = |axis: AxisState| if axis.by_size { "size" } else { "count" };
        Some(serde_json::json!({ "rows": axis(rows), "rowsBy": by(rows), "columns": axis(columns), "columnsBy": by(columns) }))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        let (axis, what) = match field {
            "rows" | "rowsBy" => (&mut self.axes.rows, field.trim_end_matches("By")),
            "columns" | "columnsBy" => (&mut self.axes.columns, field.trim_end_matches("By")),
            _ => return Err(format!("unknown field `{field}`")),
        };
        if field.ends_with("By") {
            axis.by_size = match value.as_str() {
                Some("count") => false,
                Some("size") => true,
                _ => return Err(format!("field `{field}` takes \"count\" or \"size\", got {value}")),
            };
        } else if value.is_null() {
            axis.on = false;
        } else {
            let number =
                value.as_f64().filter(|number| number.is_finite()).ok_or_else(|| format!("field `{what}` takes a number or null, got {value}"))?;
            if axis.by_size {
                axis.size = AxisState::size_of(number, axis.size);
            } else {
                axis.count = number.round().clamp(1.0, f64::from(AxisState::MAX_COUNT)) as u32;
            }
            axis.on = true;
        }
        ctx.request_repaint();
        Ok(())
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<DivideSlices>> {
        let words = Env::of(ui).words::<SliceWords>();
        let lengths = self.lengths;
        let mut dialog = FormDialog::new(words.divide_title.clone(), Bind::bound(&mut self.axes));
        if let Some(held) = &mut self.held {
            dialog = dialog.held(held);
        }
        let shown = dialog.show(ui, |ui, _, axes| {
            FieldGroup::new(words.divide_title.clone()).show(ui, |ui, _| {
                axis_rows(ui, &mut axes.rows, &words.horizontally, &words, lengths);
                axis_rows(ui, &mut axes.columns, &words.vertically, &words, lengths);
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
        let answer = self.axes.answer();
        let Some(held) = &mut self.held else { return answer };
        if let Some(axes) = held.finish() {
            return axes.answer();
        }
        if !held.is_waiting() && answer.is_some() {
            held.file(self.axes);
        }
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.as_ref().and_then(HeldAnswer::state)
    }
}
