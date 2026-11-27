//! Floating windows of a `DockArea`: a drop over a window lands on it.

use super::Numbered;
use craft_behavior::Axis;
use craft_behavior::dock::{Action, Column, DockEdge, Floating, Group, Layout, Node, Permissions, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, Role, Show as _, ShowContainer as _, Tracked};
use craft_ui::display::Text;
use craft_ui::docking::{DockConfig, DockPanels, GroupDrag, IconicSizes, request_actions};
use craft_ui::layout_components::Stack;
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::drag::{self, DropMark};
use craft_ui::testing::pointer::{moved, press, release, secondary};
use craft_ui::testing::snapshot::layer_rects;
use craft_ui::testing::{Fixture, trial};
use egui::{Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

const DOCKED: usize = 1;
const WINDOW: usize = 2;
const SECOND: usize = 3;
/// A panel whose content is two lines of text.
const SHORT: usize = 4;
/// A panel whose content fills its body.
const FILL: usize = 5;
/// Inside the fixture's room, so the area's bounds clamp neither the window nor the moves and resizes made of it.
const WINDOW_AT: Rect = Rect { min: Pos2::new(130.0, 90.0), max: Pos2::new(330.0, 210.0) };
const SLACK: f32 = 1.5;

fn title(panel: usize) -> String {
    format!("Panel {panel}")
}

struct Panels;

impl DockPanels<usize> for Panels {
    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        match *panel {
            SHORT => {
                Text::new("One").show(ui);
                Text::new("Two").show(ui);
            }
            FILL => {
                Stack::new().fill().show(ui, |ui, _| {
                    Text::new("Fills").show(ui);
                });
            }
            _ => {}
        }
    }
}

/// A dock area over a bound workspace; the area applies its actions. A second area over its own workspace shows
/// while `second` is set.
struct Bed {
    fixture: Fixture,
    workspace: Rc<RefCell<Arc<Workspace<usize>>>>,
    actions: Rc<RefCell<Vec<Action<usize>>>>,
    second: Rc<Cell<bool>>,
    /// The tallest a floated panel opens (`FloatSpace::default_size`), as the area reported it.
    float_height: Rc<Cell<f32>>,
    /// The height above a floating window's body (`FloatSpace::chrome`), as the area reported it.
    chrome: Rc<Cell<f32>>,
    /// The panel whose window the next pass asks to continue the held press, with its grab point.
    hand_over: Rc<Cell<Option<(usize, Vec2)>>>,
    /// Where an egui area above the dock shows, whose plain egui drag floats `WINDOW` at `WINDOW_AT` and hands the press
    /// to its window with the grab point `HOST_GRAB`, as an app's own popup does when its title is dragged out.
    popup: Rc<Cell<Option<Rect>>>,
}

/// The grab point inside the window a host's own drag hands its press to.
const HOST_GRAB: Vec2 = Vec2::new(40.0, 12.0);

fn bed(layout: Layout<usize>) -> Bed {
    bed_with(layout, DockConfig::default())
}

fn bed_with(layout: Layout<usize>, config: DockConfig) -> Bed {
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(layout))));
    let actions: Rc<RefCell<Vec<Action<usize>>>> = Rc::default();
    let second = Rc::new(Cell::new(false));
    let float_height = Rc::new(Cell::new(0.0));
    let chrome = Rc::new(Cell::new(0.0));
    let (held, seen_actions, shows_second, seen_height) = (Rc::clone(&workspace), Rc::clone(&actions), Rc::clone(&second), Rc::clone(&float_height));
    let seen_chrome = Rc::clone(&chrome);
    let hand_over: Rc<Cell<Option<(usize, Vec2)>>> = Rc::default();
    let asked = Rc::clone(&hand_over);
    let popup: Rc<Cell<Option<Rect>>> = Rc::default();
    let (shown_popup, hands_over) = (Rc::clone(&popup), Rc::clone(&hand_over));
    let mut other = Arc::new(Workspace::new(Layout::new(Some(Node::tabs(vec![SECOND], 0)), Vec::new())));
    let specs = Tracked::new(Numbered);
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = held.borrow().clone();
        let mut panels = Panels;
        let mut area = DockArea::new("Floating dock", Bind::bound(&mut workspace), &specs, &mut panels, config.clone());
        if let Some((panel, grab)) = asked.take() {
            area = area.continue_move((panel, grab));
        }
        let out = area.show(ui);
        *held.borrow_mut() = workspace;
        seen_actions.borrow_mut().extend(out.actions);
        if let Some(space) = out.float_space {
            seen_height.set(space.default_size.y);
            seen_chrome.set(space.chrome);
        }
        if shows_second.get() {
            DockArea::new("Second dock", Bind::bound(&mut other), &specs, &mut panels, DockConfig::default()).show(ui);
        }
        if let Some(rect) = shown_popup.get() {
            egui::Area::new(egui::Id::unique("host popup")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ui.ctx(), |ui| {
                let (_, drag) = ui.allocate_exact_size(rect.size(), egui::Sense::drag());
                if drag.drag_started() {
                    shown_popup.set(None);
                    let mut floated = held.borrow().layout().clone();
                    floated.floating.push(Floating::single(100, vec![WINDOW], 0, WINDOW_AT));
                    *held.borrow_mut() = Arc::new(Workspace::new(floated));
                    hands_over.set(Some((WINDOW, HOST_GRAB)));
                }
            });
        }
    });
    fixture.settle();
    Bed { fixture, workspace, actions, second, float_height, chrome, hand_over, popup }
}

/// A point on the floating window's title row, on its band: clear of the actions and the double arrow at its end.
fn window_title() -> Pos2 {
    Pos2::new(WINDOW_AT.min.x + 40.0, WINDOW_AT.min.y + 12.0)
}

fn dragging(bed: &Bed) -> bool {
    drag::active::<GroupDrag<usize>>(bed.fixture.ctx()).is_some()
}

fn with_window(root: Option<Node<usize>>) -> Layout<usize> {
    Layout::new(root, vec![Floating::single(100, vec![WINDOW], 0, WINDOW_AT)])
}

fn press_and_move(fixture: &mut Fixture, from: Pos2, by: Vec2) {
    fixture.frame(vec![moved(from)]);
    fixture.frame(vec![press(from)]);
    for step in 1..=4 {
        fixture.frame(vec![moved(from + by * (step as f32 / 4.0))]);
    }
}

#[trial]
fn a_tab_dragged_over_a_floating_window_marks_and_joins_the_window() {
    let mut bed = bed(with_window(Some(Node::tabs(vec![DOCKED, SECOND], 0))));
    let tabs = bed.fixture.by_role(Role::TAB);
    let mut docked: Vec<_> = tabs.iter().filter(|tab| !WINDOW_AT.contains(tab.center())).copied().collect();
    docked.sort_by(|a, b| a.rect.min.x.total_cmp(&b.rect.min.x));
    let second = *docked.last().expect("the second docked tab shows");
    let window_tab = tabs.iter().find(|tab| WINDOW_AT.contains(tab.center())).copied().expect("the window's tab shows");
    let over = Pos2::new(window_tab.rect.right() + 20.0, window_tab.center().y);
    press_and_move(&mut bed.fixture, second.center(), over - second.center());
    bed.fixture.frame(Vec::new());
    let mark = drag::mark(bed.fixture.ctx()).expect("the window takes the drop");
    let points = match mark {
        DropMark::Line { from, to, .. } => vec![from, to],
        DropMark::Into(rect) | DropMark::Region { area: rect, .. } => vec![rect.center()],
        _ => Vec::new(),
    };
    assert!(!points.is_empty() && points.iter().all(|point| WINDOW_AT.expand(SLACK).contains(*point)), "the mark is the window's: {mark:?}");
    bed.fixture.frame(vec![release(over)]);
    bed.fixture.settle();
    let layout = bed.workspace.borrow().layout().clone();
    let window = layout.floating.first().expect("the window stays");
    assert!(window.panels().any(|panel| *panel == SECOND), "the tab joined the window: {:?}", bed.actions.borrow());
}

/// A host that replaces the workspace while a window's move runs ends the drag, and the release drops nothing.
#[trial]
fn a_replaced_workspace_ends_a_window_drag() {
    let mut bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))));
    let by = Vec2::new(-40.0, 30.0);
    press_and_move(&mut bed.fixture, window_title(), by);
    assert!(dragging(&bed), "the window's drag runs");
    Arc::make_mut(&mut bed.workspace.borrow_mut()).replace(Workspace::new(with_window(Some(Node::tabs(vec![DOCKED, SECOND], 0)))));
    bed.fixture.frame(vec![moved(window_title() + by * 1.25)]);
    assert!(!dragging(&bed), "the replacement ended the drag");
    bed.fixture.frame(vec![release(window_title() + by * 1.25)]);
    bed.fixture.settle();
    let dropped = bed.actions.borrow().iter().any(|action| !matches!(action, Action::MoveFloating { .. }));
    assert!(!dropped, "nothing dropped, only the window moved: {:?}", bed.actions.borrow());
}

/// A body that asks the running pass to bring its own panel forward, every pass.
struct Requesting;

impl DockPanels<usize> for Requesting {
    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        assert!(request_actions(ui.ctx(), vec![Action::activate(*panel)]).is_ok(), "the running pass takes the request");
    }
}

/// A given workspace is never written: the host gets every change of the pass as actions, a window's move included,
/// and its bodies' requests unapplied beside them.
#[trial]
fn a_given_workspace_gets_the_window_move_and_the_bodies_requests_as_actions() {
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(with_window(Some(Node::tabs(vec![DOCKED], 0)))))));
    let requested: Rc<RefCell<Vec<Action<usize>>>> = Rc::default();
    let (held, seen, specs) = (Rc::clone(&workspace), Rc::clone(&requested), Tracked::new(Numbered));
    let mut fixture = Fixture::new(move |ui| {
        let given = held.borrow().clone();
        let mut panels = Requesting;
        let out = DockArea::new("Given dock", Bind::given(given), &specs, &mut panels, DockConfig::default().title_bar_docks(false)).show(ui);
        Arc::make_mut(&mut held.borrow_mut()).apply_all(out.actions, false, |_| Permissions::ALL).expect("the area's actions apply");
        seen.borrow_mut().extend(out.requested);
    });
    fixture.settle();
    assert!(request_actions(fixture.ctx(), vec![Action::activate(DOCKED)]).is_err(), "no pass runs between frames");
    let asked = requested.borrow().clone();
    let own = |action: &Action<usize>| *action == Action::activate(DOCKED) || *action == Action::activate(WINDOW);
    assert!(!asked.is_empty() && asked.iter().all(own), "each body's request came back: {asked:?}");
    assert_eq!(workspace.borrow().layout().floating[0].rect, WINDOW_AT, "nothing moved yet");
    let by = Vec2::new(-40.0, 30.0);
    press_and_move(&mut fixture, window_title(), by);
    fixture.frame(vec![release(window_title() + by)]);
    fixture.settle();
    let moved_to = workspace.borrow().layout().floating[0].rect;
    assert!(moved_to != WINDOW_AT, "the host's copy took the window's move: {moved_to:?}");
}

/// A drag of a floating window's title row shows the window under the pointer in the frame of each step, in one pass;
/// the workspace takes the move once, on the drop.
#[trial]
fn a_dragged_floating_window_stands_under_the_pointer_in_each_frame_and_moves_on_the_drop() {
    let mut bed = bed(with_window(None));
    let from = window_title();
    let step = Vec2::new(-10.0, 8.0);
    bed.fixture.frame(vec![moved(from)]);
    bed.fixture.frame(vec![press(from)]);
    for n in 1..=4 {
        let to = WINDOW_AT.translate(step * n as f32);
        bed.fixture.frame(vec![moved(from + step * n as f32)]);
        let shown = layer_rects(bed.fixture.ctx());
        assert!(
            shown.iter().any(|rect| (rect.min - to.min).length() < SLACK && (rect.size() - to.size()).length() < SLACK),
            "step {n}: the window stands under the pointer in its frame: {shown:?}, wanted {to:?}"
        );
        assert_eq!(bed.fixture.passes(), 1, "step {n}: the frame runs one pass");
        assert_eq!(bed.workspace.borrow().layout().floating[0].rect, WINDOW_AT, "step {n}: the workspace is not written while the move runs");
    }
    bed.fixture.frame(vec![release(from + step * 4.0)]);
    bed.fixture.settle();
    let rect = bed.workspace.borrow().layout().floating.first().map(|window| window.rect);
    assert!(rect.is_some_and(|rect| (rect.min - (WINDOW_AT.min + step * 4.0)).length() < SLACK), "the drop moves the window: {rect:?}");
}

/// A floating window the host asks to continue the held press takes it over from the tab it went down on: the window
/// follows the pointer at the grab point with its dock drag running, and the release lands it. Asked for a panel no
/// window holds, no window takes the press.
#[trial]
fn a_window_asked_to_continue_the_held_press_follows_the_pointer() {
    let grab = Vec2::new(40.0, 12.0);
    let follows = |panel: usize| {
        let mut bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))));
        let tab = bed.fixture.by_role(Role::TAB).into_iter().find(|tab| !WINDOW_AT.contains(tab.center())).expect("the docked tab shows");
        let from = tab.center();
        bed.fixture.frame(vec![moved(from)]);
        bed.fixture.frame(vec![press(from)]);
        bed.hand_over.set(Some((panel, grab)));
        bed.fixture.frame(Vec::new());
        let to = from + Vec2::new(60.0, 120.0);
        for step in 1..=4 {
            bed.fixture.frame(vec![moved(from + (to - from) * (step as f32 / 4.0))]);
        }
        let follows = layer_rects(bed.fixture.ctx()).iter().any(|rect| (rect.min + grab - to).length() < SLACK);
        (bed, follows, to)
    };
    let (mut bed, followed, to) = follows(WINDOW);
    assert!(followed, "the window holds the pointer at its grab point: {:?}", layer_rects(bed.fixture.ctx()));
    assert!(dragging(&bed), "the move runs the window's dock drag");
    let before = bed.workspace.borrow().layout().clone();
    bed.fixture.frame(vec![release(to)]).settle();
    assert!(*bed.workspace.borrow().layout() != before, "the release lands the window: {:?}", bed.actions.borrow());

    let (_, followed, _) = follows(DOCKED);
    assert!(!followed, "no window holds the docked panel, so none takes the press");
}

/// A floating window that took over a panel's drag docks on release as that drag would, though the window's title row
/// does not dock (`title_bar_docks(false)`): dropped on the docked group's tab strip, the panel joins the group. Its own
/// title row dragged there only moves the window.
#[trial]
fn a_window_that_took_over_a_panel_drag_docks_on_a_tab_strip_though_its_title_row_does_not() {
    let config = DockConfig::default().title_bar_docks(false);
    let joins = |handed_over: bool| {
        let mut bed = bed_with(with_window(Some(Node::tabs(vec![DOCKED], 0))), config.clone());
        let tab = bed.fixture.by_role(Role::TAB).into_iter().find(|tab| !WINDOW_AT.contains(tab.center())).expect("the docked tab shows");
        let over = Pos2::new(tab.rect.right() + 20.0, tab.center().y);
        let from = if handed_over { tab.center() } else { window_title() };
        bed.fixture.frame(vec![moved(from)]);
        bed.fixture.frame(vec![press(from)]);
        if handed_over {
            bed.hand_over.set(Some((WINDOW, Vec2::new(40.0, 12.0))));
            bed.fixture.frame(Vec::new());
        }
        for step in 1..=4 {
            bed.fixture.frame(vec![moved(from + (over - from) * (step as f32 / 4.0))]);
        }
        bed.fixture.frame(vec![release(over)]).settle();
        let layout = bed.workspace.borrow().layout().clone();
        layout.floating.is_empty() && layout.place(&WINDOW).is_some_and(|place| layout.place(&DOCKED).is_some_and(|docked| docked == place))
    };
    assert!(joins(true), "the handed-over drag joins the docked group");
    assert!(!joins(false), "the title row only moves the window");
}

/// A press a host's own egui widget holds (a popup's title, which no framework part senses) carries on as the drag of
/// the window the host floats in that pass and hands the press to: the window follows the pointer and docks on the
/// docked group's tab strip. The window shows first in the pass its drag starts, before its layer is recorded; the
/// drop targets still look through it, so its own target does not take the drop and refuse it.
#[trial]
fn a_window_takes_over_a_press_a_plain_egui_widget_holds() {
    let mut bed = bed_with(Layout::new(Some(Node::tabs(vec![DOCKED], 0)), Vec::new()), DockConfig::default().title_bar_docks(false));
    let tab = bed.fixture.by_role(Role::TAB).into_iter().next().expect("the docked tab shows");
    let over = Pos2::new(tab.rect.right() + 20.0, tab.center().y);
    let popup = Rect::from_min_size(Pos2::new(WINDOW_AT.min.x, WINDOW_AT.max.y + 20.0), Vec2::new(160.0, 24.0));
    bed.popup.set(Some(popup));
    bed.fixture.frame(Vec::new());
    let from = popup.center();
    bed.fixture.frame(vec![moved(from)]);
    bed.fixture.frame(vec![press(from)]);
    for step in 1..=4 {
        bed.fixture.frame(vec![moved(from + (over - from) * (step as f32 / 4.0))]);
    }
    assert!(dragging(&bed), "the window runs the handed-over drag");
    let mark = drag::mark(bed.fixture.ctx()).expect("a target under the dragged window takes the drop");
    let points = match mark {
        DropMark::Line { from, to, .. } | DropMark::Region { line: Some([from, to]), .. } => vec![from, to],
        DropMark::Into(rect) | DropMark::Region { area: rect, .. } => vec![rect.center()],
        _ => Vec::new(),
    };
    let strip = tab.rect.expand(SLACK);
    assert!(
        !points.is_empty() && points.iter().all(|point| point.y >= strip.top() && point.y <= strip.bottom()),
        "the mark is the tab strip's: {mark:?}"
    );
    bed.fixture.frame(vec![release(over)]).settle();
    let layout = bed.workspace.borrow().layout().clone();
    let joined = layout.floating.is_empty() && layout.place(&WINDOW).is_some_and(|place| layout.place(&DOCKED) == Some(place));
    assert!(joined, "the window's panel joins the docked group: {layout:?}");
}

/// A dock area's first pass leaves a drag running in another area alone.
#[trial]
fn an_areas_first_pass_keeps_a_running_drag() {
    let mut bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))));
    press_and_move(&mut bed.fixture, window_title(), Vec2::new(-40.0, 30.0));
    assert!(dragging(&bed), "the window drag runs");
    bed.second.set(true);
    bed.fixture.frame(Vec::new());
    assert!(dragging(&bed), "the second area's first pass kept the drag");
}

/// A floating window takes no room in the area: the docked tree reaches as far down as it does without the window.
#[trial]
fn a_floating_window_leaves_the_docked_tree_its_room() {
    let docked_bottom = |layout: Layout<usize>| {
        let bed = bed(layout);
        let panels = bed.fixture.by_role(Role::TAB_PANEL);
        let docked: Vec<f32> =
            panels.iter().filter(|panel| !WINDOW_AT.expand(SLACK).contains_rect(panel.rect)).map(|panel| panel.rect.bottom()).collect();
        assert_eq!(docked.len(), 1, "one docked group shows: {panels:?}");
        docked[0]
    };
    let alone = docked_bottom(Layout::new(Some(Node::tabs(vec![DOCKED], 0)), Vec::new()));
    let beside = docked_bottom(with_window(Some(Node::tabs(vec![DOCKED], 0))));
    assert!((beside - alone).abs() <= SLACK, "the docked group ends at {beside} beside the window, at {alone} without it");
}

/// A floating window's drag bar shows no title: its tab is the one place the panel's name shows in the window.
#[trial]
fn a_floating_windows_drag_bar_shows_no_title() {
    let bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))));
    let named = bed.fixture.texts().iter().filter(|text| text.text == title(WINDOW) && WINDOW_AT.contains_rect(text.rect)).count();
    assert_eq!(named, 1, "only the tab names the panel: {:?}", bed.fixture.texts());
}

/// The chrome the area keeps a dropped window's under the pointer and adds to a floated unit's height is the one the
/// window draws: the window's group starts right under it.
#[trial]
fn a_floating_windows_group_starts_under_the_chrome_the_area_sizes_windows_by() {
    let bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))));
    let group = bed
        .fixture
        .by_role(Role::TAB_PANEL)
        .into_iter()
        .find(|group| WINDOW_AT.expand(SLACK).contains_rect(group.rect))
        .expect("the window shows its group")
        .rect;
    let chrome = bed.chrome.get();
    assert!(chrome > 0.0, "the area reported its float space");
    assert!(
        (group.top() - WINDOW_AT.top() - chrome).abs() <= SLACK,
        "the group starts at {} in a window from {} with a chrome of {chrome}",
        group.top(),
        WINDOW_AT.top()
    );
}

/// The window's rect after a press at `from` drags by `by` and releases, in an area whose title-row drags only move a
/// window, so the release docks nothing.
fn dragged_from(from: Pos2, by: Vec2) -> Rect {
    let config = DockConfig::default().title_bar_docks(false);
    let mut bed = bed_with(with_window(Some(Node::tabs(vec![DOCKED], 0))), config);
    press_and_move(&mut bed.fixture, from, by);
    bed.fixture.frame(vec![release(from + by)]);
    bed.fixture.settle();
    bed.workspace.borrow().layout().floating[0].rect
}

/// A floating window's title row is a drag bar, shorter than the panel header of the tab strip under it, as
/// production's 18 pt bar over its tabs was.
#[trial]
fn a_floating_windows_title_row_is_shorter_than_its_tab_strip() {
    let bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))));
    let in_window = |role| bed.fixture.by_role(role).into_iter().find(|node| WINDOW_AT.expand(SLACK).contains_rect(node.rect)).map(|node| node.rect);
    let row = in_window(Role::HEADING).expect("the window shows its title row");
    let tab = in_window(Role::TAB).expect("the window shows its tab");
    assert!(row.height() < tab.height(), "the title row ({row:?}) is shorter than the tab strip ({tab:?})");
}

/// The grips of a floating window lie inside its frame and reach at least as far as production's: a corner 12 pt
/// along each of its edges, an edge 4 pt in from it.
#[trial]
fn a_floating_windows_corner_and_edge_grips_reach_as_far_in_as_productions() {
    let corner = dragged_from(Pos2::new(WINDOW_AT.right() - 11.0, WINDOW_AT.bottom() - 11.0), Vec2::new(20.0, 20.0));
    assert!(
        corner.width() > WINDOW_AT.width() + SLACK && corner.height() > WINDOW_AT.height() + SLACK && corner.min == WINDOW_AT.min,
        "a press 11 pt in from the corner resizes from the corner: {corner:?}"
    );
    let edge = dragged_from(Pos2::new(WINDOW_AT.left() + 3.5, WINDOW_AT.center().y), Vec2::new(-20.0, 0.0));
    assert!(
        edge.width() > WINDOW_AT.width() + SLACK && (edge.height() - WINDOW_AT.height()).abs() <= SLACK && edge.right() == WINDOW_AT.right(),
        "a press 3.5 pt in from the left edge resizes from that edge: {edge:?}"
    );
}

/// The top grip never lies over the title row: a press just inside the row's top moves the window, at its size.
#[trial]
fn the_top_of_a_floating_windows_title_row_moves_it_and_never_resizes_it() {
    let bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))));
    let row =
        bed.fixture.by_role(Role::HEADING).into_iter().find(|node| WINDOW_AT.expand(SLACK).contains_rect(node.rect)).expect("the title row").rect;
    let by = Vec2::new(-30.0, 20.0);
    let rect = dragged_from(Pos2::new(WINDOW_AT.min.x + 40.0, row.top() + 1.0), by);
    assert!((rect.size() - WINDOW_AT.size()).length() <= SLACK, "the press on the row at {row:?} resizes nothing: {rect:?}");
    assert!((rect.min - (WINDOW_AT.min + by)).length() <= SLACK, "it moves the window: {rect:?}");
}

/// A click on the title row neither moves nor docks the window; a press-drag on it moves the window. A guard of
/// production's behaviour, which the move already had.
#[trial]
fn a_click_on_a_floating_windows_bar_neither_moves_nor_docks_it_and_a_drag_moves_it() {
    let mut bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))));
    let at = window_title();
    bed.fixture.frame(vec![moved(at)]);
    bed.fixture.frame(vec![press(at)]);
    bed.fixture.frame(vec![release(at)]);
    bed.fixture.settle();
    assert_eq!(bed.workspace.borrow().layout().floating.first().map(|window| window.rect), Some(WINDOW_AT), "a click moves nothing");
    assert_eq!(bed.workspace.borrow().layout().floating.len(), 1, "and docks nothing");
    let by = Vec2::new(-40.0, 30.0);
    let rect = dragged_from(at, by);
    assert!((rect.min - (WINDOW_AT.min + by)).length() <= SLACK, "a drag moves the window: {rect:?}");
}

/// A tab dragged out of the window floats at the width its group had where it was docked, not a default size.
#[trial]
fn a_floated_panel_fits_its_content_up_to_the_float_height() {
    let floated = |bed: &Bed, panel: usize| {
        let layout = bed.workspace.borrow().layout().clone();
        let window = layout.floating.iter().find(|window| window.contains(&panel)).map(|window| window.rect);
        window.unwrap_or_else(|| panic!("panel {panel} floats: {:?}", bed.actions.borrow()))
    };

    // Torn out of the window: the filling panel floats at the group's width and the cap, which a measure of its
    // content alone would not reach.
    let mut torn = bed(Layout::new(Some(Node::tabs(vec![FILL, SHORT], 0)), Vec::new()));
    let group = torn.fixture.by_role(Role::TAB_PANEL).first().expect("the docked group shows").rect;
    let tab = torn.fixture.by_role(Role::TAB).into_iter().min_by(|a, b| a.rect.min.x.total_cmp(&b.rect.min.x)).expect("FILL's tab shows").rect;
    let outside = Pos2::new(group.center().x, -40.0);
    press_and_move(&mut torn.fixture, tab.center(), outside - tab.center());
    torn.fixture.frame(vec![release(outside)]);
    torn.fixture.settle();
    let fill = floated(&torn, FILL);
    let cap = torn.float_height.get();
    assert!(cap < group.height() - SLACK, "the cap {cap} is below the docked group {group:?}");
    assert!((fill.width() - group.width()).abs() <= SLACK, "floated {fill:?}, docked group {group:?}");
    assert!((fill.height() - cap).abs() <= SLACK, "filling content floats at the cap: {fill:?}, cap {cap}");

    // The group dragged by its strip's empty room into its own body and released with the key that floats: the
    // drag carries the measure the area added in the pass it started, and the short panel floats at its content's
    // height.
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![SHORT], 0), Node::tabs(vec![DOCKED], 0));
    let mut dragged = bed(Layout::new(Some(root), Vec::new()));
    let tab = dragged.fixture.by_role(Role::TAB).into_iter().min_by(|a, b| a.rect.min.x.total_cmp(&b.rect.min.x)).expect("the tab shows").rect;
    let group = dragged.fixture.by_role(Role::TAB_PANEL).into_iter().find(|group| group.rect.contains(tab.center())).expect("its group shows").rect;
    assert!(cap < group.height() - SLACK, "the cap {cap} is below the docked group {group:?}");
    let grip = Pos2::new(group.center().x + group.width() / 4.0, tab.center().y);
    let inside = group.center();
    press_and_move(&mut dragged.fixture, grip, inside - grip);
    assert!(dragging(&dragged), "the strip's empty room drags the group");
    // The key goes down while the pointer still moves, so the drop target marks the float before the release.
    dragged.fixture.hold(Modifiers::COMMAND);
    dragged.fixture.frame(vec![moved(inside)]);
    dragged.fixture.frame(vec![release(inside)]);
    dragged.fixture.hold(Modifiers::NONE);
    dragged.fixture.settle();
    let short = floated(&dragged, SHORT);
    assert!((short.width() - group.width()).abs() <= SLACK, "floated {short:?}, docked group {group:?}");
    assert!(short.height() < cap - SLACK, "short content floats at its height: {short:?}, cap {cap}");
}

/// A floating window's title row is its column's bar: its double arrow collapses the column to icons, which show in
/// the window in place of the groups, and expands it again; the window keeps its expanded rect throughout.
#[trial]
fn a_floating_windows_double_arrow_collapses_its_column_to_icons_and_expands_it() {
    let mut bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))));
    let button = |bed: &Bed, name: &str| {
        let tree = bed.fixture.tree();
        bed.fixture
            .by_role(Role::BUTTON)
            .into_iter()
            .find(|button| WINDOW_AT.expand(SLACK).contains_rect(button.rect) && craft_ui::testing::access::name(tree, button.node) == Some(name))
    };
    let in_window =
        |bed: &Bed| bed.fixture.by_role(Role::TAB_PANEL).into_iter().filter(|panel| WINDOW_AT.expand(SLACK).contains_rect(panel.rect)).count();
    assert_eq!(in_window(&bed), 1, "the expanded window shows its group");
    let collapse = button(&bed, "Collapse column to icons").expect("the title row shows the column's double arrow");
    bed.fixture.click(&collapse);
    bed.fixture.settle();
    let window = bed.workspace.borrow().layout().floating[0].clone();
    assert!(window.column.iconic, "the window's column collapsed: {:?}", bed.actions.borrow());
    assert_eq!(window.rect, WINDOW_AT, "the window keeps its expanded rect");
    assert_eq!(in_window(&bed), 0, "the collapsed window shows no group");
    let icon = button(&bed, &title(WINDOW)).expect("the collapsed window shows the panel's icon");
    assert!(icon.rect.width() < WINDOW_AT.width() / 2.0, "the window shows as a strip of icons: {:?}", icon.rect);
    let expand = button(&bed, "Expand column").expect("the collapsed window's title row shows the double arrow");
    bed.fixture.click(&expand);
    bed.fixture.settle();
    let window = bed.workspace.borrow().layout().floating[0].clone();
    assert!(!window.column.iconic && window.rect == WINDOW_AT, "the column expanded at its rect: {window:?}");
    assert_eq!(in_window(&bed), 1, "the expanded window shows its group again");
}

/// The window of `with_window` at `rect` with its column collapsed to icons, and no docked tree under it.
fn collapsed_at(rect: Rect) -> Layout<usize> {
    let mut window = Floating::single(101, vec![WINDOW], 0, rect);
    window.column.iconic = true;
    Layout::new(None, vec![window])
}

/// The rect of the button named `name`.
fn button_rect(bed: &Bed, name: &str) -> Option<Rect> {
    let tree = bed.fixture.tree();
    bed.fixture
        .by_role(Role::BUTTON)
        .into_iter()
        .find(|button| craft_ui::testing::access::name(tree, button.node) == Some(name))
        .map(|button| button.rect)
}

/// A collapsed window keeps to the edge it kept when its move began: dragged across the middle of the float area,
/// its strip stays under the pointer, and it stays where it is dropped. So does a strip dropped in the middle band,
/// where the strip is nearer one edge and its window's expanded rect the other.
#[trial]
fn a_collapsed_window_dragged_across_the_middle_keeps_its_strip_under_the_pointer() {
    // Across the middle; then to the strip's centre just past the middle of the 400-point room, on the end side,
    // while the expanded rect hanging from it reaches past the middle to the start side.
    for middle in [None, Some(210.0)] {
        let mut bed = bed(collapsed_at(WINDOW_AT));
        let icon = button_rect(&bed, &title(WINDOW)).expect("the collapsed window shows its icon");
        let by = Vec2::new(middle.map_or(-250.0, |x: f32| x - icon.center().x), 0.0);
        let band = strip_band(icon);
        bed.fixture.drag_hold_at(band, by);
        let held = button_rect(&bed, &title(WINDOW)).expect("the icon shows while the window moves");
        assert!((held.left() - (icon.left() + by.x)).abs() <= SLACK, "by {by:?}: the strip follows the pointer: {icon:?}, then {held:?}");
        bed.fixture.release_at(band + by).settle();
        bed.fixture.idle(3);
        let dropped = button_rect(&bed, &title(WINDOW)).expect("the icon shows after the move");
        assert!((dropped.left() - held.left()).abs() <= SLACK, "by {by:?}: the strip stays where it was dropped: {held:?}, then {dropped:?}");
        let window = bed.workspace.borrow().layout().floating[0].clone();
        assert!(window.column.iconic && window.edge.is_some(), "by {by:?}: the window stays collapsed and keeps its edge: {window:?}");
    }
}

/// An icon dragged out of a docked collapsed column and floated in the middle band of the float area floats
/// collapsed, its strip under the pointer, and stays there.
#[trial]
fn an_icon_floated_in_the_middle_band_lands_collapsed_under_the_pointer() {
    let mut column = Column::new(vec![Group::new(vec![WINDOW], 0)]);
    column.iconic = true;
    let root = Node::split(Axis::X, SplitSize::Ratio(0.8), Node::tabs(vec![DOCKED], 0), Node::dock(DockEdge::End, vec![column]));
    let mut bed = bed(Layout::new(Some(root), Vec::new()));
    let icon = button_rect(&bed, &title(WINDOW)).expect("the docked collapsed column shows its icon");
    // Just past the middle of the 400-point room: the strip is nearer the end edge, a window of the float width
    // around it the start edge.
    let at = Pos2::new(210.0, 150.0);
    press_and_move(&mut bed.fixture, icon.center(), at - icon.center());
    // The key that floats goes down while the pointer still moves, so the drop target marks the float.
    bed.fixture.hold(Modifiers::COMMAND);
    bed.fixture.frame(vec![moved(at)]);
    bed.fixture.frame(vec![release(at)]);
    bed.fixture.hold(Modifiers::NONE);
    bed.fixture.settle();
    let window = bed.workspace.borrow().layout().floating.first().cloned();
    let window = window.unwrap_or_else(|| panic!("the icon floats: {:?}", bed.actions.borrow()));
    assert!(window.column.iconic && window.edge.is_some(), "it floats collapsed, keeping to the edge its strip was dropped near: {window:?}");
    let floated = button_rect(&bed, &title(WINDOW)).expect("the floating window shows its icon");
    assert!(floated.x_range().contains(at.x), "the strip lands under the pointer: {floated:?}, released at {at:?}");
    bed.fixture.idle(3);
    let settled = button_rect(&bed, &title(WINDOW)).expect("the icon still shows");
    assert!((settled.left() - floated.left()).abs() <= SLACK, "the strip stays: {floated:?}, then {settled:?}");
}

/// The rect of the collapsed window's layer: the smallest layer around its icon.
fn strip_window(bed: &Bed) -> Rect {
    let icon = button_rect(bed, &title(WINDOW)).expect("the collapsed window shows its icon");
    let layers = layer_rects(bed.fixture.ctx()).into_iter().filter(|rect| rect.contains(icon.center()));
    layers.min_by(|a, b| a.area().total_cmp(&b.area())).expect("the window's layer holds its icon")
}

/// A point of a collapsed window's title band: the band's start, clear of the side grip (`size.handle.sm` deep) and
/// of the double arrow at the end the window keeps to.
fn strip_band(icon: Rect) -> Pos2 {
    Pos2::new(icon.left() + 10.0, window_title().y)
}

/// A collapsed window's sides resize its width, and only its width, as a docked collapsed column's inner edge does:
/// the column's iconic width follows, names show beside the icons from the docked column's labels width on, the
/// column stays as tall as its icons, and the expanded rect keeps its size. The width stays across expanding and
/// collapsing again.
#[trial]
fn a_collapsed_windows_sides_resize_its_width_and_show_names_from_the_docked_columns_step() {
    let mut bed = bed(collapsed_at(WINDOW_AT));
    let sizes = IconicSizes::of(bed.fixture.ctx());
    let height = sizes.icons_height(&bed.workspace.borrow().layout().floating[0].column);
    let named = |bed: &Bed| bed.fixture.texts().iter().any(|run| run.text == title(WINDOW));
    let height = height + bed.chrome.get();
    let strip = strip_window(&bed);
    assert!(!named(&bed), "a rail-wide strip shows no names");
    assert!((strip.height() - height).abs() <= SLACK, "the window is as tall as its icons and its chrome: {strip:?}, {height}");
    assert!((strip.right() - WINDOW_AT.right()).abs() <= SLACK, "the strip keeps to the end edge: {strip:?}");
    // The start side faces the room; a press just inside it holds the window's grip.
    let mut side = Pos2::new(strip.left() + 2.0, strip.center().y);
    for width in [sizes.labels - 10.0, sizes.labels + 20.0] {
        let by = Vec2::new(strip_window(&bed).width() - width, 0.0);
        press_and_move(&mut bed.fixture, side, by);
        bed.fixture.frame(vec![release(side + by)]);
        bed.fixture.settle();
        side += by;
        let window = bed.workspace.borrow().layout().floating[0].clone();
        let shown = strip_window(&bed);
        let kept = window.column.iconic_width;
        assert!(window.column.iconic && kept.is_some_and(|kept| (kept - width).abs() <= SLACK), "{width}: the column keeps the width: {window:?}");
        assert!((shown.width() - width).abs() <= SLACK, "{width}: the strip shows at it: {shown:?}");
        assert!((shown.height() - height).abs() <= SLACK, "{width}: the height stays its icons' and its chrome's: {shown:?}");
        assert!(
            (window.rect.min - WINDOW_AT.min).length() <= SLACK && window.rect.size() == WINDOW_AT.size(),
            "{width}: the expanded rect stays: {window:?}"
        );
        assert_eq!(named(&bed), width >= sizes.labels, "{width}: names show from the labels width ({}) on", sizes.labels);
    }
    let resized = strip_window(&bed);
    for iconic in [false, true] {
        Arc::make_mut(&mut bed.workspace.borrow_mut())
            .apply(Action::set_iconic(WINDOW, iconic), false, |_| Permissions::ALL)
            .expect("the column toggles");
        bed.fixture.settle();
    }
    let again = strip_window(&bed);
    assert!((again.width() - resized.width()).abs() <= SLACK, "collapsed again, the strip keeps its width: {resized:?}, then {again:?}");
    assert_eq!(bed.workspace.borrow().layout().floating[0].rect, WINDOW_AT, "the expanded rect is unchanged");
}

/// A floating window whose every group is minimized shows its title row and tab strip at its place and width. Its free
/// side resizes its width and not its height, and its rect keeps the expanded height, which the window shows at again
/// once the group is restored.
#[trial]
fn a_window_whose_groups_are_all_minimized_shows_its_strip_and_comes_back_at_its_height() {
    let mut bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))));
    let window_layer = |bed: &Bed| {
        let layers = layer_rects(bed.fixture.ctx()).into_iter().filter(|rect| rect.contains(window_title()));
        layers.min_by(|a, b| a.area().total_cmp(&b.area())).expect("the window's layer holds its title row")
    };
    let strip = |bed: &Bed| {
        let lists = bed.fixture.by_role(Role::TAB_LIST).into_iter().filter(|list| WINDOW_AT.expand(SLACK).contains_rect(list.rect));
        lists.map(|list| list.rect).next().expect("the window shows its tab strip")
    };
    Arc::make_mut(&mut bed.workspace.borrow_mut())
        .apply(Action::set_minimized(WINDOW, true), false, |_| Permissions::ALL)
        .expect("the group minimizes");
    bed.fixture.settle();
    let shown = window_layer(&bed);
    assert!((shown.bottom() - strip(&bed).bottom()).abs() <= SLACK, "the window ends at its strip: {shown:?}, {:?}", strip(&bed));
    assert!((shown.min - WINDOW_AT.min).length() <= SLACK && (shown.width() - WINDOW_AT.width()).abs() <= SLACK, "at its place and width: {shown:?}");

    // The window keeps to the end edge; its start side is free, and a press just inside it holds the side's grip.
    let side = Pos2::new(shown.left() + 2.0, shown.center().y);
    let by = Vec2::new(-40.0, 10.0);
    press_and_move(&mut bed.fixture, side, by);
    bed.fixture.frame(vec![release(side + by)]);
    bed.fixture.settle();
    let resized = window_layer(&bed);
    let rect = bed.workspace.borrow().layout().floating[0].rect;
    assert!((resized.width() - (shown.width() - by.x)).abs() <= SLACK, "the side resizes the width: {shown:?}, then {resized:?}");
    assert!((resized.height() - shown.height()).abs() <= SLACK, "the height stays the strip's: {resized:?}");
    assert!(
        (rect.width() - resized.width()).abs() <= SLACK && rect.height() == WINDOW_AT.height(),
        "the rect takes the width and keeps its height: {rect:?}"
    );

    Arc::make_mut(&mut bed.workspace.borrow_mut())
        .apply(Action::set_minimized(WINDOW, false), false, |_| Permissions::ALL)
        .expect("the group restores");
    bed.fixture.settle();
    let restored = window_layer(&bed);
    assert!((restored.height() - WINDOW_AT.height()).abs() <= SLACK, "restored, the window shows at its height: {restored:?}");
    assert!((restored.width() - resized.width()).abs() <= SLACK, "at the width the side gave it: {restored:?}");
}

/// A floating collapsed column opens its flyouts towards the room of the float area, away from the edge it keeps to.
#[trial]
fn a_floating_collapsed_columns_flyout_opens_towards_the_room() {
    // In the end half of the area, then in the start half.
    for (rect, opens_right) in [(WINDOW_AT, false), (WINDOW_AT.translate(Vec2::new(-110.0, 0.0)), true)] {
        let mut bed = bed(collapsed_at(rect));
        let icon = button_rect(&bed, &title(WINDOW)).expect("the collapsed window shows its icon");
        bed.fixture.click_at(icon.center()).settle();
        let flyout = bed.fixture.by_role(Role::TAB_PANEL).first().map(|group| group.rect).expect("the flyout shows the panel's group");
        let towards = if opens_right { flyout.left() >= icon.right() - SLACK } else { flyout.right() <= icon.left() + SLACK };
        assert!(towards, "window at {rect:?}: the flyout {flyout:?} opens on the side with room of the icon {icon:?}");
    }
}

/// A double click on a floating window's title band collapses its column, as one on a dock column's bar does, and a
/// double click on the collapsed window's band expands it again; the groups keep their state.
#[trial]
fn a_double_click_on_a_floating_windows_band_collapses_and_expands_its_column() {
    let mut bed = bed(with_window(None));
    let band = Pos2::new(WINDOW_AT.left() + 40.0, window_title().y);
    bed.fixture.click_at(band).click_at(band).settle();
    let window = bed.workspace.borrow().layout().floating[0].clone();
    assert!(window.column.iconic, "the band's double click collapsed the column: {:?}", bed.actions.borrow());
    assert!(window.groups().iter().all(|group| !group.minimized), "no group minimized");
    let icon = button_rect(&bed, &title(WINDOW)).expect("the collapsed window shows its icon");
    let band = strip_band(icon);
    bed.fixture.click_at(band).click_at(band).settle();
    assert!(!bed.workspace.borrow().layout().floating[0].column.iconic, "a second double click expanded it: {:?}", bed.actions.borrow());
}

/// A floating window's title row is its column's bar, with the double arrow and Close and no menu button: a context
/// request on its band opens the window's menu, collapsed or expanded, and the menu's Close entry closes the window.
#[trial]
fn a_context_request_on_a_windows_band_opens_its_menu_and_close_closes_it() {
    for collapsed in [true, false] {
        let mut bed = if collapsed { bed(collapsed_at(WINDOW_AT)) } else { bed(with_window(None)) };
        let band = if collapsed {
            let icon = button_rect(&bed, &title(WINDOW)).expect("the collapsed window shows its icon");
            Pos2::new(icon.left() + 4.0, WINDOW_AT.top() + 6.0)
        } else {
            window_title()
        };
        bed.fixture.frame(vec![moved(band)]);
        bed.fixture.frame(vec![secondary(band, true)]);
        bed.fixture.frame(vec![secondary(band, false)]);
        bed.fixture.settle();
        let tree = bed.fixture.tree();
        let items: Vec<_> =
            bed.fixture.by_role(Role::MENU_ITEM).into_iter().map(|item| (craft_ui::testing::access::name(tree, item.node), item)).collect();
        let names: Vec<_> = items.iter().map(|(name, _)| *name).collect();
        let close = items.iter().find(|(name, _)| name.is_some_and(|name| name.starts_with("Close"))).map(|(_, item)| *item);
        let close = close.unwrap_or_else(|| panic!("collapsed {collapsed}: the band's context request opens the window's menu: {names:?}"));
        bed.fixture.click(&close);
        bed.fixture.settle();
        assert!(bed.workspace.borrow().layout().floating.is_empty(), "collapsed {collapsed}: Close closed the window: {:?}", bed.actions.borrow());
    }
}

/// Nothing goes beside a floating window: a tab dragged over the start side of its group marks the join of the
/// group, not a split; and a tab dropped on a collapsed window's icon joins its group, which stays collapsed.
#[trial]
fn a_drag_over_a_floating_window_joins_or_stacks_and_never_splits() {
    let docked_tab = |bed: &Bed| {
        let tabs = bed.fixture.by_role(Role::TAB);
        let mut docked: Vec<_> = tabs.iter().filter(|tab| !WINDOW_AT.contains(tab.center())).map(|tab| tab.rect).collect();
        docked.sort_by(|a, b| a.min.x.total_cmp(&b.min.x));
        docked.last().copied().expect("the second docked tab shows")
    };
    let mut expanded = bed(with_window(Some(Node::tabs(vec![DOCKED, SECOND], 0))));
    let tab = docked_tab(&expanded);
    let side = Pos2::new(WINDOW_AT.left() + 6.0, WINDOW_AT.center().y + 20.0);
    press_and_move(&mut expanded.fixture, tab.center(), side - tab.center());
    expanded.fixture.frame(Vec::new());
    let mark = drag::mark(expanded.fixture.ctx()).expect("the window takes the drop");
    assert!(matches!(mark, DropMark::Region { .. }), "the start side of a floating group joins it: {mark:?}");
    expanded.fixture.frame(vec![release(side)]).settle();

    let mut window = Floating::single(102, vec![WINDOW], 0, WINDOW_AT);
    window.column.iconic = true;
    let mut collapsed = bed(Layout::new(Some(Node::tabs(vec![DOCKED, SECOND], 0)), vec![window]));
    let tab = docked_tab(&collapsed);
    let icon = button_rect(&collapsed, &title(WINDOW)).expect("the collapsed window shows its icon");
    press_and_move(&mut collapsed.fixture, tab.center(), icon.center() - tab.center());
    collapsed.fixture.frame(Vec::new());
    collapsed.fixture.frame(vec![release(icon.center())]).settle();
    let window = collapsed.workspace.borrow().layout().floating[0].clone();
    assert!(window.groups().len() == 1 && window.contains(&SECOND), "the tab joined the icon's group: {:?}", collapsed.actions.borrow());
    assert!(window.column.iconic, "the window stays collapsed");
}

/// A settled collapsed window runs no more passes than a settled expanded one.
#[trial]
fn a_settled_collapsed_window_runs_no_more_passes_than_an_expanded_one() {
    let passes = |layout: Layout<usize>| {
        let mut bed = bed(layout);
        bed.fixture.idle(3);
        bed.fixture.passes()
    };
    let (collapsed, expanded) = (passes(collapsed_at(WINDOW_AT)), passes(with_window(None)));
    assert!(collapsed <= expanded, "a still frame runs {collapsed} passes collapsed, {expanded} expanded");
}

/// A floating window at the top of the area sits below the top inset (the notice bars it floats over), and goes back up
/// when the inset does.
#[trial]
fn a_floating_window_clears_the_top_inset_and_follows_it() {
    let inset = Rc::new(Cell::new(0.0_f32));
    let seen = Rc::clone(&inset);
    let layout = Layout::new(
        Some(Node::tabs(vec![DOCKED], 0)),
        vec![Floating::single(103, vec![WINDOW], 0, Rect::from_min_size(Pos2::new(130.0, 0.0), Vec2::new(200.0, 120.0)))],
    );
    let held = Rc::new(RefCell::new(Arc::new(Workspace::new(layout))));
    let (shown, specs) = (Rc::clone(&held), Tracked::new(Numbered));
    let mut fixture = Fixture::new(move |ui| {
        let given = shown.borrow().clone();
        let mut panels = Panels;
        let _ = DockArea::new("Inset dock", Bind::given(given), &specs, &mut panels, DockConfig::default()).top_inset(seen.get()).show(ui);
    });
    fixture.settle();
    let window_top = |fixture: &Fixture| {
        fixture.by_role(Role::TAB).iter().filter(|tab| tab.rect.left() >= 130.0).map(|tab| tab.rect.top()).fold(f32::INFINITY, f32::min)
    };
    let free = window_top(&fixture);
    assert!(free < 60.0, "without an inset the window is at the top: {free}");
    inset.set(60.0);
    fixture.settle();
    let clear = window_top(&fixture);
    assert!(clear >= free + 59.0, "the window moves down by the 60 point inset: {free} to {clear}");
    inset.set(0.0);
    fixture.settle();
    let back = window_top(&fixture);
    assert!((back - free).abs() < 1.0, "the window goes back up when the notices close: {back}");
}
