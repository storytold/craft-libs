//! A host's own drag (a rail popup's title) that floats a panel and hands its press to the panel's window, in a dock
//! that opens its floats in OS windows.

use super::Numbered;
use craft_behavior::dock::{Floating, Layout, Node, Workspace};
use craft_ui::contract::{Bind, BodyCx, Role, Tracked};
use craft_ui::docking::{DockConfig, DockPanels, GroupDrag};
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::drag;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::trial;
use craft_ui_test_kit::windows::{Out, Windows};
use egui::{Event, Key, Modifiers, Pos2, Rect, Sense, Ui, Vec2, ViewportId, ViewportInfo};
use std::sync::Arc;

const DOCKED: usize = 1;
const FLOATED: usize = 2;
/// The main window on the desktop.
const MAIN_AT: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(900.0, 700.0) };
/// Where the host floats the panel, in the main window.
const WINDOW_AT: Rect = Rect { min: Pos2::new(130.0, 90.0), max: Pos2::new(330.0, 210.0) };
/// The host's popup title, clear of the docked group's tab strip.
const TITLE_AT: Rect = Rect { min: Pos2::new(130.0, 230.0), max: Pos2::new(290.0, 254.0) };
/// The grab point inside the window the host hands its press over at.
const GRAB: Vec2 = Vec2::new(40.0, 12.0);
const SLACK: f32 = 1.5;

struct Empty;

impl DockPanels<usize> for Empty {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}
}

/// A dock over a host that shows a popup title, and the OS windows the dock opened.
struct Host {
    windows: Windows,
    workspace: Arc<Workspace<usize>>,
    specs: Tracked<Numbered>,
    popup: bool,
    hand_over: Option<(usize, Vec2)>,
    /// Where the dock opened its first OS window on the desktop.
    opened_at: Option<Pos2>,
}

impl Host {
    fn new(root: Option<Node<usize>>) -> Self {
        let windows = Windows::new();
        windows.root_info(info(MAIN_AT));
        windows.child_info(info(WINDOW_AT.translate(MAIN_AT.min.to_vec2())));
        let workspace = Arc::new(Workspace::new(Layout::new(root, Vec::new())));
        let mut host = Host { windows, workspace, specs: Tracked::new(Numbered), popup: true, hand_over: None, opened_at: None };
        for _ in 0..3 {
            host.frame(vec![]);
        }
        host
    }

    fn frame(&mut self, events: Vec<Event>) -> Out {
        let Host { windows, workspace, specs, popup, hand_over, .. } = self;
        let out = windows.frame(events, vec![], |ui| {
            let mut panels = Empty;
            let config = DockConfig::default().title_bar_docks(false);
            let mut area = DockArea::new("Dock", Bind::bound(&mut *workspace), &*specs, &mut panels, config);
            if let Some(request) = hand_over.take() {
                area = area.continue_move(request);
            }
            area.show(ui);
            if !*popup {
                return;
            }
            egui::Area::new(egui::Id::unique("host popup")).order(egui::Order::Foreground).fixed_pos(TITLE_AT.min).show(ui.ctx(), |ui| {
                let (_, title) = ui.allocate_exact_size(TITLE_AT.size(), Sense::drag());
                if title.drag_started() {
                    *popup = false;
                    let mut floated = workspace.layout().clone();
                    floated.floating.push(Floating::single(100, vec![FLOATED], 0, WINDOW_AT));
                    *workspace = Arc::new(Workspace::new(floated));
                    *hand_over = Some((FLOATED, GRAB));
                }
            });
        });
        if self.opened_at.is_none() {
            self.opened_at = out.builders.iter().find(|(viewport, _)| *viewport != ViewportId::ROOT).and_then(|(_, builder)| builder.position);
        }
        out
    }

    fn frames(&mut self, first: Vec<Event>) {
        self.frame(first);
        for _ in 0..3 {
            self.frame(vec![]);
        }
    }

    /// Presses the popup's title and drags it to `to`, held there.
    fn drag_to(&mut self, to: Pos2) {
        let from = TITLE_AT.center();
        self.frame(vec![moved(from)]);
        self.frame(vec![press(from)]);
        for step in 1..=4 {
            self.frame(vec![moved(from + (to - from) * (step as f32 / 4.0))]);
        }
        self.frame(vec![]);
    }

    fn dragging(&self) -> bool {
        drag::active::<GroupDrag<usize>>(&self.windows.ctx).is_some()
    }

    fn floats(&self) -> bool {
        self.workspace.layout().floating.iter().any(|window| window.contains(&FLOATED))
    }
}

fn info(rect: Rect) -> ViewportInfo {
    ViewportInfo {
        focused: Some(true),
        inner_rect: Some(rect),
        outer_rect: Some(rect),
        native_pixels_per_point: Some(1.0),
        ..ViewportInfo::default()
    }
}

/// A dock of one docked group, and a point on the free room of its tab strip.
fn docked() -> (Host, Pos2) {
    let mut host = Host::new(Some(Node::tabs(vec![DOCKED], 0)));
    let out = host.frame(vec![]);
    let tree = &out.trees.iter().find(|(viewport, _)| *viewport == ViewportId::ROOT).expect("the main window ran a pass").1;
    let tab = tree.with_role(Role::TAB).into_iter().find_map(|key| tree.bounds_of(key)).expect("the docked tab shows");
    (host, Pos2::new(tab.right() + 20.0, tab.center().y))
}

/// Dropped on a docked group's tab strip, the panel joins the group: the drag shows the dock's mark, and no OS window
/// opens at any point of it.
#[trial]
fn a_press_handed_to_a_float_docks_it_on_the_strip_it_is_released_on() {
    let (mut host, strip) = docked();
    host.drag_to(strip);
    assert_eq!(host.opened_at, None, "no OS window opens while the press is held");
    assert!(drag::mark(&host.windows.ctx).is_some(), "the strip under the pointer shows the drop mark");
    host.frames(vec![release(strip)]);
    let layout = host.workspace.layout();
    assert!(layout.floating.is_empty() && layout.place(&FLOATED).is_some_and(|at| layout.place(&DOCKED) == Some(at)), "it joins: {layout:?}");
    assert_eq!(host.opened_at, None, "a docked panel opens no OS window");
}

/// Released where nothing takes it, the panel floats in an OS window at the place it was dropped.
#[trial]
fn a_press_handed_to_a_float_opens_its_os_window_where_it_is_released() {
    let mut host = Host::new(None);
    let to = Pos2::new(400.0, 300.0);
    host.drag_to(to);
    assert!(host.dragging(), "the panel's drag runs");
    assert_eq!(host.opened_at, None, "no OS window opens while the press is held");
    host.frames(vec![release(to)]);
    let at = host.opened_at.expect("the OS window opens after the release");
    assert!(at.distance(MAIN_AT.min + (to - GRAB).to_vec2()) < SLACK, "where it was dropped: {at:?}");
    assert!(host.floats() && !host.dragging(), "the panel floats and no drag is left");
}

/// Escape, or the main window losing the focus, ends the drag: the panel's OS window opens where the host floated it,
/// and the button's later release drops nothing.
#[trial]
fn a_cancelled_press_handed_to_a_float_leaves_it_floating() {
    for cancel in [key_press(Key::Escape, Modifiers::NONE), Event::WindowFocused(false)] {
        let (mut host, strip) = docked();
        host.drag_to(strip);
        assert!(host.dragging(), "{cancel:?}: the panel's drag runs");
        host.frames(vec![cancel.clone()]);
        assert!(!host.dragging(), "{cancel:?} ends the drag");
        host.frames(vec![release(strip)]);
        assert!(host.floats(), "{cancel:?}: the release docks nothing");
        let at = host.opened_at.expect("the panel's OS window opens");
        assert!(at.distance(MAIN_AT.min + WINDOW_AT.min.to_vec2()) < SLACK, "{cancel:?}: where the host floated it: {at:?}");
    }
}
