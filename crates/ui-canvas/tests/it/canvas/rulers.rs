//! The ruler tick rule (`Rulers::ticks`): labelled ticks stay at least `LABEL_STEP_MIN` apart at every zoom.

use craft_behavior::geometry::{SnapOptions, SnapTargets};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{CancelReason, CanvasView, RulerUnits, Ticks};
use craft_ui::canvas::{
    CanvasTool, INCH_EIGHTHS, INCH_SIXTEENTHS, LABEL_STEP_MIN, PageFrame, Pasteboard, Rulers, Snap, StepLadder, TickRule, ToolAction, ToolContext,
    ToolPath, ToolPointer,
};
use craft_ui::contract::Show as _;
use craft_ui::primitives::TickRank;
use craft_ui::primitives::{Draw, PaintShape};
use craft_ui::style::StyleColor;
use craft_ui::testing::{ToolFixture, trial};
use egui::{Color32, CornerRadius, Rect, Shape, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// Screen points per document unit, from far out to deep in.
const ZOOMS: [f64; 10] = [0.01, 0.0625, 0.25, 1.0 / 3.0, 1.0, 3.7, 16.0, 64.0, 320.0, 3200.0];

/// The least distance in points between two consecutive labelled ticks of `rule` at `zoom`, over a ruler 2000 points
/// long.
fn least_label_gap(units: &RulerUnits, rule: TickRule<'_>, zoom: f64) -> f64 {
    let labelled: Vec<f64> = Rulers::ticks(units, rule, zoom, 0.0..=2000.0 / zoom * units.per_doc)
        .into_iter()
        .filter(|tick| tick.rank == TickRank::Major)
        .map(|tick| tick.value)
        .collect();
    assert!(labelled.len() >= 2, "{rule:?} at zoom {zoom} labels at least two ticks");
    labelled.windows(2).map(|pair| (pair[1] - pair[0]) / units.per_doc * zoom).fold(f64::INFINITY, f64::min)
}

#[trial]
fn labelled_ticks_never_touch_at_any_zoom() {
    let pixels = RulerUnits::pixels();
    let centimetres = RulerUnits::new(2.54 / 72.0, Ticks::decimal(false));
    let inches = RulerUnits::new(1.0 / 72.0, Ticks::decimal(false));
    // Frames at 30 per second, one frame per document unit.
    let frames = RulerUnits::new(1.0, Ticks::decimal(true));
    let ladder = StepLadder::new(30, 48.0, 6.0);
    let least = f64::from(LABEL_STEP_MIN) - 1e-9;
    for zoom in ZOOMS {
        for (units, rule) in [
            (&pixels, TickRule::Metric),
            (&centimetres, TickRule::Metric),
            (&inches, TickRule::Inches(&INCH_SIXTEENTHS)),
            (&inches, TickRule::Inches(&INCH_EIGHTHS)),
        ] {
            let gap = least_label_gap(units, rule, zoom);
            assert!(gap >= least, "{rule:?} at zoom {zoom}: labels {gap} points apart");
        }
        // The ladder's coarsest step is an hour of frames; below that the labels may touch by design.
        if 3600.0 * 30.0 * zoom >= f64::from(ladder.label_gap) {
            let gap = least_label_gap(&frames, TickRule::Steps(ladder), zoom);
            assert!(gap >= f64::from(ladder.label_gap) - 1e-9, "the step ladder at zoom {zoom}: labels {gap} points apart");
        }
    }
}

/// A vertical label stacks its characters one label-role size apart; the longest label the metric rule gives at the
/// least step (five characters) and its gap to the tick stay inside that step, so neighbouring labels never touch.
#[trial]
fn a_five_character_vertical_label_fits_the_least_label_step() {
    for mode in [craft_theme::Mode::Light, craft_theme::Mode::Dark] {
        let tokens = craft_ui::testing::style::tokens(mode, craft_theme::Contrast::Standard);
        let pitch = tokens.typography.get(craft_theme::TypeRole::Caption).size;
        assert!(5.0 * pitch + tokens.space.gap.tight < LABEL_STEP_MIN, "{mode:?}: five characters at pitch {pitch}");
    }
}

/// How far past the canvas the tool's fill and the marquee reach.
const PAST: f32 = 60.0;

/// A tool that paints past the canvas's rect.
struct Overpaint;

impl CanvasTool for Overpaint {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        draw.fill(&PaintShape::Rect(cx.rect.expand(PAST), CornerRadius::ZERO), StyleColor(Color32::RED));
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Select Area... command")
    }
}

// With rulers on, what the body shows (the tool's paint and a canvas control) is clipped to the view inside the
// strips; the rulers keep the canvas rect.
#[trial]
fn with_rulers_on_the_tool_and_a_canvas_control_paint_inside_the_view_and_the_rulers_keep_the_canvas() {
    let zero = Rc::new(RefCell::new(Point::ZERO));
    let view = |room: Rect| CanvasView::new(room, Point::new(f64::from(room.center().x), f64::from(room.center().y)), 1.0);
    let snap = Snap::new(std::sync::Arc::new(SnapTargets::new()), SnapOptions::default());
    let mut fixture = ToolFixture::with_rulers("Document", Overpaint, view, snap, zero, |ui, body| {
        let pages = [PageFrame::new(DocRect::new(-f64::from(PAST), -f64::from(PAST), 500.0 + f64::from(PAST), 400.0 + f64::from(PAST)))];
        let _ = Pasteboard::new("Pasteboard", &pages).show_at(ui, body.rect);
    });
    fixture.fixture().room(Vec2::new(500.0, 400.0)).settle();
    let canvas = fixture.canvas().rect;
    let shapes = fixture.fixture().frame_clipped_shapes();
    let past = |shape: &Shape| {
        let bounds = shape.visual_bounding_rect();
        bounds.is_finite() && !canvas.expand(PAST / 2.0).contains_rect(bounds)
    };
    let overflowing: Vec<&(Rect, Shape)> = shapes.iter().filter(|(_, shape)| past(shape)).collect();
    // The tool's fill and the marquee's outline reach past the canvas; the strips and the surface do not.
    assert!(overflowing.len() >= 2, "the tool and the pasteboard paint past the canvas: {overflowing:?}");
    // The view starts where the strips end: the top strip is as thick as the view's clip lies below the canvas's top.
    let thickness = overflowing.iter().map(|(clip, _)| clip.min.y - canvas.min.y).fold(f32::INFINITY, f32::min);
    for (clip, shape) in overflowing {
        assert!(
            clip.min.x > canvas.min.x && clip.min.y > canvas.min.y && canvas.contains_rect(*clip),
            "past the canvas, a shape paints inside the view, past the strips: clip {clip:?} in {canvas:?} for {shape:?}"
        );
    }
    // The canvas's own surface spans the whole rect, so only a shape inside the top strip, under the canvas's rect as
    // its clip, comes from the rulers.
    let strip = Rect::from_min_max(canvas.min, egui::pos2(canvas.max.x, canvas.min.y + thickness));
    let strips_keep_the_canvas = shapes.iter().any(|(clip, shape)| {
        let bounds = shape.visual_bounding_rect();
        bounds.is_finite() && bounds.width() > 0.0 && strip.expand(1.0).contains_rect(bounds) && clip.contains_rect(strip)
    });
    assert!(strips_keep_the_canvas, "the rulers paint inside the top strip under the canvas's rect: {strip:?} in {shapes:?}");
}
