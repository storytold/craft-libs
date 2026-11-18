//! The text ruler's keys: the ruler is one Tab stop, Page Down moves between its markers and stops, and the arrows
//! move the focused one. Alt in a column border drag shows the column widths.

use craft_behavior::GesturePhase;
use craft_text::ui::DocUnit;
use craft_text::ui::ruler::{GapDrag, GapEdge, RulerValues, TabAlign, TabLeader, TabStop, TextRuler};
use craft_ui::buttons::Button;
use craft_ui::canvas::Rulers;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::RefCell;
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
            let (strip, _) = Rulers::strips(ui, stage);
            let origin = if rtl { strip.max.x - ORIGIN * scale } else { strip.min.x + ORIGIN * scale };
            TextRuler::new("Ruler", Bind::bound(&mut *values.borrow_mut()), WIDTH)
                .origin(origin)
                .scale(scale)
                .right_to_left(rtl)
                .show_at(ui, strip);
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
