//! Floating windows (title bar, stacks, close), the split-tree settings (outer edge, Maximize,
//! Ctrl/Cmd undock, frame menu items), hidden panels, native windows and the public chrome.

use super::*;
use crate::layout::{SplitAxis, SplitSize};
use egui::{Context, Event, Id, Modifiers, PointerButton, RawInput, Rect, Vec2, pos2, vec2};

const AREA: &str = "floating-area";

fn tabs(panels: &[u32], active: usize) -> Node<u32> {
    Node::Tabs { panels: panels.to_vec(), active }
}

/// `[1] | [2, 3]` with panel 3 selected.
fn frames() -> Layout<u32> {
    Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedFirst(300.0),
            first: Box::new(tabs(&[1], 0)),
            second: Box::new(tabs(&[2, 3], 1)),
        }),
        floating: Vec::new(),
    }
}

/// A start dock (`[2, 3]` above `[4]`) beside protected canvas 0; window W1 stacks `[5]` above
/// `[6, 7]`; window W2 holds `[8]`.
fn workspace() -> Layout<u32> {
    Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedFirst(240.0),
            first: Box::new(Node::Dock {
                side: Side::Start,
                columns: vec![Column::new(vec![Group::new(vec![2, 3], 1), Group::new(vec![4], 0)], 240.0)],
            }),
            second: Box::new(tabs(&[0], 0)),
        }),
        floating: vec![
            Floating::stack(vec![Group::new(vec![5], 0), Group::new(vec![6, 7], 1)], [300.0, 80.0, 240.0, 300.0]).unwrap(),
            Floating::single(vec![8], 0, [560.0, 300.0, 200.0, 200.0]),
        ],
    }
}

#[derive(Default)]
struct Content {
    native: bool,
    keep: Vec<u32>,
    /// Canvas 0 accepts side splits, as an app that opens panels beside it does.
    beside: bool,
    drawn: Vec<u32>,
}

impl DockContent<u32> for Content {
    fn body(&mut self, ui: &mut egui::Ui, panel: &u32) {
        self.drawn.push(*panel);
        ui.label("Body");
    }
    fn logical_label(&self, panel: &u32) -> String {
        format!("Panel {panel}")
    }
    fn permissions(&self, panel: &u32) -> Permissions {
        match *panel {
            0 if self.beside => Permissions { accepts_splits: true, ..Permissions::PROTECTED },
            0 => Permissions::PROTECTED,
            _ => Permissions::default(),
        }
    }
    fn allow_native(&self, _: &u32) -> bool {
        self.native
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Hide {
    None,
    All,
    Keep,
}

/// The style of an editor whose layout is one split tree of tab groups, as FilmCraft sets it.
pub(super) fn frame_style(style: &mut DockStyle) {
    style.side_edge_zones = true;
    style.new_dock_zones = false;
    style.close_others_item = true;
    style.maximize_item = true;
    style.command_press_floats = true;
    style.float_on_empty_drop = false;
    style.title_bar_docks = false;
    style.float_hint = true;
    style.wheel_selects_tabs = true;
    style.middle_click_closes_tabs = true;
    style.float_label = "Undock Panel".into();
    style.float_group_label = "Undock Panel Group".into();
    style.close_group_label = "Close Panel Group".into();
}

struct Driver {
    ctx: Context,
    layout: Layout<u32>,
    content: Content,
    split_tree: bool,
    maximized: Option<u32>,
    hide: Hide,
    modifiers: Modifiers,
    time: f64,
}

impl Driver {
    fn new(layout: Layout<u32>, split_tree: bool) -> Self {
        let mut driver = Self {
            ctx: Context::default(),
            layout,
            content: Content::default(),
            split_tree,
            maximized: None,
            hide: Hide::None,
            modifiers: Modifiers::NONE,
            time: 0.0,
        };
        driver.step(vec![]);
        driver.step(vec![]);
        driver
    }

    fn frame(&mut self, mut events: Vec<Event>) -> DockOutput<u32> {
        let mut output = DockOutput::default();
        events.insert(0, Event::ModifiersChanged(self.modifiers));
        self.time += 1.0 / 60.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            events,
            focused: true,
            time: Some(self.time),
            ..Default::default()
        };
        let (layout, content, split_tree, maximized, hide) = (&self.layout, &mut self.content, self.split_tree, self.maximized, self.hide);
        let keep: Vec<u32> = content.keep.clone();
        let keep = move |p: &u32| keep.contains(p);
        let full = self.ctx.run_ui(input, |ui| {
            let mut style = DockStyle::from_ui(ui);
            if split_tree {
                frame_style(&mut style);
            }
            let hide = match hide {
                Hide::None => HidePanels::None,
                Hide::All => HidePanels::All,
                Hide::Keep => HidePanels::Except(&keep),
            };
            output =
                DockArea::new(Id::new(AREA)).native_windows(true).maximized(maximized).hide_panels(hide).show_content(ui, layout, &style, content);
        });
        let mut full = full;
        full.textures_delta.clear();
        output
    }

    fn step(&mut self, events: Vec<Event>) -> DockOutput<u32> {
        let output = self.frame(events);
        assert!(output.error.is_none(), "{output:?}");
        for action in &output.actions {
            self.layout.apply_with_permissions(action.clone(), |p| self.content.permissions(p)).unwrap();
        }
        output
    }

    fn response(&self, id: Id) -> Option<Rect> {
        self.ctx.read_response(id).map(|r| r.rect)
    }

    fn title(&self, first: u32) -> Rect {
        self.response(Id::new(AREA).with(("move-group", Some(&first)))).unwrap()
    }

    fn tab(&self, panel: u32) -> Option<Rect> {
        self.response(Id::new(AREA).with(("tab", &panel)))
    }

    fn hold(&mut self, from: egui::Pos2, to: egui::Pos2) -> DockOutput<u32> {
        self.step(vec![Event::PointerMoved(from), button(from, true)]);
        self.step(vec![Event::PointerMoved(from + vec2(8.0, 6.0))]);
        self.step(vec![Event::PointerMoved(to)])
    }

    fn drag(&mut self, from: egui::Pos2, to: egui::Pos2) -> DockOutput<u32> {
        self.hold(from, to);
        self.step(vec![button(to, false)])
    }

    fn click(&mut self, at: egui::Pos2) -> DockOutput<u32> {
        self.step(vec![Event::PointerMoved(at), button(at, true)]);
        self.step(vec![button(at, false)])
    }

    fn wait(&mut self) {
        for _ in 0..45 {
            self.step(vec![]);
        }
    }
}

fn done(mut full: egui::FullOutput) {
    full.textures_delta.clear();
}

fn button(pos: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn window_of(layout: &Layout<u32>, panel: u32) -> Option<&Floating<u32>> {
    layout.floating.iter().find(|w| w.contains(&panel))
}

fn groups_of(window: &Floating<u32>) -> Vec<Vec<u32>> {
    window.groups().iter().map(|g| g.panels.clone()).collect()
}

#[test]
fn title_bar_drag_moves_every_stacked_group_and_drops_into_a_dock_as_stacked_groups() {
    // The window follows the pointer with every group; Escape puts it back.
    let mut d = Driver::new(workspace(), false);
    let title = d.title(5).center();
    d.hold(title, title + vec2(40.0, 30.0));
    assert_eq!(d.layout.floating[0].rect, [340.0, 110.0, 240.0, 300.0]);
    assert_eq!(groups_of(&d.layout.floating[0]), vec![vec![5], vec![6, 7]]);
    d.step(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    d.step(vec![button(title + vec2(40.0, 30.0), false)]);
    assert_eq!(d.layout, workspace());

    // Released over another window's lower band: its tree goes below that window's group,
    // unless that leaves a group without room; then its panels join that group.
    let out = d.drag(title, pos2(660.0, 480.0));
    assert!(
        out.actions.contains(&Action::MoveWindow { panel: 5, to: Target::At { anchor: 8, placement: Placement::Split(Zone::Center) } }),
        "{:?}",
        out.actions
    );
    assert_eq!(d.layout.floating.len(), 1);
    assert_eq!(groups_of(&d.layout.floating[0]), vec![vec![8, 5, 6, 7]]);

    // Released over a docked group's strip: every panel joins it as tabs.
    let mut d = Driver::new(workspace(), false);
    let title = d.title(5).center();
    let strip = d.tab(4).unwrap().center();
    let out = d.drag(title, strip);
    assert!(out.actions.iter().any(|a| matches!(a, Action::MoveWindow { panel: 5, to: Target::At { anchor: 4, .. } })), "{:?}", out.actions);
    assert!(window_of(&d.layout, 5).is_none() && window_of(&d.layout, 7).is_none());
    assert_eq!(d.layout.place(&4), d.layout.place(&6));

    // Released over the canvas: the window stays where it was moved and hosts get the release.
    let mut d = Driver::new(workspace(), false);
    let title = d.title(5).center();
    let out = d.drag(title, pos2(500.0, 300.0));
    assert!(!out.actions.iter().any(|a| matches!(a, Action::MoveWindow { .. })), "{:?}", out.actions);
    assert_eq!(out.group_drop, Some((5, pos2(500.0, 300.0))));
    assert_eq!(groups_of(window_of(&d.layout, 5).unwrap()), vec![vec![5], vec![6, 7]]);
}

#[test]
fn title_bar_drag_onto_dock_targets_docks_the_window() {
    // Released on the seam between two docked groups: the window's group stacks between them.
    let mut d = Driver::new(workspace(), false);
    let title = d.title(8).center();
    let seam = pos2(120.0, d.tab(4).unwrap().top());
    let out = d.drag(title, seam);
    assert!(
        out.actions
            .contains(&Action::MoveWindow { panel: 8, to: Target::At { anchor: 2, placement: Placement::Group { below: true, height: None } } }),
        "{:?}",
        out.actions
    );
    assert!(window_of(&d.layout, 8).is_none());
    let Some(Node::Split { first, .. }) = &d.layout.root else { panic!("{:?}", d.layout.root) };
    let Node::Dock { columns, .. } = first.as_ref() else { panic!("{first:?}") };
    let stacked: Vec<_> = columns[0].groups.iter().map(|g| g.panels.clone()).collect();
    assert_eq!(stacked, vec![vec![2, 3], vec![8], vec![4]]);

    // Released on the new-dock band along the right edge: the window becomes an end dock.
    let mut d = Driver::new(workspace(), false);
    let title = d.title(5).center();
    let out = d.drag(title, pos2(796.0, 300.0));
    assert!(out.actions.contains(&Action::MoveWindow { panel: 5, to: Target::NewDock { side: Side::End } }), "{:?}", out.actions);
    assert!(window_of(&d.layout, 5).is_none() && window_of(&d.layout, 6).is_none());
}

#[test]
fn a_floating_window_is_a_tree_whose_groups_split_and_resize_like_frames() {
    // A group dragged by its bar to another window's top band splits that window's group.
    let mut d = Driver::new(workspace(), false);
    let bar = d.response(Id::new(AREA).with(("drag-bar", Some(&6u32)))).unwrap().center();
    let out = d.drag(bar, pos2(660.0, 360.0));
    assert!(
        out.actions.contains(&Action::MoveGroup { panel: 6, to: Target::At { anchor: 8, placement: Placement::Split(Zone::Top) } }),
        "{:?}",
        out.actions
    );
    assert_eq!(groups_of(window_of(&d.layout, 8).unwrap()), vec![vec![6, 7], vec![8]]);
    assert_eq!(groups_of(window_of(&d.layout, 5).unwrap()), vec![vec![5]]);

    // The gap between two groups of a window is no target.
    let mut d = Driver::new(workspace(), false);
    let tab = d.tab(8).unwrap().center();
    let gap = pos2(420.0, d.tab(6).unwrap().top() - 2.0);
    d.drag(tab, gap);
    assert_eq!(d.layout, workspace());

    // The divider between them resizes both.
    let mut d = Driver::new(workspace(), false);
    let divider = d.response(DockArea::hit_id(Id::new(AREA), &Hit::Divider(Divider::WindowSplit(5u32, Vec::new())))).unwrap().center();
    d.drag(divider, divider + vec2(0.0, 40.0));
    let Node::Split { size: SplitSize::Ratio(ratio), .. } = window_of(&d.layout, 5).unwrap().root else { panic!("{:?}", d.layout) };
    assert!(ratio > 0.6, "{ratio}");
}

#[test]
fn title_bar_double_click_minimizes_every_group_and_close_box_closes_the_window() {
    let mut d = Driver::new(workspace(), false);
    let title = d.title(5).center();
    d.click(title);
    d.click(title);
    assert!(window_of(&d.layout, 5).unwrap().minimized);
    d.wait();
    d.click(title);
    d.click(title);
    assert!(!window_of(&d.layout, 5).unwrap().minimized);

    let close = d.response(Id::new(AREA).with(("move-group", Some(&5u32))).with("close")).unwrap();
    assert_eq!(close.center().x, 530.0, "× sits 10 pt in from the right edge");
    let before = d.layout.clone();
    d.wait();
    let out = d.click(close.center());
    assert_eq!(out.actions, vec![Action::CloseWindow { panel: 5 }]);
    assert_eq!(before.closed_by(&Action::CloseWindow { panel: 5 }), Ok(vec![5, 6, 7]));
    assert!(window_of(&d.layout, 6).is_none() && d.layout.contains(&8));
}

#[test]
fn closed_floating_window_reopens_each_panel_at_its_last_rect() {
    let mut layout = workspace();
    let locations: Vec<_> = [5, 6, 7].map(|p| (p, layout.location(&p).unwrap())).into();
    layout.apply(Action::CloseWindow { panel: 6 }).unwrap();
    for (panel, location) in locations {
        layout.restore(panel, &location).unwrap();
        assert_eq!(window_of(&layout, panel).unwrap().rect, [300.0, 80.0, 240.0, 300.0], "{panel}");
    }
}

#[test]
fn ctrl_drag_from_a_frame_strip_undocks_to_a_floating_window() {
    let mut d = Driver::new(frames(), true);
    let tab = d.tab(3).unwrap().center();
    let other = d.tab(1).unwrap().center();
    d.modifiers = Modifiers::COMMAND;
    d.hold(tab, other);
    d.modifiers = Modifiers::NONE;
    let out = d.step(vec![button(other, false)]);
    assert!(matches!(out.actions.as_slice(), [Action::Float { panel: 3, .. }]), "{:?}", out.actions);
    assert!(window_of(&d.layout, 3).is_some());

    // Without the modifier the same drag joins the other frame.
    let mut d = Driver::new(frames(), true);
    let out = d.drag(tab, other);
    assert!(matches!(out.actions.as_slice(), [Action::Move { panel: 3, anchor: 1, .. }]), "{:?}", out.actions);
}

#[test]
fn outer_edge_band_splits_at_the_root() {
    // With side edge zones: all four edges.
    let mut d = Driver::new(frames(), true);
    let tab = d.tab(3).unwrap().center();
    let out = d.drag(tab, pos2(3.0, 300.0));
    assert!(out.actions.contains(&Action::Move { panel: 3, anchor: 1, placement: Placement::Edge(Zone::Left) }), "{:?}", out.actions);
    let Some(Node::Split { axis: SplitAxis::Horizontal, first, .. }) = &d.layout.root else { panic!("{:?}", d.layout) };
    assert_eq!(**first, tabs(&[3], 0));

    // The defaults: the bottom edge of the part between the docks, for a Timeline.
    let mut d = Driver::new(Layout { floating: Vec::new(), ..workspace() }, false);
    let tab = d.tab(2).unwrap().center();
    let out = d.drag(tab, pos2(500.0, 597.0));
    assert!(out.actions.iter().any(|a| matches!(a, Action::Move { panel: 2, placement: Placement::Edge(Zone::Bottom), .. })), "{:?}", out.actions);
    let Some(Node::Split { first, second, .. }) = &d.layout.root else { panic!() };
    assert!(matches!(**first, Node::Dock { .. }));
    assert!(matches!(&**second, Node::Split { axis: SplitAxis::Vertical, second, .. } if **second == tabs(&[2], 0)));

    // Its left and right edges are the docks' (new columns and docks), not root splits.
    let mut d = Driver::new(Layout { floating: Vec::new(), ..workspace() }, false);
    let tab = d.tab(4).unwrap().center();
    let out = d.drag(tab, pos2(246.0, 300.0));
    assert!(!out.actions.iter().any(|a| matches!(a, Action::Move { placement: Placement::Edge(_), .. })), "{:?}", out.actions);
}

#[test]
fn maximized_frame_fills_the_area_and_other_frames_do_not_render_or_take_input() {
    let mut layout = frames();
    layout.floating.push(Floating::single(vec![5], 0, [400.0, 300.0, 200.0, 150.0]));
    let mut d = Driver::new(layout.clone(), true);
    let window_title = d.title(5).center();
    d.maximized = Some(2);
    d.step(vec![]);
    d.step(vec![]);
    assert_eq!(d.tab(2).unwrap().left(), 0.0, "the maximized frame starts at the area's edge");
    assert!(d.tab(1).is_none());
    assert!(d.response(Id::new(AREA).with(("move-group", Some(&5u32)))).is_none());
    let out = d.drag(window_title, window_title + vec2(30.0, 30.0));
    assert!(out.actions.iter().all(|a| !matches!(a, Action::MoveFloating { .. } | Action::MoveWindow { .. })), "{:?}", out.actions);

    // A floating panel does not maximize.
    let mut d = Driver::new(layout, true);
    d.maximized = Some(5);
    d.step(vec![]);
    d.step(vec![]);
    assert!(d.tab(1).is_some());
}

#[test]
fn close_other_panels_keeps_only_the_chosen_tab() {
    use egui_kittest::{Harness, kittest::Queryable};
    type State = (Content, Vec<Action<u32>>, Vec<DockEvent<u32>>);
    let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        |ui, (content, actions, events): &mut State| {
            let mut style = DockStyle::from_ui(ui);
            frame_style(&mut style);
            let out = DockArea::new(Id::new(AREA)).native_windows(true).show_content(ui, &frames(), &style, content);
            actions.extend(out.actions);
            events.extend(out.events);
        },
        (Content::default(), Vec::new(), Vec::new()),
    );
    h.run_steps(2);
    let at = h.ctx.read_response(Id::new(AREA).with(("tab", &3u32))).unwrap().rect.center();
    let open_menu = |h: &mut Harness<'_, State>| {
        h.input_mut().events.extend([
            Event::PointerMoved(at),
            Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
        ]);
        h.run_steps(1);
        h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
    };
    open_menu(&mut h);
    for label in ["Undock Panel", "Close Panel Group", "Undock Panel Group", "Close Other Panels in Group", "Maximize Frame"] {
        assert_eq!(h.query_all_by_label(label).count(), 1, "{label}");
    }
    assert_eq!(h.query_all_by_label("Minimize").count(), 0);
    h.get_by_label("Close Other Panels in Group").click();
    h.run_steps(2);
    assert_eq!(h.state().1, vec![Action::CloseOthers { panel: 3 }]);
    let mut layout = frames();
    layout.apply(Action::CloseOthers { panel: 3 }).unwrap();
    assert!(layout.contains(&3) && !layout.contains(&2));

    open_menu(&mut h);
    h.get_by_label("Maximize Frame").click();
    h.run_steps(2);
    assert_eq!(h.state().2, vec![DockEvent::ToggleMaximize { panel: 3 }]);
}

#[test]
fn hide_panels_keeps_protected_panels_and_the_except_set() {
    let mut d = Driver::new(workspace(), false);
    d.hide = Hide::All;
    d.step(vec![]);
    d.step(vec![]);
    for panel in [2, 3, 4, 5, 6, 8] {
        assert!(d.tab(panel).is_none(), "{panel}");
    }
    assert!(d.layout == workspace(), "hiding leaves the layout unchanged");

    d.hide = Hide::Keep;
    d.content.keep = vec![4, 6];
    d.step(vec![]);
    d.step(vec![]);
    assert_eq!(d.tab(4).map(|r| r.left()), Some(0.0));
    assert!(d.tab(6).is_some());
    for panel in [2, 3, 5, 7, 8] {
        assert!(d.tab(panel).is_none(), "{panel}");
    }
    // A window keeps its rect with its hidden groups left out.
    d.step(vec![]);
    assert_eq!(d.title(6).min, pos2(300.0, 84.0), "below the resize band");
}

#[test]
fn a_canvas_that_accepts_splits_is_still_protected() {
    let mut d = Driver::new(workspace(), false);
    d.content.beside = true;
    d.hide = Hide::All;
    d.step(vec![]);
    d.content.drawn.clear();
    d.step(vec![]);
    assert_eq!(d.content.drawn, vec![0], "hiding every panel keeps the canvas");
    assert!(d.tab(0).is_none());
    assert!(d.response(DockArea::<u32>::hit_id(Id::new(AREA), &Hit::DragBar(0))).is_none());
}

#[test]
fn native_window_falls_back_in_window_when_viewports_are_embedded() {
    let mut layout = workspace();
    layout.floating[0].native = Some([1200.0, 900.0, 240.0, 300.0]);
    let mut d = Driver::new(layout.clone(), false);
    d.content.native = true;
    assert!(d.ctx.embed_viewports());
    let out = d.step(vec![]);
    assert!(out.actions.is_empty(), "{out:?}");
    assert_eq!(d.title(5).min, pos2(300.0, 84.0));
    assert_eq!(d.layout, layout);
}

#[cfg(feature = "native-windows")]
#[test]
fn native_window_close_closes_its_panels() {
    use std::{cell::Cell, rc::Rc};
    let close = Rc::new(Cell::new(false));
    let requested = close.clone();
    // A backend with real viewports: the child reports its place and, when asked, a close.
    Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
        let mut info = egui::ViewportInfo {
            outer_rect: Some(Rect::from_min_size(pos2(1210.0, 905.0), vec2(250.0, 330.0))),
            inner_rect: Some(Rect::from_min_size(pos2(1210.0, 935.0), vec2(250.0, 300.0))),
            ..Default::default()
        };
        if requested.get() {
            info.events.push(egui::ViewportEvent::Close);
        }
        let input = RawInput {
            viewport_id: viewport.ids.this,
            viewports: std::iter::once((viewport.ids.this, info)).collect(),
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(250.0, 300.0))),
            ..Default::default()
        };
        done(ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui)));
    });
    let mut layout = workspace();
    layout.floating[0].native = Some([1200.0, 900.0, 240.0, 300.0]);
    let mut d = Driver::new(Layout::default(), false);
    d.ctx.set_embed_viewports(false);
    d.content.native = true;
    d.layout = layout;
    d.step(vec![]);
    assert!(d.response(Id::new(AREA).with(("move-group", Some(&5u32)))).is_none(), "drawn in its own OS window");
    // Where the window system put its content, once a report that late is no longer the
    // opening move landing.
    let reported = (0..60).any(|_| d.step(vec![]).actions.contains(&Action::SetNative { panel: 5, rect: Some([1210.0, 935.0, 250.0, 300.0]) }));
    assert!(reported, "{:?}", d.layout.floating[0].native);

    close.set(true);
    let out = d.step(vec![]);
    assert!(out.actions.contains(&Action::CloseWindow { panel: 5 }), "{:?}", out.actions);
    assert!(window_of(&d.layout, 5).is_none() && !d.layout.contains(&6) && !d.layout.contains(&7));
}

#[test]
fn chrome_frame_and_title_bar_work_without_a_dock_area() {
    use chrome::{ChromeStyle, FloatingFrame, TitleBar};
    let ctx = Context::default();
    let id = Id::new("task-pane");
    let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(300.0, 200.0));
    let mut time = 0.0;
    let mut frame = |events: Vec<Event>| {
        time += 1.0 / 60.0;
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), events, time: Some(time), ..Default::default() };
        let mut result = None;
        done(ctx.run_ui(input, |ui| {
            let style = ChromeStyle::from_ui(ui);
            let shown = FloatingFrame::new(id, rect).show(ui.ctx(), &style, |ui, inner| {
                let title = Rect::from_min_size(inner.min, vec2(inner.width(), 22.0));
                let bar = TitleBar::new(id.with("title"), "Move pane").title("Styles").menu("Pane menu").close("Close pane").show(ui, title, &style);
                (bar.bar.dragged(), bar.bar.drag_delta(), bar.menu.is_some_and(|r| r.clicked()), bar.close.is_some_and(|r| r.clicked()))
            });
            result = Some((shown.inner, shown.resized));
        }));
        result.unwrap()
    };
    frame(vec![]);
    frame(vec![]);
    type Shown = ((bool, Vec2, bool, bool), Option<Rect>);
    let click = |frame: &mut dyn FnMut(Vec<Event>) -> Shown, at| {
        frame(vec![Event::PointerMoved(at), button(at, true)]);
        frame(vec![button(at, false)])
    };
    let ((_, _, _, closed), _) = click(&mut frame, pos2(390.0, 111.0));
    assert!(closed, "× 10 pt in from the right");
    let ((_, _, menu, _), _) = click(&mut frame, pos2(374.0, 111.0));
    assert!(menu, "▾ left of ×");

    frame(vec![Event::PointerMoved(pos2(200.0, 110.0)), button(pos2(200.0, 110.0), true)]);
    frame(vec![Event::PointerMoved(pos2(210.0, 115.0))]);
    let ((dragged, delta, ..), _) = frame(vec![Event::PointerMoved(pos2(230.0, 125.0))]);
    assert!(dragged && delta == vec2(20.0, 10.0), "{delta:?}");
    frame(vec![button(pos2(230.0, 125.0), false)]);

    let edge = pos2(398.0, 200.0);
    frame(vec![Event::PointerMoved(edge), button(edge, true)]);
    frame(vec![Event::PointerMoved(edge + vec2(10.0, 0.0))]);
    let (_, resized) = frame(vec![Event::PointerMoved(edge + vec2(50.0, 0.0))]);
    assert_eq!(resized, Some(Rect::from_min_size(rect.min, vec2(350.0, 200.0))));
}

#[test]
fn hidden_panels_leave_their_split_and_resizing_names_the_layout_path() {
    let layout = Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedFirst(200.0),
            first: Box::new(tabs(&[9], 0)),
            second: Box::new(Node::Split {
                axis: SplitAxis::Vertical,
                size: SplitSize::Ratio(0.5),
                first: Box::new(tabs(&[1], 0)),
                second: Box::new(tabs(&[2], 0)),
            }),
        }),
        floating: Vec::new(),
    };
    let mut d = Driver::new(layout, true);
    d.hide = Hide::Keep;
    d.content.keep = vec![1, 2];
    d.step(vec![]);
    d.step(vec![]);
    assert_eq!(d.tab(1).map(|r| r.left()), Some(0.0), "the hidden frame's space goes to the others");
    // Hits and widget ids name the split by its layout path, as `ResizeSplit` does.
    let hit = Hit::Divider(Divider::Split(vec![true]));
    assert!(DockArea::<u32>::hit_rects(&d.ctx, Id::new(AREA)).iter().any(|(h, _)| *h == hit), "the hits name the layout path");
    let divider = d.response(DockArea::<u32>::hit_id(Id::new(AREA), &hit)).unwrap().center();
    let out = d.hold(divider, divider + vec2(0.0, 60.0));
    assert!(out.actions.iter().any(|a| matches!(a, Action::ResizeSplit { path, .. } if path == &vec![true])), "{:?}", out.actions);
    let Some(Node::Split { size: SplitSize::FixedFirst(200.0), second, .. }) = &d.layout.root else { panic!("{:?}", d.layout) };
    assert!(matches!(**second, Node::Split { size, .. } if size != SplitSize::Ratio(0.5)));
}

#[test]
fn a_hidden_floating_window_does_not_cover_dock_targets() {
    // Window 5 covers the column's seam at x 150..240 until it is hidden.
    let mut layout = workspace();
    layout.floating = vec![Floating::single(vec![5], 0, [150.0, 250.0, 200.0, 150.0]), Floating::single(vec![8], 0, [560.0, 300.0, 200.0, 200.0])];
    let mut d = Driver::new(layout, false);
    d.hide = Hide::Keep;
    d.content.keep = vec![2, 3, 4, 8];
    d.step(vec![]);
    d.step(vec![]);
    d.drag(d.title(8).center(), pos2(200.0, 309.0));
    let Some(Node::Split { first, .. }) = &d.layout.root else { panic!("{:?}", d.layout.root) };
    let Node::Dock { columns, .. } = &**first else { panic!("{first:?}") };
    let groups: Vec<Vec<u32>> = columns[0].groups.iter().map(|g| g.panels.clone()).collect();
    assert_eq!(groups, vec![vec![2, 3], vec![8], vec![4]]);
}

#[test]
fn top_edges_resize_from_the_band_above_the_title_bar() {
    // W2 holds [8] at [560, 300, 200, 200]; each top handle keeps the opposite edges.
    let area = Id::new(AREA);
    for (edge, by, expected) in [
        (ResizeEdge::Top, vec2(0.0, -40.0), [560.0, 260.0, 200.0, 240.0]),
        (ResizeEdge::TopLeft, vec2(-30.0, 20.0), [530.0, 320.0, 230.0, 180.0]),
        (ResizeEdge::TopRight, vec2(20.0, -10.0), [560.0, 290.0, 220.0, 210.0]),
    ] {
        let mut d = Driver::new(workspace(), false);
        let handle = d.response(DockArea::hit_id(area, &Hit::Resize(8u32, edge))).unwrap();
        assert_eq!(handle.bottom(), d.title(8).top(), "{edge:?} lies above the title bar");
        d.drag(handle.center(), handle.center() + by);
        assert_eq!(window_of(&d.layout, 8).unwrap().rect, expected, "{edge:?}");
    }
    // The title bar still moves the window, and arrow keys move the focused top edge.
    let mut d = Driver::new(workspace(), false);
    let title = d.title(8);
    d.drag(pos2(title.center().x, title.top() + 1.0), pos2(title.center().x - 50.0, title.top() + 1.0));
    assert_eq!(window_of(&d.layout, 8).unwrap().rect, [510.0, 300.0, 200.0, 200.0]);
    d.ctx.memory_mut(|m| m.request_focus(DockArea::hit_id(area, &Hit::Resize(8u32, ResizeEdge::Top))));
    d.step(vec![Event::Key { key: egui::Key::ArrowUp, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    assert_eq!(window_of(&d.layout, 8).unwrap().rect, [510.0, 292.0, 200.0, 208.0]);
}

#[test]
fn undock_panel_group_floats_one_group_of_a_floating_window() {
    use egui_kittest::{Harness, kittest::Queryable};
    let stacked = || Layout {
        root: Some(tabs(&[1], 0)),
        floating: vec![Floating::stack(vec![Group::new(vec![5], 0), Group::new(vec![6, 7], 0)], [300.0, 80.0, 300.0, 400.0]).unwrap()],
    };
    type State = (Content, Layout<u32>, Vec<Action<u32>>);
    let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        |ui, (content, layout, actions): &mut State| {
            let mut style = DockStyle::from_ui(ui);
            frame_style(&mut style);
            let out = DockArea::new(Id::new(AREA)).native_windows(true).show_content(ui, layout, &style, content);
            actions.extend(out.actions);
        },
        (Content::default(), stacked(), Vec::new()),
    );
    h.run_steps(2);
    let menu = |h: &mut Harness<'_, State>, panel: u32| {
        let at = h.ctx.read_response(Id::new(AREA).with(("tab", &panel))).unwrap().rect.center();
        h.input_mut().events.extend([
            Event::PointerMoved(at),
            Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
        ]);
        h.run_steps(1);
        h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
    };
    menu(&mut h, 6);
    h.get_by_label("Undock Panel Group").click();
    h.run_steps(2);
    let actions = std::mem::take(&mut h.state_mut().2);
    assert!(matches!(&actions[..], [Action::MoveGroup { panel: 6, to: Target::Float { rect: Some(_) } }]), "{actions:?}");
    let mut layout = stacked();
    layout.apply_all(actions, |_| Permissions::default()).unwrap();
    assert_eq!(layout.floating.iter().map(groups_of).collect::<Vec<_>>(), vec![vec![vec![5]], vec![vec![6, 7]]]);

    // A window's only group has nothing to undock from.
    h.state_mut().1 = layout;
    h.run_steps(2);
    menu(&mut h, 5);
    assert_eq!(h.query_all_by_label("Undock Panel Group").count(), 0);
}
