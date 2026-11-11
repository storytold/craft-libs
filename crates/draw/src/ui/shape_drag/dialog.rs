//! [`CreateShapeDialog`]: Create Rectangle, Ellipse, Triangle, Polygon (and Illustrator's Rounded
//! Rectangle and Star), opened by a Shape tool click.

use std::cell::RefCell;
use std::ops::RangeInclusive;
use std::rc::Rc;

use craft_behavior::Mixed;
use craft_theme::variant::Tone;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, Props as _, Remember as _, Show as _, ShowContainer as _};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::patterns::form_dialogs::{FormDialog, GuideUnits};
use craft_ui::pickers::UnitSelect;
use craft_ui::runtime::env::{Env, Label, UnitKind, Units};
use craft_ui::runtime::memory::SaveKey;
use craft_ui::sections::DialogRole;
use craft_ui::text::{LinkMode, LinkedFields, LinkedLayout, NumberField};
use egui::Ui;
use kurbo::{Point, Rect};
use serde::{Deserialize, Serialize};

use super::ShapeKind;
use super::words::CreateShapeWords;
use crate::geometry::shape::{SIDES, ShapeGeometry};

/// The farthest the click may lie from the origin on either axis, in pixels: the same bound as
/// a side (PhotoCraft's `MAX_PX`, `shape_dialog.rs:19`, checked at `:111`).
const MAX_POSITION: f64 = 300_000.0;

/// The largest radius a field takes, in pixels (PhotoCraft's `value_field` range,
/// `shape_dialog.rs:223`); the shape clamps each radius to half its shorter side.
const MAX_RADIUS: f64 = 10_000.0;

/// What Create Shape opens with: the kind, the clicked point and the document's units, and the
/// values the fields start from. The dialog replaces width, height, From Center and the star ratio
/// with the remembered ones for the kind on its first pass unless [`recalled`](Self::recalled) is
/// set.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CreateShapeOptions {
    /// Which shape the dialog creates.
    pub kind: ShapeKind,
    /// The click in document pixels, rounded: the shape's top left, or its centre with From Center.
    pub at: Point,
    /// The ruler unit the size fields show, and the document's size and resolution for percent
    /// and physical units. Pixels when `None`.
    pub units: Option<GuideUnits>,
    /// Width and height in document pixels (100 × 100 by default).
    pub size: (f64, f64),
    /// A rectangle's radii, top-left, top-right, bottom-right, bottom-left (from the tool's corner
    /// radius).
    pub radii: [f64; 4],
    /// "Link radii": one radius for all four corners; on by default.
    pub link_radii: bool,
    /// A polygon's sides or a star's points (from the tool).
    pub sides: u32,
    /// A polygon's or star's inner over outer radius (1 for a plain polygon; the dialog shows
    /// 1–100 % for Polygon and Star).
    pub star_ratio: f64,
    /// "From Center": the click is the shape's centre.
    pub from_center: bool,
    /// The remembered values are already in (a reopened dialog, a test).
    pub recalled: bool,
}

impl Default for CreateShapeOptions {
    /// A rectangle at the origin with the defaults of [`initial`](Self::initial).
    fn default() -> Self {
        CreateShapeOptions::initial(ShapeKind::Rectangle, Point::ZERO)
    }
}

impl CreateShapeOptions {
    /// Width and height in pixels: 1 to 300,000 (PhotoCraft's `MAX_PX`, `shape_dialog.rs:19`).
    pub const SIZE: RangeInclusive<f64> = 1.0..=300_000.0;

    /// The dialog for a click of `kind` at `at`, with the defaults (100 × 100 px, square corners,
    /// radii linked, 5 sides, star ratio 1, not from the centre) in pixels.
    pub fn initial(kind: ShapeKind, at: Point) -> Self {
        CreateShapeOptions {
            kind,
            at,
            units: None,
            size: (100.0, 100.0),
            radii: [0.0; 4],
            link_radii: true,
            sides: 5,
            star_ratio: 1.0,
            from_center: false,
            recalled: false,
        }
    }

    /// The ruler unit and the document the size fields convert through.
    #[must_use]
    pub fn units(mut self, units: GuideUnits) -> Self {
        self.units = Some(units);
        self
    }

    /// The tool's corner radius (all four corners), sides and star ratio.
    #[must_use]
    pub fn tool(mut self, corner_radius: f64, sides: u32, star_ratio: f64) -> Self {
        self.radii = [corner_radius; 4];
        self.sides = sides;
        self.star_ratio = star_ratio;
        self
    }

    /// Opens with these values and skips the remembered ones.
    #[must_use]
    pub fn recalled(mut self) -> Self {
        self.recalled = true;
        self
    }
}

/// What Create Shape answers on OK.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CreatedShape {
    /// Which shape was created.
    pub kind: ShapeKind,
    /// The shape at the click (centred on it with From Center, which may leave its top left on a
    /// half pixel, as PhotoCraft's).
    pub geometry: ShapeGeometry,
    /// Whether From Center was on.
    pub from_center: bool,
}

/// The remembered values of one kind. The ratio is the field's percentage.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Memory {
    width: Option<f64>,
    height: Option<f64>,
    from_center: Option<bool>,
    star_ratio: Option<f64>,
}

/// Photoshop's Create Rectangle (Ellipse, Triangle, Polygon) dialog, on the app's `DialogHost`.
///
/// - Title: "Create Rectangle" and so on ([`CreateShapeWords`]).
/// - Fields: Width and Height (a number with a unit select each, in the ruler unit to start; the unit
///   picked for a field is not remembered, storytold/photocraft#7786). Rectangle and Rounded
///   Rectangle: four radii Top Left, Top Right, Bottom Left, Bottom Right in two columns under one
///   "Radius" label, with the "Link radii" toggle between the columns (`LinkMode::Equal`, on by
///   default). Polygon and Star: Sides (3–100) and Star Ratio (1–100 %, default 100); the Polygon drag
///   keeps ratio 1. Then "From Center".
/// - A width or height outside [`CreateShapeOptions::SIZE`] shows [`CreateShapeWords::bad_size`] under
///   the fields and disables OK; so does a click farther than 300,000 px from the origin
///   ([`CreateShapeWords::bad_position`]).
/// - OK answers [`CreatedShape`] at the click (centred on it with From Center, unrounded) and
///   remembers width, height, From Center and the star ratio per kind; Cancel answers nothing and
///   remembers nothing.
/// - Keyboard: Enter is OK, Esc Cancel; every field is a Tab stop in reading order.
pub struct CreateShapeDialog {
    id: DialogId,
    options: CreateShapeOptions,
    /// Each size field's unit for this opening.
    units: [UnitKind; 2],
}

/// The document a dialog without [`CreateShapeOptions::units`] measures in: pixels, with no percent.
fn pixels_only() -> GuideUnits {
    GuideUnits::new(UnitKind::Pixels, 1.0, 1.0, UnitKind::POINTS_PER_INCH)
}

/// `value` rounded to a thousandth of a pixel (PhotoCraft stores sizes so).
fn thousandths(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// `value` rounded to the places a field in `unit` shows.
fn shown_places(value: f64, unit: UnitKind) -> f64 {
    let scale = 10f64.powi(i32::try_from(*unit.decimals().end()).unwrap_or(0));
    (value * scale).round() / scale
}

/// The shape the fields describe at the click, or why not: the error text under the fields.
fn answer(o: &CreateShapeOptions, words: &CreateShapeWords, units: &Units) -> Result<CreatedShape, String> {
    // pc: crates/ui-egui/src/dialogs/shape_dialog.rs:102-138 `geometry` (from the fields: at the
    // rounded `__at`, centred with From Center, radii, sides, star ratio), :42 `valid`, :19 `MAX_PX`.
    let side = |value: f64, caption: &str| {
        if value.is_finite() && CreateShapeOptions::SIZE.contains(&value) {
            return Ok(value);
        }
        let max = units.number(*CreateShapeOptions::SIZE.end(), 0..=0);
        Err(words.bad_size.fill(&[("field", caption), ("max", &max)]))
    };
    let (w, h) = (side(o.size.0, words.width.as_str())?, side(o.size.1, words.height.as_str())?);
    let at = Point::new(o.at.x.round(), o.at.y.round());
    if !(at.x.abs() <= MAX_POSITION && at.y.abs() <= MAX_POSITION) {
        return Err(words.bad_position.as_str().to_owned());
    }
    let (x0, y0) = if o.from_center { (at.x - w / 2.0, at.y - h / 2.0) } else { (at.x, at.y) };
    let rect = Rect::new(x0, y0, x0 + w, y0 + h);
    let sides = o.sides.clamp(*SIDES.start(), *SIDES.end());
    let geometry = match o.kind {
        ShapeKind::Rectangle | ShapeKind::RoundedRectangle => {
            let most = w.min(h) / 2.0;
            let radii = o.radii.map(|r| if r.is_finite() { r.clamp(0.0, most) } else { 0.0 });
            ShapeGeometry::Rect { rect, radii }
        }
        ShapeKind::Ellipse => ShapeGeometry::Ellipse { rect },
        ShapeKind::Triangle => ShapeGeometry::Polygon { rect, sides: 3, star_ratio: 1.0 },
        ShapeKind::Polygon | ShapeKind::Star => {
            let ratio = if o.star_ratio.is_finite() { o.star_ratio.clamp(0.01, 1.0) } else { 1.0 };
            ShapeGeometry::Polygon { rect, sides, star_ratio: ratio }
        }
        ShapeKind::Line | ShapeKind::Custom => return Err(String::new()),
    };
    Ok(CreatedShape { kind: o.kind, geometry, from_center: o.from_center })
}

/// The save key of the kind: `craft_draw.shape.create.<kind>`.
fn save_key(kind: ShapeKind) -> SaveKey {
    let kind = match kind {
        ShapeKind::Rectangle => "rectangle",
        ShapeKind::RoundedRectangle => "roundedRectangle",
        ShapeKind::Ellipse => "ellipse",
        ShapeKind::Triangle => "triangle",
        ShapeKind::Polygon => "polygon",
        ShapeKind::Star => "star",
        ShapeKind::Line => "line",
        ShapeKind::Custom => "custom",
    };
    SaveKey::named(format!("craft_draw.shape.create.{kind}"))
}

/// The values remembered over `o`: the sizes, From Center, and the ratio for the kinds that have one.
fn recalled(o: &mut CreateShapeOptions, memory: &Memory) {
    // pc: crates/ui-egui/src/dialogs/shape_dialog.rs:58-96 `open` (the remembered values over the
    // defaults).
    let size = |value: Option<f64>| value.filter(|v| v.is_finite() && CreateShapeOptions::SIZE.contains(v));
    if let Some(width) = size(memory.width) {
        o.size.0 = width;
    }
    if let Some(height) = size(memory.height) {
        o.size.1 = height;
    }
    if let Some(from_center) = memory.from_center {
        o.from_center = from_center;
    }
    // PhotoCraft applies a remembered value only for a kind that has the field.
    if matches!(o.kind, ShapeKind::Polygon | ShapeKind::Star)
        && let Some(percent) = memory.star_ratio.filter(|v| v.is_finite() && (1.0..=100.0).contains(v))
    {
        o.star_ratio = percent / 100.0;
    }
}

/// What the answer remembers.
fn remembered(o: &CreateShapeOptions, shape: &CreatedShape) -> Memory {
    // pc: crates/ui-egui/src/dialogs/shape_dialog.rs:140-155 `confirm` (remembers on OK).
    let bounds = shape.geometry.bounds();
    Memory {
        width: Some(bounds.width()),
        height: Some(bounds.height()),
        from_center: Some(shape.from_center),
        star_ratio: Some((o.star_ratio * 100.0).clamp(1.0, 100.0)),
    }
}

impl CreateShapeDialog {
    /// Create Shape with `options`.
    pub fn new(options: CreateShapeOptions) -> Self {
        let ruler = options.units.map_or(UnitKind::Pixels, |units| units.ruler);
        CreateShapeDialog { id: dialog_id("shape.create"), options, units: [ruler; 2] }
    }

    /// Whether `kind` has a Create dialog: every kind but Line and Custom (PhotoCraft has none for
    /// them; VectorCraft's Line Segment dialog is its own).
    pub fn supports(kind: ShapeKind) -> bool {
        !matches!(kind, ShapeKind::Line | ShapeKind::Custom)
    }

    /// The values as the fields show them now.
    pub fn options(&self) -> &CreateShapeOptions {
        &self.options
    }
}

/// One size field (`axis` 0 is Width, 1 Height) in its unit; sets `size` when the number changed.
fn size_field(
    ui: &mut Ui,
    axis: usize,
    caption: &Label,
    unit: &mut UnitKind,
    size: &mut (f64, f64),
    document: &GuideUnits,
    entries: &[Entry<UnitKind>],
) {
    let extent = if axis == 0 { document.width_px } else { document.height_px };
    let pixels = if axis == 0 { size.0 } else { size.1 };
    let shown = document.from_pixels(pixels, *unit, extent).unwrap_or(0.0);
    let (low, high) = (
        document.from_pixels(*CreateShapeOptions::SIZE.start(), *unit, extent),
        document.from_pixels(*CreateShapeOptions::SIZE.end(), *unit, extent),
    );
    let mut field = UnitSelect::new(caption.clone(), Bind::given(Some(shown)), Bind::given(*unit), entries).id_salt(("size", axis));
    if let (Some(low), Some(high)) = (low, high) {
        field = field.range(low.min(high)..=low.max(high));
    }
    let out = field.show(ui);
    if let Some(kind) = out.unit.value().copied() {
        *unit = kind;
    }
    if let Some(value) = out.edit.value().copied().flatten()
        && shown_places(value, *unit) != shown_places(shown, *unit)
        && let Some(px) = document.to_pixels(length(value, *unit), extent)
    {
        let px = thousandths(px);
        if axis == 0 {
            size.0 = px;
        } else {
            size.1 = px;
        }
    }
}

/// The units a shape's size is entered in: pixels, a percentage of the document and the physical lengths.
fn length_units() -> impl Iterator<Item = UnitKind> {
    [UnitKind::Pixels, UnitKind::Percent].into_iter().chain(UnitKind::LENGTHS)
}

/// `value` in `unit` as a length the document converts.
fn length(value: f64, unit: UnitKind) -> craft_ui::patterns::form_dialogs::GuideLength {
    craft_ui::patterns::form_dialogs::GuideLength::new(value, unit)
}

/// The fields; whether the values can be accepted.
fn body(
    ui: &mut Ui,
    values: &mut CreateShapeOptions,
    units: &mut [UnitKind; 2],
    words: &CreateShapeWords,
    numbers: &Units,
    latest: &RefCell<Option<Memory>>,
) -> bool {
    let document = values.units.unwrap_or_else(pixels_only);
    let entries: Vec<Entry<UnitKind>> = length_units()
        .filter(|kind| values.units.is_some() || *kind != UnitKind::Percent)
        .map(|kind| Entry::new(kind, Label::from(numbers.symbol(kind))))
        .collect();
    let kind = values.kind;
    let name = match kind {
        ShapeKind::RoundedRectangle => &words.rounded_rectangle,
        ShapeKind::Ellipse => &words.ellipse,
        ShapeKind::Triangle => &words.triangle,
        ShapeKind::Polygon => &words.polygon,
        ShapeKind::Star => &words.star,
        ShapeKind::Rectangle | ShapeKind::Line | ShapeKind::Custom => &words.rectangle,
    };
    FieldGroup::new(name.clone()).show(ui, |ui, _| {
        let (width, height) = (Label::from(words.width.as_str().to_owned()), Label::from(words.height.as_str().to_owned()));
        Field::new(width.clone()).show(ui, |ui, _| size_field(ui, 0, &width, &mut units[0], &mut values.size, &document, &entries));
        Field::new(height.clone()).show(ui, |ui, _| size_field(ui, 1, &height, &mut units[1], &mut values.size, &document, &entries));
        if matches!(kind, ShapeKind::Rectangle | ShapeKind::RoundedRectangle) {
            Field::new(Label::from(words.radius.as_str().to_owned())).show(ui, |ui, _| {
                // The radii are top-left, top-right, bottom-right, bottom-left; the two columns read
                // top-left, bottom-left | top-right, bottom-right.
                let [tl, tr, br, bl] = values.radii;
                let parts = [tl, bl, tr, br].map(|r| Mixed::Same(Some(r)));
                let captions =
                    [&words.top_left, &words.bottom_left, &words.top_right, &words.bottom_right].map(|c| Label::from(c.as_str().to_owned()));
                let out = LinkedFields::<4>::new(words.radius.as_str().to_owned(), Bind::given(parts), Bind::bound(&mut values.link_radii))
                    .mode(LinkMode::Equal)
                    .layout(LinkedLayout::Columns)
                    .captions(captions)
                    .range(0.0..=MAX_RADIUS)
                    .show(ui);
                if let Some([Mixed::Same(Some(tl)), Mixed::Same(Some(bl)), Mixed::Same(Some(tr)), Mixed::Same(Some(br))]) = out.edit.value().copied()
                {
                    values.radii = [tl, tr, br, bl];
                }
            });
        }
        if matches!(kind, ShapeKind::Polygon | ShapeKind::Star) {
            Field::new(Label::from(words.sides.as_str().to_owned())).show(ui, |ui, _| {
                let field = NumberField::new(words.sides.as_str().to_owned(), Bind::given(Some(f64::from(values.sides))))
                    .range(f64::from(*SIDES.start())..=f64::from(*SIDES.end()))
                    .step(1.0)
                    .commit_decimals(0);
                if let Some(value) = field.show(ui).value().copied().flatten() {
                    values.sides = value.round().clamp(f64::from(*SIDES.start()), f64::from(*SIDES.end())) as u32;
                }
            });
            Field::new(Label::from(words.star_ratio.as_str().to_owned())).show(ui, |ui, _| {
                let percent = (values.star_ratio * 100.0).clamp(1.0, 100.0);
                let field = NumberField::new(words.star_ratio.as_str().to_owned(), Bind::given(Some(percent)))
                    .range(1.0..=100.0)
                    .unit(numbers.symbol(UnitKind::Percent))
                    .commit_decimals(1);
                if let Some(value) = field.show(ui).value().copied().flatten()
                    && (value - percent).abs() > f64::EPSILON
                {
                    values.star_ratio = (value / 100.0).clamp(0.01, 1.0);
                }
            });
        }
        let center = Checkbox::new(Bind::given(values.from_center)).label(words.from_center.as_str().to_owned()).show(ui);
        if let Some(on) = center.value().copied() {
            values.from_center = on;
        }
    });
    match answer(values, words, numbers) {
        Ok(shape) => {
            *latest.borrow_mut() = Some(remembered(values, &shape));
            true
        }
        Err(message) => {
            if !message.is_empty() {
                Notice::new(Tone::Bad, message).layout(NoticeLayout::Inline).show(ui);
            }
            false
        }
    }
}

impl StackDialog for CreateShapeDialog {
    type Result = CreatedShape;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<CreatedShape>> {
        let env = Env::of(ui);
        let words = env.words::<CreateShapeWords>();
        let numbers = env.units();
        let title = Label::from(
            match self.options.kind {
                ShapeKind::RoundedRectangle => &words.rounded_rectangle,
                ShapeKind::Ellipse => &words.ellipse,
                ShapeKind::Triangle => &words.triangle,
                ShapeKind::Polygon => &words.polygon,
                ShapeKind::Star => &words.star,
                ShapeKind::Rectangle | ShapeKind::Line | ShapeKind::Custom => &words.rectangle,
            }
            .as_str()
            .to_owned(),
        );
        let key = save_key(self.options.kind);
        let latest = Rc::new(RefCell::new(None::<Memory>));
        let (write, write_key) = (Rc::clone(&latest), key.clone());
        let CreateShapeDialog { options, units, .. } = self;
        let shown = FormDialog::new(title, Bind::bound(options))
            .alt_resets(false)
            .shaped(move |dialog| {
                dialog.on_pick(move |button, cx| {
                    if button.role() == DialogRole::Accept
                        && let Some(memory) = write.borrow().clone()
                    {
                        *cx.saved::<Memory>(&write_key) = memory;
                    }
                })
            })
            .show(ui, |ui, form, values| {
                if form.first() && !values.recalled {
                    values.recalled = true;
                    let memory = form.saved::<Memory>(&key).clone();
                    recalled(values, &memory);
                }
                body(ui, values, units, &words, &numbers, &latest)
            });
        match (shown.accepted, shown.closed) {
            (Some(values), _) => answer(&values, &words, &numbers).ok().map(DialogResult::Ok),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}
