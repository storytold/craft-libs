//! Whole-group gestures: drag bar, top border, drop resolution, minimize and the floating frame.

use super::*;
use crate::layout::{SplitAxis, SplitSize};
use egui::{Color32, Context, Event, Id, Modifiers, PointerButton, RawInput, Rect, UiBuilder, Vec2, pos2, vec2};

const AREA: &str = "group-area";

#[path = "trial_tests.rs"]
mod trial;

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

/// A start dock with one column holding `[2, 3]` (3 selected) above `[4]`, beside canvas 0.
fn docked() -> Layout<u32> {
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
        floating: Vec::new(),
    }
}

#[derive(Default)]
struct TestContent {
    protected: Vec<u32>,
    header: f32,
    header_clicks: usize,
    /// The strip width the last header was sized for.
    strip_width: std::cell::Cell<f32>,
    floating_size: Option<Vec2>,
    host_owned: bool,
    whole_group: bool,
    /// Height every body takes; `None` draws a one-line label.
    body_height: Option<f32>,
    /// Every body takes its whole rect.
    fill: bool,
}

impl DockContent<u32> for TestContent {
    fn body(&mut self, ui: &mut egui::Ui, _: &u32) {
        if self.fill {
            ui.allocate_rect(ui.max_rect(), egui::Sense::hover());
            return;
        }
        match self.body_height {
            Some(height) => _ = ui.allocate_exact_size(vec2(10.0, height), egui::Sense::hover()),
            None => _ = ui.label("Body"),
        }
    }
    fn logical_label(&self, panel: &u32) -> String {
        format!("Panel {panel}")
    }
    fn permissions(&self, panel: &u32) -> Permissions {
        if self.protected.contains(panel) { Permissions::PROTECTED } else { Permissions::default() }
    }
    fn floating_size(&self, _: &u32) -> Option<Vec2> {
        self.floating_size
    }
    fn resolve_drag(&mut self, _: &Context, _: &DockStyle, request: &DropRequest<'_, u32>) -> Option<DropFeedback> {
        self.whole_group |= request.whole_group;
        self.host_owned.then(DropFeedback::default)
    }
    fn group_header_width(&self, _: &[u32], _: &u32, _: bool, strip_width: f32) -> f32 {
        self.strip_width.set(strip_width);
        self.header
    }
    fn group_header(&mut self, ui: &mut egui::Ui, _: &[u32], _: &u32, _: bool) {
        if ui.button("≡").clicked() {
            self.header_clicks += 1;
        }
    }
}

/// Renders a layout through `show_content` in `area` of an 800x600 window and applies what
/// it emits, the way an application's command handler would.
struct Driver {
    ctx: Context,
    layout: Layout<u32>,
    content: TestContent,
    area: Rect,
    modifiers: Modifiers,
    focused: bool,
    transparent: bool,
    /// Applied to the style every frame.
    tweak: fn(&mut DockStyle),
    shapes: Vec<egui::epaint::ClippedShape>,
    time: f64,
}

impl Driver {
    fn new(layout: Layout<u32>) -> Self {
        Self::with(layout, TestContent::default(), Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0)))
    }

    fn with(layout: Layout<u32>, content: TestContent, area: Rect) -> Self {
        let mut driver = Self {
            ctx: Context::default(),
            layout,
            content,
            area,
            modifiers: Modifiers::NONE,
            focused: true,
            transparent: false,
            tweak: |_| {},
            shapes: Vec::new(),
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
            focused: self.focused,
            time: Some(self.time),
            ..Default::default()
        };
        let (layout, content, area, transparent, tweak) = (&self.layout, &mut self.content, self.area, self.transparent, self.tweak);
        let mut full = self.ctx.run_ui(input, |ui| {
            let mut style = DockStyle::from_ui(ui);
            if transparent {
                style.background = Color32::TRANSPARENT;
                style.tab_background = Color32::TRANSPARENT;
            }
            tweak(&mut style);
            ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
                output = DockArea::new(Id::new(AREA)).show_content(ui, layout, &style, content);
            });
        });
        full.textures_delta.clear();
        self.shapes = full.shapes;
        output
    }

    fn step(&mut self, events: Vec<Event>) -> DockOutput<u32> {
        let output = self.frame(events);
        assert!(output.error.is_none(), "{output:?}");
        let permissions = |p: &u32| self.content.permissions(p);
        for action in &output.actions {
            self.layout.apply_with_permissions(action.clone(), permissions).unwrap();
        }
        output
    }

    fn rect(&self, name: &str, panel: u32) -> Rect {
        self.ctx.read_response(Id::new(AREA).with((name, Some(&panel)))).unwrap().rect
    }

    fn tab(&self, panel: u32) -> Rect {
        self.ctx.read_response(Id::new(AREA).with(("tab", &panel))).unwrap().rect
    }

    /// Press at `from`, pass the drag threshold, move to `to`; returns the last move's output.
    fn hold(&mut self, from: egui::Pos2, to: egui::Pos2) -> DockOutput<u32> {
        self.step(vec![Event::PointerMoved(from), button(from, true)]);
        self.step(vec![Event::PointerMoved(from + vec2(8.0, 6.0))]);
        self.step(vec![Event::PointerMoved(to)])
    }

    /// A full drag; returns the release frame's output.
    fn drag(&mut self, from: egui::Pos2, to: egui::Pos2) -> DockOutput<u32> {
        self.hold(from, to);
        self.step(vec![button(to, false)])
    }

    /// Lets egui's double- and triple-click windows pass.
    fn wait(&mut self) {
        for _ in 0..45 {
            self.step(vec![]);
        }
    }

    fn click(&mut self, at: egui::Pos2) -> DockOutput<u32> {
        self.step(vec![Event::PointerMoved(at), button(at, true)]);
        self.step(vec![button(at, false)])
    }
}

fn button(pos: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn key(key: egui::Key) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

fn moved_group(actions: &[Action<u32>]) -> Option<&Target<u32>> {
    actions.iter().find_map(|a| match a {
        Action::MoveGroup { to, .. } => Some(to),
        _ => None,
    })
}

#[test]
fn docked_two_tab_group_drags_by_its_bar_and_keeps_order_and_active_tab() {
    // Onto another group's strip: one MoveGroup, both panels in order, 3 still selected.
    let mut d = Driver::new(frames());
    let bar = d.rect("drag-bar", 2);
    assert!(bar.width() >= 40.0);
    let out = d.drag(bar.center(), d.tab(1).center());
    assert!(matches!(moved_group(&out.actions), Some(Target::At { anchor: 1, placement: Placement::Tab { .. } })), "{:?}", out.actions);
    assert_eq!(out.actions.len(), 1);
    assert!(out.group_drop.is_none(), "a resolved target leaves group_drop empty");
    assert_eq!(d.layout.root, Some(tabs(&[1, 2, 3], 2)));

    // Onto empty space: a floating window at the group's docked size.
    let mut d = Driver::new(frames());
    let border = d.rect("drag-border", 2);
    let bar = d.rect("drag-bar", 2);
    let out = d.drag(bar.center(), pos2(700.0, 200.0));
    let Some(Target::Float { rect: Some([_, _, width, height]) }) = moved_group(&out.actions) else { panic!("{:?}", out.actions) };
    assert_eq!((*width, *height), (border.width(), 600.0));
    assert_eq!(out.group_drop, Some((2, pos2(700.0, 200.0))));
    assert_eq!(d.layout.root, Some(tabs(&[1], 0)));
    assert_eq!(d.layout.floating[0].groups(), vec![Group::new(vec![2, 3], 1)]);

    // The same group in a dock column joins the group below it.
    let mut d = Driver::new(docked());
    let bar = d.rect("drag-bar", 2);
    let out = d.drag(bar.center(), d.tab(4).center());
    assert!(matches!(moved_group(&out.actions), Some(Target::At { anchor: 4, .. })), "{:?}", out.actions);
    let Some(Node::Split { first, .. }) = &d.layout.root else { panic!() };
    let Node::Dock { columns, .. } = first.as_ref() else { panic!() };
    assert_eq!(columns[0].groups.len(), 1);
    assert_eq!(columns[0].groups[0].panels, vec![4, 2, 3]);
    assert_eq!(columns[0].groups[0].active_panel(), Some(&3));
}

#[test]
fn drag_bar_survives_a_strip_full_of_tabs() {
    let layout = Layout { root: Some(tabs(&[1, 2, 3, 4, 5, 6, 7, 8], 0)), floating: Vec::new() };
    let content = TestContent { header: 24.0, ..Default::default() };
    let mut d = Driver::with(layout, content, Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 400.0)));
    let bar = d.rect("drag-bar", 1);
    assert!(bar.width() >= 40.0, "{bar:?}");
    assert_eq!(bar.right(), 300.0 - 24.0, "the bar ends at the header");
    assert_eq!(d.content.strip_width.get(), 300.0, "the header is sized for the whole strip");
    assert!(d.ctx.read_response(Id::new(AREA).with(("overflow", &1u32))).is_some(), "tabs shrink, then overflow, before the bar narrows");
    let out = d.drag(bar.center(), pos2(500.0, 300.0));
    assert!(matches!(moved_group(&out.actions), Some(Target::Float { .. })), "{:?}", out.actions);
    assert_eq!(d.layout.floating[0].groups()[0].panels.len(), 8);
}

/// Panels with ordinary names.
struct Named;

impl DockContent<u32> for Named {
    fn body(&mut self, ui: &mut egui::Ui, _: &u32) {
        ui.label("Body");
    }
    fn logical_label(&self, panel: &u32) -> String {
        if *panel == 1 { "Properties" } else { "Layers" }.into()
    }
}

#[test]
fn narrow_frames_shrink_their_tabs_before_the_rest_overflow() {
    let texts = |d: &testing::DockDriver<u32, Named>| {
        let mut out = Vec::new();
        fn walk(shape: &egui::Shape, out: &mut Vec<egui::epaint::TextShape>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                egui::Shape::Text(text) => out.push(text.clone()),
                _ => {}
            }
        }
        d.shapes().iter().for_each(|clipped| walk(&clipped.shape, &mut out));
        out
    };
    let layout = Layout { root: Some(tabs(&[1, 2], 0)), floating: Vec::new() };
    let area = |width: f32| Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 300.0));

    // Both tabs show, cut to fit, with their labels inside them.
    let d = testing::DockDriver::new(layout.clone(), Named).with_area(area(168.0));
    assert!(d.hit(&Hit::Overflow(1)).is_none(), "no overflow while the tabs fit at their minimum width");
    let min = DockStyle::default().min_tab_width;
    for (panel, name) in [(1, "Properties"), (2, "Layers")] {
        let tab = d.hit(&Hit::Tab(panel)).unwrap_or_else(|| panic!("{name} is hidden"));
        assert!(tab.width() >= min - 0.5, "{name}: {tab:?}");
        let label = texts(&d).into_iter().find(|t| t.galley.text() == name && tab.contains(t.pos)).unwrap_or_else(|| panic!("no {name} label"));
        assert!(tab.contains_rect(label.visual_bounding_rect()), "{name}: {:?} outside {tab:?}", label.visual_bounding_rect());
    }
    let properties = texts(&d).into_iter().find(|t| t.galley.text() == "Properties").unwrap();
    assert!(properties.galley.elided, "the wider tab shrinks to make room");

    // Too narrow for both at their minimum width: the active tab stays, the other is listed.
    let mut d = testing::DockDriver::new(layout, Named).with_area(area(120.0));
    assert!(d.hit(&Hit::Tab(1)).is_some() && d.hit(&Hit::Tab(2)).is_none());
    let overflow = d.hit(&Hit::Overflow(1)).expect("an overflow button");
    d.click(overflow.center());
    d.settle();
    let listed = texts(&d).into_iter().find(|t| t.galley.text() == "Layers").expect("the menu lists Layers");
    assert!(listed.pos.y >= overflow.bottom(), "{listed:?}");
    let out = d.click(listed.visual_bounding_rect().center());
    assert!(out.actions.contains(&Action::Activate { panel: 2 }), "{:?}", out.actions);
}

#[test]
fn top_border_drags_the_group_and_the_menu_button_only_clicks() {
    let content = TestContent { header: 30.0, ..Default::default() };
    let mut d = Driver::with(frames(), content, Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0)));
    let border = d.rect("drag-border", 2);
    assert!(border.height() <= 4.0);
    // The band lies over the tabs and wins there.
    let over_tab = pos2(d.tab(2).center().x, border.center().y);
    let menu = pos2(576.0, d.tab(2).center().y);

    // Pressing ≡ and dragging moves nothing; clicking it reaches the host.
    let held = d.hold(menu, menu + vec2(0.0, 200.0));
    assert!(held.group_drag.is_none());
    let out = d.step(vec![button(menu + vec2(0.0, 200.0), false)]);
    assert!(out.actions.iter().all(|a| !matches!(a, Action::MoveGroup { .. } | Action::Move { .. } | Action::Float { .. })), "{:?}", out.actions);
    // A press in the header focuses the group's shown panel, like a press on its tab.
    let pressed = d.step(vec![Event::PointerMoved(menu), button(menu, true)]);
    assert_eq!(pressed.focused, Some(3));
    d.step(vec![button(menu, false)]);
    assert_eq!(d.content.header_clicks, 1);

    let out = d.drag(over_tab, d.tab(1).center());
    assert!(matches!(moved_group(&out.actions), Some(Target::At { anchor: 1, .. })), "{:?}", out.actions);
    assert_eq!(d.layout.root, Some(tabs(&[1, 2, 3], 2)));
}

#[test]
fn group_drop_on_a_frame_edge_splits_and_on_a_protected_canvas_floats() {
    let canvas = || TestContent { protected: vec![1], ..Default::default() };
    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0));
    let mut d = Driver::with(frames(), TestContent::default(), area);
    let out = d.drag(d.rect("drag-bar", 2).center(), pos2(20.0, 300.0));
    assert!(matches!(moved_group(&out.actions), Some(Target::At { anchor: 1, placement: Placement::Split(Zone::Left) })), "{:?}", out.actions);
    let Some(Node::Split { first, second, .. }) = &d.layout.root else { panic!() };
    assert_eq!(**first, tabs(&[2, 3], 1));
    assert_eq!(**second, tabs(&[1], 0));

    // A protected canvas takes nothing, at its center or its edge: no join, no split, no
    // preview; the group floats there.
    for at in [pos2(150.0, 300.0), pos2(20.0, 300.0)] {
        let mut d = Driver::with(frames(), canvas(), area);
        let out = d.drag(d.rect("drag-bar", 2).center(), at);
        assert!(matches!(moved_group(&out.actions), Some(Target::Float { .. })), "{:?}", out.actions);
        assert_eq!(d.layout.root, Some(tabs(&[1], 0)));
    }
}

#[test]
fn ctrl_or_cmd_held_at_release_always_floats() {
    for modifiers in [Modifiers::CTRL, Modifiers::MAC_CMD, Modifiers::COMMAND] {
        let mut d = Driver::new(frames());
        let target = d.tab(1).center();
        d.hold(d.rect("drag-bar", 2).center(), target);
        d.modifiers = modifiers;
        let out = d.step(vec![button(target, false)]);
        assert!(matches!(moved_group(&out.actions), Some(Target::Float { .. })), "{modifiers:?}: {:?}", out.actions);
        assert_eq!(d.layout.root, Some(tabs(&[1], 0)));

        // A tab drag floats the same way.
        let mut d = Driver::new(frames());
        let target = d.tab(1).center();
        d.hold(d.tab(3).center(), target);
        d.modifiers = modifiers;
        let out = d.step(vec![button(target, false)]);
        assert!(out.actions.iter().any(|a| matches!(a, Action::Float { panel: 3, .. })), "{modifiers:?}: {:?}", out.actions);
    }
}

#[test]
fn group_drag_escape_focus_loss_pointer_exit_and_cancel_drag_leave_the_layout_unchanged() {
    for case in ["escape", "focus", "exit", "cancel"] {
        let mut d = Driver::new(frames());
        let target = d.tab(1).center();
        let held = d.hold(d.rect("drag-bar", 2).center(), target);
        assert_eq!(held.group_drag, Some(2));
        let out = match case {
            "escape" => d.step(vec![key(egui::Key::Escape)]),
            "focus" => {
                d.focused = false;
                let out = d.step(vec![]);
                d.focused = true;
                out
            }
            "exit" => d.step(vec![Event::PointerGone]),
            _ => {
                cancel_drag::<u32>(&d.ctx, Id::new(AREA));
                d.step(vec![Event::PointerMoved(target + vec2(1.0, 0.0))])
            }
        };
        assert!(out.actions.is_empty() && out.group_drop.is_none(), "{case}: {out:?}");
        let out = d.step(vec![Event::PointerMoved(target), button(target, false)]);
        assert!(out.actions.is_empty() && out.group_drop.is_none() && out.group_drag.is_none(), "{case}: {out:?}");
        assert_eq!(d.layout, frames(), "{case}");
        assert!(!is_dragging::<u32>(&d.ctx, Id::new(AREA)), "{case}");
    }
}

#[test]
fn floated_panel_fits_its_content_up_to_floating_size_not_the_column_height() {
    for size in [Some(vec2(200.0, 150.0)), None] {
        let mut layout = docked();
        let Some(Node::Split { first, .. }) = &mut layout.root else { panic!() };
        **first = Node::Dock { side: Side::Start, columns: vec![Column::new(vec![Group::new(vec![2, 3], 1)], 240.0)] };
        let content = TestContent { protected: vec![0], floating_size: size, body_height: Some(200.0), ..Default::default() };
        let mut d = Driver::with(layout, content, Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0)));
        let out = d.drag(d.tab(3).center(), pos2(700.0, 100.0));
        let Some([_, _, width, height]) = out.actions.iter().find_map(|a| match a {
            Action::Float { panel: 3, rect } => Some(*rect),
            _ => None,
        }) else {
            panic!("{:?}", out.actions)
        };
        let fitted = crate::docking::chrome::TITLE_HEIGHT + d.rect("drag-bar", 2).height() + 200.0;
        match size {
            Some(size) => assert_eq!((width, height), (size.x, size.y)),
            None => assert_eq!(height, fitted),
        }
    }
}

#[test]
fn floating_window_is_opaque_with_a_transparent_host_background() {
    let layout = Layout { root: Some(tabs(&[1], 0)), floating: vec![Floating::single(vec![5, 6], 1, [80.0, 70.0, 300.0, 220.0])] };
    let mut d = Driver::new(layout);
    d.transparent = true;
    d.step(vec![]);
    let window = Rect::from_min_size(pos2(80.0, 70.0), vec2(300.0, 220.0));
    let fill = d.ctx.global_style().visuals.window_fill;
    assert_ne!(fill, Color32::TRANSPARENT);
    fn filled(shape: &egui::Shape, rect: Rect, fill: Color32) -> bool {
        match shape {
            egui::Shape::Rect(shape) => shape.rect == rect && shape.fill == fill,
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| filled(shape, rect, fill)),
            _ => false,
        }
    }
    assert!(d.shapes.iter().any(|clipped| filled(&clipped.shape, window, fill)), "no opaque fill under the floating window");
}

#[test]
fn double_click_on_a_floating_tab_or_bar_minimizes_and_another_tab_expands() {
    let layout = Layout { root: Some(tabs(&[1], 0)), floating: vec![Floating::single(vec![5, 6], 1, [80.0, 70.0, 300.0, 220.0])] };
    let mut d = Driver::new(layout);
    let minimized = |d: &Driver| d.layout.floating[0].minimized;
    let tab = d.tab(6).center();
    d.click(tab);
    let out = d.click(tab);
    assert!(out.actions.contains(&Action::SetMinimized { panel: 6, minimized: true }), "{:?}", out.actions);
    assert!(minimized(&d));
    d.wait();

    // Another tab of the minimized group expands it and selects that tab.
    let other = d.tab(5).center();
    d.click(other);
    assert!(!minimized(&d));
    assert_eq!(d.layout.floating[0].groups()[0].active_panel(), Some(&5));

    d.wait();
    let bar = d.rect("drag-bar", 5).center();
    d.click(bar);
    d.click(bar);
    assert!(minimized(&d));
    d.wait();
    d.click(bar);
    d.click(bar);
    assert!(!minimized(&d));
}

#[test]
fn release_over_own_position_is_a_noop() {
    let mut d = Driver::new(frames());
    let bar = d.rect("drag-bar", 2);
    let out = d.drag(bar.center(), bar.center() + vec2(0.0, 200.0));
    assert!(out.actions.is_empty() && out.group_drop.is_none(), "{out:?}");
    assert_eq!(d.layout, frames());
}

#[test]
fn floating_border_and_drag_bar_move_and_cancel_as_a_group() {
    for name in ["drag-border", "drag-bar"] {
        let layout = Layout { root: Some(tabs(&[1], 0)), floating: vec![Floating::single(vec![5, 6], 1, [80.0, 70.0, 300.0, 220.0])] };
        let mut d = Driver::new(layout.clone());
        let from = d.rect(name, 5).center();
        d.hold(from, from + vec2(50.0, 40.0));
        assert_eq!(d.layout, layout, "{name}: a ghost moves, not the window");
        d.step(vec![key(egui::Key::Escape)]);
        d.step(vec![button(from + vec2(50.0, 40.0), false)]);
        assert_eq!(d.layout, layout, "{name}");

        let out = d.drag(from, from + vec2(50.0, 40.0));
        assert!(matches!(moved_group(&out.actions), Some(Target::Float { .. })), "{name}: {:?}", out.actions);
        let [x, y, w, h] = d.layout.floating[0].rect;
        assert_eq!([x, y, w, h], [130.0, 110.0, 300.0, 220.0], "{name}");
        assert_eq!(d.layout.floating[0].groups()[0], Group::new(vec![5, 6], 1));
    }
}

#[test]
fn floating_border_movement_does_not_accumulate_fractional_step_rounding() {
    let layout = Layout { root: Some(tabs(&[1], 0)), floating: vec![Floating::single(vec![5, 6], 1, [80.0, 70.0, 300.0, 220.0])] };
    let mut d = Driver::new(layout);
    let from = d.rect("drag-border", 5).center();
    d.step(vec![Event::PointerMoved(from), button(from, true)]);
    for index in 1..=8 {
        d.step(vec![Event::PointerMoved(from + vec2(80.3, 60.7) * (index as f32 / 8.0))]);
    }
    d.step(vec![button(from + vec2(80.3, 60.7), false)]);
    let [x, y, ..] = d.layout.floating[0].rect;
    assert!((x - 160.3).abs() <= 1.0 && (y - 130.7).abs() <= 1.0, "{x} {y}");
}

#[test]
fn docked_header_drag_reaches_the_host_hook_and_preserves_the_group() {
    for name in ["drag-border", "drag-bar"] {
        let content = TestContent { host_owned: true, ..Default::default() };
        let mut d = Driver::with(frames(), content, Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0)));
        let out = d.drag(d.rect(name, 2).center(), pos2(70.0, 300.0));
        assert!(d.content.whole_group, "{name}");
        assert!(out.actions.is_empty() && out.group_drop.is_none(), "{name}: the host owns the release");
        assert_eq!(d.layout, frames());
    }
}

#[test]
fn a_lone_protected_panel_draws_no_strip_and_no_bar() {
    struct Bare;
    impl DockContent<u32> for Bare {
        fn body(&mut self, _: &mut egui::Ui, _: &u32) {}
        fn permissions(&self, panel: &u32) -> Permissions {
            if *panel == 1 { Permissions::PROTECTED } else { Permissions::default() }
        }
    }
    let ctx = Context::default();
    for _ in 0..2 {
        ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), ..Default::default() }, |ui| {
            DockArea::new(Id::new(AREA)).show_content(ui, &frames(), &DockStyle::from_ui(ui), &mut Bare);
        })
        .textures_delta
        .clear();
    }
    assert!(ctx.read_response(Id::new(AREA).with(("drag-bar", Some(&1u32)))).is_none());
    assert!(ctx.read_response(Id::new(AREA).with(("tab", &1u32))).is_none());
    assert!(ctx.read_response(Id::new(AREA).with(("drag-bar", Some(&2u32)))).is_some());
}

#[test]
fn drag_bar_menu_has_the_group_items_and_opens_from_the_keyboard() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        |ui, (content, actions): &mut (TestContent, Vec<Action<u32>>)| {
            let out = DockArea::new(Id::new(AREA)).show_content(ui, &docked(), &DockStyle::from_ui(ui), content);
            actions.extend(out.actions);
        },
        (TestContent { protected: vec![0], ..Default::default() }, Vec::new()),
    );
    h.run_steps(2);
    let bar_id = Id::new(AREA).with(("drag-bar", Some(&2u32)));
    let at = h.ctx.read_response(bar_id).unwrap().rect.center();
    h.input_mut().events.extend([
        Event::PointerMoved(at),
        Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(1);
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    for label in ["Close Tab Group", "Minimize", "Float Tab Group", "Move Group To"] {
        assert_eq!(h.query_all_by_label(label).count(), 1, "{label}");
    }
    h.get_by_label("Close Tab Group").click();
    h.run_steps(2);
    assert!(h.state().1.contains(&Action::CloseGroup { panel: 3 }), "{:?}", h.state().1);

    h.ctx.memory_mut(|memory| memory.request_focus(bar_id));
    h.run_steps(1);
    assert!(DockArea::<u32>::new(Id::new(AREA)).chrome_has_focus(&h.ctx, &docked()));
    h.input_mut().events.push(key(egui::Key::Enter));
    h.run_steps(2);
    assert!(egui::Popup::is_any_open(&h.ctx));
}

#[test]
fn a_group_dragged_out_of_a_floating_stack_tears_out_alone() {
    let stack = Floating::stack(vec![Group::new(vec![5], 0), Group::new(vec![6, 7], 1)], [80.0, 70.0, 300.0, 400.0]).unwrap();
    let layout = Layout { root: Some(tabs(&[1], 0)), floating: vec![stack] };
    let mut d = Driver::new(layout);
    let from = d.rect("drag-bar", 6).center();
    let out = d.drag(from, pos2(700.0, 300.0));
    assert!(matches!(moved_group(&out.actions), Some(Target::Float { rect: Some([_, _, 300.0, 189.0]) })), "{:?}", out.actions);
    assert_eq!(d.layout.floating.len(), 2);
    assert_eq!(d.layout.floating[0].groups(), vec![Group::new(vec![5], 0)]);
    assert_eq!(d.layout.floating[1].groups(), vec![Group::new(vec![6, 7], 1)]);
}

#[test]
fn a_short_docked_panel_floats_at_its_content_height_up_to_its_floating_size() {
    let title = crate::docking::chrome::TITLE_HEIGHT;
    let float = |body: f32, floating_size: Option<Vec2>| {
        let content = TestContent { body_height: Some(body), floating_size, ..Default::default() };
        let mut d = Driver::with(docked(), content, Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0)));
        let strip = d.rect("drag-bar", 4).height();
        let recorded = crate::docking::float_space::<u32>(&d.ctx, Id::new(AREA)).unwrap().panel(&4).unwrap();
        let target = pos2(400.0, 300.0);
        d.hold(d.tab(4).center(), target);
        d.modifiers = Modifiers::COMMAND;
        let out = d.step(vec![button(target, false)]);
        let Some([_, _, width, height]) = out.actions.iter().find_map(|a| match a {
            Action::Float { panel: 4, rect } => Some(*rect),
            _ => None,
        }) else {
            panic!("{:?}", out.actions)
        };
        assert_eq!(recorded, vec2(width, height), "a command floats at the size a drag does");
        (strip, vec2(width, height))
    };
    let (strip, size) = float(150.0, None);
    assert_eq!(size.y, title + strip + 150.0);
    assert_eq!(float(0.0, None).1.y, 120.0, "an empty panel floats at the least fitted height");
    assert_eq!(float(600.0, None).1.y, 320.0, "a tall docked panel keeps the docked cap");
    // The host's floating size gives the width and the most height.
    assert_eq!(float(150.0, Some(vec2(300.0, 360.0))).1, vec2(300.0, title + strip + 150.0));
    assert_eq!(float(150.0, Some(vec2(300.0, 160.0))).1, vec2(300.0, 160.0));

    // A group takes its active panel's content height.
    let content = TestContent { body_height: Some(150.0), ..Default::default() };
    let mut d = Driver::with(docked(), content, Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0)));
    let target = pos2(400.0, 300.0);
    d.hold(d.rect("drag-bar", 2).center(), target);
    d.modifiers = Modifiers::COMMAND;
    let out = d.step(vec![button(target, false)]);
    let Some(Target::Float { rect: Some([_, _, _, height]) }) = moved_group(&out.actions) else { panic!("{:?}", out.actions) };
    assert_eq!(*height, title + strip + 150.0);
}

#[test]
fn a_panel_that_fills_its_frame_floats_at_its_floating_size_or_320() {
    let float = |floating_size: Option<Vec2>| {
        let content = TestContent { fill: true, floating_size, ..Default::default() };
        let mut d = Driver::with(frames(), content, Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0)));
        let target = pos2(400.0, 300.0);
        d.hold(d.tab(3).center(), target);
        d.modifiers = Modifiers::COMMAND;
        let out = d.step(vec![button(target, false)]);
        out.actions
            .iter()
            .find_map(|a| match a {
                Action::Float { panel: 3, rect } => Some(rect[3]),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{:?}", out.actions))
    };
    assert_eq!(float(None), 320.0);
    assert_eq!(float(Some(vec2(300.0, 400.0))), 400.0);
}

#[test]
fn tab_focus_follows_only_the_users_input() {
    let mut d = Driver::new(docked());
    let tab = Id::new(AREA).with(("tab", &4u32));
    // Keyboard focus as Tab gives it.
    d.ctx.memory_mut(|m| m.request_focus(tab));
    d.step(vec![]);
    assert!(d.ctx.memory(|m| m.has_focus(tab)));

    // A reset that moves the panel takes its focus away and reports no new focus.
    let mut reset = docked();
    let Some(Node::Split { first, .. }) = &mut reset.root else { panic!() };
    let Node::Dock { columns, .. } = first.as_mut() else { panic!() };
    columns[0].groups.reverse();
    d.layout = reset;
    let out = d.step(vec![]);
    assert_eq!(out.focused, None);
    d.step(vec![]);
    assert_eq!(d.ctx.memory(|m| m.focused()), None, "the moved tab gives up keyboard focus");

    // A user drag keeps the dragged tab's focus where the user put it.
    let mut d = Driver::new(docked());
    d.ctx.memory_mut(|m| m.request_focus(tab));
    d.drag(d.tab(4).center(), d.tab(2).center());
    d.step(vec![]);
    assert!(d.layout.place(&4) != docked().place(&4));
    assert!(d.ctx.memory(|m| m.has_focus(tab)), "a tab moved by the user keeps focus");

    // A closed panel's tab gives focus up too.
    let out = d.step(vec![]);
    assert!(out.actions.is_empty());
    d.layout.apply(Action::Close { panel: 4 }).unwrap();
    d.step(vec![]);
    d.step(vec![]);
    assert_eq!(d.ctx.memory(|m| m.focused()), None);
}

#[test]
fn frame_tabs_follow_the_wheel_and_close_on_a_middle_click() {
    let second = |d: &Driver| match &d.layout.root {
        Some(Node::Split { second, .. }) => (**second).clone(),
        _ => panic!("{:?}", d.layout.root),
    };
    let wheel = |unit, y| Event::MouseWheel { unit, delta: vec2(0.0, y), phase: egui::TouchPhase::Move, modifiers: Modifiers::NONE };
    let middle = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Middle, pressed, modifiers: Modifiers::NONE };

    // Off by default.
    let mut d = Driver::new(frames());
    let over = d.tab(2).center();
    d.step(vec![Event::PointerMoved(over), wheel(egui::MouseWheelUnit::Line, 1.0)]);
    d.step(vec![middle(over, true)]);
    d.step(vec![middle(over, false)]);
    assert_eq!(second(&d), tabs(&[2, 3], 1));

    let mut d = Driver::new(frames());
    d.tweak = super::floating_tests::frame_style;
    d.step(vec![]);
    let over = d.tab(2).center();
    d.step(vec![Event::PointerMoved(over), wheel(egui::MouseWheelUnit::Line, 1.0)]);
    assert_eq!(second(&d), tabs(&[2, 3], 0), "wheel up selects the previous tab");
    d.step(vec![wheel(egui::MouseWheelUnit::Line, 1.0)]);
    assert_eq!(second(&d), tabs(&[2, 3], 0), "and stops at the first");
    d.step(vec![wheel(egui::MouseWheelUnit::Point, -30.0)]);
    assert_eq!(second(&d), tabs(&[2, 3], 0), "trackpad scrolling adds up to a notch");
    let out = d.step(vec![wheel(egui::MouseWheelUnit::Point, -30.0)]);
    assert_eq!(second(&d), tabs(&[2, 3], 1));
    assert_eq!(out.focused, Some(3));
    d.step(vec![wheel(egui::MouseWheelUnit::Line, -1e30)]);
    assert_eq!(second(&d), tabs(&[2, 3], 1), "a huge delta stops at the last tab");
    d.step(vec![wheel(egui::MouseWheelUnit::Line, 1e30)]);
    assert_eq!(second(&d), tabs(&[2, 3], 0), "and at the first");

    d.step(vec![middle(over, true)]);
    let out = d.step(vec![middle(over, false)]);
    assert_eq!(out.actions, vec![Action::Close { panel: 2 }]);
    assert_eq!(second(&d), tabs(&[3], 0));
}
