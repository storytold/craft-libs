//! The host's focus request: `DockArea::focus` moves key focus to the group that shows a panel, as a press on the group's
//! body does.

use super::Numbered;
use craft_behavior::dock::{Action, Column, Floating, Group, Layout, Node, SplitSize, Workspace};
use craft_behavior::{Axis, PartKey};
use craft_ui::contract::{Bind, BodyCx, Tracked};
use craft_ui::docking::{DockConfig, DockPanels};
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

struct Panels;

impl DockPanels<usize> for Panels {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}
}

/// What one pass of the area reported.
#[derive(Debug)]
struct Seen {
    focused: Option<usize>,
    picked: Option<usize>,
    actions: Vec<Action<usize>>,
}

/// The fixture, the host's focus request for the next pass, and each pass's report.
type TwoGroups = (Fixture, Rc<Cell<Option<usize>>>, Rc<RefCell<Vec<Seen>>>);

/// A dock of group A (panel 0) beside group B (panels 1 and 2, 1 shown) with `config`; the host's focus request for the
/// next pass goes in the cell, and each pass's report is pushed to the list.
fn two_groups(config: DockConfig) -> TwoGroups {
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0], 0), Node::tabs(vec![1, 2], 0));
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let request = Rc::new(Cell::new(None));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let (asked, reported, specs) = (Rc::clone(&request), Rc::clone(&seen), Tracked::new(Numbered));
    let mut fixture = Fixture::new(move |ui| {
        let mut panels = Panels;
        let mut area = DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut panels, config.clone());
        if let Some(panel) = asked.take() {
            area = area.focus(panel);
        }
        let output = area.show(ui);
        reported.borrow_mut().push(Seen { focused: output.focused, picked: output.picked, actions: output.actions });
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    (fixture, request, seen)
}

/// With the user's focus on group A, the host's request for panel 2 of group B moves key focus to B's stop (the one a
/// press on B's body focuses) and picks 2's tab; a key there reaches B. Guards against a request that picked the tab
/// but left key focus, and so the keys, in the group the user pressed last.
#[trial]
fn the_hosts_focus_request_moves_key_focus_to_the_panels_group_and_picks_its_tab() {
    let (mut fixture, request, seen) = two_groups(DockConfig::default());
    // A point in the empty body below a group's tab.
    let body = |fixture: &Fixture, name: &str| {
        let tab = fixture.by_label(name).expect("a group's tab").rect;
        Pos2::new(tab.center().x, tab.bottom() + 200.0)
    };
    let b_body = body(&fixture, "Panel 1");
    fixture.click_at(b_body).idle(1);
    let b_stop = fixture.focused_stop().expect("a press on B's body focuses B's stop");
    let a_body = body(&fixture, "Panel 0");
    fixture.click_at(a_body).idle(1);
    assert_ne!(fixture.focused_stop(), Some(b_stop), "a press on A's body moves focus to A's stop");
    assert_eq!(seen.borrow().last().and_then(|seen| seen.focused), Some(0), "A holds the focus");
    seen.borrow_mut().clear();

    request.set(Some(2));
    fixture.idle(2);
    assert_eq!(fixture.focused_stop(), Some(b_stop), "the request moves key focus to B's stop");
    {
        let seen = seen.borrow();
        assert!(seen.iter().any(|pass| pass.actions.contains(&Action::activate(2))), "the request picks 2's tab: {seen:?}");
        assert!(seen.iter().all(|pass| pass.picked.is_none()), "the host's pick is not the user's: {seen:?}");
        assert_eq!(seen.last().and_then(|pass| pass.focused), Some(2), "B holds the focus, with 2 shown: {seen:?}");
    }
    seen.borrow_mut().clear();

    fixture.key(Key::ArrowLeft, Modifiers::NONE).idle(1);
    fixture.key(Key::Space, Modifiers::NONE).idle(1);
    let picked = seen.borrow().iter().find_map(|pass| pass.picked);
    assert!(matches!(picked, Some(1 | 2)), "the keys reach B's tabs, not A's: {picked:?}");
}

/// A titled header has no tab row: the request moves key focus to its drag band, the group's stop, and the group
/// holds the focus. Guards against a request that a titled group dropped.
#[trial]
fn the_hosts_focus_request_focuses_a_titled_groups_drag_band() {
    let (mut fixture, request, seen) = two_groups(DockConfig::default().titled_headers(true));
    request.set(Some(2));
    fixture.idle(2);
    let part = fixture.focused_stop().and_then(|(_, part)| part).map(|part| part.key);
    assert_eq!(part, Some(PartKey::new("grip")), "the drag band holds key focus");
    assert_eq!(seen.borrow().last().and_then(|pass| pass.focused), Some(2), "B holds the focus, with 2 shown");
}

/// Floating window A (panel 10) overlaps floating window B (panel 20); the docked group shows panel 0.
const A_AT: Rect = Rect::from_min_max(Pos2::new(100.0, 100.0), Pos2::new(400.0, 350.0));
const B_AT: Rect = Rect::from_min_max(Pos2::new(250.0, 180.0), Pos2::new(550.0, 430.0));

/// A docked group (panel 0) and the overlapping floating windows A and B, with the host's focus request in the cell.
fn two_windows() -> (Fixture, Rc<Cell<Option<usize>>>) {
    let window = |id, panel, at| Floating::new(id, Column::new(vec![Group::new(vec![panel], 0)]), at);
    let layout = Layout::new(Some(Node::tabs(vec![0], 0)), vec![window(100, 10, A_AT), window(101, 20, B_AT)]);
    let mut workspace = Arc::new(Workspace::new(layout));
    let request = Rc::new(Cell::new(None));
    let (asked, specs) = (Rc::clone(&request), Tracked::new(Numbered));
    let mut fixture = Fixture::new(move |ui| {
        let mut panels = Panels;
        let mut area = DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default());
        if let Some(panel) = asked.take() {
            area = area.focus(panel);
        }
        let _ = area.show(ui);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    (fixture, request)
}

/// With A in front, the host's focus request for B's panel brings B to the front: the overlap shows and hits B. The
/// request for A's panel brings A back, and one for the docked panel leaves the windows' order alone. Guards against
/// a revealed panel left under another floating window (FilmCraft's Text to Speech panel on a narration pick).
#[trial]
fn the_hosts_focus_request_brings_the_panels_floating_window_to_the_front() {
    let (mut fixture, request) = two_windows();
    let top_at = |fixture: &Fixture, at: Pos2| fixture.ctx().layer_id_at(at);
    let overlap = A_AT.intersect(B_AT).center();
    let (a_only, b_only) = (Pos2::new(A_AT.left() + 30.0, A_AT.bottom() - 30.0), Pos2::new(B_AT.right() - 30.0, B_AT.bottom() - 30.0));
    let (a, b) = (top_at(&fixture, a_only).expect("A's layer"), top_at(&fixture, b_only).expect("B's layer"));
    assert_ne!(a, b, "A and B are layers of their own");
    fixture.click_at(a_only).settle();
    assert_eq!(top_at(&fixture, overlap), Some(a), "a press on A brings it in front of B");

    request.set(Some(20));
    fixture.idle(2);
    assert_eq!(top_at(&fixture, overlap), Some(b), "the request for B's panel brings B in front of A");

    request.set(Some(10));
    fixture.idle(2);
    assert_eq!(top_at(&fixture, overlap), Some(a), "the request for A's panel brings A back");

    request.set(Some(0));
    fixture.idle(2);
    assert_eq!(top_at(&fixture, overlap), Some(a), "a docked panel's request leaves the windows' order alone");
}
