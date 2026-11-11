//! [`ShapePanel`]: a shape layer's Appearance and Shape sections in the Properties panel, and the cells the
//! options bar shares with it.

use std::ops::RangeInclusive;

use super::{LiveShape, ShapeAppearance, ShapeChange, ShapeWords};
use crate::ui::stroke::StrokeButton;
use craft_behavior::{EditEvent, EditEvents, NumberCodec};
use craft_color::ui::{Paint, PaintChip, PaintChipOutput, ScreenPickGesture};
use craft_theme::Srgba;
use craft_ui::contract::{Bind, GlyphLabel, LabelKind, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::runtime::env::{Env, Label, LengthUnits, UnitKind, Units, with_env};
use craft_ui::sections::Section;
use craft_ui::text::NumberField;
use egui::Ui;

/// Which paint a chip edits, for [`ShapePanel::chips`] and [`ShapeBar::chips`](super::ShapeBar::chips).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ShapePaint {
    /// The fill chip.
    Fill,
    /// The stroke chip.
    Stroke,
}

/// The app's decoration of a paint chip before it shows: the Gradient, Pattern and Picture editors.
pub struct ChipSetup<'a>(Box<dyn for<'b> FnMut(ShapePaint, PaintChip<'b>) -> PaintChip<'b> + 'a>);

impl<'a> ChipSetup<'a> {
    pub(super) fn new(setup: impl for<'b> FnMut(ShapePaint, PaintChip<'b>) -> PaintChip<'b> + 'a) -> Self {
        ChipSetup(Box::new(setup))
    }
}

/// What [`ShapePanel::show`] reports.
#[derive(Debug, Default)]
#[non_exhaustive]
#[must_use = "the app turns each event into a document command"]
pub struct ShapePanelOutput {
    /// Each changed field this pass, in drawing order: a scrub or drag is `Begin`, `Update`, `Commit` (one undo
    /// step per gesture); a chip pick, a typed value or an arrow step is `Set`.
    pub events: Vec<EditEvent<ShapeChange>>,
    /// A chip's screen eyedropper was pressed: the host samples the screen and answers with
    /// [`ShapePanel::sampled`].
    pub screen_pick: Option<(ShapePaint, ScreenPickGesture)>,
}

/// A shape layer's Properties sections, as PhotoCraft's `shape_properties`.
///
/// - Appearance (a collapsible section): Fill chip, Stroke chip, stroke width (a number field, 0 to 288 px) and the
///   Stroke style button (enabled while the width is over 0, its popover open flag from
///   [`stroke_open`](Self::stroke_open)). Stroke width 0 is no stroke; an edit that turns a stroke on at 0 carries
///   [`ShapeChange::ENABLED_STROKE_WIDTH`] in the same change; an arrow step that clamps at 0 reports nothing.
/// - Shape (a collapsible section), by the [`LiveShape`]: W and H (1 to 300,000 px, whole pixels) for a rect,
///   ellipse or polygon; a rect's "Corner radius" (0 to 100,000, sets all four) then Top Left, Top Right, Bottom
///   Left, Bottom Right rows (PhotoCraft's row order, mapped to the radii order TL, TR, BR, BL); a polygon's Corner
///   radius, Sides (3 to 100) and Star ratio (1 to 100 %, stored 0.01 to 1); a line's Weight (1 to 10,000).
/// - A locked layer ([`ShapeAppearance::locked`]) makes the whole panel read-only.
/// - Keyboard: every field and chip is a Tab stop in reading order; sections fold with their title rows. Right to
///   left the rows mirror; the corner names stay.
#[must_use = "show the panel with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct ShapePanel<'a> {
    label: Label,
    shape: &'a LiveShape,
    appearance: &'a ShapeAppearance,
    stroke_open: Option<Bind<'a, bool>>,
    chips: Option<ChipSetup<'a>>,
    fallbacks: (Srgba, Srgba),
    sections: (bool, bool),
    sampled: Option<(ShapePaint, Srgba)>,
    pixels_per_inch: f64,
}

impl<'a> ShapePanel<'a> {
    /// The panel named `label` for `shape` with `appearance`, in a document of `pixels_per_inch`, which sizes a length typed in
    /// inches or points.
    pub fn new(label: impl Into<Label>, shape: &'a LiveShape, appearance: &'a ShapeAppearance, pixels_per_inch: f64) -> Self {
        ShapePanel {
            label: label.into(),
            shape,
            appearance,
            stroke_open: None,
            chips: None,
            fallbacks: (Srgba::BLACK, Srgba::WHITE),
            sections: (true, true),
            sampled: None,
            pixels_per_inch,
        }
    }

    /// The Stroke style popover's open flag, kept by the app between passes. Without it the button is not shown.
    pub fn stroke_open(mut self, open: Bind<'a, bool>) -> Self {
        self.stroke_open = Some(open);
        self
    }

    /// Decorates each paint chip before it shows (its Gradient, Pattern and Picture editors).
    pub fn chips(mut self, setup: impl for<'b> FnMut(ShapePaint, PaintChip<'b>) -> PaintChip<'b> + 'a) -> Self {
        self.chips = Some(ChipSetup::new(setup));
        self
    }

    /// The colours a switch to Solid starts from when a chip has none: the app's foreground (fill) and background
    /// (stroke). Black and white by default.
    pub fn fallbacks(mut self, fill: Srgba, stroke: Srgba) -> Self {
        self.fallbacks = (fill, stroke);
        self
    }

    /// Which sections show; both by default (an app may show one, or a custom path's layer only Appearance).
    pub fn sections(mut self, appearance: bool, shape: bool) -> Self {
        self.sections = (appearance, shape);
        self
    }

    /// The host's answer to a chip's screen eyedropper: the colour sampled for that chip.
    pub fn sampled(mut self, sampled: Option<(ShapePaint, Srgba)>) -> Self {
        self.sampled = sampled;
        self
    }

    /// Adds the sections to `ui` and reports the changed fields.
    pub fn show(self, ui: &mut Ui) -> ShapePanelOutput {
        let ShapePanel { label, shape, appearance, stroke_open, chips, fallbacks, sections, sampled, pixels_per_inch } = self;
        let env = Env::of(ui);
        let (words, units) = (env.words::<ShapeWords>(), env.units());
        let mut pass = Cells::new(&words, &units, pixels_per_inch, appearance, fallbacks, sampled, chips);
        pass.open = stroke_open;
        let mut body = |ui: &mut Ui| {
            FieldGroup::new(label.clone()).show(ui, |ui, _| {
                pass.begin();
                if sections.0 {
                    Section::new(words.appearance.clone()).id_salt("appearance").show(ui, |ui, _| {
                        pass.paint_row(ui, ShapePaint::Fill);
                        pass.paint_row(ui, ShapePaint::Stroke);
                        pass.width_row(ui);
                        pass.style_row(ui, !matches!(shape, LiveShape::Ellipse { .. }));
                    });
                }
                if sections.1 {
                    Section::new(words.shape.clone()).id_salt("liveShape").show(ui, |ui, _| pass.shape_rows(ui, shape));
                }
            });
        };
        if appearance.locked {
            with_env(ui, |scope| scope.read_only = Some(true), body);
        } else {
            body(ui);
        }
        ShapePanelOutput { events: pass.events, screen_pick: pass.screen_pick }
    }
}

/// One number cell: the field's key, caption, value, range, the lengths it converts between and what reads its text.
pub(super) struct NumberCell {
    pub key: &'static str,
    pub caption: Label,
    pub value: f64,
    pub range: RangeInclusive<f64>,
    lengths: Option<LengthUnits>,
    codec: NumberCodec,
}

impl NumberCell {
    /// A length in pixels with up to two decimals, in a document of `pixels_per_inch`.
    pub fn pixels(units: &Units, pixels_per_inch: f64, key: &'static str, caption: &Label, value: f64, range: RangeInclusive<f64>) -> Self {
        Self::length(units, pixels_per_inch, PIXEL_DECIMALS, key, caption, value, range)
    }

    /// A length in whole pixels, in a document of `pixels_per_inch`.
    pub fn whole_pixels(units: &Units, pixels_per_inch: f64, key: &'static str, caption: &Label, value: f64, range: RangeInclusive<f64>) -> Self {
        Self::length(units, pixels_per_inch, 0, key, caption, value, range)
    }

    fn length(
        units: &Units,
        pixels_per_inch: f64,
        decimals: usize,
        key: &'static str,
        caption: &Label,
        value: f64,
        range: RangeInclusive<f64>,
    ) -> Self {
        let lengths = LengthUnits::new(UnitKind::Pixels, UnitKind::Pixels, pixels_per_inch);
        NumberCell {
            key,
            caption: caption.clone(),
            value,
            range,
            lengths: Some(lengths),
            codec: NumberCodec::new(units.length_reader(lengths), decimals),
        }
    }

    /// A plain whole number.
    pub fn count(units: &Units, key: &'static str, caption: &Label, value: f64, range: RangeInclusive<f64>) -> Self {
        NumberCell { key, caption: caption.clone(), value, range, lengths: None, codec: NumberCodec::new(units.reader(None), 0) }
    }

    /// A percentage with up to one decimal.
    pub fn percent(units: &Units, key: &'static str, caption: &Label, value: f64, range: RangeInclusive<f64>) -> Self {
        NumberCell {
            key,
            caption: caption.clone(),
            value,
            range,
            lengths: None,
            codec: NumberCodec::new(units.reader(Some(UnitKind::Percent)), PERCENT_DECIMALS),
        }
    }

    /// The field over the cell's value.
    pub fn field(&self) -> NumberField<'static> {
        let value = Bind::given(Some(self.value));
        let field = match self.lengths {
            Some(lengths) => NumberField::length(self.caption.clone(), value, lengths),
            None => NumberField::new(self.caption.clone(), value),
        };
        field.range(self.range.clone()).codec(self.codec.clone())
    }

    /// The edit of the field shown in `ui`; an emptied field keeps the value.
    pub fn show(&self, ui: &mut Ui) -> EditEvents<f64> {
        let kept = self.value;
        self.field().scrub_by_label().show(ui).edit.events.map(|value| value.unwrap_or(kept))
    }
}

/// The most decimals a length in pixels shows.
const PIXEL_DECIMALS: usize = 2;
/// The most decimals a percentage shows.
const PERCENT_DECIMALS: usize = 1;

/// The cells of the Appearance and Shape sections, and the events they raise in one pass.
pub(super) struct Cells<'p, 'a> {
    words: &'p ShapeWords,
    units: &'p Units,
    /// Pixels per inch of the document, for lengths typed in other units.
    pixels_per_inch: f64,
    appearance: &'p ShapeAppearance,
    fallbacks: (Srgba, Srgba),
    sampled: Option<(ShapePaint, Srgba)>,
    chips: Option<ChipSetup<'a>>,
    pub(super) open: Option<Bind<'a, bool>>,
    pub events: Vec<EditEvent<ShapeChange>>,
    pub screen_pick: Option<(ShapePaint, ScreenPickGesture)>,
}

impl<'p, 'a> Cells<'p, 'a> {
    pub fn new(
        words: &'p ShapeWords,
        units: &'p Units,
        pixels_per_inch: f64,
        appearance: &'p ShapeAppearance,
        fallbacks: (Srgba, Srgba),
        sampled: Option<(ShapePaint, Srgba)>,
        chips: Option<ChipSetup<'a>>,
    ) -> Self {
        Cells { words, units, pixels_per_inch, appearance, fallbacks, sampled, chips, open: None, events: Vec::new(), screen_pick: None }
    }

    /// Starts a run of the body: it may run more than once in a pass, and each run starts clean.
    pub fn begin(&mut self) {
        self.events.clear();
        self.screen_pick = None;
    }

    /// The paint chip of `which`, decorated by the app's setup.
    pub fn chip(&mut self, ui: &mut Ui, which: ShapePaint) -> PaintChipOutput {
        let (paint, fallback, name, tip) = match which {
            ShapePaint::Fill => (&self.appearance.fill, self.fallbacks.0, &self.words.fill_color, &self.words.set_fill_type),
            ShapePaint::Stroke => (&self.appearance.stroke, self.fallbacks.1, &self.words.stroke_color, &self.words.set_stroke_type),
        };
        let mut chip = PaintChip::new(name.clone(), Bind::given(paint.clone()), fallback).none_allowed().tooltip(tip.clone());
        if let Some((_, color)) = self.sampled.filter(|(target, _)| *target == which) {
            chip = chip.sampled(color);
        }
        if let Some(setup) = self.chips.as_mut() {
            chip = (setup.0)(which, chip);
        }
        chip.show(ui)
    }

    /// Takes a chip's output: its edit as a change, its eyedropper press.
    pub fn take_chip(&mut self, which: ShapePaint, out: PaintChipOutput) {
        if let Some(gesture) = out.screen_pick {
            self.screen_pick = Some((which, gesture));
        }
        for event in out.edit.events {
            let paints = match &event {
                EditEvent::Set { after, .. } => !matches!(after, Paint::None),
                EditEvent::Begin { .. } => true,
                _ => false,
            };
            // A stroke at width 0 is no stroke: the edit that gives it a paint carries the width that shows it
            // (PhotoCraft's `enabled_stroke_width`), so the app applies one change.
            let enabling =
                (which == ShapePaint::Stroke && self.appearance.stroke_width <= 0.0 && paints).then_some(ShapeChange::ENABLED_STROKE_WIDTH);
            self.events.push(event.map(|paint| match which {
                ShapePaint::Fill => ShapeChange::Fill(paint),
                ShapePaint::Stroke => ShapeChange::Stroke { paint, width: enabling },
            }));
        }
    }

    /// A labelled paint row of the panel.
    fn paint_row(&mut self, ui: &mut Ui, which: ShapePaint) {
        let caption = match which {
            ShapePaint::Fill => self.words.fill.clone(),
            ShapePaint::Stroke => self.words.stroke.clone(),
        };
        let key = match which {
            ShapePaint::Fill => "fill",
            ShapePaint::Stroke => "stroke",
        };
        let out = Field::new(caption).id_salt(key).show(ui, |ui, _| self.chip(ui, which)).inner;
        if let Some(out) = out {
            self.take_chip(which, out);
        }
    }

    /// The stroke width cell: 0 to 288 px.
    pub fn width_cell(&self) -> NumberCell {
        NumberCell::pixels(
            self.units,
            self.pixels_per_inch,
            "stroke-width",
            &self.words.stroke_width,
            self.appearance.stroke_width,
            0.0..=STROKE_WIDTH_MAX,
        )
    }

    /// Takes the stroke width field's event. An arrow step that stays at 0 changes nothing, so it adds no stroke.
    pub fn take_width(&mut self, events: EditEvents<f64>) {
        for event in events {
            let unchanged = matches!(&event, EditEvent::Set { after, .. } | EditEvent::Update { value: after, .. } if *after <= 0.0 && self.appearance.stroke_width <= 0.0);
            if !unchanged {
                self.events.push(event.map(ShapeChange::StrokeWidth));
            }
        }
    }

    fn width_row(&mut self, ui: &mut Ui) {
        let cell = self.width_cell();
        let side = SideLabel::new().scrub(true);
        let events = Field::new(cell.caption.clone()).id_salt(cell.key).side(side).show(ui, |ui, _| cell.show(ui)).inner.unwrap_or_default();
        self.take_width(events);
    }

    /// The Stroke style button over the appearance's stroke style, when the app gave the popover's open flag. It is
    /// off while the width is 0; a pick then also carries the stroke's width in the same change.
    pub fn style_button(&mut self, ui: &mut Ui, corners: bool) {
        let off = self.appearance.stroke_width <= 0.0;
        let Some(open) = self.open.as_mut() else { return };
        let style = Bind::given(self.appearance.stroke_style.clone());
        let button = StrokeButton::new(self.words.stroke_style.clone(), style, open.reborrowed()).corners(corners).stroked(!off).enabled(!off);
        for event in button.show(ui).events {
            let width = (off && matches!(event, EditEvent::Set { .. } | EditEvent::Begin { .. })).then_some(ShapeChange::ENABLED_STROKE_WIDTH);
            self.events.push(event.map(|style| ShapeChange::StrokeStyle { style, width }));
        }
    }

    fn style_row(&mut self, ui: &mut Ui, corners: bool) {
        if self.open.is_some() {
            Field::new(self.words.stroke_style.clone()).id_salt("stroke-style").show(ui, |ui, _| self.style_button(ui, corners));
        }
    }

    /// A labelled row with `cell`; its edit goes out through `change`.
    fn number_row(&mut self, ui: &mut Ui, cell: NumberCell, change: impl Fn(f64) -> ShapeChange) {
        let side = SideLabel::new().scrub(true);
        let events = Field::new(cell.caption.clone()).id_salt(cell.key).side(side).show(ui, |ui, _| cell.show(ui)).inner.unwrap_or_default();
        self.events.extend(events.map(&change));
    }

    /// The W and H row of a rect, ellipse or polygon: whole pixels, 1 to 300,000.
    fn size_row(&mut self, ui: &mut Ui, (width, height): (f64, f64)) {
        let (units, ppi, words) = (self.units, self.pixels_per_inch, self.words);
        let cells = [
            NumberCell::whole_pixels(units, ppi, "width", &words.width_name, width, SIZE_RANGE),
            NumberCell::whole_pixels(units, ppi, "height", &words.height_name, height, SIZE_RANGE),
        ];
        let glyphs = [&words.width, &words.height];
        let mut edits = [EditEvents::new(), EditEvents::new()];
        Row::new().show(ui, |ui, _| {
            for ((cell, glyph), edit) in cells.iter().zip(glyphs).zip(edits.iter_mut()) {
                let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text(glyph.clone()))).scrub(true);
                *edit = Field::new(cell.caption.clone()).id_salt(cell.key).side(side).show(ui, |ui, _| cell.show(ui)).inner.unwrap_or_default();
            }
        });
        let [w, h] = edits;
        self.events.extend(w.map(|w| ShapeChange::Size { width: w, height }));
        self.events.extend(h.map(|h| ShapeChange::Size { width, height: h }));
    }

    /// The Shape section's rows for `shape`.
    fn shape_rows(&mut self, ui: &mut Ui, shape: &LiveShape) {
        let (units, ppi, words) = (self.units, self.pixels_per_inch, self.words);
        match shape {
            LiveShape::Rect { width, height, radii } => {
                self.size_row(ui, (*width, *height));
                let radii = *radii;
                let cell = NumberCell::pixels(units, ppi, "corner-radius", &words.corner_radius, radii[0], RADIUS_RANGE);
                self.number_row(ui, cell, |r| ShapeChange::Radii([r; 4]));
                // PhotoCraft's row order; the radii order is top-left, top-right, bottom-right, bottom-left.
                let corners = [
                    ("corner-top-left", &words.top_left, 0),
                    ("corner-top-right", &words.top_right, 1),
                    ("corner-bottom-left", &words.bottom_left, 3),
                    ("corner-bottom-right", &words.bottom_right, 2),
                ];
                for (key, caption, index) in corners {
                    let cell = NumberCell::pixels(units, ppi, key, caption, radii[index], RADIUS_RANGE);
                    self.number_row(ui, cell, |r| {
                        let mut next = radii;
                        next[index] = r;
                        ShapeChange::Radii(next)
                    });
                }
            }
            LiveShape::Ellipse { width, height } => self.size_row(ui, (*width, *height)),
            LiveShape::Polygon { width, height, corner_radius, sides, star_ratio } => {
                self.size_row(ui, (*width, *height));
                let radius = NumberCell::pixels(units, ppi, "corner-radius", &words.corner_radius, *corner_radius, RADIUS_RANGE);
                self.number_row(ui, radius, ShapeChange::CornerRadius);
                let count = NumberCell::count(units, "sides", &words.sides, f64::from(*sides), SIDES_RANGE);
                self.number_row(ui, count, sides_change);
                let ratio = NumberCell::percent(units, "star-ratio", &words.star_ratio, star_ratio * 100.0, RATIO_PERCENT);
                self.number_row(ui, ratio, |percent| ShapeChange::StarRatio(percent / 100.0));
            }
            LiveShape::Line { weight } => {
                let cell = NumberCell::pixels(units, ppi, "weight", &words.weight, *weight, 1.0..=10_000.0);
                self.number_row(ui, cell, ShapeChange::Weight);
            }
        }
    }
}

/// The change a Sides value makes: a whole number of sides in range.
pub(super) fn sides_change(sides: f64) -> ShapeChange {
    ShapeChange::Sides(sides.round().clamp(*SIDES_RANGE.start(), *SIDES_RANGE.end()) as u32)
}

/// The largest stroke width in pixels.
const STROKE_WIDTH_MAX: f64 = 288.0;
/// The range of a shape's width and height in whole pixels.
const SIZE_RANGE: RangeInclusive<f64> = 1.0..=300_000.0;
/// The range of a corner radius in pixels.
const RADIUS_RANGE: RangeInclusive<f64> = 0.0..=100_000.0;
/// The number of sides of a polygon or points of a star.
pub(super) const SIDES_RANGE: RangeInclusive<f64> = 3.0..=100.0;
/// The star ratio in percent.
const RATIO_PERCENT: RangeInclusive<f64> = 1.0..=100.0;
