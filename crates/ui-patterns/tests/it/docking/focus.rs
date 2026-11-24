//! The host's focus request: `DockArea::focus` moves key focus to the group that shows a panel, as a press on the group's
//! body does.

use super::Numbered;
use craft_behavior::dock::{Action, Layout, Node, SplitSize, Workspace};
use craft_behavior::{Axis, PartKey};
use craft_ui::contract::{Bind, BodyCx, Tracked};
use craft_ui::docking::{DockConfig, DockPanels};
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Pos2, Ui, Vec2};
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
