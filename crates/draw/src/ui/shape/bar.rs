//! [`ShapeBar`]: the Shape tools' options bar cells.

use super::panel::{Cells, ChipSetup, NumberCell, SIDES_RANGE, sides_change};
use super::{ShapeAppearance, ShapeChange, ShapePaint, ShapeToolSettings, ShapeWords};
use crate::ui::shape_drag::ShapeKind;
use craft_behavior::EditEvent;
use craft_color::ui::{PaintChip, ScreenPickGesture};
use craft_theme::Srgba;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, LabelKind, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing};
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::runtime::env::Env;
use egui::Ui;

/// What [`ShapeBar::show`] reports.
#[derive(Debug, Default)]
#[non_exhaustive]
#[must_use = "the app routes each change to the selected layer and to the tool settings"]
pub struct ShapeBarOutput {
    /// Each changed cell this pass, as [`ShapePanelOutput::events`](super::ShapePanelOutput::events). With a shape
    /// layer selected the app applies it to the layer and to [`ShapeToolSettings`]; without, to the settings only.
    pub events: Vec<EditEvent<ShapeChange>>,
    /// A chip's screen eyedropper was pressed, as [`ShapePanelOutput::screen_pick`](super::ShapePanelOutput::screen_pick).
    pub screen_pick: Option<(ShapePaint, ScreenPickGesture)>,
}

/// The cells a Shape tool adds to the options bar, in PhotoCraft's order: the "Fill" check box and chip, the
/// "Stroke" chip, the stroke width (0 to 288 px), the Stroke style button; then the kind's cell: Rectangle and
/// Rounded Rectangle "Radius" (0 to 10,000), Polygon and Star "Sides" (3 to 100), Line "Weight" (1 to 1,000). A
/// pattern shown as children of a `Bar`.
///
/// - The values are the selected shape layer's when one is selected, else the tool settings' (the app passes the
///   right [`ShapeAppearance`]).
/// - Stroke width 0 is no stroke; an edit that turns the stroke on at 0 carries
///   [`ShapeChange::ENABLED_STROKE_WIDTH`] in the same change.
/// - Keyboard: every cell is a Tab stop in the bar's order; captions scrub their fields.
#[must_use = "show the cells with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct ShapeBar<'a> {
    kind: ShapeKind,
    settings: &'a ShapeToolSettings,
    appearance: &'a ShapeAppearance,
    stroke_open: Option<Bind<'a, bool>>,
    chips: Option<ChipSetup<'a>>,
    fallbacks: (Srgba, Srgba),
    sampled: Option<(ShapePaint, Srgba)>,
    pixels_per_inch: f64,
}

impl<'a> ShapeBar<'a> {
    /// The cells for `kind` with the `settings` and `appearance` shown, in a document of `pixels_per_inch`, which sizes a length
    /// typed in inches or points.
    pub fn new(kind: ShapeKind, settings: &'a ShapeToolSettings, appearance: &'a ShapeAppearance, pixels_per_inch: f64) -> Self {
        ShapeBar {
            kind,
            settings,
            appearance,
            stroke_open: None,
            chips: None,
            fallbacks: (Srgba::BLACK, Srgba::WHITE),
            sampled: None,
            pixels_per_inch,
        }
    }

    /// The Stroke style popover's open flag; without it the button is not shown.
    pub fn stroke_open(mut self, open: Bind<'a, bool>) -> Self {
        self.stroke_open = Some(open);
        self
    }

    /// Decorates each paint chip before it shows (its Gradient, Pattern and Picture editors).
    pub fn chips(mut self, setup: impl for<'b> FnMut(ShapePaint, PaintChip<'b>) -> PaintChip<'b> + 'a) -> Self {
        self.chips = Some(ChipSetup::new(setup));
        self
    }

    /// The foreground and background colours a switch to Solid starts from.
    pub fn fallbacks(mut self, fill: Srgba, stroke: Srgba) -> Self {
        self.fallbacks = (fill, stroke);
        self
    }

    /// The host's answer to a chip's screen eyedropper: the colour sampled for that chip.
    pub fn sampled(mut self, sampled: Option<(ShapePaint, Srgba)>) -> Self {
        self.sampled = sampled;
        self
    }

    /// Adds the cells to the bar's row `ui`.
    pub fn show(self, ui: &mut Ui) -> ShapeBarOutput {
        let ShapeBar { kind, settings, appearance, stroke_open, chips, fallbacks, sampled, pixels_per_inch } = self;
        let env = Env::of(ui);
        let (words, units) = (env.words::<ShapeWords>(), env.units());
        let mut pass = Cells::new(&words, &units, pixels_per_inch, appearance, fallbacks, sampled, chips);
        pass.open = stroke_open;
        Row::new().enabled(!appearance.locked).show(ui, |ui, _| {
            pass.begin();
            let fill_on = !matches!(appearance.fill, craft_color::ui::Paint::None);
            let check = Checkbox::new(Bind::given(fill_on)).label(words.fill.clone()).show(ui).events;
            pass.events.extend(check.map(ShapeChange::FillOn));
            let hidden = SideLabel::new().kind(LabelKind::Hidden);
            let fill = Field::new(words.fill_color.clone())
                .id_salt("fill")
                .side(hidden.clone())
                .sizing(Sizing::Content)
                .show(ui, |ui, _| pass.chip(ui, ShapePaint::Fill));
            if let Some(out) = fill.inner {
                pass.take_chip(ShapePaint::Fill, out);
            }
            let stroke =
                Field::new(words.stroke.clone()).id_salt("stroke").sizing(Sizing::Content).show(ui, |ui, _| pass.chip(ui, ShapePaint::Stroke));
            if let Some(out) = stroke.inner {
                pass.take_chip(ShapePaint::Stroke, out);
            }
            let width = pass.width_cell();
            let event =
                Field::new(width.caption.clone()).id_salt(width.key).side(hidden).sizing(Sizing::Content).show(ui, |ui, _| width.show(ui)).inner;
            pass.take_width(event.unwrap_or_default());
            pass.style_button(ui, kind != ShapeKind::Ellipse);
            let cell = match kind {
                ShapeKind::Rectangle | ShapeKind::RoundedRectangle => Some((
                    NumberCell::pixels(&units, pixels_per_inch, "radius", &words.radius, settings.corner_radius, 0.0..=RADIUS_MAX),
                    ShapeChange::CornerRadius as fn(f64) -> ShapeChange,
                )),
                ShapeKind::Polygon | ShapeKind::Star => Some((
                    NumberCell::count(&units, "sides", &words.sides, f64::from(settings.sides), SIDES_RANGE),
                    sides_change as fn(f64) -> ShapeChange,
                )),
                ShapeKind::Line => Some((
                    NumberCell::pixels(&units, pixels_per_inch, "weight", &words.weight, settings.line_weight, 1.0..=LINE_WEIGHT_MAX),
                    ShapeChange::Weight as fn(f64) -> ShapeChange,
                )),
                _ => None,
            };
            if let Some((cell, change)) = cell {
                let side = SideLabel::new().scrub(true);
                let event =
                    Field::new(cell.caption.clone()).id_salt(cell.key).side(side).sizing(Sizing::Content).show(ui, |ui, _| cell.show(ui)).inner;
                pass.events.extend(event.unwrap_or_default().map(change));
            }
        });
        ShapeBarOutput { events: pass.events, screen_pick: pass.screen_pick }
    }
}

/// The largest corner radius of the bar's Radius cell, in pixels.
const RADIUS_MAX: f64 = 10_000.0;
/// The largest weight of the bar's Line cell, in pixels.
const LINE_WEIGHT_MAX: f64 = 1_000.0;
