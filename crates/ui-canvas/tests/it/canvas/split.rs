//! `SplitViews`: the panes of the arrangements in reading order, the divider lists that do not fit, the dividers'
//! bounds and spans as the pointer drags them, and the document tabs dropped on the views.

use craft_behavior::PartKey;
use craft_ui::canvas::{PanePress, SplitAt, SplitLayout, SplitViews, TabDrop, TabDropTarget};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::runtime::env::Direction;
use craft_ui::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Pos2, Rect, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// The area the geometry rows lay out in, in points.
const FULL: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(600.0, 400.0));

/// The room the drag rows show the split views in: wide enough that a Tile of five keeps `least_pane` on both sides
/// of each column line.
const ROOM: Vec2 = Vec2::new(600.0, 400.0);

/// The slack of a coordinate comparison after f32 maths, in points.
const TOLERANCE: f32 = 0.5;

const PANE: PartKey = PartKey::new("pane");
const DIVIDER: PartKey = PartKey::new("divider");

#[trial]
fn clicking_a_control_in_an_inactive_pane_activates_both() {
    use craft_ui::buttons::Button;
    let seen = Rc::new(RefCell::new((Vec::new(), 0)));
    let held = seen.clone();
    let mut at = SplitAt::default();
    let mut active = 0;
    let mut fixture = Fixture::new(move |ui| {
        let views = SplitViews::new("Views", SplitLayout::TwoUpVertical, Bind::bound(&mut at))
            .active(active)
            .pane_press(PanePress::Interact)
            .show_panes(ui, |ui, _, pane| {
                if pane.index == 1 && Button::new("Pane action").show(ui).clicked() {
                    held.borrow_mut().1 += 1;
                }
            });
        if let Some(pane) = views.output.activated {
            active = pane;
            held.borrow_mut().0.push(pane);
        }
    });
    fixture.room(ROOM).settle();
    let button = fixture.by_label("Pane action").unwrap_or_else(|| panic!("the pane action is shown"));
    fixture.click(&button).settle();
    let seen = seen.borrow();
    assert_eq!(seen.0, [1], "the inactive pane becomes active");
    assert_eq!(seen.1, 1, "the same click operates the control");
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= TOLERANCE
}

/// The panes follow reading order (row by row, then along the row) and no two overlap.
fn in_reading_order_without_overlap(panes: &[Rect], what: &str) {
    for (i, a) in panes.iter().enumerate() {
        for (j, b) in panes.iter().enumerate().skip(i + 1) {
            let overlap = a.intersect(*b);
            assert!(overlap.width() <= TOLERANCE || overlap.height() <= TOLERANCE, "{what}: panes {i} and {j} overlap: {a:?} {b:?}");
            let later = b.center().y > a.bottom() - TOLERANCE || (near(a.top(), b.top()) && b.center().x > a.center().x);
            assert!(later, "{what}: pane {j} comes after pane {i} in reading order: {a:?} {b:?}");
        }
    }
}

fn widths(panes: &[Rect]) -> Vec<f32> {
    panes.iter().map(|pane| pane.width()).collect()
}

fn show(ui: &mut egui::Ui, data: &mut StoryData, layout: SplitLayout, count: usize) -> StoryOutput {
    let (at, at_at) = data.value_at("split", SplitAt::default());
    SplitViews::new("Views", layout, at_at.bind(at)).count(count).show(ui, |_, _| ()).output.into()
}

fn tiled(ui: &mut egui::Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, SplitLayout::Tile, 5)
}

fn three_stacked(ui: &mut egui::Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, SplitLayout::ThreeUpStacked, 3)
}

fn rect_of(fixture: &Fixture, part: PartKey, index: u64) -> Rect {
    let id = fixture.root_id().unwrap_or_else(|| panic!("the split views were shown"));
    fixture.part(id, part.at(index)).unwrap_or_else(|| panic!("part {index} is shown")).rect
}

fn drag_divider(fixture: &mut Fixture, index: u64, by: Vec2) {
    let id = fixture.root_id().unwrap_or_else(|| panic!("the split views were shown"));
    let grip = fixture.part(id, DIVIDER.at(index)).unwrap_or_else(|| panic!("divider {index} is shown"));
    fixture.drag(&grip, by);
}

/// What the tab-drop host reported since the last drag.
struct Seen {
    drops: Vec<TabDrop<usize>>,
    torn: usize,
    views: Rect,
}

// `egui::Rect` has no `Default`; nothing is shown before the first frame.
impl Default for Seen {
    fn default() -> Self {
        Seen { drops: Vec::new(), torn: 0, views: Rect::NOTHING }
    }
}

/// A document strip above split views in `layout` with `count` documents that take every drop but at `refused`.
fn tab_host(layout: SplitLayout, count: usize, refused: Option<TabDropTarget>, direction: Direction) -> (Fixture, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let held = seen.clone();
    let mut at = SplitAt::default();
    let mut fixture = Fixture::new(move |ui| {
        let tabs = [DocumentTab::new(0_usize, "One"), DocumentTab::new(1, "Two")];
        let strip = DocumentTabs::new("Documents", &tabs).selected(0).reorderable(true).tear_off(true).show(ui);
        let views = SplitViews::new("Views", layout, Bind::bound(&mut at))
            .count(count)
            .takes_tabs(move |_: &[usize], target| Some(target) != refused)
            .show(ui, |_, _| ());
        let mut seen = held.borrow_mut();
        seen.torn += strip.events.iter().filter(|event| matches!(event, DocumentTabsEvent::TornOff { .. })).count();
        seen.drops.extend(views.output.tab_drop);
        seen.views = views.output.edit.response.rect;
    });
    fixture.direction(direction).room(ROOM).settle();
    (fixture, seen)
}

/// Drags the tab "One" to the point `across` of the views' width at their middle height and lets it go there: the
/// drops the views reported, as documents and target, and how many tear-offs the strip reported.
fn drop_tab(fixture: &mut Fixture, seen: &RefCell<Seen>, across: f32) -> (Vec<(Vec<usize>, TabDropTarget)>, usize) {
    let tab = fixture.by_label("One").unwrap_or_else(|| panic!("the tab is shown"));
    let views = seen.borrow().views;
    let to = Pos2::new(views.left() + views.width() * across, views.center().y);
    {
        let mut seen = seen.borrow_mut();
        seen.drops.clear();
        seen.torn = 0;
    }
    fixture.drag_at(tab.center(), to - tab.center()).settle();
    let seen = seen.borrow();
    (seen.drops.iter().map(|drop| (drop.documents.clone(), drop.target)).collect(), seen.torn)
}

#[trial]
fn a_tab_dropped_on_the_views_splits_or_moves_to_the_other_pane() {
    // An unsplit view takes the tab on its reading-end half, and the strip does not tear it off there.
    let (mut fixture, seen) = tab_host(SplitLayout::Tile, 1, None, Direction::Ltr);
    let (drops, torn) = drop_tab(&mut fixture, &seen, 0.75);
    assert_eq!(drops, vec![(vec![0], TabDropTarget::SplitEnd)], "a drop on the right half splits the view");
    assert_eq!(torn, 0, "over the views the drop is theirs, not a tear-off");
    let (drops, _) = drop_tab(&mut fixture, &seen, 0.25);
    assert!(drops.is_empty(), "the reading-start half takes nothing: {drops:?}");

    // Under right-to-left the reading end is the left half.
    let (mut fixture, seen) = tab_host(SplitLayout::Tile, 1, None, Direction::Rtl);
    let (drops, torn) = drop_tab(&mut fixture, &seen, 0.25);
    assert_eq!(drops, vec![(vec![0], TabDropTarget::SplitEnd)], "under right-to-left the left half splits the view");
    assert_eq!(torn, 0, "over the views the drop is theirs, not a tear-off");
    let (drops, _) = drop_tab(&mut fixture, &seen, 0.75);
    assert!(drops.is_empty(), "under right-to-left the right half takes nothing: {drops:?}");

    // A split view reports the pane under the pointer, in reading order.
    let (mut fixture, seen) = tab_host(SplitLayout::TwoUpVertical, 2, None, Direction::Ltr);
    let (drops, torn) = drop_tab(&mut fixture, &seen, 0.75);
    assert_eq!(drops, vec![(vec![0], TabDropTarget::Pane(1))], "a drop on the right pane moves the tab there");
    assert_eq!(torn, 0, "over the views the drop is theirs, not a tear-off");
    let (drops, _) = drop_tab(&mut fixture, &seen, 0.25);
    assert_eq!(drops, vec![(vec![0], TabDropTarget::Pane(0))], "a drop on the left pane moves the tab there");

    // A pane the caller's rule refuses takes nothing; the others still do.
    let (mut fixture, seen) = tab_host(SplitLayout::TwoUpVertical, 2, Some(TabDropTarget::Pane(0)), Direction::Ltr);
    let (drops, _) = drop_tab(&mut fixture, &seen, 0.25);
    assert!(drops.is_empty(), "a refused pane takes nothing: {drops:?}");
    let (drops, _) = drop_tab(&mut fixture, &seen, 0.75);
    assert_eq!(drops, vec![(vec![0], TabDropTarget::Pane(1))], "the pane the rule takes still reports the drop");
}

/// Each pane's body shows at its pane as the views lay it out in the pass: mid-drag, the bodies sit where the value the
/// drag gives in that same pass puts the panes, pane 0 at the start edge under either direction.
#[trial]
fn each_pane_body_shows_at_its_pane_in_the_pass_a_divider_moves() {
    use craft_ui::testing::pointer;
    for rtl in [false, true] {
        let seen = Rc::new(RefCell::new((Vec::<(usize, Rect)>::new(), Rect::NOTHING, SplitAt::default(), None::<egui::Id>)));
        let held = seen.clone();
        let mut at = SplitAt::default();
        let mut fixture = Fixture::new(move |ui| {
            let mut bodies = Vec::new();
            let views = SplitViews::new("Views", SplitLayout::TwoUpVertical, Bind::bound(&mut at)).count(2).show_panes(ui, |ui, _, pane| {
                // A body has a measuring run before its real run: keep the last run's panes.
                if pane.index == 0 {
                    bodies.clear();
                }
                bodies.push((pane.index, ui.max_rect()));
            });
            *held.borrow_mut() = (bodies, views.output.edit.response.rect, at.clone(), Some(views.output.edit.response.id));
        });
        fixture.direction(if rtl { Direction::Rtl } else { Direction::Ltr }).room(ROOM).settle();
        let id = seen.borrow().3.unwrap_or_else(|| panic!("the split views were shown"));
        let from = fixture.part(id, DIVIDER.at(0)).unwrap_or_else(|| panic!("the divider is shown")).center();
        fixture.frame(vec![pointer::moved(from)]);
        fixture.frame(vec![pointer::press(from)]);
        for step in 1..=4 {
            fixture.frame(vec![pointer::moved(from + Vec2::new(15.0 * step as f32, 0.0))]);
        }
        let (bodies, full, at, _) = seen.borrow().clone();
        assert_ne!(at, SplitAt::default(), "the drag moved the divider (rtl {rtl})");
        let panes = at.placed(SplitLayout::TwoUpVertical, 2, full, rtl);
        assert_eq!(bodies.iter().map(|(index, _)| *index).collect::<Vec<_>>(), [0, 1], "one body per pane, in reading order");
        for (index, rect) in &bodies {
            let pane = panes[*index];
            assert!(near(rect.left(), pane.left()) && near(rect.right(), pane.right()), "body {index} is its pane this pass: {rect:?} {pane:?}");
        }
        assert_eq!(bodies[0].1.center().x > bodies[1].1.center().x, rtl, "pane 0's body sits at the start edge (rtl {rtl})");
        fixture.frame(vec![pointer::release(from + Vec2::new(60.0, 0.0))]);
    }
}

/// The views fill the room they are given whatever their bodies hold: a pane's body, and `show`'s plain body, are the
/// room's full height pass after pass, not the height of content shorter than it.
#[trial]
fn bodies_shorter_than_the_room_keep_the_full_height_pass_after_pass() {
    use craft_ui::display::Text;
    let seen = Rc::new(RefCell::new((Vec::<Rect>::new(), Rect::NOTHING)));
    let held = seen.clone();
    let mut at = SplitAt::default();
    let mut fixture = Fixture::new(move |ui| {
        let mut bodies = Vec::new();
        let views = SplitViews::new("Views", SplitLayout::TwoUpVertical, Bind::bound(&mut at)).count(2).show_panes(ui, |ui, _, pane| {
            // A body has a measuring run before its real run: keep the last run's panes.
            if pane.index == 0 {
                bodies.clear();
            }
            Text::new("Page").show(ui);
            bodies.push(ui.max_rect());
        });
        *held.borrow_mut() = (bodies, views.output.edit.response.rect);
    });
    fixture.room(ROOM).settle();
    for pass in 0..4 {
        fixture.frame(Vec::new());
        let (bodies, views) = seen.borrow().clone();
        assert!(near(views.height(), ROOM.y), "pass {pass}: the views fill the room's height: {views:?}");
        assert_eq!(bodies.len(), 2, "pass {pass}: one body per pane");
        for (index, body) in bodies.iter().enumerate() {
            assert!(
                near(body.top(), views.top()) && near(body.bottom(), views.bottom()),
                "pass {pass}: body {index} is the full height: {body:?} in {views:?}"
            );
        }
    }

    // A pane holding a view that fills (as a canvas does) gives it the whole pane.
    let seen = Rc::new(RefCell::new((Vec::<Rect>::new(), Vec::<Rect>::new(), Rect::NOTHING)));
    let held = seen.clone();
    let mut at = SplitAt::default();
    let mut inner_at = [SplitAt::default(), SplitAt::default()];
    let mut fixture = Fixture::new(move |ui| {
        let (mut panes, mut inner) = (Vec::new(), Vec::new());
        let views = SplitViews::new("Views", SplitLayout::TwoUpVertical, Bind::bound(&mut at)).count(2).show_panes(ui, |ui, _, pane| {
            if pane.index == 0 {
                panes.clear();
                inner.clear();
            }
            let label = if pane.index == 0 { "Inner 0" } else { "Inner 1" };
            let shown = SplitViews::new(label, SplitLayout::Tile, Bind::bound(&mut inner_at[pane.index])).count(1).show(ui, |ui, _| {
                Text::new("Page").show(ui);
            });
            panes.push(pane.rect);
            inner.push(shown.output.edit.response.rect);
        });
        *held.borrow_mut() = (panes, inner, views.output.edit.response.rect);
    });
    fixture.room(ROOM).settle();
    for pass in 0..4 {
        fixture.frame(Vec::new());
        let (panes, inner, views) = seen.borrow().clone();
        assert!(near(views.height(), ROOM.y), "pass {pass}: the views around filling panes fill the room's height: {views:?}");
        assert_eq!(inner.len(), 2, "pass {pass}: one inner view per pane");
        for (index, (shown, pane)) in inner.iter().zip(&panes).enumerate() {
            assert!(near(pane.height(), views.height()), "pass {pass}: pane {index} is the views' height: {pane:?} in {views:?}");
            assert!(
                near(shown.top(), pane.top()) && near(shown.bottom(), pane.bottom()),
                "pass {pass}: inner view {index} fills its pane: {shown:?} in {pane:?}"
            );
        }
    }

    let seen = Rc::new(RefCell::new((Rect::NOTHING, Rect::NOTHING)));
    let held = seen.clone();
    let mut at = SplitAt::default();
    let mut fixture = Fixture::new(move |ui| {
        let mut body = Rect::NOTHING;
        let views = SplitViews::new("Views", SplitLayout::Tile, Bind::bound(&mut at)).count(1).show(ui, |ui, _| {
            Text::new("Page").show(ui);
            body = ui.max_rect();
        });
        *held.borrow_mut() = (body, views.output.edit.response.rect);
    });
    fixture.room(ROOM).settle();
    for pass in 0..4 {
        fixture.frame(Vec::new());
        let (body, views) = *seen.borrow();
        assert!(near(views.height(), ROOM.y), "pass {pass}: the plain views fill the room's height: {views:?}");
        assert!(near(body.height(), views.height()), "pass {pass}: the plain body is the full height: {body:?} in {views:?}");
    }
}

#[trial]
fn grid_and_stacked_splits_lay_out_and_drag_their_dividers() {
    // Tile with five documents: three columns, two rows, the last row two cells.
    let even = SplitAt::lines(Vec::new(), Vec::new()).rects(SplitLayout::Tile, 5, FULL);
    assert_eq!(even.len(), 5, "five documents tile into five panes");
    in_reading_order_without_overlap(&even, "Tile of 5");
    assert!(even.iter().all(|pane| near(pane.width(), 200.0) && near(pane.height(), 200.0)), "even cells: {even:?}");
    let moved = SplitAt::lines(vec![0.2, 0.7], vec![0.4]).rects(SplitLayout::Tile, 5, FULL);
    in_reading_order_without_overlap(&moved, "Tile of 5, moved lines");
    assert!(near(moved[0].width(), 120.0) && near(moved[1].width(), 300.0) && near(moved[2].width(), 180.0), "{moved:?}");
    assert!(near(moved[3].width(), 120.0) && near(moved[3].height(), 240.0), "the second row follows the lines: {moved:?}");

    // A column list of the wrong length, out of order or outside 0–1 lays out even columns; the row list still applies.
    for columns in [vec![0.2], vec![0.7, 0.2], vec![0.2, 1.5], vec![0.2, f32::NAN]] {
        let panes = SplitAt::lines(columns.clone(), vec![0.4]).rects(SplitLayout::Tile, 5, FULL);
        assert!(widths(&panes).iter().all(|&width| near(width, 200.0)), "{columns:?} lays out even columns: {panes:?}");
        assert!(near(panes[0].height(), 160.0), "{columns:?} keeps the fitting row list: {panes:?}");
    }

    // Stacked documents: one row each, the full width.
    let stacked = SplitAt::lines(Vec::new(), vec![0.2, 0.5]).rects(SplitLayout::TileAllHorizontally, 3, FULL);
    in_reading_order_without_overlap(&stacked, "stacked");
    let heights: Vec<f32> = stacked.iter().map(|pane| pane.height()).collect();
    assert!(near(heights[0], 80.0) && near(heights[1], 120.0) && near(heights[2], 200.0), "{heights:?}");
    assert!(stacked.iter().all(|pane| near(pane.width(), FULL.width())), "{stacked:?}");

    // Two side by side: pane 0 at the start edge, mirrored under right-to-left.
    let two = SplitAt::lines(vec![0.3], Vec::new());
    let ltr = two.placed(SplitLayout::TwoUpVertical, 2, FULL, false);
    in_reading_order_without_overlap(&ltr, "two");
    assert!(near(ltr[0].left(), FULL.left()) && near(ltr[0].width(), 180.0), "{ltr:?}");
    let rtl = two.placed(SplitLayout::TwoUpVertical, 2, FULL, true);
    assert!(near(rtl[0].right(), FULL.right()) && near(rtl[0].width(), 180.0), "pane 0 sits at the right under RTL: {rtl:?}");
    assert!(near(rtl[1].left(), FULL.left()) && near(rtl[1].width(), 420.0), "{rtl:?}");

    // One document shows one pane over the whole area.
    assert_eq!(SplitAt::default().rects(SplitLayout::Tile, 1, FULL), vec![FULL]);

    // A dragged column line stops `least_pane` short of the area's edge and of the next line: the same gap both ways.
    let mut fixture = Fixture::story(&Story::new("tiled", tiled));
    fixture.room(ROOM).settle();
    drag_divider(&mut fixture, 0, Vec2::new(-1000.0, 0.0));
    let gap = rect_of(&fixture, PANE, 0).width();
    assert!(gap > 1.0, "the first pane keeps the least width: {gap}");
    drag_divider(&mut fixture, 0, Vec2::new(1000.0, 0.0));
    let second = rect_of(&fixture, PANE, 1).width();
    assert!(near(second, gap), "the line stops as far short of its neighbour as of the edge: {second} against {gap}");
    let kept = fixture.data().present::<SplitAt>("split").cloned();
    assert_eq!(kept.map(|at| at.columns.len()), Some(2), "a drag writes the whole column list");

    // 3-up Stacked: the row line spans only the second column.
    let mut fixture = Fixture::story(&Story::new("three stacked", three_stacked));
    fixture.room(ROOM).settle();
    let (tall, top) = (rect_of(&fixture, PANE, 0), rect_of(&fixture, PANE, 1));
    let row = rect_of(&fixture, DIVIDER, 1);
    assert!(row.left() >= tall.right() - TOLERANCE, "the row line starts at the second column: {row:?} beside {tall:?}");
    assert!(near(row.right(), top.right()), "the row line reaches the area's end: {row:?} {top:?}");
    drag_divider(&mut fixture, 1, Vec2::new(0.0, 40.0));
    let (tall_after, top_after) = (rect_of(&fixture, PANE, 0), rect_of(&fixture, PANE, 1));
    assert!(top_after.height() > top.height() + 1.0, "dragging the row line down grows the top cell: {top:?} to {top_after:?}");
    assert!(near(tall_after.height(), tall.height()), "the tall pane keeps its height: {tall:?} to {tall_after:?}");
}
