//! The divider every docking container shares: dragging and the layout write-back.

use craft_behavior::Axis;
use craft_behavior::dock::{Column, Group, Layout, Node, SplitSize};
use craft_ui::contract::{Role, Tracked};
use craft_ui::docking::frame::maximize;
use craft_ui::docking::tree;
use craft_ui::docking::{DockColumn, HidePanels, PanelKey, PanelSpec};
use craft_ui::testing::memory::global;
use craft_ui::testing::pointer;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Pos2, Rect, Vec2};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

fn spec(panel: &u8) -> PanelSpec {
    PanelSpec::new(format!("Panel {panel}"))
}

type Rects = Rc<RefCell<Vec<Rect>>>;

/// A column of two open groups whose resizes the host applies each pass, and the groups' rects of the last pass.
fn column() -> (Fixture, Rects) {
    let column = Rc::new(RefCell::new(Column::new(vec![Group::new(vec![1], 0), Group::new(vec![2], 0)])));
    let rects: Rects = Rc::default();
    let seen = Rc::clone(&rects);
    let fixture = Fixture::new(move |ui| {
        seen.borrow_mut().clear();
        let shown = column.borrow().clone();
        let out =
            DockColumn::new("Panels", global("dock", "split test"), &shown, &spec).show_groups(ui, |_, rect, _, _| seen.borrow_mut().push(rect));
        for (anchor, height) in out.resized {
            if let Some(group) = column.borrow_mut().groups.iter_mut().find(|group| group.panels.first() == Some(&anchor)) {
                group.height = Some(height);
            }
        }
    });
    (fixture, rects)
}

fn divider(fixture: &Fixture) -> craft_ui::testing::Target {
    fixture.by_role(Role::SPLITTER).into_iter().next().expect("a divider between the two open groups")
}

#[trial]
fn a_column_divider_dragged_past_a_minimum_and_back_stays_under_the_pointer() {
    let (mut fixture, rects) = column();
    fixture.settle();
    let grip = divider(&fixture);
    let at = grip.center();
    fixture.press(&grip);
    for y in [at.y + 2000.0, at.y + 2000.0, at.y + 15.0] {
        fixture.frame(vec![pointer::moved(Pos2::new(at.x, y))]);
    }
    let bottom = rects.borrow()[0].bottom();
    assert!(
        (bottom - (at.y + 15.0)).abs() < 1.0,
        "the divider is at the pointer after the drag came back from past the minimum: {bottom} for {}",
        at.y + 15.0
    );
}

#[trial]
fn a_resize_under_a_pruned_view_keeps_the_hidden_panels() {
    let rest = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![1], 0), Node::tabs(vec![2], 0));
    let mut layout = Layout::new(Some(Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0], 0), rest)), Vec::new());
    let keep = Tracked::new(BTreeSet::from([PanelKey::of(&0_usize), PanelKey::of(&1_usize)]));
    let none_protected = |_: &usize| false;
    let original = layout.clone();
    let view = maximize(&original, None, HidePanels::Except(&keep), &none_protected);
    let mut edited = view.layout.clone().into_owned();
    if let Some(root) = edited.root.as_mut() {
        tree::apply_sizes(root, &[(Vec::new(), SplitSize::Ratio(0.8))]);
    }
    view.write_back(&edited, &mut layout);
    assert!(layout.panels().contains(&&2), "the hidden panel stays in the layout");
    assert!(
        matches!(layout.root, Some(Node::Split { size: SplitSize::Ratio(ratio), .. }) if (ratio - 0.8).abs() < 1e-6),
        "the resize reached the layout"
    );
}

/// Tabs until the divider holds key focus.
fn tab_to_divider(fixture: &mut Fixture) {
    for _ in 0..12 {
        if craft_ui::testing::access::focused_key(fixture.tree()) == divider(fixture).node {
            return;
        }
        fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    }
    panic!("Tab never reached the divider");
}

/// A pointer drag of a divider leaves key focus where it was, so the next arrow key does not resize the dock; a
/// divider reached by Tab still resizes with the arrows.
#[trial]
fn a_dragged_divider_does_not_keep_the_arrow_keys_but_a_tabbed_one_takes_them() {
    let (mut fixture, rects) = column();
    fixture.settle();
    let grip = divider(&fixture);
    fixture.drag(&grip, Vec2::new(0.0, 20.0)).idle(2);
    let dragged = rects.borrow()[0].bottom();
    assert_ne!(craft_ui::testing::access::focused_key(fixture.tree()), divider(&fixture).node, "the drag did not focus the divider");
    fixture.key(Key::ArrowDown, Modifiers::NONE).idle(2);
    assert!((rects.borrow()[0].bottom() - dragged).abs() < 0.5, "the arrows after a pointer drag left the dock as it was");

    let (mut fixture, rects) = column();
    fixture.settle();
    tab_to_divider(&mut fixture);
    let tabbed = rects.borrow()[0].bottom();
    fixture.key(Key::ArrowDown, Modifiers::NONE).idle(2);
    assert!((rects.borrow()[0].bottom() - tabbed).abs() > 0.5, "an arrow key moves the divider Tab reached");
}
