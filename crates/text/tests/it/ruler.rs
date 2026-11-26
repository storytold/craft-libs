//! The text ruler's keys: the ruler is one Tab stop, Page Down moves between its markers and stops, and the arrows
//! move the focused one. Alt in a column border drag shows the column widths.

use craft_behavior::GesturePhase;
use craft_text::ui::DocUnit;
use craft_text::ui::ruler::{GapDrag, GapEdge, RulerItem, RulerValues, TabAlign, TabLeader, TabStop, TextRuler};
use craft_theme::ComponentSet;
use craft_ui::buttons::Button;
use craft_ui::canvas::Rulers;
use craft_ui::contract::{Bind, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::popover::Popover;
use craft_ui::primitives::RulerFace;
use craft_ui::runtime::env::UnitKind;
use craft_ui::runtime::env::with_env;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Which of `labels` holds the focus.
fn focused<'a>(fixture: &Fixture, labels: &[&'a str]) -> Option<&'a str> {
    let focused = craft_ui::testing::access::focused_key(fixture.tree());
    labels.iter().copied().find(|label| fixture.by_label(label).is_some_and(|target| target.node == focused))
}

#[trial]
fn the_ruler_is_one_tab_stop_whose_page_keys_pick_and_arrows_move() {
    let start = RulerValues { stops: vec![TabStop::new(72.0, TabAlign::Left, TabLeader::None)], ..Default::default() };
    let values = Rc::new(RefCell::new(start));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        Stack::new().show(ui, |ui, _| {
            Button::new("Before").show(ui);
            TextRuler::new("Ruler", Bind::bound(&mut *shown.borrow_mut()), 300.0).show(ui);
            Button::new("After").show(ui);
        });
    });
    fixture.idle(4);
    let labels = ["Before", "First Line Indent", "Hanging Indent", "After"];
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some("Before"), "Tab reaches the first button");
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some("First Line Indent"), "Tab enters the ruler at its first marker");
    fixture.key(Key::PageDown, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some("Hanging Indent"), "Page Down moves to the next marker");
    let before_move = values.borrow().clone();
    fixture.key(Key::ArrowRight, Modifiers::NONE).idle(2);
    assert_ne!(*values.borrow(), before_move, "Right moves the focused marker");
    assert_eq!(focused(&fixture, &labels), Some("Hanging Indent"), "Right keeps the focus on the marker");
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some("After"), "Tab leaves the ruler: its markers and stops are one stop");
}

/// While a column border drag runs, holding Alt writes the widths of the columns between the borders.
#[trial]
fn alt_in_a_column_border_drag_shows_the_column_widths() {
    let values = Rc::new(RefCell::new(RulerValues::default()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        TextRuler::new("Ruler", Bind::bound(&mut *shown.borrow_mut()), 300.0).columns(&[50.0, 150.0, 250.0]).show(ui);
    });
    fixture.settle();
    let border = fixture.by_label("Table Column Width").expect("a column border").center();
    fixture.drag_hold_at(border, Vec2::new(6.0, 0.0)).idle(2);
    let without = fixture.texts().len();
    fixture.hold(Modifiers::ALT).idle(2);
    let with = fixture.texts().len();
    assert!(with > without, "Alt adds the widths to the strip ({without} runs without, {with} with)");
    fixture.hold(Modifiers::NONE).idle(2);
    assert_eq!(fixture.texts().len(), without, "letting Alt go hides them");
    fixture.release_at(border + Vec2::new(6.0, 0.0)).settle();
}

/// The centres of the left stops' sliders, left to right.
fn stop_centres(fixture: &Fixture) -> Vec<egui::Pos2> {
    let mut found: Vec<egui::Pos2> = fixture
        .by_role(Role::SLIDER)
        .into_iter()
        .filter(|target| fixture.tree().name_of(target.node).is_some_and(|name| name.starts_with("Left Tab at")))
        .map(|target| target.center())
        .collect();
    found.sort_by(|a, b| a.x.total_cmp(&b.x));
    found
}

#[trial]
fn a_command_drag_moves_every_stop_and_keeps_their_spacing_within_the_column() {
    let id = egui::Id::unique("ruler");
    let stops = [72.0, 144.0, 216.0].map(|at| TabStop::new(at, TabAlign::Left, TabLeader::None)).to_vec();
    let values = Rc::new(RefCell::new(RulerValues { stops, ..Default::default() }));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        TextRuler::new("Ruler", Bind::bound(&mut *shown.borrow_mut()), 300.0).id(id).show(ui);
    });
    fixture.settle();
    let grabbed = stop_centres(&fixture)[1];
    let by = Vec2::new(600.0, 0.0);
    fixture.hold(Modifiers::COMMAND);
    fixture.drag_hold_at(grabbed, by).idle(2);
    let drag = TextRuler::dragging(fixture.ctx(), id).expect("the drag is running");
    assert_eq!(drag.item, RulerItem::Stop(1), "the grabbed stop keeps its index");
    assert!((drag.position - 228.0).abs() < 0.01, "the highest stop stops at the column width: {}", drag.position);
    fixture.release_at(grabbed + by).settle();
    fixture.hold(Modifiers::NONE).idle(2);
    let moved: Vec<f32> = values.borrow().stops.iter().map(|stop| stop.position).collect();
    assert_eq!(moved.len(), 3);
    for (got, want) in moved.iter().zip([156.0, 228.0, 300.0]) {
        assert!((got - want).abs() < 0.01, "every stop moved by one offset: {moved:?}");
    }
    assert_eq!(TextRuler::dragging(fixture.ctx(), id), None, "no drag after the release");
}

#[trial]
fn dragging_is_none_off_the_ruler_after_escape_and_once_the_ruler_stops_showing() {
    let id = egui::Id::unique("ruler");
    let values = Rc::new(RefCell::new(RulerValues { stops: vec![TabStop::new(72.0, TabAlign::Left, TabLeader::None)], ..Default::default() }));
    let (shown, visible) = (values.clone(), Rc::new(Cell::new(true)));
    let show = visible.clone();
    let mut fixture = Fixture::new(move |ui| {
        if show.get() {
            TextRuler::new("Ruler", Bind::bound(&mut *shown.borrow_mut()), 300.0).id(id).show(ui);
        }
    });
    fixture.settle();
    let from = stop_centres(&fixture)[0];
    fixture.drag_hold_at(from, Vec2::new(20.0, 0.0)).idle(2);
    assert!(TextRuler::dragging(fixture.ctx(), id).is_some(), "a drag on the ruler is reported");
    fixture.frame(vec![egui::Event::PointerMoved(from + Vec2::new(20.0, 200.0))]).idle(1);
    assert_eq!(TextRuler::dragging(fixture.ctx(), id), None, "off the ruler");
    // Back on the ruler before the release: a stop released off the ruler is cleared, and the next drags need it.
    fixture.frame(vec![egui::Event::PointerMoved(from + Vec2::new(20.0, 0.0))]);
    fixture.release_at(from + Vec2::new(20.0, 0.0)).settle();

    let from = stop_centres(&fixture)[0];
    fixture.drag_hold_at(from, Vec2::new(20.0, 0.0)).idle(2);
    assert!(TextRuler::dragging(fixture.ctx(), id).is_some());
    fixture.key(egui::Key::Escape, Modifiers::NONE).idle(2);
    assert_eq!(TextRuler::dragging(fixture.ctx(), id), None, "after Escape");
    fixture.release_at(from + Vec2::new(20.0, 0.0)).settle();

    let from = stop_centres(&fixture)[0];
    fixture.drag_hold_at(from, Vec2::new(20.0, 0.0)).idle(2);
    assert!(TextRuler::dragging(fixture.ctx(), id).is_some());
    visible.set(false);
    fixture.idle(2);
    assert_eq!(TextRuler::dragging(fixture.ctx(), id), None, "once the ruler stops showing");
}

/// The ruler's rect on screen: its start edge is where zero sits when the ruler runs at scale 1 with no corner.
fn ruler_rect(fixture: &Fixture) -> egui::Rect {
    fixture.by_label("Ruler").expect("the ruler is named").rect
}

/// The painted runs that are numbers.
fn numbers(fixture: &Fixture) -> Vec<egui::Rect> {
    fixture.texts().iter().filter(|text| text.text.trim().parse::<f64>().is_ok()).map(|text| text.rect).collect()
}

/// A gap edge is a handle on each side of the gap: the end edge of the column before it and the start edge of the one
/// after. Dragging one is one gesture; Alt turns the snap off; Escape reports a cancel and no commit.
#[trial]
fn a_gap_edge_drag_is_one_gesture_snaps_unless_alt_and_escape_cancels() {
    let seen: Rc<RefCell<Vec<GapDrag>>> = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let values = Rc::new(RefCell::new(RulerValues::default()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        let out = TextRuler::new("Ruler", Bind::bound(&mut *shown.borrow_mut()), 300.0)
            .unit(DocUnit::new(UnitKind::Inches, 96.0))
            .scale(1.0)
            .gaps(&[(100.0, 140.0)])
            .show(ui);
        if let Some(gap) = out.gap {
            log.borrow_mut().push(gap);
        }
    });
    fixture.settle();
    let ruler = ruler_rect(&fixture);
    let y = ruler.center().y;
    let end_edge = egui::pos2(ruler.min.x + 100.0, y);
    let start_edge = egui::pos2(ruler.min.x + 140.0, y);

    fixture.drag_at(end_edge, Vec2::new(12.3, 0.0));
    let phases: Vec<GesturePhase> = seen.borrow().iter().map(|gap| gap.phase).collect();
    assert_eq!(phases.first(), Some(&GesturePhase::Begin), "the drag opens with Begin: {phases:?}");
    assert_eq!(phases.last(), Some(&GesturePhase::Commit), "the drag closes with Commit: {phases:?}");
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Commit).count(), 1, "one drag is one gesture");
    let snapped = *seen.borrow().last().expect("a report");
    assert_eq!((snapped.index, snapped.edge), (0, GapEdge::End), "the end edge of the column before the gap moved");
    // Word's inch ruler snaps to sixteenths: 72 / 16 points.
    const SIXTEENTH: f32 = 72.0 / 16.0;
    let steps = snapped.position / SIXTEENTH;
    assert!((steps - steps.round()).abs() < 1e-3, "without Alt the edge snaps to the ruler's step: {}", snapped.position);
    assert!((snapped.position - 112.3).abs() > 0.01, "the snap moved the edge off the raw pointer position");

    seen.borrow_mut().clear();
    fixture.hold(Modifiers::ALT);
    fixture.drag_at(end_edge, Vec2::new(12.3, 0.0));
    fixture.hold(Modifiers::NONE);
    let free = *seen.borrow().last().expect("a report");
    assert!((free.position - 112.3).abs() < 0.01, "Alt drags freely: {}", free.position);

    seen.borrow_mut().clear();
    fixture.drag_at(start_edge, Vec2::new(-12.3, 0.0));
    let other = *seen.borrow().last().expect("a report");
    assert_eq!((other.index, other.edge), (0, GapEdge::Start), "the start edge of the column after the gap moves too");
    assert!(other.position < 140.0, "dragging left moves the start edge left: {}", other.position);

    seen.borrow_mut().clear();
    fixture.click_at(end_edge);
    assert!(seen.borrow().is_empty(), "a click with no drag reports nothing");

    seen.borrow_mut().clear();
    fixture.drag_hold_at(end_edge, Vec2::new(12.3, 0.0));
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.release_at(end_edge + Vec2::new(12.3, 0.0));
    let phases: Vec<GesturePhase> = seen.borrow().iter().map(|gap| gap.phase).collect();
    assert!(phases.contains(&GesturePhase::Cancel), "Escape reports a cancel: {phases:?}");
    assert!(!phases.contains(&GesturePhase::Commit), "a cancelled drag does not commit: {phases:?}");
}

/// Numbers count from the column's start edge: none in the margins, fewer when zoomed out, mirrored in a
/// right-to-left paragraph, and none under a marker or a stop's glyph.
#[trial]
fn ruler_numbers_skip_margins_and_markers_thin_out_mirror_for_rtl_and_keep_clear_of_stop_glyphs() {
    const ORIGIN: f32 = 50.0;
    const WIDTH: f32 = 200.0;
    let show = |scale: f32, rtl: bool| {
        let values = Rc::new(RefCell::new(RulerValues {
            stops: vec![TabStop::new(60.0, TabAlign::Center, TabLeader::None), TabStop::new(130.0, TabAlign::Decimal, TabLeader::None)],
            left_indent: 20.0,
            ..Default::default()
        }));
        let mut fixture = Fixture::new(move |ui| {
            let stage = craft_ui::testing::stage(ui);
            let (strip, _, _) = Rulers::strips(ui, stage);
            let origin = if rtl { strip.max.x - ORIGIN * scale } else { strip.min.x + ORIGIN * scale };
            TextRuler::new("Ruler", Bind::bound(&mut *values.borrow_mut()), WIDTH).origin(origin).scale(scale).right_to_left(rtl).show_at(ui, strip);
        });
        fixture.settle();
        fixture
    };
    let fixture = show(2.0, false);
    let ruler = ruler_rect(&fixture);
    let found = numbers(&fixture);
    assert!(!found.is_empty(), "the ruler writes numbers");
    for number in &found {
        assert!(number.min.x >= ruler.min.x + ORIGIN * 2.0 - 0.5, "no number in the left margin: {number:?}");
        assert!(number.min.x <= ruler.min.x + (ORIGIN + WIDTH) * 2.0, "no number in the right margin: {number:?}");
    }
    let keep_clear: Vec<egui::Rect> = fixture.by_role(craft_ui::contract::Role::SLIDER).into_iter().map(|target| target.rect).collect();
    assert!(keep_clear.len() >= 6, "the markers and both stops are there ({})", keep_clear.len());
    for number in &found {
        assert!(!keep_clear.iter().any(|clear| clear.intersects(*number)), "a number overlaps a marker or stop glyph: {number:?}");
    }

    let thin = numbers(&show(0.5, false)).len();
    let wide = numbers(&show(2.0, false)).len();
    assert!(thin < wide, "zoomed out shows fewer numbers ({thin} against {wide})");

    let mirrored = show(2.0, true);
    let ruler = ruler_rect(&mirrored);
    let found = numbers(&mirrored);
    assert!(!found.is_empty(), "a right-to-left ruler writes numbers");
    for number in &found {
        assert!(number.max.x <= ruler.max.x - ORIGIN * 2.0 + 0.5, "numbers run from the right edge, past the margin: {number:?}");
        // A number stands beside its tick, which lies over the column: it reaches at most its own width past the end.
        assert!(number.max.x + number.width() >= ruler.max.x - (ORIGIN + WIDTH) * 2.0, "numbers stay over the column: {number:?}");
    }
}

/// A ruler cut to an extent (a page's x range) paints its ticks over that extent only: the tick rule's labelled step
/// beyond each end stays off the strip, as nothing of the ruler shows past the page.
#[trial]
fn a_ruler_cut_to_an_extent_paints_no_tick_past_it() {
    const FROM: f32 = 100.0;
    const TO: f32 = 300.0;
    let values = Rc::new(RefCell::new(RulerValues::default()));
    let strip_at: Rc<RefCell<Option<egui::Rect>>> = Rc::new(RefCell::new(None));
    let seen = strip_at.clone();
    let mut fixture = Fixture::new(move |ui| {
        let stage = craft_ui::testing::stage(ui);
        let (strip, _, _) = Rulers::strips(ui, stage);
        *seen.borrow_mut() = Some(strip);
        TextRuler::new("Ruler", Bind::bound(&mut *values.borrow_mut()), 140.0)
            .unit(DocUnit::new(UnitKind::Inches, 96.0))
            .origin(strip.min.x + 130.0)
            .extent(egui::Rangef::new(strip.min.x + FROM, strip.min.x + TO))
            .scale(1.0)
            .show_at(ui, strip);
    });
    fixture.settle();
    let shapes = fixture.frame_shapes();
    let strip = strip_at.borrow().expect("the ruler showed");
    let (from, to) = (strip.min.x + FROM - 0.5, strip.min.x + TO + 0.5);
    let ticks: Vec<egui::Rect> = shapes
        .iter()
        .map(egui::Shape::visual_bounding_rect)
        .filter(|rect| rect.is_finite() && rect.width() <= 2.0 && rect.height() > 0.0)
        .filter(|rect| rect.min.y >= strip.min.y - 0.5 && rect.max.y <= strip.max.y + 0.5)
        .collect();
    assert!(ticks.iter().any(|rect| rect.center().x > from && rect.center().x < to), "the ruler paints ticks over its extent");
    let past: Vec<&egui::Rect> = ticks.iter().filter(|rect| rect.center().x < from || rect.center().x > to).collect();
    assert!(past.is_empty(), "no tick past the extent {from}..{to}: {past:?}");
}

/// The ruler's column (the text's width) is lighter than its margins, as a word processor's ruler reads: the column
/// takes the light well colour and the margins the shade of the desk round the page.
#[trial]
fn the_ruler_column_is_lighter_than_its_margins() {
    const FROM: f32 = 100.0;
    const TO: f32 = 300.0;
    const ORIGIN: f32 = 130.0;
    const WIDTH: f32 = 140.0;
    let values = Rc::new(RefCell::new(RulerValues::default()));
    let strip_at: Rc<RefCell<Option<egui::Rect>>> = Rc::new(RefCell::new(None));
    let seen = strip_at.clone();
    let mut fixture = Fixture::new(move |ui| {
        let stage = craft_ui::testing::stage(ui);
        let (strip, _, _) = Rulers::strips(ui, stage);
        *seen.borrow_mut() = Some(strip);
        TextRuler::new("Ruler", Bind::bound(&mut *values.borrow_mut()), WIDTH)
            .origin(strip.min.x + ORIGIN)
            .extent(egui::Rangef::new(strip.min.x + FROM, strip.min.x + TO))
            .scale(1.0)
            .show_at(ui, strip);
    });
    fixture.settle();
    let shapes = fixture.frame_shapes();
    let strip = strip_at.borrow().expect("the ruler showed");
    // The fill of the filled rect that spans the strip's depth over exactly `from..to` (strip x).
    let fill_over = |from: f32, to: f32| {
        shapes.iter().find_map(|shape| match shape {
            egui::Shape::Rect(r)
                if (r.rect.min.x - (strip.min.x + from)).abs() < 0.6
                    && (r.rect.max.x - (strip.min.x + to)).abs() < 0.6
                    && (r.rect.height() - strip.height()).abs() < 0.6 =>
            {
                Some(r.fill)
            }
            _ => None,
        })
    };
    let luma = |c: egui::Color32| 0.2126 * f32::from(c.r()) + 0.7152 * f32::from(c.g()) + 0.0722 * f32::from(c.b());
    let column = fill_over(ORIGIN, ORIGIN + WIDTH).expect("the column is filled");
    let margins = fill_over(FROM, TO).expect("the ruler's extent is filled with the margin shade");
    assert!(luma(column) > luma(margins), "the column {column:?} is lighter than the margins {margins:?}");
}

/// Only the primary button starts a stop drag; Escape cancels it; a release well away from the strip clears the stop and
/// one near the strip keeps it.
#[trial]
fn a_tab_stop_gesture_is_primary_only_cancellable_and_a_drag_off_the_ruler_clears_it() {
    let values = Rc::new(RefCell::new(RulerValues { stops: vec![TabStop::new(100.0, TabAlign::Left, TabLeader::None)], ..Default::default() }));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        TextRuler::new("Ruler", Bind::bound(&mut *shown.borrow_mut()), 300.0).scale(1.0).show(ui);
    });
    fixture.settle();
    let stop = |fixture: &Fixture| {
        let at = ruler_rect(fixture).min.x + values.borrow().stops.first().map_or(100.0, |stop| stop.position);
        fixture
            .by_role(craft_ui::contract::Role::SLIDER)
            .into_iter()
            .map(|target| target.center())
            .min_by(|a, b| (a.x - at).abs().total_cmp(&(b.x - at).abs()))
            .expect("a stop")
    };
    let start = stop(&fixture);
    let first = values.borrow().clone();

    let by = Vec2::new(20.0, 0.0);
    fixture.frame(vec![pointer::moved(start)]);
    fixture.frame(vec![pointer::secondary(start, true)]);
    fixture.frame(vec![pointer::moved(start + by)]);
    fixture.frame(vec![pointer::moved(start + by * 2.0)]);
    fixture.frame(vec![pointer::secondary(start + by * 2.0, false)]);
    fixture.idle(2);
    assert_eq!(*values.borrow(), first, "a secondary drag does not move the stop");

    fixture.drag_hold_at(start, by);
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.release_at(start + by);
    assert_eq!(*values.borrow(), first, "Escape puts the stop back");

    fixture.drag_at(start, Vec2::new(24.0, 6.0));
    let near = values.borrow().clone();
    assert_eq!(near.stops.len(), 1, "a release near the strip keeps the stop");
    assert_ne!(near, first, "and the stop moved");

    let at = stop(&fixture);
    fixture.drag_at(at, Vec2::new(0.0, 80.0));
    assert!(values.borrow().stops.is_empty(), "a release well away from the strip clears the stop");
}

/// What one frame of a page-wide ruler painted: its strip, its tick marks and its numbers.
struct Painted {
    strip: egui::Rect,
    ticks: Vec<egui::Rect>,
    numbers: Vec<egui::Rect>,
}

/// The tick marks among `shapes`: thin upright lines inside `strip`.
fn tick_marks(shapes: &[egui::Shape], strip: egui::Rect) -> Vec<egui::Rect> {
    shapes
        .iter()
        .map(egui::Shape::visual_bounding_rect)
        .filter(|rect| rect.is_finite() && rect.width() <= 2.0 && rect.height() > 0.0)
        .filter(|rect| strip.expand(0.5).contains_rect(*rect))
        .collect()
}

/// A page-wide ruler in inches, shown with `face` when given, in a fixture whose root takes `set`; `popup` shows it in
/// an open popover inside a scope that sets the productivity set.
fn page_ruler(set: ComponentSet, face: Option<RulerFace>, popup: bool) -> (Fixture, Rc<RefCell<Option<egui::Rect>>>) {
    let values = Rc::new(RefCell::new(RulerValues::default()));
    let strip_at: Rc<RefCell<Option<egui::Rect>>> = Rc::new(RefCell::new(None));
    let seen = strip_at.clone();
    let mut fixture = Fixture::new(move |ui| {
        let ruler = |ui: &mut egui::Ui| {
            let stage = craft_ui::testing::stage(ui);
            let (strip, _, _) = Rulers::strips(ui, stage);
            *seen.borrow_mut() = Some(strip);
            let mut values = values.borrow_mut();
            let mut ruler = TextRuler::new("Ruler", Bind::bound(&mut *values), 432.0)
                .unit(DocUnit::new(UnitKind::Inches, 96.0))
                .origin(strip.min.x + 72.0)
                .extent(egui::Rangef::new(strip.min.x, strip.min.x + 576.0))
                .scale(1.0);
            if let Some(face) = face {
                ruler = ruler.face(face);
            }
            ruler.show_at(ui, strip);
        };
        if popup {
            with_env(
                ui,
                |env| {
                    env.values.set(ComponentSet::Productivity);
                },
                |ui| {
                    Popover::new("Card", Bind::given(true)).show(ui, |ui, _| ruler(ui));
                },
            );
        } else {
            ruler(ui);
        }
    });
    fixture.set(set);
    fixture.settle();
    (fixture, strip_at)
}

/// The latest paint of `fixture`.
fn painted(fixture: &mut Fixture, strip_at: &Rc<RefCell<Option<egui::Rect>>>) -> Painted {
    let shapes = fixture.frame_shapes();
    let strip = strip_at.borrow().expect("the ruler showed");
    Painted { strip, ticks: tick_marks(&shapes, strip), numbers: numbers(fixture) }
}

/// Whether the ruler paints the centred face: every tick is centred on the strip's middle line and clear of its edges,
/// and every number is centred on that line with no tick under it.
fn centred(p: &Painted) -> bool {
    let middle = p.strip.center().y;
    !p.ticks.is_empty()
        && !p.numbers.is_empty()
        && p.ticks.iter().all(|tick| (tick.center().y - middle).abs() < 1.0 && tick.max.y < p.strip.max.y - 1.0)
        && p.numbers.iter().all(|number| (number.center().y - middle).abs() < 1.0)
        && p.numbers.iter().all(|number| !p.ticks.iter().any(|tick| number.x_range().contains(tick.center().x)))
}

/// Whether the ruler paints the edge face: every tick rises from the strip's bottom edge.
fn edge(p: &Painted) -> bool {
    !p.ticks.is_empty() && p.ticks.iter().all(|tick| (tick.max.y - p.strip.max.y).abs() < 1.0)
}

/// With no face given, the component set picks it: the productivity set's ruler centres its marks on a bar and puts numbers
/// in place of the whole-unit ticks, the creative set's hangs them from the strip's bottom edge.
#[trial]
fn the_component_set_picks_the_ruler_face() {
    let (mut fixture, strip) = page_ruler(ComponentSet::Productivity, None, false);
    let productivity = painted(&mut fixture, &strip);
    assert!(
        centred(&productivity),
        "the productivity set paints the centred face: ticks {:?}, numbers {:?}",
        productivity.ticks,
        productivity.numbers
    );
    let (mut fixture, strip) = page_ruler(ComponentSet::Creative, None, false);
    let creative = painted(&mut fixture, &strip);
    assert!(edge(&creative), "the creative set paints the edge face: ticks {:?}", creative.ticks);
}

/// A face given on the ruler wins over the component set.
#[trial]
fn a_given_face_wins_over_the_component_set() {
    let (mut fixture, strip) = page_ruler(ComponentSet::Productivity, Some(RulerFace::Edge), false);
    let given_edge = painted(&mut fixture, &strip);
    assert!(edge(&given_edge), "the edge face under the productivity set: ticks {:?}", given_edge.ticks);
    let (mut fixture, strip) = page_ruler(ComponentSet::Creative, Some(RulerFace::Centred), false);
    let given_centred = painted(&mut fixture, &strip);
    assert!(centred(&given_centred), "the centred face under the creative set: ticks {:?}", given_centred.ticks);
}

/// A scope's component set reaches a popup opened inside it, though the root's set is another.
#[trial]
fn a_scopes_component_set_reaches_a_popup_opened_inside_it() {
    let (mut fixture, strip) = page_ruler(ComponentSet::Creative, None, true);
    let in_popup = painted(&mut fixture, &strip);
    assert!(centred(&in_popup), "the popup's ruler takes the scope's productivity set: ticks {:?}", in_popup.ticks);
}

/// A set change restyles the ruler in the frame it arrives in, in one pass, and the frame after asks for nothing more.
#[trial]
fn a_component_set_change_restyles_the_ruler_in_one_pass_and_then_rests() {
    let (mut fixture, strip) = page_ruler(ComponentSet::Creative, None, false);
    assert!(edge(&painted(&mut fixture, &strip)));
    fixture.set(ComponentSet::Productivity);
    let changed = painted(&mut fixture, &strip);
    assert!(centred(&changed), "the frame of the change paints the productivity face");
    assert_eq!(fixture.passes(), 1, "the change takes one pass");
    fixture.idle(1);
    assert_eq!(fixture.passes(), 1);
    assert!(!fixture.repaint_requested(), "an idle frame after the change asks for no repaint");
}
