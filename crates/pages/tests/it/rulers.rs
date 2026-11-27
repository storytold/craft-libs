//! The page rulers: ruler numbers keep clear of table column markers, the margin handles stop short of the opposite
//! edge and swap sides on a mirrored even page, and a table row grip drags as one gesture.

use craft_behavior::{EditEvent, GesturePhase};
use craft_pages::setup::Margins;
use craft_pages::ui::fixtures::demo_placed;
use craft_pages::ui::margins::{MIN_TEXT, MarginHandlesWords};
use craft_pages::ui::page_rulers::PageRulersWords;
use craft_pages::ui::{PageEdges, PageRulers, RowDrag, VerticalRuler};
use craft_text::ui::DocUnit;
use craft_text::ui::ruler::{RulerValues, TabAlign, TabLeader, TabStop, TextRulerWords};
use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::runtime::env::{Direction, UnitKind};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Rangef, Rect, pos2, vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Where the page's top-left corner sits in the fixture, points.
const PAGE_AT: (f32, f32) = (80.0, 60.0);
/// The table's column borders, points from the column's start edge.
const BORDERS: [f32; 4] = [0.0, 156.0, 312.0, 468.0];

/// The painted runs that are numbers.
fn numbers(fixture: &Fixture) -> Vec<Rect> {
    fixture.texts().iter().filter(|text| text.text.trim().parse::<f64>().is_ok()).map(|text| text.rect).collect()
}

/// The ruler numbers sit clear of every table column marker, which take their places on the strip.
#[trial]
fn ruler_numbers_give_way_to_table_markers() {
    let column_name = TextRulerWords::default().table_column;
    let mut fixture = Fixture::new(|ui| {
        let full = craft_ui::testing::stage(ui);
        let page = demo_placed(1, pos2(full.min.x + PAGE_AT.0, full.min.y + PAGE_AT.1)).remove(0);
        let (mut edges, mut values) = (PageEdges::new(Margins::uniform(72.0)), RulerValues::default());
        PageRulers::new("Rulers", &page, Bind::bound(&mut edges), Bind::bound(&mut values))
            .unit(DocUnit::new(UnitKind::Inches, 96.0))
            .columns(&BORDERS)
            .show_at(ui, full);
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let found = numbers(&fixture);
    assert!(!found.is_empty(), "the ruler writes numbers");
    let markers: Vec<Rect> = fixture
        .by_role(Role::SPLITTER)
        .into_iter()
        .filter(|target| craft_ui::testing::access::name(fixture.tree(), target.node) == Some(column_name.as_str()))
        .map(|target| target.rect)
        .collect();
    assert_eq!(markers.len(), BORDERS.len(), "every table column border has its marker on the ruler");
    for number in &found {
        assert!(!markers.iter().any(|marker| marker.intersects(*number)), "a number overlaps a table column marker: {number:?}");
    }
}

/// A fixture over a page whose edges are `start`, the margin handles on a mirrored even page or not.
fn margin_rig(start: PageEdges, mirrored_even: bool) -> (Fixture, Rc<RefCell<PageEdges>>) {
    let edges = Rc::new(RefCell::new(start));
    let shown = edges.clone();
    let mut fixture = Fixture::new(move |ui| {
        let full = craft_ui::testing::stage(ui);
        let page = demo_placed(1, pos2(full.min.x + PAGE_AT.0, full.min.y + PAGE_AT.1)).remove(0);
        let mut values = RulerValues::default();
        // The column sits clear of the margin edges, so no marker lies on the edge being grabbed.
        PageRulers::new("Rulers", &page, Bind::bound(&mut *shown.borrow_mut()), Bind::bound(&mut values))
            .column((200.0, 400.0))
            .mirrored_even(mirrored_even)
            .show_at(ui, full);
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    (fixture, edges)
}

#[trial]
fn a_margin_drag_is_clamped_by_the_opposite_edge_and_a_mirrored_even_page_swaps_the_sides() {
    let start = PageEdges::new(Margins { top: 72.0, bottom: 72.0, left: 72.0, right: 90.0 });
    let least = MIN_TEXT;
    let page_width = f64::from(demo_placed(1, pos2(0.0, 0.0)).remove(0).rect.width());
    let left_name = MarginHandlesWords::default().left;
    let (mut fixture, edges) = margin_rig(start, false);
    let left = fixture.by_label(left_name.as_str()).expect("the left margin edge");
    fixture.drag(&left, vec2(900.0, 0.0));
    let after = *edges.borrow();
    assert!(after.margins.left > 72.0, "the left edge moved right: {}", after.margins.left);
    assert!(page_width - after.margins.left - after.margins.right >= least, "the edge stops short of the least text width: {after:?}");
    assert_eq!(after.margins.right, 90.0, "the opposite edge is not touched");

    let (mut fixture, edges) = margin_rig(start, true);
    let left = fixture.by_label(left_name.as_str()).expect("the left margin edge");
    fixture.drag(&left, vec2(40.0, 0.0));
    let after = *edges.borrow();
    assert_eq!(after.margins.left, 72.0, "on a mirrored even page the visible left edge is not the stored left margin");
    assert!(after.margins.right > 90.0, "it moves the stored right margin: {}", after.margins.right);
}

/// The texts that show once the pointer rests at `at` for the tooltip delay.
fn texts_added_by_rest(fixture: &mut Fixture, at: egui::Pos2) -> Vec<String> {
    let before: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).collect();
    fixture.frame(vec![craft_ui::testing::pointer::moved(at)]);
    fixture.advance(craft_ui::contract::tooltip::DELAY);
    fixture.settle();
    fixture.texts().iter().map(|run| run.text.clone()).filter(|text| !before.contains(text)).collect()
}

/// The margin handles' rect is the whole canvas, but only an edge shows a tooltip, naming that edge (Word's "Left
/// Margin"); the pointer resting on the page shows none, not the handles' own label.
#[trial]
fn only_a_margin_edge_shows_a_tooltip_and_it_names_the_edge() {
    let start = PageEdges::new(Margins::uniform(72.0));
    let left_name = MarginHandlesWords::default().left;
    let (mut fixture, _) = margin_rig(start, false);
    let left = fixture.by_label(left_name.as_str()).expect("the left margin edge").rect;
    // A point on the page, inside the stage (the page runs past its bottom).
    let page = demo_placed(1, pos2(PAGE_AT.0, PAGE_AT.1)).remove(0).rect;
    let on_page = pos2(page.center().x, fixture.shown().center().y);
    assert!(page.contains(on_page), "the point is on the page");
    let added = texts_added_by_rest(&mut fixture, on_page);
    assert!(added.is_empty(), "the pointer on the page shows no tooltip: {added:?}");
    let on_edge = texts_added_by_rest(&mut fixture, left.center());
    assert_eq!(on_edge, [left_name.as_str()], "the pointer on the left edge shows its name");
}

/// A row grip drag is one gesture; Alt reports free; Escape cancels; the arrow nudges the focused grip.
#[trial]
fn a_row_grip_drag_reports_one_gesture_with_alt_free_and_the_arrow_nudges() {
    let seen: Rc<RefCell<Vec<RowDrag>>> = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let full = craft_ui::testing::stage(ui);
        let page = demo_placed(1, pos2(full.min.x + 40.0, full.min.y + 30.0)).remove(0);
        let strip = Rect::from_min_size(full.min, vec2(40.0, full.height()));
        let out = VerticalRuler::new("Vertical", &page)
            .text(Rangef::new(72.0, 720.0))
            .unit(DocUnit::new(UnitKind::Inches, 96.0))
            .rows(&[100.0, 150.0])
            .show_at(ui, strip);
        if let Some(row) = out.row {
            log.borrow_mut().push(row);
        }
    });
    fixture.room(vec2(400.0, 600.0));
    fixture.settle();
    let grip = fixture.by_label(PageRulersWords::default().row_border.as_str()).expect("a row grip");

    fixture.drag(&grip, vec2(0.0, 30.0));
    let phases: Vec<GesturePhase> = seen.borrow().iter().map(|row| row.phase).collect();
    assert_eq!(phases.first(), Some(&GesturePhase::Begin), "the drag opens with Begin: {phases:?}");
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Commit).count(), 1, "one drag is one Commit: {phases:?}");
    let last = *seen.borrow().last().expect("a report");
    assert_eq!((last.phase, last.index), (GesturePhase::Commit, 0));
    assert!(last.position > 100.0, "dragging down moves the border down: {}", last.position);
    assert!(!last.mods.alt, "no Alt, no free flag");

    seen.borrow_mut().clear();
    fixture.hold(Modifiers::ALT);
    fixture.drag(&grip, vec2(0.0, 30.0));
    fixture.hold(Modifiers::NONE);
    assert!(seen.borrow().iter().any(|row| row.mods.alt), "Alt reports the drag as free");

    seen.borrow_mut().clear();
    fixture.drag_hold_at(grip.center(), vec2(0.0, 30.0));
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.release_at(grip.center() + vec2(0.0, 30.0));
    let phases: Vec<GesturePhase> = seen.borrow().iter().map(|row| row.phase).collect();
    assert!(phases.contains(&GesturePhase::Cancel), "Escape cancels: {phases:?}");
    assert!(!phases.contains(&GesturePhase::Commit), "a cancelled drag does not commit: {phases:?}");

    seen.borrow_mut().clear();
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    fixture.key(Key::ArrowDown, Modifiers::NONE).idle(2);
    let nudge = *seen.borrow().last().expect("the arrow nudges the focused grip");
    assert_eq!(nudge.phase, GesturePhase::Commit, "a nudge is a lone Commit");
    assert!(nudge.position > 100.0, "Down moves the border down: {}", nudge.position);
}

/// Where the trial's left tab stop sits, points from the column's start edge.
const STOP_AT: f32 = 144.0;

/// Guards `PageRulers::id_salt`: the caret paragraph's key salts the text ruler, so a new paragraph is a new ruler and
/// a stop drag open on the old one commits nothing when its button comes up. The first drag, with the salt kept, is the
/// control: it commits once.
#[trial]
fn a_new_salt_during_a_tab_drag_commits_nothing() {
    let salt = Rc::new(Cell::new(1u32));
    let commits = Rc::new(Cell::new(0usize));
    let mut start = RulerValues::default();
    start.stops.push(TabStop::new(STOP_AT, TabAlign::Left, TabLeader::None));
    let values = Rc::new(RefCell::new(start));
    let (shown_salt, seen, shown_values) = (Rc::clone(&salt), Rc::clone(&commits), Rc::clone(&values));
    let mut fixture = Fixture::new(move |ui| {
        let full = craft_ui::testing::stage(ui);
        let page = demo_placed(1, pos2(full.min.x + PAGE_AT.0, full.min.y + PAGE_AT.1)).remove(0);
        let mut edges = PageEdges::new(Margins::uniform(72.0));
        let out = PageRulers::new("Rulers", &page, Bind::bound(&mut edges), Bind::bound(&mut *shown_values.borrow_mut()))
            .id_salt(shown_salt.get())
            .show_at(ui, full);
        let committed = out.ruler.edit.events.iter().filter(|event| matches!(event, EditEvent::Commit { .. })).count();
        seen.set(seen.get() + committed);
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let left_tab = TextRulerWords::default().left_tab;
    let stop = |fixture: &Fixture| {
        fixture
            .by_role(Role::SLIDER)
            .into_iter()
            .find(|target| craft_ui::testing::access::name(fixture.tree(), target.node).is_some_and(|name| name.starts_with(left_tab.as_str())))
            .expect("the ruler shows the stop")
            .rect
            .center()
    };
    let by = vec2(40.0, 0.0);

    let at = stop(&fixture);
    fixture.drag_hold_at(at, by);
    fixture.release_at(at + by);
    fixture.settle();
    assert_eq!(commits.get(), 1, "a stop drag on one paragraph's ruler commits once");

    let at = stop(&fixture);
    fixture.drag_hold_at(at, by);
    salt.set(2);
    fixture.idle(1);
    fixture.release_at(at + by);
    fixture.settle();
    assert_eq!(commits.get(), 1, "the drag open when the paragraph changed commits nothing");
}

/// The indent markers move into the page's margins as negative indents, in either paragraph direction: the left marker's
/// accessible range reaches the page's edge, the arrows move the first-line marker past the column's start edge, the left
/// marker drags into the start margin and stops at the page's edge, and so does the right marker at the other edge. A tab
/// stop dragged into the margin stays in the column.
#[trial]
fn indent_markers_move_into_the_margins_up_to_the_page_edges() {
    const MARGIN: f32 = 72.0;
    const INDENT: f32 = 36.0;
    let words = TextRulerWords::default();
    for rtl in [false, true] {
        // The left and right markers start inside the column, clear of the margin edges' handles; the first line hangs
        // back to the column's start edge.
        let mut start = RulerValues::default();
        start.stops.push(TabStop::new(STOP_AT, TabAlign::Left, TabLeader::None));
        (start.left_indent, start.first_line_indent, start.right_indent) = (INDENT, -INDENT, INDENT);
        let values = Rc::new(RefCell::new(start));
        let shown = Rc::clone(&values);
        let mut fixture = Fixture::new(move |ui| {
            let full = craft_ui::testing::stage(ui);
            let page = demo_placed(1, pos2(full.min.x + PAGE_AT.0, full.min.y + PAGE_AT.1)).remove(0);
            let mut edges = PageEdges::new(Margins::uniform(f64::from(MARGIN)));
            PageRulers::new("Rulers", &page, Bind::bound(&mut edges), Bind::bound(&mut *shown.borrow_mut()))
                .right_to_left(Some(rtl))
                .show_at(ui, full);
        });
        fixture.room(vec2(900.0, 600.0));
        fixture.settle();
        // The start margin is on the left, or on the right of a right-to-left paragraph; the page is at scale 1.
        let (outward, toward_start) = if rtl { (1.0, Key::ArrowRight) } else { (-1.0, Key::ArrowLeft) };
        let at = |fixture: &Fixture, name: &str| fixture.by_label(name).expect(name).rect.center();
        // A screen reader is given the same reach: the hanging marker's range starts at the page's edge, the box's where
        // the hanging first line it carries would reach that edge.
        let range = |fixture: &Fixture, name: &str| fixture.tree().numeric_of(fixture.by_label(name).expect(name).node).expect(name).range;
        let (hanging, left) = (range(&fixture, words.hanging_indent.as_str()), range(&fixture, words.left_indent.as_str()));
        assert!((hanging.start() + f64::from(MARGIN)).abs() < 0.01, "rtl {rtl}: the hanging marker's range starts at the page's edge: {hanging:?}");
        let floor = -f64::from(MARGIN - INDENT);
        assert!((left.start() - floor).abs() < 0.01, "rtl {rtl}: the box's range stops where its first line meets the page's edge: {left:?}");

        fixture.key(Key::Tab, Modifiers::NONE).idle(2);
        fixture.key(toward_start, Modifiers::NONE).idle(2);
        let first = values.borrow().left_indent + values.borrow().first_line_indent;
        assert!(first < 0.0, "rtl {rtl}: the arrow moves the first line into the start margin: {first}");

        let left = at(&fixture, words.left_indent.as_str());
        fixture.drag_at(left, vec2(2.0 * INDENT * outward, 0.0));
        let dragged = values.borrow().left_indent;
        assert!(dragged < 0.0, "rtl {rtl}: the left indent drags into the start margin: {dragged}");
        // The box carries the hanging first line, so it stops where that line reaches the page's edge.
        let left = at(&fixture, words.left_indent.as_str());
        fixture.drag_at(left, vec2(300.0 * outward, 0.0));
        let first = values.borrow().left_indent + values.borrow().first_line_indent;
        assert!((first + MARGIN).abs() < 0.01, "rtl {rtl}: the box stops with the first line at the page's edge: {first}");
        // The hanging marker moves the left indent alone, on to the page's edge, where it stops.
        let hanging = at(&fixture, words.hanging_indent.as_str());
        fixture.drag_at(hanging, vec2(300.0 * outward, 0.0));
        let (farthest, first) = (values.borrow().left_indent, values.borrow().left_indent + values.borrow().first_line_indent);
        assert!((farthest + MARGIN).abs() < 0.01, "rtl {rtl}: the left indent stops at the page's edge: {farthest}");
        assert!(first >= -MARGIN - 0.01, "rtl {rtl}: the first line stays on the page: {first}");

        let right = at(&fixture, words.right_indent.as_str());
        fixture.drag_at(right, vec2(-300.0 * outward, 0.0));
        let farthest = values.borrow().right_indent;
        assert!((farthest + MARGIN).abs() < 0.01, "rtl {rtl}: the right indent stops at the page's other edge: {farthest}");

        let stop = fixture
            .by_role(Role::SLIDER)
            .into_iter()
            .find(|target| craft_ui::testing::access::name(fixture.tree(), target.node).is_some_and(|name| name.starts_with(words.left_tab.as_str())))
            .expect("the ruler shows the stop")
            .rect
            .center();
        fixture.drag_at(stop, vec2(300.0 * outward, 0.0));
        let stops: Vec<f32> = values.borrow().stops.iter().map(|stop| stop.position).collect();
        assert!(stops.len() == 1 && stops[0] < STOP_AT && stops[0] >= 0.0, "rtl {rtl}: the stop moved and stays in the column: {stops:?}");
    }
}

/// The tab selector shows in the canvas's corner, where the top and left strips meet, in either interface direction:
/// the host places it, so it never lies over the horizontal ruler.
#[trial]
fn the_tab_selector_sits_in_the_canvas_corner_in_both_directions() {
    let words = TextRulerWords::default();
    let name = words.tab_selector.fill(&[("kind", words.left_tab.as_str())]);
    for direction in [Direction::Ltr, Direction::Rtl] {
        let corner = Rc::new(Cell::new(Rect::NOTHING));
        let seen = corner.clone();
        let mut fixture = Fixture::new(move |ui| {
            let full = craft_ui::testing::stage(ui);
            seen.set(craft_ui::canvas::Rulers::strips(ui, full).2);
            let page = demo_placed(1, pos2(full.min.x + PAGE_AT.0, full.min.y + PAGE_AT.1)).remove(0);
            let (mut edges, mut values) = (PageEdges::new(Margins::uniform(72.0)), RulerValues::default());
            PageRulers::new("Rulers", &page, Bind::bound(&mut edges), Bind::bound(&mut values)).kind(Some(TabAlign::Left)).show_at(ui, full);
        });
        fixture.direction(direction).room(vec2(900.0, 600.0));
        fixture.settle();
        let selector = fixture.by_label(&name).expect("the tab selector shows").rect;
        assert!(corner.get().contains_rect(selector), "{direction:?}: the selector {selector:?} lies outside the canvas's corner {:?}", corner.get());
    }
}
