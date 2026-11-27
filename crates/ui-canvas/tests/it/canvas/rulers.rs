//! The ruler tick rule (`Rulers::ticks`): labelled ticks stay at least `LABEL_STEP_MIN` apart at every zoom.

use craft_behavior::geometry::{SnapOptions, SnapTargets};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{CancelReason, CanvasView, RulerUnits, TickLabels, Ticks};
use craft_ui::canvas::{
    CENTIMETRE_LABEL_STEP_MIN, Canvas, CanvasTool, INCH_EIGHTHS, INCH_SIXTEENTHS, LABEL_STEP_MIN, PageFrame, Pasteboard, RulerText, Rulers, Snap,
    StepLadder, TickRule, ToolAction, ToolContext, ToolPath, ToolPointer,
};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::primitives::TickRank;
use craft_ui::primitives::{Draw, PaintShape};
use craft_ui::runtime::env::UnitKind;
use craft_ui::style::StyleColor;
use craft_ui::testing::{Fixture, ToolFixture, trial};
use egui::{Color32, CornerRadius, Pos2, Rect, Shape, Vec2};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
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

/// A document ruler labels every whole centimetre at 100% (28.35 points each), and thins the labels out when zoomed
/// out so they never touch.
#[trial]
fn a_centimetre_document_ruler_labels_each_centimetre_and_thins_out_zoomed_out() {
    let centimetres = RulerUnits::new(2.54 / 72.0, Ticks::decimal(false));
    let rule = TickRule::of_unit(UnitKind::Centimetres);
    let at_full = least_label_gap(&centimetres, rule, 1.0);
    assert!(at_full < f64::from(LABEL_STEP_MIN), "labels at 100% are closer than the generic metric step: {at_full}");
    let one_cm = 72.0 / 2.54;
    assert!((at_full - one_cm).abs() < 1e-6, "labels at 100% are one centimetre apart: {at_full}");
    let least = f64::from(CENTIMETRE_LABEL_STEP_MIN) - 1e-9;
    for zoom in ZOOMS {
        let gap = least_label_gap(&centimetres, rule, zoom);
        assert!(gap >= least, "centimetre labels at zoom {zoom}: {gap} points apart");
    }
    assert!(least_label_gap(&centimetres, rule, 0.25) > at_full, "labels thin out when zoomed out");
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

/// The vertical extents of the tick marks the top strip paints over a canvas whose view is turned by `degrees`.
fn top_tick_extents(degrees: f64) -> BTreeSet<(i32, i32)> {
    let room = Vec2::new(800.0, 600.0);
    let view = CanvasView::new(Rect::from_min_size(Pos2::ZERO, room), Point::new(400.0, 300.0), 1.0).rotation(degrees);
    let text = RulerText::new(RulerUnits::pixels(), UnitKind::Pixels);
    let content = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&content);
    let mut zero = Point::ZERO;
    let mut fixture = Fixture::new(move |ui| {
        let strips = Rulers::new("Rulers", Bind::bound(&mut zero));
        let shown = Canvas::new("Document").view(view).with_rulers(Some(strips)).rulers(text, text).show(ui, |_, _| {});
        if let Some(rulers) = shown.output.rulers {
            seen.set(rulers.content_rect());
        }
    });
    fixture.room(room);
    fixture.settle();
    let strip_bottom = content.get().min.y;
    fixture
        .frame_shapes()
        .iter()
        .map(|shape| shape.visual_bounding_rect())
        .filter(|bounds| bounds.width() < 3.0 && bounds.height() > 1.0 && bounds.max.y <= strip_bottom + 0.5)
        .map(|bounds| ((bounds.min.y * 10.0).round() as i32, (bounds.max.y * 10.0).round() as i32))
        .collect()
}

#[trial]
fn ticks_stay_level_and_the_zero_label_sits_at_the_zero_point_in_a_turned_view() {
    let level = top_tick_extents(0.0);
    let turned = top_tick_extents(30.0);
    assert!(!turned.is_empty(), "the turned view's top strip has tick marks");
    assert!(turned.is_subset(&level), "tick marks keep the lengths and cross coordinates of the level view: {turned:?} against {level:?}");
}

/// The labels of the top strip, left to right, over an inch ruler written as `labels` whose zero point is the middle of
/// a view at 4 points per document point (72 to the inch), so the step is a fraction of an inch.
fn top_inch_labels(labels: TickLabels) -> Vec<String> {
    let room = Vec2::new(800.0, 600.0);
    let middle = Point::new(400.0, 300.0);
    let view = CanvasView::new(Rect::from_min_size(Pos2::ZERO, room), middle, 4.0);
    let text = RulerText::new(RulerUnits::new(1.0 / 72.0, Ticks::decimal(false)).with_labels(labels), UnitKind::Inches);
    let content = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&content);
    let mut zero = middle;
    let mut fixture = Fixture::new(move |ui| {
        let strips = Rulers::new("Rulers", Bind::bound(&mut zero));
        let shown = Canvas::new("Document").view(view).with_rulers(Some(strips)).rulers(text, text).show(ui, |_, _| {});
        if let Some(rulers) = shown.output.rulers {
            seen.set(rulers.content_rect());
        }
    });
    fixture.room(room);
    fixture.settle();
    let content = content.get();
    let mut runs: Vec<(f32, String)> = fixture
        .texts()
        .iter()
        .filter(|run| run.rect.max.y <= content.min.y + 0.5 && run.rect.min.x >= content.min.x - 0.5)
        .map(|run| (run.rect.min.x, run.text.clone()))
        .collect();
    runs.sort_by(|a, b| a.0.total_cmp(&b.0));
    runs.into_iter().map(|(_, text)| text).collect()
}

/// VectorCraft's signed labels write the same ticks as the default distance labels, with a minus sign left of the zero
/// point and only the decimals each tick needs; the default keeps PhotoCraft's unsigned, padded form.
#[trial]
fn signed_tick_labels_carry_the_sign_and_drop_trailing_zeros() {
    let distance = top_inch_labels(TickLabels::Distance);
    let signed = top_inch_labels(TickLabels::Signed);
    assert_eq!(distance.len(), signed.len(), "both styles label the same ticks: {distance:?} against {signed:?}");
    let fractional = |labels: &[String]| labels.iter().any(|label| label.contains('.'));
    assert!(fractional(&distance) && fractional(&signed), "the step is a fraction of an inch: {signed:?}");
    assert!(!distance.iter().any(|label| label.starts_with('-')), "distance labels carry no sign: {distance:?}");
    assert!(distance.iter().any(|label| label.contains('.') && label.ends_with('0')), "distance labels pad the decimals: {distance:?}");
    assert!(signed.iter().any(|label| label.starts_with('-')), "labels left of the zero point are negative: {signed:?}");
    assert!(!signed.iter().any(|label| label.contains('.') && label.ends_with('0')), "signed labels drop trailing zeros: {signed:?}");
    for (padded, plain) in distance.iter().zip(&signed) {
        let (padded, plain) = (padded.parse::<f64>(), plain.parse::<f64>().map(f64::abs));
        assert_eq!(padded, plain, "each tick is the same distance from the zero point in both styles");
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

/// The units menu names each unit as a list of units does ("Points", "Millimetres"), not by the symbol a number
/// carries ("pt", "mm").
#[trial]
fn the_units_menu_names_each_unit() {
    const UNITS: [UnitKind; 4] = [UnitKind::Points, UnitKind::Picas, UnitKind::Inches, UnitKind::Millimetres];
    let zero = Rc::new(RefCell::new(Point::ZERO));
    let shown_zero = Rc::clone(&zero);
    let room = Rc::new(Cell::new(Rect::NOTHING));
    let shown_room = Rc::clone(&room);
    let mut fixture = Fixture::new(move |ui| {
        let at = ui.available_rect_before_wrap();
        shown_room.set(at);
        let view = CanvasView::new(at, Point::new(f64::from(at.center().x), f64::from(at.center().y)), 1.0);
        let mut zero = shown_zero.borrow_mut();
        let _ =
            Canvas::new("Document").view(view).with_rulers(Some(Rulers::new("Rulers", Bind::bound(&mut *zero)).units(&UNITS))).show(ui, |_, _| {});
    });
    fixture.room(Vec2::new(500.0, 400.0)).settle();
    let at = egui::pos2(room.get().center().x, room.get().min.y + 4.0);
    fixture.frame(vec![craft_ui::testing::pointer::moved(at)]);
    fixture.frame(vec![craft_ui::testing::pointer::secondary(at, true)]);
    fixture.frame(vec![craft_ui::testing::pointer::secondary(at, false)]);
    fixture.settle();
    assert!(fixture.by_label("Reset Zero Point").is_some(), "the units menu is open");
    for (name, symbol) in [("Points", "pt"), ("Picas", "pica"), ("Inches", "in"), ("Millimetres", "mm")] {
        assert!(fixture.by_label(name).is_some(), "the menu lists {name}");
        assert!(fixture.by_label(symbol).is_none(), "no row reads {symbol}");
    }
}

/// A drag of the rulers' corner with one move past the drag threshold, released there, moves the zero point to where it
/// was released: the drag proposes its landing at its start and at its release, not only on the moves between.
#[trial]
fn a_corner_drag_of_one_move_lands_the_zero_point() {
    let zero = Rc::new(RefCell::new(Point::ZERO));
    let view = |room: Rect| CanvasView::new(room, Point::new(f64::from(room.center().x), f64::from(room.center().y)), 1.0);
    let snap = Snap::new(std::sync::Arc::new(SnapTargets::new()), SnapOptions::default());
    let mut fixture = ToolFixture::with_rulers("Document", Overpaint, view, snap, zero.clone(), |_, _| {});
    fixture.fixture().room(Vec2::new(500.0, 400.0)).settle();
    let canvas = fixture.canvas().rect;
    let (from, to) = (canvas.min + Vec2::splat(4.0), canvas.center());
    let fixture = fixture.fixture();
    fixture.frame(vec![craft_ui::testing::pointer::moved(from)]);
    fixture.frame(vec![craft_ui::testing::pointer::press(from)]);
    fixture.frame(vec![craft_ui::testing::pointer::moved(to)]);
    fixture.frame(vec![craft_ui::testing::pointer::release(to)]);
    fixture.settle();
    assert_ne!(*zero.borrow(), Point::ZERO, "the zero point lands where the one-move drag released");
}

/// A host that moves the zero point under a corner drag cancels the drag's edit while the press goes on; the release
/// that follows lands nothing, so the host's zero point stays.
#[trial]
fn a_corner_drag_the_host_cancelled_lands_nothing_at_its_release() {
    let zero = Rc::new(RefCell::new(Point::ZERO));
    let view = |room: Rect| CanvasView::new(room, Point::new(f64::from(room.center().x), f64::from(room.center().y)), 1.0);
    let snap = Snap::new(std::sync::Arc::new(SnapTargets::new()), SnapOptions::default());
    let mut fixture = ToolFixture::with_rulers("Document", Overpaint, view, snap, zero.clone(), |_, _| {});
    fixture.fixture().room(Vec2::new(500.0, 400.0)).settle();
    let canvas = fixture.canvas().rect;
    let (from, to) = (canvas.min + Vec2::splat(4.0), canvas.center());
    let fixture = fixture.fixture();
    fixture.frame(vec![craft_ui::testing::pointer::moved(from)]);
    fixture.frame(vec![craft_ui::testing::pointer::press(from)]);
    fixture.frame(vec![craft_ui::testing::pointer::moved(to)]);
    fixture.settle();
    let dragged = *zero.borrow();
    assert_ne!(dragged, Point::ZERO, "the drag moves the zero point");
    let host = Point::new(-dragged.x, -dragged.y);
    *zero.borrow_mut() = host;
    fixture.frame(vec![craft_ui::testing::pointer::moved(to + Vec2::splat(10.0))]);
    fixture.frame(vec![craft_ui::testing::pointer::release(to + Vec2::splat(10.0))]);
    fixture.settle();
    assert_eq!(*zero.borrow(), host, "the release after the host's change leaves the zero point where the host put it");
}

/// The strokes of the vertical cross hair that a corner drag shows in `mode` and `contrast`, in paint order, and the
/// colour of the pasteboard under them.
fn cross_hair_strokes(mode: craft_theme::Mode, contrast: craft_theme::Contrast) -> (Vec<Color32>, craft_theme::Rgba) {
    let zero = Rc::new(RefCell::new(Point::ZERO));
    let view = |room: Rect| CanvasView::new(room, Point::new(f64::from(room.center().x), f64::from(room.center().y)), 1.0);
    let snap = Snap::new(std::sync::Arc::new(SnapTargets::new()), SnapOptions::default());
    let mut fixture = ToolFixture::with_rulers("Document", Overpaint, view, snap, zero, |_, _| {});
    fixture.fixture().theme(mode, contrast).room(Vec2::new(500.0, 400.0)).settle();
    let canvas = fixture.canvas().rect;
    let (from, to) = (canvas.min + Vec2::splat(4.0), canvas.center());
    let fixture = fixture.fixture();
    fixture.frame(vec![craft_ui::testing::pointer::moved(from)]);
    fixture.frame(vec![craft_ui::testing::pointer::press(from)]);
    fixture.frame(vec![craft_ui::testing::pointer::moved(to)]);
    let shapes = fixture.frame_shapes();
    let vertical = |points: &[Pos2]| {
        points.len() == 2
            && (points[0].x - to.x).abs() < 1.0
            && (points[1].x - to.x).abs() < 1.0
            && (points[0].y - points[1].y).abs() > canvas.height() / 2.0
    };
    let strokes = shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::LineSegment { points, stroke } if vertical(points) => Some(stroke.color),
            Shape::Path(path) if vertical(&path.points) => match path.stroke.color {
                egui::epaint::ColorMode::Solid(color) => Some(color),
                _ => None,
            },
            _ => None,
        })
        .collect();
    let pasteboard = craft_ui::testing::style::tokens(mode, contrast).color.surface.canvas;
    (strokes, pasteboard)
}

/// With artboards hidden the paper fills the canvas, so the zero point's cross hairs cross the paper as well as the
/// pasteboard and dark art. Over each, the top line contrasts with what lies under it (the strokes below it over the
/// backdrop) by the minimum the theme sets for overlay strokes on their halo: 3 to 1, and 4.5 to 1 in high contrast.
#[trial]
fn the_zero_point_cross_hairs_contrast_with_the_paper_and_the_pasteboard() {
    use craft_theme::{Contrast, Mode, Rgba, contrast_ratio, over};
    let rgba = |color: Color32| Rgba(color.to_srgba_unmultiplied());
    for mode in [Mode::Light, Mode::Dark] {
        for contrast in [Contrast::Standard, Contrast::High] {
            let (strokes, pasteboard) = cross_hair_strokes(mode, contrast);
            let Some((line, under)) = strokes.split_last() else {
                panic!("{mode:?} {contrast:?}: a corner drag shows a vertical cross hair");
            };
            let min = if contrast == Contrast::High { 4.5 } else { 3.0 };
            for (name, backdrop) in [("the paper", Rgba::WHITE), ("dark art", Rgba::BLACK), ("the pasteboard", pasteboard)] {
                let below = under.iter().fold(backdrop, |bg, stroke| over(rgba(*stroke), bg));
                let ratio = contrast_ratio(over(rgba(*line), below), below);
                assert!(ratio >= min, "{mode:?} {contrast:?}: the cross hair over {name} contrasts {ratio:.2} to 1, needs {min}");
            }
        }
    }
}

/// A right-click on a ruler strip or on the corner is the rulers': they report the part, for their units menu, and
/// the tool host under them asks for no menu of its own.
#[trial]
fn a_right_click_on_a_ruler_is_the_rulers_not_the_tools() {
    use craft_ui::canvas::{RulerPart, ToolHost, tool_pass};
    let zero = Rc::new(RefCell::new(Point::ZERO));
    let asked: Rc<RefCell<(Option<RulerPart>, bool)>> = Rc::new(RefCell::new((None, false)));
    let (shown_zero, seen) = (Rc::clone(&zero), Rc::clone(&asked));
    let room = Rc::new(Cell::new(Rect::NOTHING));
    let shown_room = Rc::clone(&room);
    let tool = RefCell::new(Overpaint);
    let mut fixture = Fixture::new(move |ui| {
        let at = ui.available_rect_before_wrap();
        shown_room.set(at);
        let view = CanvasView::new(at, Point::new(f64::from(at.center().x), f64::from(at.center().y)), 1.0);
        let mut zero = shown_zero.borrow_mut();
        let mut host_asked = false;
        let canvas = Canvas::new("Document").view(view).with_rulers(Some(Rulers::new("Rulers", Bind::bound(&mut *zero)))).show(ui, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                host_asked = ToolHost::new().run(ui, &pass, &mut *tool.borrow_mut()).context.is_some();
            }
        });
        let mut seen = seen.borrow_mut();
        seen.0 = seen.0.or(canvas.output.rulers.as_ref().and_then(|rulers| rulers.context));
        seen.1 |= host_asked;
    });
    fixture.room(Vec2::new(500.0, 400.0)).settle();
    let mut right_click = |at: Pos2| {
        *asked.borrow_mut() = (None, false);
        fixture.frame(vec![craft_ui::testing::pointer::moved(at)]);
        fixture.frame(vec![craft_ui::testing::pointer::secondary(at, true)]);
        fixture.frame(vec![craft_ui::testing::pointer::secondary(at, false)]);
        fixture.settle();
        *asked.borrow()
    };
    let canvas = room.get();
    assert_eq!(right_click(egui::pos2(canvas.center().x, canvas.min.y + 4.0)), (Some(RulerPart::Horizontal), false), "the top strip");
    assert_eq!(right_click(canvas.min + Vec2::splat(4.0)), (Some(RulerPart::Corner), false), "the corner, its zero point unlocked");
}
