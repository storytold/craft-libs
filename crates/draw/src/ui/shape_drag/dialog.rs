//! [`CreateShapeDialog`]: Create Rectangle, Ellipse, Triangle, Polygon (and Illustrator's Rounded
//! Rectangle and Star), opened by a Shape tool click.

use std::cell::RefCell;
use std::ops::RangeInclusive;
use std::rc::Rc;

use craft_behavior::Mixed;
use craft_theme::variant::Tone;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, Props as _, Remember as _, Show as _, ShowContainer as _};
use craft_ui::dialog::{Alert, DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::patterns::form_dialogs::{FormDialog, GuideUnits};
use craft_ui::pickers::UnitSelect;
use craft_ui::runtime::env::{Env, Label, UnitKind, Units};
use craft_ui::runtime::memory::{SaveKey, saved_value, set_saved_value};
use craft_ui::sections::DialogRole;
use craft_ui::text::{LinkMode, LinkedFields, LinkedLayout, NumberField, RangeAlert};
use egui::Ui;
use kurbo::{Point, Rect};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ShapeKind;
use super::words::CreateShapeWords;
use crate::geometry::shape::{SIDES, ShapeGeometry};
use crate::ui::shape::RADIUS_RANGE;

/// The farthest the click may lie from the origin on either axis, in pixels: the same bound as
/// a side (PhotoCraft's `MAX_PX`, `shape_dialog.rs:19`, checked at `:111`).
const MAX_POSITION: f64 = 300_000.0;

/// The largest radius a field takes, in pixels (PhotoCraft's `value_field` range,
/// `shape_dialog.rs:223`); the shape clamps each radius to half its shorter side.
const MAX_RADIUS: f64 = 10_000.0;

/// What Create Shape opens with: the kind, the clicked point and the document's units, and the
/// values the fields start from. The dialog replaces width, height, From Center and the star ratio
/// with the remembered ones for the kind before its first pass, OK or field set, unless
/// [`recalled`](Self::recalled) is set.
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
    /// radius). A triangle's one corner radius is all four.
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

    /// The tool's corner radius (all four corners; a triangle's three), sides and star ratio.
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
///   default). Triangle: one "Radius" for its three corners (0 to 150,000 px, from the Triangle tool's own radius;
///   storytold/photocraft#13355). Polygon and Star: Sides (3–100) and Star Ratio (1–100 %, default 100); the Polygon
///   drag keeps ratio 1. Then "From Center".
/// - A width or height typed past [`CreateShapeOptions::SIZE`] keeps the nearest end and raises the range alert,
///   which the app opens from [`take_range_alert`](Self::take_range_alert) (#13358). A size set out of range
///   through automation shows [`CreateShapeWords::bad_size`] under the fields and disables OK; so does a click
///   farther than 300,000 px from the origin ([`CreateShapeWords::bad_position`]).
/// - OK answers [`CreatedShape`] at the click (centred on it with From Center, unrounded) and
///   remembers width, height, From Center and the star ratio per kind; Cancel answers nothing and
///   remembers nothing.
/// - Keyboard: Enter is OK, Esc Cancel; every field is a Tab stop in reading order.
pub struct CreateShapeDialog {
    id: DialogId,
    options: CreateShapeOptions,
    /// Each size field's unit for this opening.
    units: [UnitKind; 2],
    /// Whether the dialog is open: bound to the form's open state, so the host's dialog closes with it.
    open: bool,
    /// The title of the range alert; the dialog's own title when `None`.
    alert_title: Option<Label>,
    /// The last range alert a size field raised, with the env it showed in (which words it), until the app takes it.
    range_alert: Option<(RangeAlert, Env)>,
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
        ShapeKind::Triangle => {
            let radius = o.radii[0];
            let corner_radius = if radius.is_finite() { radius.clamp(*RADIUS_RANGE.start(), *RADIUS_RANGE.end()) } else { 0.0 };
            ShapeGeometry::Triangle { rect, corner_radius }
        }
        ShapeKind::Polygon | ShapeKind::Star => {
            let ratio = if o.star_ratio.is_finite() { o.star_ratio.clamp(0.01, 1.0) } else { 1.0 };
            ShapeGeometry::Polygon { rect, sides, star_ratio: ratio }
        }
        ShapeKind::Line | ShapeKind::Custom => return Err(String::new()),
    };
    Ok(CreatedShape { kind: o.kind, geometry, from_center: o.from_center })
}

/// The dialog's title for `kind`.
fn title_of(kind: ShapeKind, words: &CreateShapeWords) -> Label {
    let title = match kind {
        ShapeKind::RoundedRectangle => &words.rounded_rectangle,
        ShapeKind::Ellipse => &words.ellipse,
        ShapeKind::Triangle => &words.triangle,
        ShapeKind::Polygon => &words.polygon,
        ShapeKind::Star => &words.star,
        ShapeKind::Rectangle | ShapeKind::Line | ShapeKind::Custom => &words.rectangle,
    };
    Label::from(title.as_str().to_owned())
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
        CreateShapeDialog { id: dialog_id("shape.create"), options, units: [ruler; 2], open: true, alert_title: None, range_alert: None }
    }

    /// The title of the range alert a size field raises (the app's name); the dialog's title by default.
    #[must_use]
    pub fn alert_title(mut self, title: impl Into<Label>) -> Self {
        self.alert_title = Some(title.into());
        self
    }

    /// The range alert of a width or height typed past [`CreateShapeOptions::SIZE`], worded once; `None` until a
    /// size field raises one, and after it was taken.
    pub fn take_range_alert(&mut self) -> Option<Alert<'static>> {
        let (alert, env) = self.range_alert.take()?;
        let title = self.alert_title.clone().unwrap_or_else(|| title_of(self.options.kind, &env.words::<CreateShapeWords>()));
        Some(Alert::range(title, &alert, &env))
    }

    /// Puts the kind's remembered values over the opening ones, once, before the first show, OK or set reads them:
    /// automation may set or confirm before any pass.
    fn recall(&mut self, ctx: &egui::Context) {
        if std::mem::replace(&mut self.options.recalled, true) {
            return;
        }
        let memory = saved_value::<Memory>(ctx, &save_key(self.options.kind));
        recalled(&mut self.options, &memory);
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

/// One size field (`axis` 0 is Width, 1 Height) in its unit; sets `size` when the number changed. The range alert
/// of a typed number past [`CreateShapeOptions::SIZE`].
fn size_field(
    ui: &mut Ui,
    axis: usize,
    caption: &Label,
    unit: &mut UnitKind,
    size: &mut (f64, f64),
    document: &GuideUnits,
    entries: &[Entry<UnitKind>],
) -> Option<RangeAlert> {
    let extent = extent_of(document, axis);
    let shown = shown_size(document, axis, *unit, *size);
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
    {
        set_size(document, axis, *unit, size, value);
    }
    out.out_of_range
}

/// The pixels a percentage of size field `axis` is of: the document's width or height.
fn extent_of(document: &GuideUnits, axis: usize) -> f64 {
    if axis == 0 { document.width_px } else { document.height_px }
}

/// Size field `axis`'s value in `unit`, as the field shows it.
fn shown_size(document: &GuideUnits, axis: usize, unit: UnitKind, size: (f64, f64)) -> f64 {
    let pixels = if axis == 0 { size.0 } else { size.1 };
    document.from_pixels(pixels, unit, extent_of(document, axis)).unwrap_or(0.0)
}

/// Sets size field `axis` from `value` in `unit`, kept to a thousandth of a pixel, as a commit in the field does;
/// `false` and nothing changes when `value` does not convert.
fn set_size(document: &GuideUnits, axis: usize, unit: UnitKind, size: &mut (f64, f64), value: f64) -> bool {
    let Some(px) = document.to_pixels(length(value, unit), extent_of(document, axis)) else { return false };
    let px = thousandths(px);
    if axis == 0 {
        size.0 = px;
    } else {
        size.1 = px;
    }
    true
}

/// The units a shape's size is entered in: pixels, a percentage of the document and the physical lengths. Percent
/// needs the document (`values.units`).
fn length_units(values: &CreateShapeOptions) -> impl Iterator<Item = UnitKind> + use<> {
    let percent = values.units.is_some();
    [UnitKind::Pixels, UnitKind::Percent].into_iter().chain(UnitKind::LENGTHS).filter(move |kind| percent || *kind != UnitKind::Percent)
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
    typed: &mut Option<RangeAlert>,
) -> bool {
    let document = values.units.unwrap_or_else(pixels_only);
    let entries: Vec<Entry<UnitKind>> = length_units(values).map(|kind| Entry::new(kind, numbers.name(kind))).collect();
    let kind = values.kind;
    FieldGroup::new(title_of(kind, words)).show(ui, |ui, _| {
        let (width, height) = (Label::from(words.width.as_str().to_owned()), Label::from(words.height.as_str().to_owned()));
        for (axis, caption) in [(0, &width), (1, &height)] {
            Field::new(caption.clone()).show(ui, |ui, _| {
                if let Some(alert) = size_field(ui, axis, caption, &mut units[axis], &mut values.size, &document, &entries) {
                    *typed = Some(alert);
                }
            });
        }
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
        if kind == ShapeKind::Triangle {
            Field::new(Label::from(words.radius.as_str().to_owned())).show(ui, |ui, _| {
                let radius = values.radii[0];
                let field = NumberField::new(words.radius.as_str().to_owned(), Bind::given(Some(radius)))
                    .range(RADIUS_RANGE)
                    .unit(numbers.symbol(UnitKind::Pixels))
                    .commit_decimals(2);
                if let Some(value) = field.show(ui).value().copied().flatten()
                    && value != radius
                {
                    values.radii = [value.clamp(*RADIUS_RANGE.start(), *RADIUS_RANGE.end()); 4];
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

    /// `width` and `height` in the unit their field shows, `widthUnit` and `heightUnit`, `radii` (top-left,
    /// top-right, bottom-right, bottom-left, in pixels), `linkRadii`, a triangle's `cornerRadius` (pixels), `sides`, `starRatio` (in percent, as its
    /// field shows it) and `fromCenter`.
    fn json_fields(&self) -> Option<Value> {
        let o = &self.options;
        let document = o.units.unwrap_or_else(pixels_only);
        Some(json!({
            "width": shown_size(&document, 0, self.units[0], o.size),
            "height": shown_size(&document, 1, self.units[1], o.size),
            "widthUnit": self.units[0],
            "heightUnit": self.units[1],
            "radii": o.radii,
            "linkRadii": o.link_radii,
            "cornerRadius": o.radii[0],
            "sides": o.sides,
            "starRatio": (o.star_ratio * 100.0).clamp(1.0, 100.0),
            "fromCenter": o.from_center,
        }))
    }

    /// Sets a field as its row does, from the remembered values on. The radius rows answer only for a rectangle,
    /// `cornerRadius` only for a triangle, and Sides and Star Ratio only for a polygon or star; elsewhere they refuse
    /// as inactive. Linked radii take four equal values.
    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        self.recall(ctx);
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        let kind = self.options.kind;
        let rect = matches!(kind, ShapeKind::Rectangle | ShapeKind::RoundedRectangle);
        let polygon = matches!(kind, ShapeKind::Polygon | ShapeKind::Star);
        let shown = match field {
            "radii" | "linkRadii" => rect,
            "cornerRadius" => kind == ShapeKind::Triangle,
            "sides" | "starRatio" => polygon,
            _ => true,
        };
        if !shown {
            return Err(FieldRefusal::Inactive.message(field));
        }
        let document = self.options.units.unwrap_or_else(pixels_only);
        let offered: Vec<UnitKind> = length_units(&self.options).collect();
        let o = &mut self.options;
        match field {
            "width" | "height" => {
                let axis = usize::from(field == "height");
                let number = value.as_f64().ok_or_else(rejects)?;
                if !set_size(&document, axis, self.units[axis], &mut o.size, number) {
                    return Err(rejects());
                }
            }
            "widthUnit" | "heightUnit" => {
                let unit = serde_json::from_value::<UnitKind>(value.clone()).ok().filter(|unit| offered.contains(unit)).ok_or_else(rejects)?;
                self.units[usize::from(field == "heightUnit")] = unit;
            }
            "radii" => {
                let radii = serde_json::from_value::<[f64; 4]>(value.clone()).map_err(|_| rejects())?;
                if radii.iter().any(|radius| !radius.is_finite()) || (o.link_radii && radii.iter().any(|radius| *radius != radii[0])) {
                    return Err(rejects());
                }
                o.radii = radii.map(|radius| radius.clamp(0.0, MAX_RADIUS));
            }
            "linkRadii" => o.link_radii = value.as_bool().ok_or_else(rejects)?,
            "cornerRadius" => {
                let radius = value.as_f64().filter(|radius| radius.is_finite()).ok_or_else(rejects)?;
                o.radii = [radius.clamp(*RADIUS_RANGE.start(), *RADIUS_RANGE.end()); 4];
            }
            "sides" => {
                let sides = value.as_f64().ok_or_else(rejects)?;
                o.sides = sides.round().clamp(f64::from(*SIDES.start()), f64::from(*SIDES.end())) as u32;
            }
            "starRatio" => o.star_ratio = (value.as_f64().ok_or_else(rejects)?.clamp(1.0, 100.0) / 100.0).clamp(0.01, 1.0),
            "fromCenter" => o.from_center = value.as_bool().ok_or_else(rejects)?,
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }

    /// Answers the shape OK makes and remembers the kind's values as OK does; `None` while OK is refused.
    fn confirm(&mut self, ctx: &egui::Context) -> Option<Self::Result> {
        self.recall(ctx);
        let env = Env::of_root(ctx);
        let shape = answer(&self.options, &env.words::<CreateShapeWords>(), &env.units()).ok()?;
        let key = save_key(self.options.kind);
        set_saved_value::<Memory>(ctx, &key, remembered(&self.options, &shape));
        Some(shape)
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<CreatedShape>> {
        let env = Env::of(ui);
        let words = env.words::<CreateShapeWords>();
        let numbers = env.units();
        let title = title_of(self.options.kind, &words);
        self.recall(ui.ctx());
        let write_key = save_key(self.options.kind);
        let latest = Rc::new(RefCell::new(None::<Memory>));
        let write = Rc::clone(&latest);
        let mut typed = None::<RangeAlert>;
        let CreateShapeDialog { options, units, open, range_alert, .. } = self;
        let shown = FormDialog::new(title, Bind::bound(options))
            .opened(Bind::bound(open))
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
            .show(ui, |ui, _, values| body(ui, values, units, &words, &numbers, &latest, &mut typed));
        if let Some(alert) = typed {
            *range_alert = Some((alert, env.clone()));
        }
        match (shown.accepted, shown.closed) {
            (Some(values), _) => answer(&values, &words, &numbers).ok().map(DialogResult::Ok),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}
