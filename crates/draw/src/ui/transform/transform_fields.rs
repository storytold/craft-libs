//! `TransformFields`: the X, Y, W, H, angle and skew fields of a transform box, as an options-bar strip or a panel's
//! rows.

#[cfg(feature = "suite")]
pub(crate) mod stories;
mod units;

use self::units::Conversion;
pub use self::units::SizeUnit;
pub(crate) use self::units::size_unit_of;
use super::session::Interpolation;
use super::session_bar::TransformBarWords;
use craft_behavior::geometry::quad::BoxState;
use craft_behavior::kurbo::Rect;
use craft_behavior::{ColumnWidth, EditEvent, EditEvents, GestureId, IconId, Mixed};
use craft_ui::buttons::ToggleButton;
use craft_ui::canvas::CanvasRulers;
use craft_ui::contract::{
    Bind, BodyCx, Bracket, Entry, FieldLayout, GlyphLabel, LabelKind, LabelPress, NoValue, Outcome, Props as _, Show as _, ShowContainer as _,
    SideLabel, Sizing,
};
use craft_ui::display::Divider;
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::menu::Menu;
use craft_ui::pickers::{Anchor9, ReferencePoint, ReferencePointLook, Select};
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
        /// The relative-position toggle between X and Y.
        relative: "transform.relativePosition" = "Use relative positioning for reference point";
        y_glyph: "transform.yGlyph" = "Y";
        y_name: "transform.yName" = "Y position";
        w_glyph: "transform.wGlyph" = "W";
        w_name: "transform.wName" = "Width";
        h_glyph: "transform.hGlyph" = "H";
        h_name: "transform.hName" = "Height";
        link: "transform.link" = "Maintain aspect ratio";
        angle_name: "transform.angleName" = "Angle";
        skew_h_glyph: "transform.skewHGlyph" = "H";
        skew_h_name: "transform.skewHName" = "Horizontal skew";
        skew_v_glyph: "transform.skewVGlyph" = "V";
        skew_v_name: "transform.skewVName" = "Vertical skew";
        reference: "transform.referencePointName" = "Reference point";
    }
}

/// The groups of cells the fields show, in either form; [`TransformGroups::ALL`] is all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TransformGroups {
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

impl TransformGroups {
    /// Every group shown.
    pub const ALL: Self = Self { reference: true, position: true, size: true, angle: true, skew: true, interpolation: true };
    /// No group shown; a host starts from it and sets fields.
    pub const NONE: Self = Self { reference: false, position: false, size: false, angle: false, skew: false, interpolation: false };
}

impl Default for TransformGroups {
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

/// The part of the row an event came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransformSource {
    /// This field.
    Field(TransformField),
    /// The reference point.
    Reference,
}

/// How the panel form orders its fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldArrangement {
    /// W, the link and H on the first row, then X and Y.
    #[default]
    SizeFirst,
    /// X and W on the first row, Y and H on the second, the link beside them centred across the two rows: Tab runs X,
    /// W, Y, H.
    PositionFirst,
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

/// What each field's running gesture began from, so every value it reports is made from that row and is read against
/// the start box of that moment: the row and the start box.
#[derive(Clone, Debug, Default)]
struct Bases([Option<(GestureId, TransformRow, Rect)>; 7]);

impl Bases {
    /// The row the gesture `id` of `field` began from; `fallback` when it has no record.
    fn origin(&self, field: TransformField, id: GestureId, fallback: &TransformRow) -> TransformRow {
        self.0[field as usize].filter(|(of, ..)| *of == id).map_or(*fallback, |(_, row, _)| row)
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
    /// The relative-position toggle was set to this state.
    pub relative: Option<bool>,
    /// The part each entry of `events` came from, at the same index.
    pub sources: Vec<TransformSource>,
    /// The field an Enter committed this pass, with that Enter's modifiers; `None` for a commit by blur, Tab, scrub or
    /// click.
    pub submitted: Option<(TransformField, craft_keymap::Mods)>,
    /// A press on a link caption (see `TransformFields::caption_press`): its field.
    pub link: Option<TransformField>,
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
/// - **Fields** are number fields whose captions scrub them. X and Y are the reference point's distance from the
///   [rulers'](Self::rulers) zero in their units and move the box with it; W and H scale it about the reference point, as a
///   percentage of the box before the transform or as a length ([`size_unit`](Self::size_unit)); the angle and the two
///   skews turn and lean it about the reference point. A flipped box reads as a negative W or H and keeps its sign.
///   With [`relative`](Self::relative) on, X and Y are the reference point's offset instead of its position.
///   Limits: position ±300 000 document units, angle −180° to 180°, skew ±89°, scale as far as the box stays
///   within the position limit.
/// - **Typed text** may carry another unit's spelling ("40px", "2in"); a percentage is read against the box's side
///   before the transform, in X and Y against the [`document`](Self::document)'s side when it is given.
/// - **Linked** ([`new`](Self::new)'s binding): W or H also scales the other by the same factor, each kept within its
///   limit.
/// - **One gesture per field gesture.** A scrub or a typing run of a field is one gesture of the row; each value it
///   reports is made from the row it began from, so a perspective box does not drift. A cancelled gesture restores
///   the whole row, the V skew included.
/// - **W and H unit.** With [`size_units`](Self::size_units), a secondary click, the Menu key or Shift+F10 on W or H
///   opens a menu of the units; a pick is [`TransformFieldsOutput::unit_picked`].
/// - **Panel form** (the default): two field columns that fill the width, the link's column between them: W, the
///   link and H on one row, X and Y on the next, the angle, then the H and V skews; the compact reference point, when
///   shown, leads the first row. [`groups`](Self::groups) leaves out the rows of hidden groups. With
///   [`FieldArrangement::PositionFirst`] the grid is X and W, then Y and H, with the link beside it, centred across
///   the two rows.
/// - **Bar form** ([`bar`](Self::bar)): the compact reference point, then captioned fields in Photoshop's order with a
///   divider between the groups, the Δ toggle between X and Y, the interpolation select last.
///   [`groups`](Self::groups) narrows the strip.
#[must_use = "show the fields with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct TransformFields<'a> {
    label: Label,
    row: Bind<'a, TransformRow>,
    start: Rect,
    linked: Bind<'a, bool>,
    size_unit: SizeUnit,
    rulers: CanvasRulers,
    reference_look: ReferencePointLook,
    caption_press: LabelPress,
    arrangement: FieldArrangement,
    bar: bool,
    groups: TransformGroups,
    interpolation: Option<Bind<'a, Interpolation>>,
    size_units: Option<&'a [Entry<UnitKind>]>,
    enabled: bool,
    relative: Option<Bind<'a, bool>>,
    document: Option<craft_behavior::kurbo::Size>,
}

impl<'a> TransformFields<'a> {
    /// The fields named `label` of `row`, a box that was `start` before the transform; `linked` is the aspect link.
    pub fn new(label: impl Into<Label>, row: Bind<'a, TransformRow>, start: Rect, linked: Bind<'a, bool>) -> Self {
        Self {
            label: label.into(),
            row,
            start,
            linked,
            size_unit: SizeUnit::default(),
            rulers: CanvasRulers::default(),
            reference_look: ReferencePointLook::default(),
            caption_press: LabelPress::default(),
            arrangement: FieldArrangement::default(),
            bar: false,
            groups: TransformGroups::ALL,
            interpolation: None,
            size_units: None,
            enabled: true,
            relative: None,
            document: None,
        }
    }

    /// The rulers X and Y read from: absolute X and Y are measured from the rulers' zero in their units (pixels from the
    /// origin by default); relative X and Y ignore the zero.
    pub fn rulers(mut self, rulers: CanvasRulers) -> Self {
        self.rulers = rulers;
        self
    }

    /// The look of the reference point the fields show.
    pub fn reference_look(mut self, look: ReferencePointLook) -> Self {
        self.reference_look = look;
        self
    }

    /// The press rule of the X, Y, W, H and angle captions; a press on a link caption reports its field.
    pub fn caption_press(mut self, press: LabelPress) -> Self {
        self.caption_press = press;
        self
    }

    /// The order of the panel form's fields; the bar form ignores it.
    pub fn arrangement(mut self, arrangement: FieldArrangement) -> Self {
        self.arrangement = arrangement;
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

    /// The groups shown, in either form (all by default).
    pub fn groups(mut self, groups: TransformGroups) -> Self {
        self.groups = groups;
        self
    }

    /// The Δ toggle between X and Y: X and Y show and move the reference point's offset while it is on.
    pub fn relative(mut self, on: Bind<'a, bool>) -> Self {
        self.relative = Some(on);
        self
    }

    /// The document's size: X and Y read a typed percentage against its sides.
    pub fn document(mut self, size: craft_behavior::kurbo::Size) -> Self {
        self.document = Some(size);
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
        let TransformFields {
            label,
            row,
            start,
            linked,
            size_unit,
            rulers,
            reference_look,
            caption_press,
            arrangement,
            bar,
            groups,
            interpolation,
            size_units,
            enabled,
            relative,
            document,
        } = self;
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
            rulers,
            reference_look,
            caption_press,
            arrangement,
            groups,
            enabled,
            relative,
            document,
            out: Out::default(),
        };
        let response = if bar {
            Row::new().enabled(enabled).show(ui, |ui, cx| cells.bar(ui, cx)).output
        } else {
            FieldGroup::new(label).enabled(enabled).show(ui, |ui, cx| cells.rows(ui, cx)).output
        };
        let Out { events, sources, unit_picked, linked, interpolation, relative, submitted, link } = cells.out;
        TransformFieldsOutput { response, events, unit_picked, linked, interpolation, relative, sources, submitted, link }
    }
}

/// What the cells collected in a pass.
#[derive(Default)]
struct Out {
    events: EditEvents<TransformRow>,
    sources: Vec<TransformSource>,
    submitted: Option<(TransformField, craft_keymap::Mods)>,
    link: Option<TransformField>,
    unit_picked: Option<UnitKind>,
    linked: Option<bool>,
    interpolation: Option<Interpolation>,
    relative: Option<bool>,
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
    rulers: CanvasRulers,
    reference_look: ReferencePointLook,
    caption_press: LabelPress,
    arrangement: FieldArrangement,
    groups: TransformGroups,
    enabled: bool,
    relative: Option<Bind<'a, bool>>,
    document: Option<craft_behavior::kurbo::Size>,
    out: Out,
}

/// The panel form's columns: a field, the link, a field; the fields share the width.
const FIELD_COLUMNS: [ColumnWidth; 3] = [ColumnWidth::fill(1.0), ColumnWidth::Content, ColumnWidth::fill(1.0)];
/// The panel form's columns with the reference point before them.
const REFERENCE_COLUMNS: [ColumnWidth; 4] = [ColumnWidth::Content, ColumnWidth::fill(1.0), ColumnWidth::Content, ColumnWidth::fill(1.0)];
/// The position-first panel form's columns: two fields that share the width.
const PAIR_COLUMNS: [ColumnWidth; 2] = [ColumnWidth::fill(1.0), ColumnWidth::fill(1.0)];
/// The position-first panel form's columns with the reference point before them.
const REFERENCE_PAIR_COLUMNS: [ColumnWidth; 3] = [ColumnWidth::Content, ColumnWidth::fill(1.0), ColumnWidth::fill(1.0)];

/// Which form the cells show in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    /// The panel's grid of fields.
    Panel,
    /// The options bar's strip.
    Bar,
}

impl Form {
    /// A panel field fills its grid cell; a bar field takes its content.
    fn sizing(self) -> Sizing {
        match self {
            Form::Panel => Sizing::FillWidth,
            Form::Bar => Sizing::Content,
        }
    }
}

/// A grid cell with nothing in it.
fn empty(ui: &mut Ui) {
    Stack::new().show(ui, |_, _| {});
}

/// The pass's shared facts, read before the cells.
struct Pass {
    before: TransformRow,
    conversion: Conversion,
    bases: Bases,
    /// The row binding read no value: the fields show nothing, while gestures still start from the start box.
    blank: bool,
}

impl Cells<'_> {
    fn pass(&mut self, cx: &mut BodyCx<'_>) -> Pass {
        let start = TransformRow { state: BoxState::from_rect(self.start), skew_v: 0.0 };
        let read = self.row.reborrow().read(cx);
        let blank = read.is_none();
        let before = match read {
            Some(Mixed::Same(held)) => held,
            _ => start,
        };
        let linked = matches!(self.linked.reborrow().read(cx), Some(Mixed::Same(true)));
        let relative = self.relative.as_mut().is_some_and(|on| matches!(on.reborrow().read(cx), Some(Mixed::Same(true))));
        let conversion =
            Conversion { rect: self.start, rulers: self.rulers, size: self.size_unit.clone(), linked, document: self.document, relative };
        Pass { before, conversion, bases: cx.memory::<Bases>().clone(), blank }
    }

    /// Writes `event` to the row and adds it, with the part it came from, to the pass's.
    fn report(&mut self, cx: &mut BodyCx<'_>, event: EditEvent<TransformRow>, source: TransformSource) {
        let _ = self.row.reborrow().write(cx, &event);
        self.out.events.push(event);
        self.out.sources.push(source);
    }

    /// The field's scrubbing glyph label and its name in `form`: every glyph beside its field, so in the panel form the
    /// angle's rotate icon takes its grid column's label column as W, X and H do; in the bar form the rotate icon sits
    /// inside the field.
    fn side_and_name(&self, field: TransformField, form: Form) -> (SideLabel, Label) {
        let words = &self.words;
        let beside = |glyph: &Label| SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::caption(glyph.clone())));
        let (side, name) = match field {
            TransformField::X => (beside(&words.x_glyph), &words.x_name),
            TransformField::Y => (beside(&words.y_glyph), &words.y_name),
            TransformField::W => (beside(&words.w_glyph), &words.w_name),
            TransformField::H => (beside(&words.h_glyph), &words.h_name),
            TransformField::Angle => {
                let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::icon(IconId::new("rotate_cw"))));
                (if form == Form::Bar { side.layout(FieldLayout::Inside) } else { side }, &words.angle_name)
            }
            TransformField::SkewH => (beside(&words.skew_h_glyph), &words.skew_h_name),
            TransformField::SkewV => (beside(&words.skew_v_glyph), &words.skew_v_name),
        };
        let mut side = side.scrub(true);
        if !matches!(field, TransformField::SkewH | TransformField::SkewV) {
            side.press = self.caption_press;
        }
        (side, name.clone())
    }

    /// One captioned field of `form`. Returns the field's event lifted to the row.
    fn field(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, pass: &mut Pass, field: TransformField, form: Form) {
        let (side, name) = self.side_and_name(field, form);
        let sizing = form.sizing();
        // A running gesture of this field is shown the value it began from, read against the start box it began with:
        // the host's write-back of the gesture's own values, or a start box it moves with them, must not look like the
        // app changing the field under it.
        let running = pass.bases.0[field as usize];
        let began = running.map(|(.., rect)| Conversion { rect, ..pass.conversion.clone() });
        let conversion = began.as_ref().unwrap_or(&pass.conversion);
        let (kind, decimals) = conversion.format(field);
        let bound = conversion.bound(field);
        let basis = running.map_or(pass.before, |(_, row, _)| row);
        let current = conversion.shown(&basis.state, basis.skew_v, field);
        let codec = conversion.codec(&self.units, field);
        let unit = Label::from(self.units.symbol(kind));
        let percent = kind == UnitKind::Percent;
        let menu = self.size_units.filter(|_| matches!(field, TransformField::W | TransformField::H));
        let mut events = EditEvents::new();
        let mut picked = None;
        let mut submit = None;
        let mut link_pressed = false;
        let shown_value = (!pass.blank).then_some(current);
        Field::new(name.clone()).id_salt(field.key()).enabled(self.enabled).side(side).sizing(sizing).show(ui, |ui, _| {
            let mut number = NumberField::new(name.clone(), Bind::given(shown_value))
                .range(-bound..=bound)
                .codec(codec.clone())
                .unit(unit.clone())
                .scrub_by_label()
                .sizing(sizing);
            if percent {
                number = number.commit_decimals(decimals);
            }
            if menu.is_some() {
                number = number.own_context_menu();
            }
            let shown = number.show(ui);
            events = shown.edit.events.clone();
            submit = shown.edit.submitted.then_some(shown.edit.submit_mods);
            link_pressed = shown.link;
            if let Some(entries) = menu {
                let entries: Vec<Entry<UnitKind>> = entries.iter().map(|entry| entry.clone().checked(entry.value() == Some(&kind))).collect();
                picked = Menu::context(name.clone(), &shown.edit.response).values().pick(ui, &entries).picked;
            }
        });
        self.out.unit_picked = self.out.unit_picked.or(picked);
        if let Some(mods) = submit {
            self.out.submitted = self.out.submitted.or(Some((field, mods)));
        }
        if link_pressed {
            self.out.link = self.out.link.or(Some(field));
        }
        let places = i32::try_from(decimals).unwrap_or(0);
        for event in events {
            let lifted = lift(&mut pass.bases, &pass.before, conversion, field, event, current, 0.5 * 10f64.powi(-places));
            if let Some(lifted) = lifted {
                self.report(cx, lifted, TransformSource::Field(field));
            }
        }
    }

    /// The reference point picker; a pick moves the reference point to that point of the box.
    fn reference(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, pass: &Pass, compact: bool) {
        let anchor = pass.before.state.anchor_of_pivot().unwrap_or(Anchor9::Centre);
        let mut picker = ReferencePoint::new(self.words.reference.clone(), Bind::given(Some(anchor))).enabled(self.enabled).look(self.reference_look);
        if compact {
            picker = picker.compact();
        }
        let shown = picker.show(ui);
        let Some(picked) = shown.value().copied().flatten() else { return };
        let before = pass.before;
        let state = BoxState::new(before.state.quad, before.state.point_at(picked));
        let event = EditEvent::set(Mixed::Same(before), TransformRow { state, skew_v: before.skew_v }, self.name.clone(), None);
        self.report(cx, event, TransformSource::Reference);
    }

    /// The aspect link toggle between W and H.
    fn link(&mut self, ui: &mut Ui) {
        let toggled = link_toggle(self.words.link.clone(), self.linked.reborrowed()).enabled(self.enabled).show(ui);
        if let Some(on) = toggled.value().copied() {
            self.out.linked = Some(on);
        }
    }

    /// The Δ toggle between X and Y; shown when the host gave the `relative` binding.
    fn relative_toggle(&mut self, ui: &mut Ui) {
        let Some(bind) = self.relative.as_mut() else { return };
        let toggled = ToggleButton::new(self.words.relative.clone(), bind.reborrowed())
            .icon(IconId::new("triangle"))
            .icon_only()
            .enabled(self.enabled)
            .show(ui);
        if let Some(on) = toggled.value().copied() {
            self.out.relative = Some(on);
        }
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

    /// The panel form: one grid whose two field columns fill the width, with the link's column between them. W, the
    /// link and H, then X and Y, the angle, then the two skews; the reference point, when shown, leads the first row
    /// in a column of its own. A row with nothing in a column shows an empty cell there.
    fn rows(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let mut pass = self.pass(cx);
        match self.arrangement {
            FieldArrangement::PositionFirst => self.position_first(ui, cx, &mut pass),
            _ => self.size_first(ui, cx, &mut pass),
        }
        *cx.memory::<Bases>() = pass.bases;
    }

    /// The size-first grid of [`rows`](Self::rows).
    fn size_first(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, pass: &mut Pass) {
        let groups = self.groups;
        let columns: &'static [ColumnWidth] = if groups.reference { &REFERENCE_COLUMNS } else { &FIELD_COLUMNS };
        Grid::new(columns).id_salt("fields").fill_width().show(ui, |ui, _| {
            let mut first = true;
            if groups.size {
                self.lead(ui, cx, pass, &mut first);
                self.field(ui, cx, pass, TransformField::W, Form::Panel);
                self.link(ui);
                self.field(ui, cx, pass, TransformField::H, Form::Panel);
            }
            if groups.position || (groups.reference && first) {
                self.lead(ui, cx, pass, &mut first);
                if groups.position {
                    self.field(ui, cx, pass, TransformField::X, Form::Panel);
                    empty(ui);
                    self.field(ui, cx, pass, TransformField::Y, Form::Panel);
                } else {
                    (0..3).for_each(|_| empty(ui));
                }
            }
            if groups.angle {
                self.lead(ui, cx, pass, &mut first);
                self.field(ui, cx, pass, TransformField::Angle, Form::Panel);
                empty(ui);
                empty(ui);
            }
            if groups.skew {
                self.lead(ui, cx, pass, &mut first);
                self.field(ui, cx, pass, TransformField::SkewH, Form::Panel);
                empty(ui);
                self.field(ui, cx, pass, TransformField::SkewV, Form::Panel);
            }
        });
    }

    /// The position-first form of [`rows`](Self::rows): X and W, then Y and H, in a grid beside the link, which stands
    /// centred across their two rows; the reference point, when shown, leads the grid's first row in a column of its
    /// own. The angle and the skews follow in a grid of their own.
    fn position_first(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, pass: &mut Pass) {
        let groups = self.groups;
        if groups.position || groups.size || groups.reference {
            let columns: &'static [ColumnWidth] = if groups.reference { &REFERENCE_PAIR_COLUMNS } else { &PAIR_COLUMNS };
            Row::new().bracket(Bracket::Unlinked).fill_width().show(ui, |ui, _| {
                Grid::new(columns).id_salt("fields").fill_width().show(ui, |ui, _| {
                    let mut first = true;
                    let rows = if groups.position || groups.size { 2 } else { 1 };
                    for row in [[TransformField::X, TransformField::W], [TransformField::Y, TransformField::H]].into_iter().take(rows) {
                        self.lead(ui, cx, pass, &mut first);
                        self.field_or_empty(ui, cx, pass, groups.position, row[0]);
                        self.field_or_empty(ui, cx, pass, groups.size, row[1]);
                    }
                });
                if groups.size {
                    self.link(ui);
                }
            });
        }
        if !(groups.angle || groups.skew) {
            return;
        }
        Grid::new(&PAIR_COLUMNS).id_salt("turns").fill_width().show(ui, |ui, _| {
            if groups.angle {
                self.field(ui, cx, pass, TransformField::Angle, Form::Panel);
                empty(ui);
            }
            if groups.skew {
                self.field(ui, cx, pass, TransformField::SkewH, Form::Panel);
                self.field(ui, cx, pass, TransformField::SkewV, Form::Panel);
            }
        });
    }

    /// The panel field `field` when `shown`, else an empty cell.
    fn field_or_empty(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, pass: &mut Pass, shown: bool, field: TransformField) {
        if shown {
            self.field(ui, cx, pass, field, Form::Panel);
        } else {
            empty(ui);
        }
    }

    /// The panel form's leading cell of a row, while the reference point shows: the point on the first row, an empty
    /// cell below it.
    fn lead(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, pass: &Pass, first: &mut bool) {
        if !self.groups.reference {
            return;
        }
        if std::mem::take(first) {
            self.reference(ui, cx, pass, true);
        } else {
            empty(ui);
        }
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
            self.field(ui, cx, &mut pass, TransformField::X, Form::Bar);
            self.relative_toggle(ui);
            self.field(ui, cx, &mut pass, TransformField::Y, Form::Bar);
            any.set(true);
        }
        if divide(ui, groups.size) {
            self.field(ui, cx, &mut pass, TransformField::W, Form::Bar);
            self.link(ui);
            self.field(ui, cx, &mut pass, TransformField::H, Form::Bar);
        }
        if divide(ui, groups.angle || groups.skew) {
            if groups.angle {
                self.field(ui, cx, &mut pass, TransformField::Angle, Form::Bar);
            }
            if groups.skew {
                self.field(ui, cx, &mut pass, TransformField::SkewH, Form::Bar);
                self.field(ui, cx, &mut pass, TransformField::SkewV, Form::Bar);
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
            bases.0[slot] = Some((id, *before, conversion.rect));
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
