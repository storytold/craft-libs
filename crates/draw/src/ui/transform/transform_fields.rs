//! `TransformFields`: the X, Y, W, H, angle and skew fields of a transform box, as an options-bar strip or a panel's
//! rows.

#[cfg(feature = "screen-testing")]
mod stories;
mod units;

use self::units::Conversion;
pub use self::units::SizeUnit;
pub(crate) use self::units::size_unit_of;
use super::session::Interpolation;
use super::session_bar::TransformBarWords;
use craft_behavior::geometry::quad::BoxState;
use craft_behavior::kurbo::Rect;
use craft_behavior::{EditEvent, EditEvents, GestureId, Mixed, RulerUnits};
use craft_ui::canvas::RulerText;
use craft_ui::contract::{
    Bind, BodyCx, Entry, GlyphLabel, LabelKind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing,
};
use craft_ui::display::Divider;
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::menu::Menu;
use craft_ui::pickers::{Anchor9, ReferencePoint, Select};
use craft_ui::runtime::env::{Env, Label, UnitKind, Units};
use craft_ui::runtime::memory::Remember as _;
use craft_ui::text::{NumberField, link_toggle};
use egui::{Response, Ui};
use std::sync::Arc;

craft_ui::words! {
    /// The words of the transform fields: each field's glyph and accessible name.
    pub struct TransformFieldWords {
        x_glyph: "transform.xGlyph" = "X";
        x_name: "transform.xName" = "X position";
        y_glyph: "transform.yGlyph" = "Y";
        y_name: "transform.yName" = "Y position";
        w_glyph: "transform.wGlyph" = "W";
        w_name: "transform.wName" = "Width";
        h_glyph: "transform.hGlyph" = "H";
        h_name: "transform.hName" = "Height";
        link: "transform.link" = "Maintain aspect ratio";
        angle_glyph: "transform.angleGlyph" = "°";
        angle_name: "transform.angleName" = "Angle";
        skew_h_glyph: "transform.skewHGlyph" = "H";
        skew_h_name: "transform.skewHName" = "Horizontal skew";
        skew_v_glyph: "transform.skewVGlyph" = "V";
        skew_v_name: "transform.skewVName" = "Vertical skew";
        reference: "transform.referencePointName" = "Reference point";
    }
}

/// The groups of cells the options-bar form shows; [`BarGroups::ALL`] is all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BarGroups {
    /// The reference point picker.
    pub reference: bool,
    /// X and Y.
    pub position: bool,
    /// W, the aspect link and H.
    pub size: bool,
    /// The angle.
    pub angle: bool,
    /// H and V skew.
    pub skew: bool,
    /// The interpolation select, when [`TransformFields::interpolation`] gives one.
    pub interpolation: bool,
}

impl BarGroups {
    /// Every group shown.
    pub const ALL: Self = Self { reference: true, position: true, size: true, angle: true, skew: true, interpolation: true };
}

impl Default for BarGroups {
    fn default() -> Self {
        Self::ALL
    }
}

/// One of the fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransformField {
    /// The reference point's x.
    X,
    /// The reference point's y.
    Y,
    /// The width.
    W,
    /// The height.
    H,
    /// The angle.
    Angle,
    /// The horizontal skew.
    SkewH,
    /// The vertical skew.
    SkewV,
}

impl TransformField {
    fn key(self) -> &'static str {
        match self {
            TransformField::X => "x",
            TransformField::Y => "y",
            TransformField::W => "w",
            TransformField::H => "h",
            TransformField::Angle => "angle",
            TransformField::SkewH => "skew-h",
            TransformField::SkewV => "skew-v",
        }
    }
}

/// What the row edits and reports: the box and the V skew it reads with.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TransformRow {
    /// The box.
    pub state: BoxState,
    /// The vertical skew, in degrees.
    pub skew_v: f64,
}

impl TransformRow {
    /// The box `state` read with the V skew `skew_v`.
    pub fn new(state: BoxState, skew_v: f64) -> Self {
        Self { state, skew_v }
    }
}

/// What each field's running gesture began from, so every value it reports is made from that row.
#[derive(Clone, Debug, Default)]
struct Bases([Option<(GestureId, TransformRow)>; 7]);

impl Bases {
    /// The row the gesture `id` of `field` began from; `fallback` when it has no record.
    fn origin(&self, field: TransformField, id: GestureId, fallback: &TransformRow) -> TransformRow {
        self.0[field as usize].filter(|(of, _)| *of == id).map_or(*fallback, |(_, row)| row)
    }
}

/// What the fields report.
#[derive(Debug)]
#[non_exhaustive]
pub struct TransformFieldsOutput {
    /// The strip's or the rows' response.
    pub response: Response,
    /// The edits of the row, in order: every field's value after the box maths, one gesture per field gesture. A
    /// binding that writes through has them already; a given one applies them.
    pub events: EditEvents<TransformRow>,
    /// The unit picked from the W and H menu this pass; the host keeps it with its settings.
    pub unit_picked: Option<UnitKind>,
    /// The aspect link was toggled to this state.
    pub linked: Option<bool>,
    /// The interpolation picked this pass.
    pub interpolation: Option<Interpolation>,
}

impl Outcome for TransformFieldsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The fields of a transform box: the reference point, X and Y, W and H with the aspect link, the angle, and the
/// horizontal and vertical skew.
///
/// - **Fields** are number fields whose captions scrub them. X and Y are the reference point's position in the
///   [rulers' units](Self::units) and move the box with it; W and H scale it about the reference point, as a
///   percentage of the box before the transform or as a length ([`size_unit`](Self::size_unit)); the angle and the two
///   skews turn and lean it about the reference point. A flipped box reads as a negative W or H and keeps its sign.
///   Limits: position ±300 000 document units, angle −180° to 180°, skew ±89°, scale as far as the box stays
///   within the position limit.
/// - **Typed text** may carry another unit's spelling ("40px", "2in"); a percentage is read against the box's side
///   before the transform.
/// - **Linked** ([`new`](Self::new)'s binding): W or H also scales the other by the same factor, each kept within its
///   limit.
/// - **One gesture per field gesture.** A scrub or a typing run of a field is one gesture of the row; each value it
///   reports is made from the row it began from, so a perspective box does not drift. A cancelled gesture restores
///   the whole row, the V skew included.
/// - **W and H unit.** With [`size_units`](Self::size_units), a secondary click, the Menu key or Shift+F10 on W or H
///   opens a menu of the units; a pick is [`TransformFieldsOutput::unit_picked`].
/// - **Bar form** ([`bar`](Self::bar)): the compact reference point, then captioned fields in Photoshop's order with a
///   divider between the groups, the interpolation select last. [`groups`](Self::groups) narrows the strip.
#[must_use = "show the fields with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct TransformFields<'a> {
    label: Label,
    row: Bind<'a, TransformRow>,
    start: Rect,
    linked: Bind<'a, bool>,
    size_unit: SizeUnit,
    x: RulerText,
    y: RulerText,
    bar: bool,
    groups: BarGroups,
    interpolation: Option<Bind<'a, Interpolation>>,
    size_units: Option<&'a [Entry<UnitKind>]>,
    enabled: bool,
}

impl<'a> TransformFields<'a> {
    /// The fields named `label` of `row`, a box that was `start` before the transform; `linked` is the aspect link.
    pub fn new(label: impl Into<Label>, row: Bind<'a, TransformRow>, start: Rect, linked: Bind<'a, bool>) -> Self {
        let pixels = RulerText::new(RulerUnits::pixels(), UnitKind::Pixels);
        Self {
            label: label.into(),
            row,
            start,
            linked,
            size_unit: SizeUnit::default(),
            x: pixels,
            y: pixels,
            bar: false,
            groups: BarGroups::ALL,
            interpolation: None,
            size_units: None,
            enabled: true,
        }
    }

    /// The units of X and Y (pixels by default).
    pub fn units(mut self, x: RulerText, y: RulerText) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    /// The unit of W and H; percent by default.
    pub fn size_unit(mut self, unit: SizeUnit) -> Self {
        self.size_unit = unit;
        self
    }

    /// Shows the options-bar form.
    pub fn bar(mut self) -> Self {
        self.bar = true;
        self
    }

    /// The groups the bar form shows (all by default).
    pub fn groups(mut self, groups: BarGroups) -> Self {
        self.groups = groups;
        self
    }

    /// The bar form's interpolation select, last in the strip.
    pub fn interpolation(mut self, interpolation: Bind<'a, Interpolation>) -> Self {
        self.interpolation = Some(interpolation);
        self
    }

    /// The units of the W and H menu, the current one checked.
    pub fn size_units(mut self, units: &'a [Entry<UnitKind>]) -> Self {
        self.size_units = Some(units);
        self
    }

    /// Whether the fields take input; true by default.
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// Shows the fields.
    pub fn show(self, ui: &mut Ui) -> TransformFieldsOutput {
        let TransformFields { label, row, start, linked, size_unit, x, y, bar, groups, interpolation, size_units, enabled } = self;
        let env = Env::of(ui);
        let mut cells = Cells {
            words: env.words::<TransformFieldWords>(),
            bar_words: env.words::<TransformBarWords>(),
            units: env.units(),
            name: label.as_str().into(),
            row,
            linked,
            interpolation,
            size_units,
            start,
            size_unit,
            x,
            y,
            groups,
            enabled,
            out: Out::default(),
        };
        let response = if bar {
            Row::new().enabled(enabled).show(ui, |ui, cx| cells.bar(ui, cx)).output
        } else {
            FieldGroup::new(label).enabled(enabled).show(ui, |ui, cx| cells.rows(ui, cx)).output
        };
        let Out { events, unit_picked, linked, interpolation } = cells.out;
        TransformFieldsOutput { response, events, unit_picked, linked, interpolation }
    }
}

/// What the cells collected in a pass.
#[derive(Default)]
struct Out {
    events: EditEvents<TransformRow>,
    unit_picked: Option<UnitKind>,
    linked: Option<bool>,
    interpolation: Option<Interpolation>,
}

/// The pass's state while the cells are shown.
struct Cells<'a> {
    words: Arc<TransformFieldWords>,
    bar_words: Arc<TransformBarWords>,
    units: Units,
    name: Arc<str>,
    row: Bind<'a, TransformRow>,
    linked: Bind<'a, bool>,
    interpolation: Option<Bind<'a, Interpolation>>,
    size_units: Option<&'a [Entry<UnitKind>]>,
    start: Rect,
    size_unit: SizeUnit,
    x: RulerText,
    y: RulerText,
    groups: BarGroups,
    enabled: bool,
    out: Out,
}

/// The pass's shared facts, read before the cells.
struct Pass {
    before: TransformRow,
    conversion: Conversion,
    bases: Bases,
}

impl Cells<'_> {
    fn pass(&mut self, cx: &mut BodyCx<'_>) -> Pass {
        let start = TransformRow { state: BoxState::from_rect(self.start), skew_v: 0.0 };
        let before = match self.row.reborrow().read(cx) {
            Some(Mixed::Same(held)) => held,
            _ => start,
        };
        let linked = matches!(self.linked.reborrow().read(cx), Some(Mixed::Same(true)));
        let conversion = Conversion { rect: self.start, x: self.x, y: self.y, size: self.size_unit.clone(), linked };
        Pass { before, conversion, bases: cx.memory::<Bases>().clone() }
    }

    /// Writes `event` to the row and adds it to the pass's.
    fn report(&mut self, cx: &mut BodyCx<'_>, event: EditEvent<TransformRow>) {
        let _ = self.row.reborrow().write(cx, &event);
        self.out.events.push(event);
    }

    fn glyph_and_name(&self, field: TransformField) -> (Label, Label) {
        let words = &self.words;
        match field {
            TransformField::X => (words.x_glyph.clone(), words.x_name.clone()),
            TransformField::Y => (words.y_glyph.clone(), words.y_name.clone()),
            TransformField::W => (words.w_glyph.clone(), words.w_name.clone()),
            TransformField::H => (words.h_glyph.clone(), words.h_name.clone()),
            TransformField::Angle => (words.angle_glyph.clone(), words.angle_name.clone()),
            TransformField::SkewH => (words.skew_h_glyph.clone(), words.skew_h_name.clone()),
            TransformField::SkewV => (words.skew_v_glyph.clone(), words.skew_v_name.clone()),
        }
    }

    /// One captioned field. Returns the field's event lifted to the row.
    fn field(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, pass: &mut Pass, field: TransformField) {
        let (glyph, name) = self.glyph_and_name(field);
        let conversion = &pass.conversion;
        let (kind, decimals) = conversion.format(field);
        let bound = conversion.bound(field);
        // A running gesture of this field is shown the value it began from: the host's write-back of the gesture's own
        // values must not look like the app changing the field under it.
        let basis = pass.bases.0[field as usize].map_or(pass.before, |(_, row)| row);
        let current = conversion.shown(&basis.state, basis.skew_v, field);
        let codec = conversion.codec(&self.units, field);
        let unit = Label::from(self.units.symbol(kind));
        let percent = kind == UnitKind::Percent;
        let menu = self.size_units.filter(|_| matches!(field, TransformField::W | TransformField::H));
        let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text(glyph))).scrub(true);
        let mut events = EditEvents::new();
        let mut picked = None;
        Field::new(name.clone()).id_salt(field.key()).enabled(self.enabled).side(side).sizing(Sizing::Content).show(ui, |ui, _| {
            let mut number = NumberField::new(name.clone(), Bind::given(Some(current)))
                .range(-bound..=bound)
                .codec(codec.clone())
                .unit(unit.clone())
                .scrub_by_label();
            if percent {
                number = number.commit_decimals(decimals);
            }
            let shown = number.show(ui);
            events = shown.edit.events.clone();
            if let Some(entries) = menu {
                let entries: Vec<Entry<UnitKind>> = entries.iter().map(|entry| entry.clone().checked(entry.value() == Some(&kind))).collect();
                picked = Menu::context(name.clone(), &shown.edit.response).values().pick(ui, &entries).picked;
            }
        });
        self.out.unit_picked = self.out.unit_picked.or(picked);
        let places = i32::try_from(decimals).unwrap_or(0);
        for event in events {
            let lifted = lift(&mut pass.bases, &pass.before, conversion, field, event, current, 0.5 * 10f64.powi(-places));
            if let Some(lifted) = lifted {
                self.report(cx, lifted);
            }
        }
    }

    /// The reference point picker; a pick moves the reference point to that point of the box.
    fn reference(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, pass: &Pass, compact: bool) {
        let anchor = pass.before.state.anchor_of_pivot().unwrap_or(Anchor9::Centre);
        let mut picker = ReferencePoint::new(self.words.reference.clone(), Bind::given(anchor)).enabled(self.enabled);
        if compact {
            picker = picker.compact();
        }
        let shown = picker.show(ui);
        let Some(picked) = shown.value().copied() else { return };
        let before = pass.before;
        let state = BoxState::new(before.state.quad, before.state.point_at(picked));
        let event = EditEvent::set(Mixed::Same(before), TransformRow { state, skew_v: before.skew_v }, self.name.clone(), None);
        self.report(cx, event);
    }

    /// The aspect link toggle between W and H.
    fn link(&mut self, ui: &mut Ui) {
        let toggled = link_toggle(self.words.link.clone(), self.linked.reborrowed()).enabled(self.enabled).show(ui);
        if let Some(on) = toggled.value().copied() {
            self.out.linked = Some(on);
        }
    }

    /// W, the link and H in one row.
    fn size(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, pass: &mut Pass) {
        Row::new().show(ui, |ui, _| {
            self.field(ui, cx, pass, TransformField::W);
            self.link(ui);
            self.field(ui, cx, pass, TransformField::H);
        });
    }

    /// The interpolation select.
    fn interpolation(&mut self, ui: &mut Ui) {
        let Some(bind) = self.interpolation.as_mut() else { return };
        let words = &self.bar_words;
        let entries: Vec<Entry<Interpolation>> = Interpolation::ALL
            .into_iter()
            .map(|choice| {
                let label = match choice {
                    Interpolation::Bicubic => &words.bicubic,
                    Interpolation::Bilinear => &words.bilinear,
                    _ => &words.nearest_neighbor,
                };
                Entry::new(choice, label.clone())
            })
            .collect();
        let shown = Select::new(words.interpolation.clone(), bind.reborrowed(), &entries).enabled(self.enabled).show(ui);
        if let Some(choice) = shown.value().copied() {
            self.out.interpolation = Some(choice);
        }
    }

    /// The panel form: one field per row.
    fn rows(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let mut pass = self.pass(cx);
        Field::new(self.words.reference.clone()).id_salt("reference").enabled(self.enabled).show(ui, |ui, _| self.reference(ui, cx, &pass, false));
        self.field(ui, cx, &mut pass, TransformField::X);
        self.field(ui, cx, &mut pass, TransformField::Y);
        self.size(ui, cx, &mut pass);
        for field in [TransformField::Angle, TransformField::SkewH, TransformField::SkewV] {
            self.field(ui, cx, &mut pass, field);
        }
        *cx.memory::<Bases>() = pass.bases;
    }

    /// The options-bar form: the compact reference point, X and Y, W with the link and H, the angle and the skews, the
    /// interpolation, with a divider between the groups shown.
    fn bar(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let mut pass = self.pass(cx);
        let groups = self.groups;
        let any = std::cell::Cell::new(false);
        let divide = |ui: &mut Ui, shown: bool| {
            if shown && any.replace(true) {
                Divider::vertical().show(ui);
            }
            shown
        };
        if divide(ui, groups.reference) {
            self.reference(ui, cx, &pass, true);
        }
        if groups.position {
            self.field(ui, cx, &mut pass, TransformField::X);
            self.field(ui, cx, &mut pass, TransformField::Y);
            any.set(true);
        }
        if divide(ui, groups.size) {
            self.field(ui, cx, &mut pass, TransformField::W);
            self.link(ui);
            self.field(ui, cx, &mut pass, TransformField::H);
        }
        if divide(ui, groups.angle || groups.skew) {
            if groups.angle {
                self.field(ui, cx, &mut pass, TransformField::Angle);
            }
            if groups.skew {
                self.field(ui, cx, &mut pass, TransformField::SkewH);
                self.field(ui, cx, &mut pass, TransformField::SkewV);
            }
        }
        if self.interpolation.is_some() && divide(ui, groups.interpolation) {
            self.interpolation(ui);
        }
        *cx.memory::<Bases>() = pass.bases;
    }
}

/// A field's event as an event of the row. Every value of a gesture is made from the row the gesture began from; a
/// cancel restores that row. A one-step value that the field's rounding leaves where it was (within `tolerance` of
/// `current`) is no change.
fn lift(
    bases: &mut Bases,
    before: &TransformRow,
    conversion: &Conversion,
    field: TransformField,
    event: EditEvent<Option<f64>>,
    current: f64,
    tolerance: f64,
) -> Option<EditEvent<TransformRow>> {
    let slot = field as usize;
    match event {
        EditEvent::Begin { id, label, doc_key, .. } => {
            bases.0[slot] = Some((id, *before));
            Some(EditEvent::begin(id, Mixed::Same(*before), label, doc_key))
        }
        EditEvent::Update { id, value: Some(value), .. } => {
            let origin = bases.origin(field, id, before);
            Some(EditEvent::update(id, conversion.row(&origin, field, value)))
        }
        EditEvent::Commit { id, after: Some(value), .. } => {
            let origin = bases.origin(field, id, before);
            bases.0[slot] = None;
            Some(EditEvent::commit(id, Mixed::Same(origin), conversion.row(&origin, field, value)))
        }
        EditEvent::Cancel { id, reason, .. } => {
            let origin = bases.origin(field, id, before);
            bases.0[slot] = None;
            Some(EditEvent::cancel(id, reason, Some(Mixed::Same(origin))))
        }
        EditEvent::Set { after: Some(value), label, doc_key, .. } => {
            if (value - current).abs() < tolerance {
                return None;
            }
            Some(EditEvent::set(Mixed::Same(*before), conversion.row(before, field, value), label, doc_key))
        }
        _ => None,
    }
}
