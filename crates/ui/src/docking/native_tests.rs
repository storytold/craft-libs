//! Floating windows in their own OS windows (feature `native-windows`), drawn by the driver's
//! stand-in for OS windows.

use egui::{Color32, Event, Key, Modifiers, PointerButton, Pos2, Rect, ViewportCommand, epaint::ClippedShape, pos2, vec2};

use super::render::{
    APPKIT_CORNER, DragPointer, INACTIVE_PALETTE, MACOS_CLOSE, WindowDrag, appkit_resize_areas, close_rects, drag_opacity, resize_bands, wayland,
};
use super::testing::DockDriver;
use super::{Action, DockContent, Floating, Hit, Layout, Monitor, Node, Permissions, Placement, Target, WindowKind, Workspace};
use crate::layout::{SplitAxis, SplitSize};

/// Desktop position of the main window's content.
const ORIGIN: Pos2 = pos2(100.0, 50.0);

#[derive(Default)]
struct Content {
    /// Panels that may not close.
    fixed: Vec<u32>,
    /// The protected canvas that takes splits beside it, as PhotoCraft's does.
    canvas: Option<u32>,
    /// Panels that stay in-window.
    in_window: Vec<u32>,
    /// Viewer panels: their windows are normal windows.
    normal: Vec<u32>,
    /// The panel with a text field, what it holds, and whether it takes focus next frame.
    text: Option<u32>,
    typed: String,
    focus_text: bool,
    /// The text field gives up keyboard focus on the next frame.
    blur_text: bool,
    /// The panel with a button that takes focus.
    focused_button: Option<u32>,
    /// The panel with a counter button, its clicks, and where it was drawn.
    counter: Option<u32>,
    count: usize,
    counter_rect: Option<Rect>,
    /// Pointer buttons pressed in each panel's body.
    pressed: Vec<(u32, PointerButton)>,
    /// A panel body saw hovered or dropped files.
    saw_files: bool,
    /// Input visible to a selected panel, including native control events while blocked.
    input_probe: Option<u32>,
    events: Vec<Event>,
    raw_events: Vec<Event>,
    /// The panel with an app drag source, and where it was drawn.
    source: Option<u32>,
    /// How the source marks its drag as one that carries something.
    marks: Marks,
    source_rect: Option<Rect>,
    /// The source's drag so far, and how often its drag stopped.
    source_delta: egui::Vec2,
    source_stopped: usize,
    /// How often the source's body saw a primary-button release in its input.
    source_releases: usize,
    /// The source asks egui to run its window's pass again, once, the next time it draws.
    discard_next: bool,
    /// The source asks for that many more passes, drawn or not, before the frame completes.
    discard_passes: usize,
    /// A drop site asks for its window's pass again when it reads a release.
    discard_drop: bool,
    /// Panels whose bodies are app drop sites: where each saw the pointer, and where a release
    /// came with the pointer inside.
    drop_sites: Vec<u32>,
    hovers: Vec<(u32, Pos2)>,
    drops: Vec<(u32, Pos2)>,
    /// Drop sites whose release found egui's drag-and-drop payload.
    payload_drops: Vec<u32>,
    /// Key and clipboard events panel 1 saw in the main window, and the text field's panel in
    /// its own window.
    main_keys: Vec<Event>,
    field_keys: Vec<Event>,
}

/// How a test's app drag source marks its drag.
#[derive(Default)]
enum Marks {
    /// It does not: a plain egui drag, such as a slider's.
    #[default]
    Nothing,
    /// [`super::route_app_drag`] for this area, each frame while it drags.
    Route(egui::Id),
    /// An egui drag-and-drop payload, its panel.
    Payload,
}

impl DockContent<u32> for Content {
    /// Test body that probes native input: records events/raw events, text focus and typing, button
    /// focus, a click counter, drag source state, drop sites and payloads, file hover/drop, and
    /// primary/secondary presses inside the body.
    fn body(&mut self, ui: &mut egui::Ui, panel: &u32) {
        ui.label(format!("panel {panel}"));
        if self.input_probe == Some(*panel) {
            (self.events, self.raw_events) = ui.input(|i| (i.events.clone(), i.raw.events.clone()));
        }
        if self.text == Some(*panel) {
            let field = ui.text_edit_singleline(&mut self.typed);
            if std::mem::take(&mut self.focus_text) {
                field.request_focus();
            }
            if std::mem::take(&mut self.blur_text) {
                field.surrender_focus();
            }
        }
        if self.focused_button == Some(*panel) {
            ui.button("OK").request_focus();
        }
        if self.counter == Some(*panel) {
            let response = ui.button("Add");
            self.count += usize::from(response.clicked());
            self.counter_rect = Some(response.rect);
        }
        if self.source == Some(*panel) {
            let response = ui.allocate_response(vec2(120.0, 60.0), egui::Sense::drag());
            self.source_rect = Some(response.rect);
            self.source_delta += response.drag_delta();
            self.source_stopped += usize::from(response.drag_stopped());
            self.source_releases += usize::from(ui.input(|i| i.pointer.primary_released()));
            let discard = std::mem::take(&mut self.discard_next);
            let more = self.discard_passes > 0;
            if more {
                self.discard_passes -= 1;
            }
            if discard || more {
                ui.ctx().request_discard("the source's pass runs twice");
            }
            match self.marks {
                Marks::Route(area) if response.dragged() => super::route_app_drag(ui.ctx(), area),
                Marks::Payload => response.dnd_set_drag_payload(*panel),
                _ => {}
            }
        }
        // As apps' drop sites do: the pointer over the body, and a release there.
        if self.drop_sites.contains(panel)
            && let Some(pos) = ui.ctx().pointer_hover_pos().filter(|p| ui.max_rect().contains(*p))
        {
            self.hovers.push((*panel, pos));
            if ui.input(|i| i.pointer.any_released()) {
                self.drops.push((*panel, pos));
                if egui::DragAndDrop::has_payload_of_type::<u32>(ui.ctx()) {
                    self.payload_drops.push(*panel);
                }
                if std::mem::take(&mut self.discard_drop) {
                    ui.ctx().request_discard("a drop site's pass runs twice");
                }
            }
        }
        self.saw_files |= ui.input(|i| !i.raw.hovered_files.is_empty() || !i.raw.dropped_files.is_empty());
        let main = ui.ctx().viewport_id() == egui::ViewportId::ROOT;
        if (main && *panel == 1) || (!main && self.text == Some(*panel)) {
            let keys = ui.input(|i| {
                i.events.iter().filter(|e| matches!(e, Event::Key { .. } | Event::Copy | Event::Cut | Event::Paste(_))).cloned().collect::<Vec<_>>()
            });
            (if main { &mut self.main_keys } else { &mut self.field_keys }).extend(keys);
        }
        if ui.rect_contains_pointer(ui.max_rect()) {
            let pressed = [PointerButton::Primary, PointerButton::Secondary].into_iter().filter(|b| ui.input(|i| i.pointer.button_pressed(*b)));
            self.pressed.extend(pressed.map(|b| (*panel, b)));
        }
    }

    fn logical_label(&self, panel: &u32) -> String {
        format!("Panel {panel}")
    }

    fn permissions(&self, panel: &u32) -> Permissions {
        if self.canvas == Some(*panel) {
            return Permissions { accepts_splits: true, ..Permissions::PROTECTED };
        }
        Permissions { closable: !self.fixed.contains(panel), ..Permissions::default() }
    }

    fn allow_native(&self, panel: &u32) -> bool {
        !self.in_window.contains(panel)
    }

    fn window_kind(&self, panel: &u32) -> WindowKind {
        if self.normal.contains(panel) { WindowKind::Normal } else { WindowKind::Palette }
    }
}

fn tabs(panels: &[u32]) -> Node<u32> {
    Node::Tabs { panels: panels.to_vec(), active: 0 }
}

/// Frames `[1, 2]` and `[3, 4]`, and window `[5, 6]` when `floating`.
fn layout(floating: bool) -> Layout<u32> {
    Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::Ratio(0.5),
            first: Box::new(tabs(&[1, 2])),
            second: Box::new(tabs(&[3, 4])),
        }),
        floating: if floating { vec![Floating::single(vec![5, 6], 0, [300.0, 80.0, 240.0, 300.0])] } else { Vec::new() },
    }
}

fn driver(layout: Layout<u32>) -> DockDriver<u32, Content> {
    DockDriver::new(layout, Content::default()).with_style(|s| s.command_press_floats = true).with_native_host(ORIGIN)
}

#[allow(clippy::expect_used)]
fn window(d: &DockDriver<u32, Content>, panel: u32) -> &Floating<u32> {
    d.layout.floating.iter().find(|w| w.contains(&panel)).expect("floating")
}

/// The window's OS window rect, and how many frames it drew.
fn os_window(d: &DockDriver<u32, Content>, panel: u32) -> Option<(Rect, usize)> {
    let viewport = d.viewport(window(d, panel).id);
    d.host().windows.get(&viewport).map(|w| (w.rect, w.frames))
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

#[test]
fn every_float_opens_in_its_own_os_window() {
    // Undock Panel.
    let mut d = driver(layout(false));
    d.layout.apply(Action::Float { panel: 2, rect: [200.0, 100.0, 300.0, 200.0] }).unwrap();
    let out = d.step(vec![]);
    assert!(out.actions.contains(&Action::SetNative { panel: 2, rect: Some([300.0, 150.0, 300.0, 200.0]) }), "{:?}", out.actions);
    assert_eq!(window(&d, 2).native, Some([300.0, 150.0, 300.0, 200.0]));
    assert_eq!(os_window(&d, 2).map(|(rect, _)| rect), Some(Rect::from_min_size(pos2(300.0, 150.0), vec2(300.0, 200.0))));
    assert!(d.hit(&Hit::Title(2)).is_none(), "not drawn in the main window");

    // Undock Panel Group.
    let mut d = driver(layout(false));
    d.layout.apply(Action::MoveGroup { panel: 3, to: Target::Float { rect: Some([20.0, 30.0, 250.0, 200.0]) } }).unwrap();
    d.step(vec![]);
    assert_eq!(window(&d, 3).native, Some([120.0, 80.0, 250.0, 200.0]));
    assert!(os_window(&d, 4).is_some());

    // Ctrl/Cmd-drag.
    let mut d = driver(layout(false));
    let (tab, to) = (d.hit(&Hit::Tab(1)).unwrap().center(), pos2(600.0, 400.0));
    d.modifiers = Modifiers::COMMAND;
    d.hold(tab, to);
    d.modifiers = Modifiers::NONE;
    let out = d.release(to);
    assert!(matches!(out.actions.as_slice(), [Action::Float { panel: 1, .. }]), "{:?}", out.actions);
    d.step(vec![]);
    let [x, y, ..] = window(&d, 1).rect;
    assert_eq!(window(&d, 1).native.map(|r| [r[0], r[1]]), Some([x + ORIGIN.x, y + ORIGIN.y]));
    assert!(os_window(&d, 1).is_some());

    // A restored layout.
    let mut d = driver(layout(true));
    d.settle();
    assert_eq!(window(&d, 5).native, Some([400.0, 130.0, 240.0, 300.0]));
    assert!(os_window(&d, 5).is_some_and(|(_, frames)| frames > 0));

    // A panel that must stay in-window keeps its window there.
    let mut d = driver(layout(false));
    d.content.in_window = vec![5, 6];
    d.layout = layout(true);
    d.settle();
    assert_eq!(window(&d, 5).native, None);
    assert!(d.hit(&Hit::Title(5)).is_some());
    assert!(d.host().windows.is_empty());
}

#[test]
fn closing_the_os_window_closes_its_panels() {
    let mut d = driver(layout(true));
    let viewport = d.viewport(window(&d, 5).id);
    d.host().windows.get_mut(&viewport).unwrap().close = true;
    let out = d.step(vec![]);
    assert!(out.actions.contains(&Action::CloseWindow { panel: 5 }), "{:?}", out.actions);
    assert!(!d.layout.contains(&5) && !d.layout.contains(&6));
    let frames = d.host().windows[&viewport].frames;
    d.settle();
    assert_eq!(d.host().windows[&viewport].frames, frames, "no longer drawn");

    // The bar's × does the same.
    let mut d = driver(layout(true));
    let id = window(&d, 5).id;
    let out = d.window_click(id, pos2(230.0, 9.0));
    assert!(out.actions.contains(&Action::CloseWindow { panel: 5 }), "{:?}", out.actions);
}

#[test]
fn the_os_close_of_a_window_that_may_not_close_is_cancelled() {
    // A panel that may not close, or a locked area.
    for locked in [false, true] {
        let mut d = driver(layout(true));
        if locked {
            d.locked = true;
        } else {
            d.content.fixed = vec![6];
        }
        let viewport = d.viewport(window(&d, 5).id);
        let sent = d.host().commands.len();
        d.host().windows.get_mut(&viewport).unwrap().close = true;
        let out = d.step(vec![]);
        assert!(!out.actions.iter().any(|a| matches!(a, Action::CloseWindow { .. })), "locked {locked}: {:?}", out.actions);
        assert!(out.error.is_none(), "locked {locked}: {:?}", out.error);
        assert!(d.host().commands[sent..].contains(&(viewport, ViewportCommand::CancelClose)), "locked {locked}: cancelled");
        assert!(d.layout.contains(&5) && d.layout.contains(&6));
    }
}

#[test]
fn a_window_keeps_its_id_and_os_window_when_its_first_panel_moves_out() {
    let mut d = driver(layout(true));
    let id = window(&d, 5).id;
    let (_, frames) = os_window(&d, 6).unwrap();
    d.layout.apply(Action::Move { panel: 5, anchor: 1, placement: Placement::Tab { before: None } }).unwrap();
    d.settle();
    assert_eq!(window(&d, 6).id, id);
    assert_eq!(d.host().windows.len(), 1, "the same OS window");
    assert!(os_window(&d, 6).unwrap().1 > frames);

    // Moving a whole window keeps its id; merging into another keeps the receiver's.
    let mut layout = layout(true);
    layout.floating.push(Floating::single(vec![7], 0, [10.0, 10.0, 200.0, 200.0]));
    let (first, second) = (layout.floating[0].id, layout.floating[1].id);
    assert_ne!(first, second);
    layout.apply(Action::MoveWindow { panel: 5, to: Target::Float { rect: Some([0.0, 0.0, 300.0, 300.0]) } }).unwrap();
    assert_eq!(layout.floating.iter().find(|w| w.contains(&5)).unwrap().id, first);
    layout.apply(Action::Float { panel: 7, rect: [50.0, 50.0, 200.0, 200.0] }).unwrap();
    assert_eq!(layout.floating.iter().find(|w| w.contains(&7)).unwrap().id, second);
    layout.apply(Action::MoveWindow { panel: 7, to: Target::At { anchor: 5, placement: Placement::Tab { before: None } } }).unwrap();
    assert_eq!(layout.floating.len(), 1);
    assert_eq!(layout.floating[0].id, first);
    // A group split out of a window makes a new one.
    layout.apply(Action::MovePanel { panel: 7, to: Target::Float { rect: Some([0.0, 0.0, 200.0, 200.0]) } }).unwrap();
    let ids: Vec<u64> = layout.floating.iter().map(|w| w.id).collect();
    assert!(ids.contains(&first) && ids.len() == 2 && ids[0] != ids[1], "{ids:?}");
}

#[test]
fn a_moved_window_reports_its_place_and_monitor() {
    let mut d = driver(layout(true));
    d.monitors = vec![Monitor { name: Some("left".into()), rect: [0.0, 0.0, 1920.0, 1080.0], scale: 1.0 }];
    let viewport = d.viewport(window(&d, 5).id);
    d.host().windows.get_mut(&viewport).unwrap().rect = Rect::from_min_size(pos2(700.0, 200.0), vec2(240.0, 300.0));
    let out = d.step(vec![]);
    assert!(out.actions.contains(&Action::SetNative { panel: 5, rect: Some([700.0, 200.0, 240.0, 300.0]) }), "{:?}", out.actions);
    assert_eq!(window(&d, 5).rect, [600.0, 150.0, 240.0, 300.0], "the in-window rect follows");
    assert_eq!(window(&d, 5).monitor.as_ref().and_then(|m| m.name.as_deref()), Some("left"));

    // Without a monitor list, the size egui reports.
    let mut d = driver(layout(true));
    d.host().monitor_size = Some(vec2(2560.0, 1440.0));
    d.settle();
    assert_eq!((window(&d, 5).monitor.clone(), window(&d, 5).monitor_size), (None, Some([2560.0, 1440.0])));
}

#[test]
fn a_restored_window_opens_on_a_connected_monitor() {
    let left = Monitor { name: Some("left".into()), rect: [0.0, 0.0, 1920.0, 1080.0], scale: 1.0 };
    let right = Monitor { name: Some("right".into()), rect: [1920.0, 0.0, 2560.0, 1440.0], scale: 2.0 };
    let mut saved = layout(true);
    saved.floating[0].native = Some([3000.0, 100.0, 240.0, 300.0]);
    saved.floating[0].monitor = Some(right.clone());

    // Its monitor is connected: where it was.
    let mut d = DockDriver::new(Layout::default(), Content::default()).with_native_host(ORIGIN);
    d.monitors = vec![left.clone(), right];
    d.layout = saved.clone();
    d.settle();
    assert_eq!(os_window(&d, 5).unwrap().0.min, pos2(3000.0, 100.0));

    // Unplugged: the main window's monitor, at the same offset.
    let mut d = DockDriver::new(Layout::default(), Content::default()).with_native_host(ORIGIN);
    d.monitors = vec![left];
    d.layout = saved;
    d.settle();
    assert_eq!(os_window(&d, 5).unwrap().0.min, pos2(1080.0, 100.0));
    assert_eq!(window(&d, 5).native, Some([1080.0, 100.0, 240.0, 300.0]));
    assert_eq!(window(&d, 5).monitor.as_ref().and_then(|m| m.name.as_deref()), Some("left"));
}

/// The last `InnerSize`, `MinInnerSize` and `MaxInnerSize` sent to `viewport`.
fn size_commands(d: &DockDriver<u32, Content>, viewport: egui::ViewportId) -> [Option<egui::Vec2>; 3] {
    let mut sizes = [None; 3];
    for (_, c) in d.host().commands.iter().filter(|(v, _)| *v == viewport) {
        match c {
            ViewportCommand::InnerSize(s) => sizes[0] = Some(*s),
            ViewportCommand::MinInnerSize(s) => sizes[1] = Some(*s),
            ViewportCommand::MaxInnerSize(s) => sizes[2] = Some(*s),
            _ => {}
        }
    }
    sizes
}

#[test]
fn a_minimized_native_window_shrinks_to_its_bar_and_keeps_its_expanded_size() {
    let mut d = driver(layout(true));
    let viewport = d.viewport(window(&d, 5).id);
    let open = os_window(&d, 5).unwrap().0;
    let (native, rect) = (window(&d, 5).native, window(&d, 5).rect);
    assert_eq!(native, Some([open.left(), open.top(), 240.0, 300.0]));

    // Its bar and its group's tab strip, at the same width and top-left corner; only the width
    // resizes.
    let collapsed = BAR + super::DockStyle::default().tab_height;
    d.layout.apply(Action::SetMinimized { panel: 5, minimized: true }).unwrap();
    d.settle();
    assert_eq!(os_window(&d, 5).unwrap().0, Rect::from_min_size(open.min, vec2(240.0, collapsed)));
    let [_, min, max] = size_commands(&d, viewport);
    assert_eq!((min.map(|s| s.y), max.map(|s| s.y)), (Some(collapsed), Some(collapsed)));

    // The layout keeps the expanded size after the window reports the collapsed one, past the
    // wait for commanded moves, and takes a new width.
    for _ in 0..40 {
        d.step(vec![]);
    }
    assert_eq!((window(&d, 5).native, window(&d, 5).rect), (native, rect));
    d.host().windows.get_mut(&viewport).unwrap().rect = Rect::from_min_size(open.min, vec2(300.0, collapsed));
    for _ in 0..40 {
        d.step(vec![]);
    }
    assert_eq!(window(&d, 5).native, Some([open.left(), open.top(), 300.0, 300.0]));
    assert_eq!(window(&d, 5).rect[2..], [300.0, 300.0]);

    // Expanded: the saved height comes back, and the height resizes again.
    d.layout.apply(Action::SetMinimized { panel: 5, minimized: false }).unwrap();
    d.settle();
    assert_eq!(os_window(&d, 5).unwrap().0, Rect::from_min_size(open.min, vec2(300.0, 300.0)));
    assert_eq!(size_commands(&d, viewport), [Some(vec2(300.0, 300.0)), Some(vec2(160.0, 100.0)), Some(egui::Vec2::INFINITY)]);
    // The limits change before the size: the OS clamps a new size to the limits it has.
    let commands: Vec<_> = d.host().commands.iter().filter(|(v, _)| *v == viewport).map(|(_, c)| c.clone()).collect();
    let last = |f: fn(&ViewportCommand) -> bool| commands.iter().rposition(f);
    assert!(last(|c| matches!(c, ViewportCommand::MaxInnerSize(_))) < last(|c| matches!(c, ViewportCommand::InnerSize(_))), "{commands:?}");
    assert_eq!(window(&d, 5).native, Some([open.left(), open.top(), 300.0, 300.0]));
}

#[test]
fn a_locked_area_moves_no_native_window_and_records_where_the_os_put_it() {
    let mut d = driver(layout(true));
    d.locked = true;
    let id = window(&d, 5).id;
    let viewport = d.viewport(id);
    let sent = d.host().commands.len();
    d.window_step(id, vec![Event::PointerMoved(pos2(40.0, 9.0)), button(pos2(40.0, 9.0), true)]);
    d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 19.0))]);
    let out = d.window_step(id, vec![Event::PointerMoved(pos2(70.0, 9.0)), button(pos2(70.0, 9.0), false)]);
    assert!(out.actions.is_empty(), "{:?}", out.actions);
    let moves = d.host().commands[sent..]
        .iter()
        .filter(|(v, c)| *v == viewport && matches!(c, ViewportCommand::OuterPosition(_) | ViewportCommand::StartDrag))
        .count();
    assert_eq!(moves, 0, "the bar is inert");

    // The window system moves the window anyway: the locked workspace takes its new place.
    let mut workspace = Workspace { locked: true, ..Workspace::new(d.layout.clone()) };
    d.host().windows.get_mut(&viewport).unwrap().rect = Rect::from_min_size(pos2(700.0, 200.0), vec2(240.0, 300.0));
    for _ in 0..60 {
        let out = d.step(vec![]);
        assert!(!out.actions.iter().any(|a| matches!(a, Action::MoveFloating { .. })), "{:?}", out.actions);
        workspace.apply_all(out.actions, |_| Permissions::default()).unwrap();
    }
    assert_eq!(workspace.layout.floating[0].native, Some([700.0, 200.0, 240.0, 300.0]));
}

#[test]
fn dragging_the_bar_moves_the_os_window() {
    let mut d = driver(layout(true));
    let id = window(&d, 5).id;
    let viewport = d.viewport(id);
    d.window_step(id, vec![Event::PointerMoved(pos2(40.0, 9.0)), button(pos2(40.0, 9.0), true)]);
    // Each move is relative to where the window was last moved.
    d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 19.0))]);
    d.window_step(id, vec![Event::PointerMoved(pos2(70.0, 9.0))]);
    let moves: Vec<Pos2> = d
        .host()
        .commands
        .iter()
        .filter_map(|(v, c)| match c {
            ViewportCommand::OuterPosition(pos) if *v == viewport => Some(*pos),
            _ => None,
        })
        .collect();
    assert_eq!(moves.last(), Some(&pos2(450.0, 140.0)), "{moves:?}");
    // Past the main window's right edge, the desktop: released there, it stays.
    d.window_step(id, vec![Event::PointerMoved(pos2(540.0, 9.0))]);
    let out = d.window_step(id, vec![button(pos2(40.0, 9.0), false)]);
    assert!(out.actions.iter().all(|a| matches!(a, Action::SetNative { .. } | Action::MoveFloating { .. })), "{:?}", out.actions);
    d.step(vec![]);
    assert_eq!(window(&d, 5).native, Some([950.0, 140.0, 240.0, 300.0]));

    // Without a reported position (Wayland) the window system moves it.
    let mut d = driver(layout(true));
    d.host().no_position = true;
    d.step(vec![]);
    d.window_step(id_of(&d), vec![Event::PointerMoved(pos2(40.0, 9.0)), button(pos2(40.0, 9.0), true)]);
    d.window_step(id_of(&d), vec![Event::PointerMoved(pos2(60.0, 19.0))]);
    assert!(d.host().commands.iter().any(|(_, c)| *c == ViewportCommand::StartDrag));
}

fn id_of(d: &DockDriver<u32, Content>) -> u64 {
    window(d, 5).id
}

#[test]
fn palettes_stay_shown_at_the_normal_level_while_the_app_is_inactive() {
    use egui::{ViewportId, WindowLevel};
    let mut d = driver(layout(true));
    let palette = d.viewport(window(&d, 5).id);
    let focus = |d: &mut DockDriver<u32, Content>, window: Option<ViewportId>, frames: usize| {
        d.host().focus = window;
        for _ in 0..frames {
            d.step(vec![]);
        }
    };
    let level = |d: &DockDriver<u32, Content>| d.host().windows[&palette].level;
    let told = |d: &DockDriver<u32, Content>| d.host().commands.iter().any(|(v, c)| *v == palette && matches!(c, ViewportCommand::Visible(_)));
    assert_eq!(level(&d), Some(WindowLevel::AlwaysOnTop));
    // Focus moving to the palette, with a frame of none between, keeps it on top.
    focus(&mut d, None, 1);
    focus(&mut d, Some(palette), 30);
    assert_eq!(level(&d), Some(WindowLevel::AlwaysOnTop));
    // Another application: the palette drops behind it after the delay, and shows on, so
    // files dragged from that application can reach it.
    focus(&mut d, None, 10);
    assert_eq!(level(&d), Some(WindowLevel::AlwaysOnTop), "not yet");
    focus(&mut d, None, 10);
    assert_eq!(level(&d), Some(INACTIVE_PALETTE));
    // Back: on top again.
    focus(&mut d, Some(ViewportId::ROOT), 1);
    assert_eq!(level(&d), Some(WindowLevel::AlwaysOnTop));
    // Blocked by a modal, it stays at the normal level, away and back.
    d.input_blocked = true;
    focus(&mut d, None, 20);
    assert_eq!(level(&d), Some(WindowLevel::Normal));
    focus(&mut d, Some(ViewportId::ROOT), 5);
    assert_eq!(level(&d), Some(WindowLevel::Normal));
    assert!(d.host().windows[&palette].visible);
    assert!(!told(&d), "{:?}", d.host().commands);
}

#[test]
fn viewer_windows_stay_shown_at_normal_level() {
    let mut layout = layout(false);
    layout.floating = vec![
        Floating::single(vec![5], 0, [300.0, 80.0, 240.0, 300.0]),
        Floating::single(vec![6], 0, [560.0, 80.0, 240.0, 300.0]),
        // A palette in front of a viewer's tab: the viewer makes the window normal.
        Floating::single(vec![7, 8], 0, [820.0, 80.0, 240.0, 300.0]),
    ];
    let mut d = driver(layout);
    d.content.normal = vec![5, 8];
    d.step(vec![]);
    let [viewer, palette, mixed] = [5, 6, 7].map(|p| d.viewport(window(&d, p).id));
    let level = |d: &DockDriver<u32, Content>, v| d.host().windows[&v].level;
    assert_eq!(level(&d, viewer), Some(egui::WindowLevel::Normal));
    assert_eq!(level(&d, palette), Some(egui::WindowLevel::AlwaysOnTop));
    assert_eq!(level(&d, mixed), Some(egui::WindowLevel::Normal));
    // Another application: the palette drops behind it; the viewers stay at the normal level.
    d.host().focus = None;
    for _ in 0..20 {
        d.step(vec![]);
    }
    assert_eq!(
        [viewer, palette, mixed].map(|v| level(&d, v)),
        [Some(egui::WindowLevel::Normal), Some(INACTIVE_PALETTE), Some(egui::WindowLevel::Normal)]
    );
    d.host().focus = Some(egui::ViewportId::ROOT);
    d.step(vec![]);
    assert_eq!(level(&d, palette), Some(egui::WindowLevel::AlwaysOnTop));
    assert_eq!(level(&d, viewer), Some(egui::WindowLevel::Normal));
    assert!(!d.host().commands.iter().any(|(_, c)| matches!(c, ViewportCommand::Visible(_))), "no window hides");
}

#[test]
fn the_app_comes_forward_with_its_viewers_when_it_is_activated() {
    use egui::{ViewportId, WindowLevel};
    let start = || {
        let mut layout = layout(false);
        layout.floating = vec![
            Floating::single(vec![5], 0, [300.0, 80.0, 240.0, 300.0]),
            Floating::single(vec![6], 0, [560.0, 80.0, 240.0, 300.0]),
            Floating::single(vec![7], 0, [820.0, 80.0, 240.0, 300.0]),
        ];
        let mut d = driver(layout);
        d.content.normal = vec![5, 7];
        d.step(vec![]);
        let [back, palette, front] = [5, 6, 7].map(|p| d.viewport(window(&d, p).id));
        (d, back, palette, front)
    };
    let focus = |d: &mut DockDriver<u32, Content>, window: Option<ViewportId>, frames: usize| {
        d.host().focus = window;
        for _ in 0..frames {
            d.step(vec![]);
        }
    };
    let levels = |d: &mut DockDriver<u32, Content>| -> Vec<(ViewportId, WindowLevel)> {
        let mut host = d.host();
        assert!(!host.commands.iter().any(|(_, c)| matches!(c, ViewportCommand::Focus)), "no window takes focus");
        let levels = host.commands.iter().filter_map(|(v, c)| if let ViewportCommand::WindowLevel(l) = c { Some((*v, *l)) } else { None }).collect();
        host.commands.clear();
        levels
    };
    // Each window once, in this order: the main window's commands are the root's.
    let raised = |order: &[ViewportId]| -> Vec<(ViewportId, WindowLevel)> {
        if cfg!(target_os = "macos") {
            return Vec::new();
        }
        order.iter().flat_map(|v| [(*v, WindowLevel::AlwaysOnTop), (*v, WindowLevel::Normal)]).collect()
    };
    const MAIN: ViewportId = ViewportId::ROOT;

    let (mut d, back, palette, front) = start();
    // Focus moving between the app's own windows, with a frame of none between two, raises
    // nothing.
    focus(&mut d, Some(back), 2);
    focus(&mut d, None, 1);
    focus(&mut d, Some(front), 2);
    focus(&mut d, Some(MAIN), 2);
    assert_eq!(levels(&mut d), []);
    // Another application, then back through the main window: the viewers are raised back to
    // front and the main window last, without focus; the palette only goes back on top.
    focus(&mut d, None, 20);
    focus(&mut d, Some(MAIN), 10);
    assert_eq!(levels(&mut d), raised(&[back, front, MAIN]));
    assert!(d.host().windows[&palette].visible);
    // Back through the back viewer: it is raised last, so it stays in front, and the main
    // window comes forward behind it.
    focus(&mut d, None, 20);
    focus(&mut d, Some(back), 10);
    assert_eq!(levels(&mut d), raised(&[front, MAIN, back]));
    // A queued viewer that turns into a palette before its turn is not raised: the raise
    // would leave it at the normal level.
    focus(&mut d, None, 20);
    focus(&mut d, Some(MAIN), 1);
    d.content.normal = vec![7];
    focus(&mut d, Some(MAIN), 10);
    assert_eq!(levels(&mut d), raised(&[front, MAIN]));

    // A press in the main window brings it in front of the viewers that opened over it, though
    // it kept focus.
    let (mut d, back, _, front) = start();
    d.click(pos2(100.0, 300.0));
    levels(&mut d);
    focus(&mut d, None, 20);
    focus(&mut d, Some(back), 10);
    assert_eq!(levels(&mut d), raised(&[front, MAIN, back]));

    // While a modal blocks input, the main window with the modal is raised last, even when
    // the app is activated through a viewer.
    d.input_blocked = true;
    focus(&mut d, None, 20);
    focus(&mut d, Some(front), 10);
    assert_eq!(levels(&mut d), raised(&[back, front, MAIN]));
}

#[test]
fn unused_keys_in_a_native_window_reach_the_app() {
    let mut d = driver(layout(true));
    let id = window(&d, 5).id;
    let key = Event::Key { key: Key::S, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
    d.modifiers = Modifiers::COMMAND;
    let out = d.window_step(id, vec![key.clone(), Event::Text("s".into()), Event::Copy, Event::PointerMoved(pos2(50.0, 50.0))]);
    d.modifiers = Modifiers::NONE;
    assert_eq!(out.forwarded_events, vec![key, Event::Text("s".into()), Event::Copy]);
    assert_eq!(out.forwarded_modifiers, Some(Modifiers::COMMAND));
    // Keys of the main window stay its own.
    let out = d.key(Key::A);
    assert!(out.forwarded_events.is_empty());
}

#[cfg(feature = "serde")]
#[test]
fn a_native_window_saves_its_id_place_and_monitor() {
    let mut layout = layout(true);
    let monitor = Monitor { name: Some("DP-1".into()), rect: [-1920.0, 0.0, 1920.0, 1080.0], scale: 1.5 };
    layout.floating[0].native = Some([-1500.0, 100.0, 240.0, 300.0]);
    layout.floating[0].monitor = Some(monitor);
    layout.floating.push(Floating {
        monitor_size: Some([2560.0, 1440.0]),
        native: Some([5.0, 5.0, 300.0, 200.0]),
        ..Floating::single(vec![7], 0, [0.0, 0.0, 300.0, 200.0])
    });
    let json = serde_json::to_string(&layout).unwrap();
    let back: Layout<u32> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, layout);
    assert_eq!(back.floating.iter().map(|w| w.id).collect::<Vec<_>>(), layout.floating.iter().map(|w| w.id).collect::<Vec<_>>());

    // Old layouts get fresh ids, and copies of one id unique ones.
    let old = r#"{"root": null, "floating": [{"panels": [5], "active": 0, "rect": [0, 0, 100, 100]}, {"panels": [6], "active": 0, "rect": [0, 0, 100, 100]}]}"#;
    let old: Layout<u32> = serde_json::from_str(old).unwrap();
    assert_ne!(old.floating[0].id, old.floating[1].id);
    let mut copies = layout.clone();
    copies.floating[1].id = copies.floating[0].id;
    let copies: Layout<u32> = serde_json::from_str(&serde_json::to_string(&copies).unwrap()).unwrap();
    assert_eq!(copies.floating[0].id, layout.floating[0].id);
    assert_ne!(copies.floating[1].id, copies.floating[0].id);
    // An in-window window writes its id too.
    let in_window = Floating::single(vec![1u32], 0, [0.0, 0.0, 9.0, 9.0]).with_id(3);
    assert!(serde_json::to_string(&in_window).unwrap().contains(r#""id":3"#));
}

/// Height of the bar at the top of a native window.
const BAR: f32 = 18.0;
/// A tab row's middle in a native window, below the bar.
const TAB_ROW: f32 = BAR + 13.0;

/// Window `[5, 6]` of [`layout`] and window `[7]` below it, apart on the desktop.
fn two_windows() -> Layout<u32> {
    let mut layout = layout(true);
    layout.floating.push(Floating::single(vec![7], 0, [20.0, 420.0, 240.0, 150.0]));
    layout
}

fn in_root(d: &DockDriver<u32, Content>, panel: u32) -> bool {
    d.layout.contains(&panel) && !d.layout.floating.iter().any(|w| w.contains(&panel))
}

#[test]
fn a_tab_dragged_past_the_main_windows_edge_floats_at_the_desktop_point() {
    let mut d = driver(layout(false));
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    // Left of the main window, which starts at x = 100 on the desktop.
    let out = pos2(-60.0, 120.0);
    d.hold(tab, out);
    // The pointer leaves the window: with native windows that is no release.
    let gone = d.leave();
    assert!(gone.actions.is_empty(), "{:?}", gone.actions);
    let released = d.release(out);
    assert!(matches!(released.actions.as_slice(), [Action::Float { panel: 1, .. }]), "{:?}", released.actions);
    d.step(vec![]);
    // The point grabbed (where the drag began, past the nudge) stays under the pointer: the
    // tab's group starts at the window's top-left, below the bar.
    let (desktop, grab) = (ORIGIN + out.to_vec2(), tab + vec2(8.0, 6.0));
    let min = pos2(desktop.x - grab.x, desktop.y - grab.y - BAR);
    let native = window(&d, 1).native.unwrap();
    assert_eq!(pos2(native[0], native[1]), min);
    assert_eq!(os_window(&d, 1).unwrap().0.min, min);
}

#[test]
fn a_tab_dragged_out_and_back_docks_in_the_main_window() {
    let mut d = driver(layout(false));
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    d.hold(tab, pos2(-60.0, 120.0));
    d.leave();
    // Back over the middle of frame [3, 4].
    let back = pos2(600.0, 300.0);
    d.move_to(back);
    let out = d.release(back);
    assert!(matches!(out.actions.as_slice(), [Action::Move { panel: 1, anchor: 3 | 4, .. }]), "{:?}", out.actions);
    assert!(d.layout.floating.is_empty());
}

#[test]
fn a_tab_from_a_native_window_docks_in_a_frame_of_the_main_window() {
    let mut d = driver(layout(true));
    let id = window(&d, 5).id;
    // Frame [1, 2] at x = 100 in the main window, on the desktop.
    let local = d.window_local(id, ORIGIN + vec2(100.0, 300.0)).unwrap();
    d.window_hold(id, pos2(15.0, TAB_ROW), local);
    let out = d.window_release(id, local);
    assert!(matches!(out.actions.as_slice(), [Action::Move { panel: 5, anchor: 1 | 2, .. }]), "{:?}", out.actions);
    assert!(in_root(&d, 5));
    assert!(window(&d, 6).contains(&6));
}

#[test]
fn a_tab_from_a_native_window_joins_another_native_windows_tab_row() {
    let mut d = driver(two_windows());
    let (from, to) = (window(&d, 5).id, window(&d, 7).id);
    let row = d.host().windows[&d.viewport(to)].rect.min + vec2(200.0, TAB_ROW);
    let local = d.window_local(from, row).unwrap();
    d.window_hold(from, pos2(15.0, TAB_ROW), local);
    let out = d.window_release(from, local);
    assert!(matches!(out.actions.as_slice(), [Action::Move { panel: 5, anchor: 7, placement: Placement::Tab { .. } }]), "{:?}", out.actions);
    assert!(window(&d, 7).contains(&5));
}

#[test]
fn dragging_the_bar_onto_the_main_window_docks_the_whole_window() {
    let mut d = driver(layout(true));
    let id = window(&d, 5).id;
    d.window_step(id, vec![Event::PointerMoved(pos2(40.0, 9.0)), button(pos2(40.0, 9.0), true)]);
    d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 19.0))]);
    // The pointer over the tab row of frame [3, 4], which starts at x = 400 in the main window.
    // A moving window joins a group there; it splits only groups of other floating windows.
    let local = d.window_local(id, ORIGIN + vec2(450.0, 13.0)).unwrap();
    d.window_step(id, vec![Event::PointerMoved(local)]);
    let out = d.window_step(id, vec![button(pos2(40.0, 9.0), false)]);
    assert!(out.actions.iter().any(|a| matches!(a, Action::MoveWindow { panel: 5, to: Target::At { anchor: 3 | 4, .. } })), "{:?}", out.actions);
    assert!(d.layout.floating.is_empty());
    assert!(in_root(&d, 5) && in_root(&d, 6));
}

#[test]
fn dragging_the_only_tab_of_a_native_window_moves_the_window() {
    let mut layout = layout(false);
    layout.floating.push(Floating::single(vec![7], 0, [300.0, 80.0, 240.0, 200.0]));
    let mut d = driver(layout);
    let id = window(&d, 7).id;
    d.window_hold(id, pos2(15.0, TAB_ROW), pos2(1000.0, TAB_ROW));
    let out = d.window_release(id, pos2(15.0, TAB_ROW));
    assert!(!out.actions.iter().any(|a| matches!(a, Action::Float { .. } | Action::Move { .. })), "{:?}", out.actions);
    d.step(vec![]);
    assert_eq!(d.layout.floating.len(), 1);
    assert_eq!(window(&d, 7).id, id, "the same window");
    // It followed the pointer onto the desktop, the pressed point under it: the nudge moved it
    // by (8, 6), then the pointer went 1000 - 15 points right of the press.
    assert_eq!(window(&d, 7).native.map(|r| [r[0], r[1]]), Some([408.0 + 985.0, 136.0]));
}

#[test]
fn leaving_the_window_ends_a_drag_only_with_embedded_viewports() {
    // Native viewports: the release still arrives, so leaving is no drop.
    let mut d = driver(layout(false));
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    d.hold(tab, pos2(790.0, 300.0));
    let out = d.leave();
    assert!(out.actions.is_empty(), "{:?}", out.actions);
    assert!(super::is_dragging::<u32>(&d.ctx, d.id));
    d.step(vec![]);
    assert!(super::is_dragging::<u32>(&d.ctx, d.id));

    // Embedded: leaving floats the panel at the edge, as before.
    let mut d = DockDriver::new(layout(false), Content::default());
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    d.hold(tab, pos2(790.0, 300.0));
    let out = d.leave();
    assert!(matches!(out.actions.as_slice(), [Action::Float { panel: 1, .. }]), "{:?}", out.actions);
}

#[test]
fn escape_cancels_a_drag_outside_the_window() {
    let mut d = driver(layout(false));
    let before = d.layout.clone();
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    d.hold(tab, pos2(-60.0, 120.0));
    d.leave();
    d.key(Key::Escape);
    let out = d.release(pos2(-60.0, 120.0));
    assert!(out.actions.is_empty(), "{:?}", out.actions);
    assert_eq!(d.layout, before);
}

/// Shapes drawn in `color`: a drop preview's outline, or a fill.
fn accented(shapes: &[ClippedShape], color: Color32) -> usize {
    shapes
        .iter()
        .filter(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) => rect.stroke.color == color || rect.fill == color,
            egui::Shape::Path(path) => path.fill == color.gamma_multiply(0.25),
            _ => false,
        })
        .count()
}

#[test]
fn the_preview_is_drawn_in_the_window_under_the_pointer() {
    let accent = Color32::from_rgb(250, 10, 200);
    let mut d = DockDriver::new(two_windows(), Content::default())
        .with_style(move |s| {
            s.accent = accent;
            s.float_hint = false;
        })
        .with_native_host(ORIGIN);
    let to = window(&d, 7).id;
    let viewport = d.viewport(to);
    let (main_before, window_before) = (accented(d.shapes(), accent), accented(&d.host().windows[&viewport].shapes, accent));
    // Tab 1 of the main window over window [7]'s body.
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    let body = d.host().windows[&viewport].rect.center() - ORIGIN.to_vec2();
    d.hold(tab, body);
    d.move_to(body);
    assert!(accented(&d.host().windows[&viewport].shapes, accent) > window_before, "drawn in the destination");
    assert_eq!(accented(d.shapes(), accent), main_before, "not in the source");
    let out = d.release(body);
    assert!(matches!(out.actions.as_slice(), [Action::Move { panel: 1, anchor: 7, .. }]), "{:?}", out.actions);
}

/// The texts drawn in `shapes`.
fn texts(shapes: &[ClippedShape]) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
            _ => {}
        }
    }
    let mut out = Vec::new();
    shapes.iter().for_each(|clipped| walk(&clipped.shape, &mut out));
    out
}

#[test]
fn the_undock_hint_is_drawn_in_the_window_under_the_pointer() {
    let key = if egui::os::OperatingSystem::from_target_os() == egui::os::OperatingSystem::Mac { "⌘" } else { "Ctrl" };
    let hint = format!("Hold {key} to undock");
    let hinted = |shapes: &[ClippedShape]| texts(shapes).contains(&hint);
    let driver = || {
        DockDriver::new(two_windows(), Content::default())
            .with_style(|s| {
                s.command_press_floats = true;
                s.float_hint = true;
            })
            .with_native_host(ORIGIN)
    };

    // Tab 1 of the main window over window [7]'s body.
    let mut d = driver();
    let viewport = d.viewport(window(&d, 7).id);
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    let body = d.host().windows[&viewport].rect.center() - ORIGIN.to_vec2();
    d.hold(tab, body);
    d.move_to(body);
    assert!(hinted(&d.host().windows[&viewport].shapes), "{:?}", texts(&d.host().windows[&viewport].shapes));
    assert!(!hinted(d.shapes()), "not in the source");

    // Tab 5 of window [5, 6] over frame [1, 2] of the main window.
    let mut d = driver();
    let id = window(&d, 5).id;
    let viewport = d.viewport(id);
    let local = d.window_local(id, ORIGIN + vec2(100.0, 300.0)).unwrap();
    d.window_hold(id, pos2(15.0, TAB_ROW), local);
    assert!(hinted(d.shapes()), "{:?}", texts(d.shapes()));
    assert!(!hinted(&d.host().windows[&viewport].shapes), "not in the source");
}

/// Zone outlines: closed 1 pt paths without a fill.
fn zone_outlines(shapes: &[ClippedShape]) -> usize {
    shapes
        .iter()
        .filter(|clipped| matches!(&clipped.shape, egui::Shape::Path(path) if path.closed && path.fill == Color32::TRANSPARENT && path.stroke.width == 1.0))
        .count()
}

#[test]
fn drop_zones_are_drawn_in_the_window_under_the_pointer() {
    let mut d = DockDriver::new(two_windows(), Content::default())
        .with_style(|s| {
            s.side_edge_zones = true;
            s.float_hint = false;
        })
        .with_native_host(ORIGIN);
    let viewport = d.viewport(window(&d, 7).id);
    // Tab 1 of the main window over window [7]'s body, too short to split top or bottom: its
    // left and right sides, center and strip.
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    let body = d.host().windows[&viewport].rect.center() - ORIGIN.to_vec2();
    d.hold(tab, body);
    d.move_to(body);
    assert_eq!(zone_outlines(&d.host().windows[&viewport].shapes), 4, "drawn in the destination");
    assert_eq!(zone_outlines(d.shapes()), 0, "not in the source");
}

#[test]
fn a_command_drag_floats_even_over_another_window() {
    let mut d = driver(two_windows());
    let viewport = d.viewport(window(&d, 7).id);
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    let over = d.host().windows[&viewport].rect.center() - ORIGIN.to_vec2();
    d.modifiers = Modifiers::COMMAND;
    d.hold(tab, over);
    let out = d.release(over);
    assert!(matches!(out.actions.as_slice(), [Action::Float { panel: 1, .. }]), "{:?}", out.actions);
    d.step(vec![]);
    let native = window(&d, 1).native.unwrap();
    let desktop = ORIGIN + over.to_vec2();
    assert_eq!(pos2(native[0], native[1]), pos2(desktop.x - tab.x - 8.0, desktop.y - tab.y - 6.0 - BAR));
}

#[test]
fn without_window_positions_a_drag_out_of_a_native_window_floats() {
    let mut d = driver(two_windows());
    d.host().no_position = true;
    d.settle();
    let id = window(&d, 5).id;
    // Over window [7] on the desktop, which cannot be known: a new window.
    d.window_hold(id, pos2(15.0, TAB_ROW), pos2(-200.0, 400.0));
    let out = d.window_release(id, pos2(-200.0, 400.0));
    assert!(matches!(out.actions.as_slice(), [Action::Float { panel: 5, .. }]), "{:?}", out.actions);
    // The only tab left moves its window, by handing the move to the window system. A press
    // where the last one was, this soon, would be a double-click.
    let id = window(&d, 6).id;
    for _ in 0..40 {
        d.step(vec![]);
    }
    d.window_hold(id, pos2(15.0, TAB_ROW), pos2(120.0, 200.0));
    assert!(d.host().commands.iter().any(|(v, c)| *v == d.viewport(id) && *c == ViewportCommand::StartDrag));
    let out = d.window_release(id, pos2(120.0, 200.0));
    assert!(!out.actions.iter().any(|a| matches!(a, Action::Float { .. } | Action::Move { .. })), "{:?}", out.actions);
}

/// Drag window `panel`'s bar until the pointer is at `to` in the main window, and release.
fn drag_bar_to(d: &mut DockDriver<u32, Content>, panel: u32, to: Pos2) -> super::DockOutput<u32> {
    let id = window(d, panel).id;
    d.window_step(id, vec![Event::PointerMoved(pos2(40.0, 9.0)), button(pos2(40.0, 9.0), true)]);
    d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 19.0))]);
    let Some(local) = d.window_local(id, ORIGIN + to.to_vec2()) else { return Default::default() };
    d.window_step(id, vec![Event::PointerMoved(local)]);
    d.window_step(id, vec![button(pos2(40.0, 9.0), false)])
}

fn window_move(out: &super::DockOutput<u32>) -> Option<&Target<u32>> {
    out.actions.iter().find_map(|a| match a {
        Action::MoveWindow { to, .. } => Some(to),
        _ => None,
    })
}

#[test]
fn a_native_windows_bar_docks_it_at_frame_edges_and_dock_lines_but_not_a_body_center() {
    // The right edge zone of frame [3, 4], 40 pt in, clear of the new-dock band along the window's edge.
    let mut d = driver(layout(true));
    let out = drag_bar_to(&mut d, 5, pos2(760.0, 300.0));
    assert!(matches!(window_move(&out), Some(Target::At { anchor: 3 | 4, placement: Placement::Split(super::Zone::Right) })), "{:?}", out.actions);
    assert!(d.layout.floating.is_empty());
    assert!(in_root(&d, 5) && in_root(&d, 6));

    // The middle of the frame is no target for a whole window.
    let mut d = driver(layout(true));
    let out = drag_bar_to(&mut d, 5, pos2(600.0, 300.0));
    assert_eq!(window_move(&out), None, "{:?}", out.actions);
    assert!(window(&d, 5).contains(&6));

    // The seam between the dock's two groups, which meet at y = 307.
    let dock = Node::Dock {
        side: super::Side::Start,
        columns: vec![super::Column::new(vec![super::Group::new(vec![2], 0), super::Group::new(vec![3], 0)], 240.0)],
    };
    let mut docked = layout(true);
    docked.root =
        Some(Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::FixedFirst(240.0), first: Box::new(dock), second: Box::new(tabs(&[1])) });
    let mut d = driver(docked);
    let out = drag_bar_to(&mut d, 5, pos2(120.0, 309.0));
    assert_eq!(window_move(&out), Some(&Target::At { anchor: 2, placement: Placement::Group { below: true, height: None } }), "{:?}", out.actions);
    assert!(d.layout.floating.is_empty());
    assert!(in_root(&d, 5) && in_root(&d, 6));
}

#[test]
fn over_a_canvas_body_a_window_stays_floating_and_a_tab_floats_while_the_edge_bands_dock() {
    // PhotoCraft: a dock column of [2] above [3, 4] beside canvas 0, which takes splits, and
    // window [5, 6] floating.
    let accent = Color32::from_rgb(250, 10, 200);
    let dock = Node::Dock {
        side: super::Side::Start,
        columns: vec![super::Column::new(vec![super::Group::new(vec![2], 0), super::Group::new(vec![3, 4], 0)], 240.0)],
    };
    let root = Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::FixedFirst(240.0), first: Box::new(dock), second: Box::new(tabs(&[0])) };
    let layout = Layout { root: Some(root), floating: vec![Floating::single(vec![5, 6], 0, [300.0, 80.0, 240.0, 200.0])] };
    let start = || {
        DockDriver::new(layout.clone(), Content { canvas: Some(0), ..Content::default() })
            .with_style(move |s| {
                s.accent = accent;
                s.float_hint = false;
            })
            .with_native_host(ORIGIN)
    };
    // The canvas spans x = 244 to 800 and the whole height: no tab strip.
    for at in [pos2(520.0, 150.0), pos2(520.0, 450.0)] {
        let mut d = start();
        let before = accented(d.shapes(), accent);
        let id = window(&d, 5).id;
        d.window_step(id, vec![Event::PointerMoved(pos2(40.0, 9.0)), button(pos2(40.0, 9.0), true)]);
        d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 19.0))]);
        let local = d.window_local(id, ORIGIN + at.to_vec2()).unwrap();
        d.window_step(id, vec![Event::PointerMoved(local)]);
        d.step(vec![]);
        assert_eq!(accented(d.shapes(), accent), before, "{at:?}: no preview");
        let out = d.window_step(id, vec![button(pos2(40.0, 9.0), false)]);
        assert_eq!(window_move(&out), None, "{at:?}: {:?}", out.actions);
        assert!(window(&d, 5).contains(&6), "{at:?}");
    }
    // A docked tab dropped there floats.
    let mut d = start();
    let tab = d.hit(&Hit::Tab(3)).unwrap().center();
    let out = d.drag(tab, pos2(520.0, 450.0));
    assert!(matches!(out.actions.as_slice(), [Action::Float { panel: 3, .. }]), "{:?}", out.actions);
    // The window's right edge, where no dock is yet, and the bottom edge band beside the canvas
    // still dock the window.
    let mut d = start();
    let out = drag_bar_to(&mut d, 5, pos2(795.0, 300.0));
    assert_eq!(window_move(&out), Some(&Target::NewDock { side: super::Side::End }), "{:?}", out.actions);
    let mut d = start();
    let out = drag_bar_to(&mut d, 5, pos2(520.0, 590.0));
    assert!(matches!(window_move(&out), Some(Target::At { placement: Placement::Edge(super::Zone::Bottom), .. })), "{:?}", out.actions);
}

#[test]
fn escape_during_a_bar_drag_puts_the_window_back() {
    let mut d = driver(layout(true));
    let id = window(&d, 5).id;
    let viewport = d.viewport(id);
    d.window_step(id, vec![Event::PointerMoved(pos2(40.0, 9.0)), button(pos2(40.0, 9.0), true)]);
    d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 19.0))]);
    d.window_step(id, vec![Event::PointerMoved(pos2(140.0, 59.0))]);
    assert_ne!(d.host().windows[&viewport].rect.min, pos2(400.0, 130.0), "moved");
    let escape = Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    d.window_step(id, vec![escape]);
    assert_eq!(d.host().windows[&viewport].rect.min, pos2(400.0, 130.0));
    // The release that follows docks nothing, wherever it is.
    let out = d.window_step(id, vec![button(pos2(40.0, 9.0), false)]);
    assert_eq!(window_move(&out), None, "{:?}", out.actions);
    d.step(vec![]);
    assert_eq!(window(&d, 5).native.map(|r| [r[0], r[1]]), Some([400.0, 130.0]));
}

#[test]
fn the_window_focused_last_is_in_front_where_windows_overlap() {
    // Window [7] overlaps window [5, 6] on the desktop, at (550, 280) among others.
    let mut layout = layout(true);
    layout.floating.push(Floating::single(vec![7], 0, [350.0, 150.0, 240.0, 150.0]));
    let overlap = pos2(550.0, 280.0) - ORIGIN.to_vec2();
    for (front, anchors) in [(7, vec![7]), (5, vec![5, 6]), (7, vec![7])] {
        let mut d = driver(layout.clone());
        let viewport = d.viewport(window(&d, front).id);
        d.host().focus = Some(viewport);
        d.step(vec![]);
        d.host().focus = Some(egui::ViewportId::ROOT);
        d.step(vec![]);
        let tab = d.hit(&Hit::Tab(1)).unwrap().center();
        let out = d.drag(tab, overlap);
        let anchor = out.actions.iter().find_map(|a| match a {
            Action::Move { panel: 1, anchor, .. } => Some(*anchor),
            _ => None,
        });
        assert!(anchor.is_some_and(|a| anchors.contains(&a)), "front {front}: {:?}", out.actions);
    }
}

/// Every `OuterPosition` sent to `viewport`, in order.
fn moves(d: &DockDriver<u32, Content>, viewport: egui::ViewportId) -> Vec<Pos2> {
    d.host()
        .commands
        .iter()
        .filter_map(|(v, c)| match c {
            ViewportCommand::OuterPosition(pos) if *v == viewport => Some(*pos),
            _ => None,
        })
        .collect()
}

fn escape() -> Event {
    Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

#[test]
fn a_window_made_this_session_opens_where_it_was_put() {
    // egui reports the monitor's size but no monitor list. A tab dropped on the desktop right
    // of the main window's monitor opens there.
    let size = Some(vec2(1920.0, 1080.0));
    let mut d = driver(layout(false));
    d.host().monitor_size = size;
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    let out = pos2(2400.0, 120.0);
    d.hold(tab, out);
    d.release(out);
    d.settle();
    let desktop = ORIGIN + out.to_vec2();
    let min = pos2(desktop.x - tab.x - 8.0, desktop.y - tab.y - 6.0 - BAR);
    let native = window(&d, 1).native.unwrap();
    assert_eq!(pos2(native[0], native[1]), min);
    assert_eq!(os_window(&d, 1).unwrap().0.min, min);

    // Undock Panel, to a place off that monitor.
    let mut d = driver(layout(false));
    d.host().monitor_size = size;
    d.layout.apply(Action::Float { panel: 2, rect: [2400.0, 100.0, 300.0, 200.0] }).unwrap();
    d.settle();
    assert_eq!(window(&d, 2).native, Some([2500.0, 150.0, 300.0, 200.0]));

    // A loaded layout's window still opens on the main window's monitor.
    let mut saved = layout(true);
    saved.floating[0].native = Some([2500.0, 100.0, 240.0, 300.0]);
    let mut d = driver(Layout::default());
    d.host().monitor_size = size;
    d.layout = saved;
    d.settle();
    assert_eq!(window(&d, 5).native, Some([1680.0, 100.0, 240.0, 300.0]));
    assert_eq!(os_window(&d, 5).unwrap().0.min, pos2(1680.0, 100.0));
}

#[test]
fn a_changed_layout_moves_the_os_window() {
    let mut d = driver(layout(true));
    let viewport = d.viewport(id_of(&d));
    assert_eq!(window(&d, 5).native, Some([400.0, 130.0, 240.0, 300.0]));
    // An undo of a move, a reset, or a layout reloaded with the same ids.
    d.layout.floating[0].native = Some([700.0, 200.0, 260.0, 320.0]);
    let out = d.step(vec![]);
    assert!(!out.actions.iter().any(|a| matches!(a, Action::SetNative { .. })), "{:?}", out.actions);
    d.settle();
    assert_eq!(d.host().windows[&viewport].rect, Rect::from_min_size(pos2(700.0, 200.0), vec2(260.0, 320.0)));
    assert_eq!(window(&d, 5).native, Some([700.0, 200.0, 260.0, 320.0]));

    // Off every monitor: onto the nearest, and the layout follows.
    d.monitors = vec![Monitor { name: Some("left".into()), rect: [0.0, 0.0, 1920.0, 1080.0], scale: 1.0 }];
    d.layout.floating[0].native = Some([5000.0, 200.0, 260.0, 320.0]);
    d.settle();
    assert_eq!(d.host().windows[&viewport].rect.min, pos2(1660.0, 200.0));
    assert_eq!(window(&d, 5).native, Some([1660.0, 200.0, 260.0, 320.0]));

    // A bar drag that leaves too little of the bar on a monitor ends with the window on it.
    let id = id_of(&d);
    let grab = pos2(40.0, 9.0);
    d.window_step(id, vec![Event::PointerMoved(grab), button(grab, true)]);
    let far = d.window_local(id, pos2(1950.0, 300.0)).unwrap();
    d.window_step(id, vec![Event::PointerMoved(far)]);
    let far = d.window_local(id, pos2(1950.0, 300.0)).unwrap();
    d.window_step(id, vec![Event::PointerMoved(far), button(far, false)]);
    d.settle();
    assert_eq!(d.host().windows[&viewport].rect.min, pos2(1660.0, 291.0));
    assert_eq!(window(&d, 5).native, Some([1660.0, 291.0, 260.0, 320.0]));
}

#[test]
fn a_bar_drag_follows_the_pointer_when_moves_land_late() {
    let mut d = driver(layout(true));
    d.host().lag = true;
    let id = id_of(&d);
    let viewport = d.viewport(id);
    let grab = pos2(40.0, 9.0);
    let mut pointer = d.host().windows[&viewport].rect.min + grab.to_vec2();
    d.window_step(id, vec![Event::PointerMoved(grab), button(grab, true)]);
    // On to the desktop right of the main window, where the release docks nothing.
    for _ in 0..6 {
        pointer += vec2(90.0, -20.0);
        // The window system reports the pointer relative to where the window is now.
        let local = d.window_local(id, pointer).unwrap();
        d.window_step(id, vec![Event::PointerMoved(local)]);
        assert_eq!(moves(&d, viewport).last(), Some(&(pointer - grab.to_vec2())), "no overshoot");
    }
    // Frames with no pointer move leave it, though the last move has landed since.
    let sent = moves(&d, viewport).len();
    for _ in 0..3 {
        d.step(vec![]);
    }
    assert_eq!(moves(&d, viewport).len(), sent, "no creep");
    assert_eq!(d.host().windows[&viewport].rect.min, pointer - grab.to_vec2());
    let local = d.window_local(id, pointer).unwrap();
    d.window_step(id, vec![button(local, false)]);
    d.settle();
    let to = pointer - grab.to_vec2();
    assert_eq!(d.host().windows[&viewport].rect.min, to);
    assert_eq!(window(&d, 5).native.map(|r| [r[0], r[1]]), Some([to.x, to.y]));
}

#[test]
fn a_native_windows_content_is_its_origin_where_its_frame_differs() {
    // A title bar's height between the window's outer position and its content.
    let mut d = DockDriver::new(Layout::default(), Content::default()).with_native_host(ORIGIN);
    d.host().frame = vec2(0.0, 28.0);
    d.layout = layout(true);
    d.settle();
    // It opened with its frame there, then moved so its content is.
    let id = id_of(&d);
    let viewport = d.viewport(id);
    assert_eq!(d.host().windows[&viewport].rect.min, pos2(400.0, 130.0));
    assert_eq!(window(&d, 5).native, Some([400.0, 130.0, 240.0, 300.0]));

    // Its bar moves it by its outer position, here onto the desktop right of the main window.
    let grab = pos2(40.0, 9.0);
    d.window_step(id, vec![Event::PointerMoved(grab), button(grab, true)]);
    d.window_step(id, vec![Event::PointerMoved(grab + vec2(520.0, 20.0))]);
    assert_eq!(moves(&d, viewport).last(), Some(&pos2(920.0, 122.0)));
    d.window_step(id, vec![button(grab, false)]);
    d.settle();
    assert_eq!(d.host().windows[&viewport].rect.min, pos2(920.0, 150.0));
    assert_eq!(window(&d, 5).native.map(|r| [r[0], r[1]]), Some([920.0, 150.0]));

    // Its tab drops where the pointer is on the desktop: the main window's tab row of [3, 4].
    let row = d.window_local(id, ORIGIN + vec2(450.0, 13.0)).unwrap();
    d.window_hold(id, pos2(15.0, TAB_ROW), row);
    let out = d.window_release(id, row);
    assert!(matches!(out.actions.as_slice(), [Action::Move { panel: 5, anchor: 3 | 4, placement: Placement::Tab { .. } }]), "{:?}", out.actions);
}

#[test]
fn the_only_tab_of_a_native_window_docks_it_with_title_bar_docks_off() {
    let mut layout = layout(false);
    layout.floating.push(Floating::single(vec![7], 0, [300.0, 80.0, 240.0, 200.0]));
    let driver = || DockDriver::new(layout.clone(), Content::default()).with_style(|s| s.title_bar_docks = false).with_native_host(ORIGIN);
    let mut d = driver();
    let id = window(&d, 7).id;
    let row = ORIGIN + vec2(450.0, 13.0);
    let from = pos2(15.0, TAB_ROW);
    d.window_step(id, vec![Event::PointerMoved(from), button(from, true)]);
    d.window_step(id, vec![Event::PointerMoved(from + vec2(8.0, 6.0))]);
    for _ in 0..2 {
        let local = d.window_local(id, row).unwrap();
        d.window_step(id, vec![Event::PointerMoved(local)]);
    }
    let local = d.window_local(id, row).unwrap();
    let out = d.window_step(id, vec![button(local, false)]);
    assert!(matches!(window_move(&out), Some(Target::At { anchor: 3 | 4, .. })), "{:?}", out.actions);
    assert!(in_root(&d, 7));

    // Its bar keeps to `title_bar_docks`.
    let mut d = driver();
    let out = drag_bar_to(&mut d, 7, pos2(450.0, 13.0));
    assert_eq!(window_move(&out), None, "{:?}", out.actions);
    assert!(!in_root(&d, 7));
}

#[test]
fn the_only_tab_of_a_native_window_fills_and_joins_a_frame_bodys_center() {
    let mut layout = layout(false);
    layout.floating.push(Floating::single(vec![7], 0, [300.0, 80.0, 240.0, 200.0]));
    let style = |s: &mut super::DockStyle| {
        s.side_edge_zones = true;
        s.title_bar_docks = false;
        s.float_hint = false;
    };
    let mut d = DockDriver::new(layout, Content::default()).with_style(style).with_native_host(ORIGIN);
    let id = window(&d, 7).id;
    // The middle of frame [3, 4], in the main window.
    let middle = pos2(600.0, 300.0);
    let from = pos2(15.0, TAB_ROW);
    d.window_step(id, vec![Event::PointerMoved(from), button(from, true)]);
    d.window_step(id, vec![Event::PointerMoved(from + vec2(8.0, 6.0))]);
    for _ in 0..2 {
        let local = d.window_local(id, ORIGIN + middle.to_vec2()).unwrap();
        d.window_step(id, vec![Event::PointerMoved(local)]);
    }
    // The center rectangle is the only zone around the pointer: outlined and filled.
    let zone = |filled: bool| {
        d.shapes().iter().any(|clipped| match &clipped.shape {
            egui::Shape::Path(path) => {
                path.closed && (path.fill != Color32::TRANSPARENT) == filled && Rect::from_points(&path.points).contains(middle)
            }
            _ => false,
        })
    };
    assert!(zone(false), "outlined");
    assert!(zone(true), "filled");
    let local = d.window_local(id, ORIGIN + middle.to_vec2()).unwrap();
    let out = d.window_step(id, vec![button(local, false)]);
    assert!(matches!(window_move(&out), Some(Target::At { anchor: 3 | 4, placement: Placement::Split(super::Zone::Center) })), "{:?}", out.actions);
    assert!(in_root(&d, 7));
}

#[test]
fn a_native_window_forwards_no_keys_while_one_of_its_widgets_has_focus() {
    let mut d = driver(layout(true));
    (d.content.text, d.content.focus_text) = (Some(5), true);
    d.settle();
    let key = Event::Key { key: Key::S, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
    let out = d.window_step(id_of(&d), vec![key, Event::Text("s".into())]);
    assert!(out.forwarded_events.is_empty(), "{:?}", out.forwarded_events);
    assert_eq!(d.content.typed, "s");

    // A focused button's Enter is its own.
    let mut d = driver(layout(true));
    d.content.focused_button = Some(5);
    d.settle();
    let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let out = d.window_step(id_of(&d), vec![enter]);
    assert!(out.forwarded_events.is_empty(), "{:?}", out.forwarded_events);
}

#[test]
fn the_input_hook_rewrites_each_native_windows_input_before_its_widgets_see_it() {
    // As an app maps Ctrl+click to a secondary click on macOS.
    let mut d = driver(layout(true)).with_native_input_hook(|_, input| {
        for event in &mut input.events {
            if let Event::PointerButton { button, .. } = event {
                *button = PointerButton::Secondary;
            }
        }
    });
    let id = id_of(&d);
    d.window_click(id, pos2(60.0, 150.0));
    assert_eq!(d.content.pressed, vec![(5, PointerButton::Secondary)]);

    // The main window's input is the app's own `raw_input_hook`'s.
    d.content.pressed.clear();
    d.click(pos2(100.0, 300.0));
    assert_eq!(d.content.pressed, vec![(1, PointerButton::Primary)]);
}

#[derive(Debug)]
struct File(std::path::PathBuf);

impl egui::DroppedFile for File {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }
}

#[test]
fn files_dragged_onto_a_native_window_are_reported_with_its_panel() {
    let mut d = driver(layout(true));
    let (id, viewport) = (id_of(&d), d.viewport(id_of(&d)));
    let hovered = egui::HoveredFile { path: Some("a.png".into()), mime: String::new() };
    d.host().windows.get_mut(&viewport).unwrap().hovered_files = vec![hovered.clone()];
    let out = d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 150.0))]);
    let [files] = out.window_files.as_slice() else { panic!("{:?}", out.window_files) };
    assert_eq!((files.window, files.viewport, files.panel, files.pos), (id, viewport, 5, Some(pos2(60.0, 150.0))));
    assert_eq!(files.hovered, vec![hovered]);
    assert!(files.dropped.is_empty());

    {
        let mut host = d.host();
        let window = host.windows.get_mut(&viewport).unwrap();
        window.hovered_files.clear();
        window.dropped_files = vec![std::sync::Arc::new(File("a.png".into()))];
    }
    let out = d.window_step(id, Vec::new());
    let [files] = out.window_files.as_slice() else { panic!("{:?}", out.window_files) };
    assert_eq!(files.panel, 5);
    assert_eq!(files.dropped.iter().map(|f| f.path().to_path_buf()).collect::<Vec<_>>(), vec![std::path::PathBuf::from("a.png")]);
    assert!(d.step(Vec::new()).window_files.is_empty());
}

#[test]
fn a_blocked_native_window_reports_no_files() {
    let mut d = driver(layout(true));
    let (id, viewport) = (id_of(&d), d.viewport(id_of(&d)));
    d.input_blocked = true;
    d.step(Vec::new());
    let drop = |d: &mut DockDriver<u32, Content>| {
        let mut host = d.host();
        let window = host.windows.get_mut(&viewport).unwrap();
        window.hovered_files = vec![egui::HoveredFile { path: Some("a.png".into()), mime: String::new() }];
        window.dropped_files = vec![std::sync::Arc::new(File("a.png".into()))];
    };
    drop(&mut d);
    let out = d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 150.0))]);
    assert!(out.window_files.is_empty(), "{:?}", out.window_files);
    assert!(!d.content.saw_files);

    d.input_blocked = false;
    drop(&mut d);
    let out = d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 150.0))]);
    assert_eq!(out.window_files.len(), 1);
    assert!(d.content.saw_files);
}

#[test]
fn escape_or_focus_loss_ends_a_drag_a_native_window_owns() {
    // Escape cancels it, and is no shortcut.
    let mut d = driver(layout(true));
    let id = id_of(&d);
    let over = d.window_local(id, ORIGIN + vec2(600.0, 300.0)).unwrap();
    d.window_hold(id, pos2(15.0, TAB_ROW), over);
    let out = d.window_step(id, vec![escape()]);
    assert!(out.forwarded_events.is_empty(), "{:?}", out.forwarded_events);
    let out = d.window_release(id, over);
    assert!(!out.actions.iter().any(|a| matches!(a, Action::Move { .. } | Action::Float { .. })), "{:?}", out.actions);
    assert!(window(&d, 5).contains(&6));

    // Escape during a bar drag is no shortcut either.
    let mut d = driver(layout(true));
    d.window_step(id_of(&d), vec![Event::PointerMoved(pos2(40.0, 9.0)), button(pos2(40.0, 9.0), true)]);
    d.window_step(id_of(&d), vec![Event::PointerMoved(pos2(60.0, 19.0))]);
    let out = d.window_step(id_of(&d), vec![escape()]);
    assert!(out.forwarded_events.is_empty(), "{:?}", out.forwarded_events);

    // Escape pressed in the main window, which has keyboard focus, cancels a drag the native
    // window owns.
    let mut d = driver(layout(true));
    let id = id_of(&d);
    let viewport = d.viewport(id);
    d.window_step(id, vec![Event::PointerMoved(pos2(40.0, 9.0)), button(pos2(40.0, 9.0), true)]);
    d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 19.0))]);
    d.window_step(id, vec![Event::PointerMoved(pos2(140.0, 59.0))]);
    d.key(Key::Escape);
    assert_eq!(d.host().windows[&viewport].rect.min, pos2(400.0, 130.0), "back where it was");
    let out = d.window_step(id, vec![button(pos2(40.0, 9.0), false)]);
    assert_eq!(window_move(&out), None, "{:?}", out.actions);

    // Another application takes focus: cancelled.
    let mut d = driver(layout(true));
    let id = id_of(&d);
    d.window_hold(id, pos2(15.0, TAB_ROW), over);
    d.host().focus = None;
    d.window_step(id, vec![Event::PointerMoved(over)]);
    d.host().focus = Some(d.viewport(id));
    let out = d.window_release(id, over);
    assert!(!out.actions.iter().any(|a| matches!(a, Action::Move { .. } | Action::Float { .. })), "{:?}", out.actions);
    assert!(!super::is_dragging::<u32>(&d.ctx, d.id));
}

#[test]
fn escape_in_a_focused_native_window_cancels_a_main_window_drag() {
    let mut d = driver(two_windows());
    let id = window(&d, 7).id;
    let viewport = d.viewport(id);
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    let body = d.host().windows[&viewport].rect.center() - ORIGIN.to_vec2();
    d.hold(tab, body);
    d.host().focus = Some(viewport);
    let out = d.window_step(id, vec![escape()]);
    assert!(out.forwarded_events.is_empty(), "the Escape is no shortcut: {:?}", out.forwarded_events);
    d.step(vec![Event::PointerMoved(body)]);
    let out = d.release(body);
    assert!(!out.actions.iter().any(|a| matches!(a, Action::Move { .. } | Action::MovePanel { .. } | Action::Float { .. })), "{:?}", out.actions);
    assert!(!super::is_dragging::<u32>(&d.ctx, d.id));
}

#[test]
fn a_main_window_without_focus_still_drags_a_tab_into_another_group() {
    let mut d = driver(layout(false));
    d.host().focus = None;
    d.settle();
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    let over = d.hit(&Hit::Tab(3)).unwrap().center();
    d.hold(tab, over);
    assert!(super::is_dragging::<u32>(&d.ctx, d.id));
    let out = d.release(over);
    assert!(out.actions.iter().any(|a| matches!(a, Action::Move { panel: 1, .. })), "{:?}", out.actions);
}

#[test]
fn focus_moving_to_a_native_window_mid_drag_keeps_the_drag() {
    let mut d = driver(two_windows());
    let viewport = d.viewport(window(&d, 7).id);
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    let body = d.host().windows[&viewport].rect.center() - ORIGIN.to_vec2();
    d.hold(tab, body);
    d.host().focus = Some(viewport);
    d.step(vec![Event::WindowFocused(false), Event::PointerMoved(body)]);
    assert!(super::is_dragging::<u32>(&d.ctx, d.id));
    let out = d.release(body);
    assert!(matches!(out.actions.as_slice(), [Action::Move { panel: 1, anchor: 7, .. }]), "{:?}", out.actions);
}

#[test]
fn a_drag_ends_when_its_native_window_or_panel_goes() {
    let pointer = |d: &DockDriver<u32, Content>| d.ctx.data(|data| data.get_temp::<DragPointer>(d.id.with("drag-pointer")));
    let moving = |d: &DockDriver<u32, Content>| d.ctx.data(|data| data.get_temp::<WindowDrag>(d.id.with("window-drag")));

    // The window closes during a tab drag it owns.
    let mut d = driver(layout(true));
    let id = id_of(&d);
    let over = d.window_local(id, ORIGIN + vec2(600.0, 300.0)).unwrap();
    d.window_hold(id, pos2(15.0, TAB_ROW), over);
    let viewport = d.viewport(id);
    d.host().windows.get_mut(&viewport).unwrap().close = true;
    d.step(vec![]);
    d.settle();
    assert!(!d.layout.contains(&5));
    assert!(!super::is_dragging::<u32>(&d.ctx, d.id));
    assert_eq!(pointer(&d), None);

    // The dragged panel is closed by the application.
    let mut d = driver(layout(true));
    let id = id_of(&d);
    d.window_hold(id, pos2(15.0, TAB_ROW), over);
    d.layout.apply(Action::Close { panel: 5 }).unwrap();
    let out = d.window_release(id, over);
    assert!(out.error.is_none() && !out.actions.iter().any(|a| matches!(a, Action::Move { .. } | Action::Float { .. })), "{:?}", out.actions);
    d.settle();
    assert!(!super::is_dragging::<u32>(&d.ctx, d.id));
    assert_eq!(pointer(&d), None);

    // The window closes during its own bar drag.
    let mut d = driver(layout(true));
    let id = id_of(&d);
    d.window_step(id, vec![Event::PointerMoved(pos2(40.0, 9.0)), button(pos2(40.0, 9.0), true)]);
    d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 19.0))]);
    assert!(moving(&d).is_some());
    d.layout.apply(Action::CloseWindow { panel: 5 }).unwrap();
    d.settle();
    assert_eq!(moving(&d), None);
    assert_eq!(pointer(&d), None);
    // The main window's drags work as before.
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    let out = d.drag(tab, pos2(600.0, 300.0));
    assert!(matches!(out.actions.as_slice(), [Action::Move { panel: 1, anchor: 3 | 4, .. }]), "{:?}", out.actions);
}

/// Press at `from` in the native window of panel 5, move the pointer by `by` on the desktop
/// and release. Returns the window's OS rect, the resize the window handed to the window
/// system, and the driver.
#[cfg(not(target_os = "macos"))]
fn press_and_drag(from: Pos2, by: egui::Vec2) -> (Rect, Option<egui::ResizeDirection>, DockDriver<u32, Content>) {
    let mut d = driver(layout(true));
    let id = window(&d, 5).id;
    let viewport = d.viewport(id);
    let press = os_window(&d, 5).map_or(Pos2::ZERO, |(rect, _)| rect.min) + from.to_vec2();
    d.window_step(id, vec![Event::PointerMoved(from), button(from, true)]);
    // The pointer is where it was moved on the desktop, wherever the window is now.
    let local = |d: &DockDriver<u32, Content>, step: egui::Vec2| d.window_local(id, press + step).unwrap_or_default();
    for step in [vec2(3.0, 3.0), by] {
        let pos = local(&d, step);
        d.window_step(id, vec![Event::PointerMoved(pos)]);
    }
    let pos = local(&d, by);
    d.window_step(id, vec![Event::PointerMoved(pos), button(pos, false)]);
    d.step(vec![]);
    let resize = d.host().commands.iter().find_map(|(v, c)| match c {
        ViewportCommand::BeginResize(direction) if *v == viewport => Some(*direction),
        _ => None,
    });
    (os_window(&d, 5).map_or(Rect::NOTHING, |(rect, _)| rect), resize, d)
}

// On macOS AppKit resizes the window and there are no bands; the macOS × layout has its own
// test.
#[cfg(not(target_os = "macos"))]
#[test]
fn the_bars_close_button_wins_over_the_resize_bands() {
    use egui::ResizeDirection as D;
    let mut d = driver(layout(true));
    let id = window(&d, 5).id;
    let viewport = d.viewport(id);
    // The × of the 240 pt wide window spans 215..233 across and the whole bar down. Its top
    // right is next to the window's top-right corner, but it closes the window.
    let out = d.window_click(id, pos2(232.0, 1.0));
    assert!(out.actions.contains(&Action::CloseWindow { panel: 5 }), "{:?}", out.actions);
    assert!(!d.host().commands.iter().any(|(v, c)| *v == viewport && matches!(c, ViewportCommand::BeginResize(_))), "not resized");

    // Beside it, at the window's edges, the bands hand the resize to the window system.
    let resize = |from: Pos2| press_and_drag(from, vec2(20.0, 20.0)).1;
    assert_eq!(resize(pos2(238.0, 2.0)), Some(D::NorthEast), "the top-right corner");
    assert_eq!(resize(pos2(237.0, 15.0)), Some(D::NorthEast), "the corner beside the ×");
    assert_eq!(resize(pos2(237.0, 100.0)), Some(D::East), "the right edge");
    assert_eq!(resize(pos2(120.0, 1.0)), Some(D::North), "the top edge above the bar");
    assert_eq!(resize(pos2(2.0, 298.0)), Some(D::SouthWest), "the bottom-left corner");
    // Inside a corner's L, the bar moves the window, here to the middle of frame [1, 2].
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
    let (moved, resize, _) = press_and_drag(pos2(10.0, 10.0), vec2(-110.0, 150.0));
    assert_eq!((moved, resize), (rect(290.0, 280.0, 530.0, 580.0), None));
}

#[test]
/// Resize bands avoid the close button, are at most 6 pt deep, use 16 pt
/// corners along each edge, and leave the bar below the top band inert.
fn resize_bands_are_a_few_points_deep_and_leave_out_the_bar_buttons() {
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(240.0, 300.0));
    let close = Rect::from_center_size(pos2(224.0, 9.0), vec2(18.0, 18.0));
    let bands = resize_bands(rect, &[close]);
    let at = |p: Pos2| bands.iter().find(|(_, band)| band.contains(p)).map(|(direction, _)| *direction);
    for (_, band) in &bands {
        assert!(!band.intersect(close).is_positive(), "{band:?} covers the ×");
        assert!(band.width().min(band.height()) <= 6.0, "{band:?} is deeper than an edge band");
    }
    // Each edge resizes along its whole length, outside the ×, from 16 pt corners.
    use egui::ResizeDirection as D;
    // Which of an edge's three parts `v` is in: the first corner, the edge, the last corner.
    let part = |v: f32, length: f32| usize::from(v >= 16.0) + usize::from(v > length - 16.0);
    for i in 0..240 {
        let x = i as f32 + 0.5;
        let top = [D::NorthWest, D::North, D::NorthEast][part(x, 240.0)];
        assert_eq!(at(pos2(x, 0.5)), (!close.contains(pos2(x, 0.5))).then_some(top), "top at {x}");
        assert_eq!(at(pos2(x, 5.5)), (!close.contains(pos2(x, 5.5))).then_some(top), "6 pt down at {x}");
        assert_eq!(at(pos2(x, 299.5)), Some([D::SouthWest, D::South, D::SouthEast][part(x, 240.0)]), "bottom at {x}");
    }
    for i in 0..300 {
        let y = i as f32 + 0.5;
        assert_eq!(at(pos2(0.5, y)), Some([D::NorthWest, D::West, D::SouthWest][part(y, 300.0)]), "left at {y}");
        assert_eq!(at(pos2(239.5, y)), Some([D::NorthEast, D::East, D::SouthEast][part(y, 300.0)]), "right at {y}");
    }
    assert_eq!(at(pos2(10.0, 10.0)), None, "a corner is an L along its edges");
    assert_eq!(at(pos2(120.0, 6.5)), None, "the bar below the top band");
}

#[test]
/// On macOS the close glyph sits 8 pt from the trailing corner in both directions; its hit area
/// extends 10 pt away from the corner, reaches the bar bottom, leaves the drag area clear, and stays
/// reachable outside AppKit's corner resize region.
fn on_macos_the_close_glyph_sits_by_the_corner_and_its_hit_area_reaches_away_from_it() {
    use crate::docking::chrome::{ChromeStyle, TitleBar};
    use crate::docking::placement::BAR_HEIGHT;
    let ctx = egui::Context::default();
    let full = Rect::from_min_size(Pos2::ZERO, vec2(240.0, 300.0));
    let bar = Rect::from_min_size(full.min, vec2(full.width(), BAR_HEIGHT));
    for right_to_left in [false, true] {
        let (hit, glyph) = close_rects(bar, MACOS_CLOSE, right_to_left);
        // Distances from the corner the × sits by: across from the trailing edge, and down.
        let across = |x: f32| if right_to_left { x - full.left() } else { full.right() - x };
        let (glyph_near, glyph_far) = (across(glyph.right()).min(across(glyph.left())), across(glyph.right()).max(across(glyph.left())));
        assert_eq!((glyph_near, glyph.top(), glyph.width(), glyph.height()), (8.0, 4.5, 9.0, 9.0), "the glyph by the corner");
        let (hit_near, hit_far) = (across(hit.right()).min(across(hit.left())), across(hit.right()).max(across(hit.left())));
        assert_eq!(hit_near, glyph_near, "the hit area reaches no closer to the corner than the glyph");
        assert_eq!(hit_far, glyph_far + 10.0, "the hit area reaches away from the corner");
        assert_eq!((hit.top(), hit.bottom()), (glyph.top(), BAR_HEIGHT), "the hit area reaches down to the bar's bottom, not below");
        // AppKit's corner area takes the presses nearest the corner; the rest reach the ×.
        let [top_left, top_right, ..] = appkit_resize_areas(full);
        let corner = if right_to_left { top_left } else { top_right };
        assert_eq!(corner.width(), APPKIT_CORNER);
        assert!(hit_far - APPKIT_CORNER >= 10.0, "{hit:?} has too little room outside {corner:?}");

        let mut shown = None;
        let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(full), ..Default::default() }, |ui| {
            let style = ChromeStyle { right_to_left, ..ChromeStyle::from_ui(ui) };
            let response = TitleBar::new(egui::Id::unique("bar"), "Move").title("Panel").close("Close").close_at(hit, glyph).show(ui, bar, &style);
            shown = response.close.map(|close| (close.rect, response.bar.rect));
        });
        output.textures_delta.clear();
        let (close, drag) = shown.unwrap_or((Rect::NOTHING, Rect::NOTHING));
        assert_eq!(close, hit, "the bar takes clicks for the × in its hit area");
        assert!(!drag.intersect(hit).is_positive(), "the bar's drag area {drag:?} leaves the × alone");
    }
}

/// [`layout`] built anew, its window with id 7 when `with_id`.
fn rebuilt_layout(with_id: bool) -> Layout<u32> {
    let mut layout = layout(true);
    if with_id {
        layout.floating = layout.floating.into_iter().map(|w| w.with_id(7)).collect();
    }
    layout
}

#[test]
fn a_layout_built_each_frame_with_ids_keeps_its_os_window() {
    let mut d = driver(rebuilt_layout(true));
    let viewport = d.viewport(7);
    for _ in 0..4 {
        d.layout = rebuilt_layout(true);
        d.step(vec![]);
    }
    assert_eq!(d.host().windows.keys().copied().collect::<Vec<_>>(), [viewport], "one OS window, the same each frame");
}

#[cfg(debug_assertions)]
#[test]
fn a_layout_built_each_frame_without_ids_is_warned_about() {
    let warned = |d: &DockDriver<u32, Content>| d.ctx.data(|m| m.get_temp::<bool>(d.id.with("native-rebuilt"))).unwrap_or(false);
    let mut d = driver(rebuilt_layout(false));
    for _ in 0..4 {
        d.layout = rebuilt_layout(false);
        d.step(vec![]);
    }
    assert!(warned(&d));
    assert!(d.host().windows.len() > 1, "a new OS window each frame");

    // A layout kept across frames, and one rebuilt with ids, are not.
    let mut d = driver(layout(true));
    d.layout = rebuilt_layout(false);
    for _ in 0..4 {
        d.step(vec![]);
    }
    assert!(!warned(&d), "loaded once");
    let mut d = driver(rebuilt_layout(true));
    for _ in 0..4 {
        d.layout = rebuilt_layout(true);
        d.step(vec![]);
    }
    assert!(!warned(&d));
}

/// Fill of the overlay over a blocked native window in [`blocked_driver`].
const DIM: Color32 = Color32::from_rgba_premultiplied(10, 20, 30, 120);

fn blocked_driver() -> DockDriver<u32, Content> {
    let mut d = DockDriver::new(layout(true), Content { counter: Some(5), ..Content::default() })
        .with_style(|s| s.blocked_dim = DIM)
        .with_native_host(ORIGIN);
    d.settle();
    d
}

/// Whether the native window `viewport` drew the overlay over all of its content.
fn dimmed(d: &DockDriver<u32, Content>, viewport: egui::ViewportId) -> bool {
    let full = Rect::from_min_size(Pos2::ZERO, d.host().windows[&viewport].rect.size());
    d.host().windows[&viewport].shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == DIM && r.rect == full))
}

#[test]
/// While input is blocked, a native window is dimmed, loses always-on-top,
/// ignores clicks, drags, key presses and close requests, but still forwards key
/// releases; unblocking restores normal input and activation.
fn a_native_window_takes_no_input_while_the_app_blocks_it() {
    let mut d = blocked_driver();
    let id = id_of(&d);
    let viewport = d.viewport(id);
    let counter = d.content.counter_rect.unwrap().center();
    let tab = pos2(90.0, TAB_ROW);
    let key = Event::Key { key: Key::S, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
    let level = |d: &DockDriver<u32, Content>| d.host().windows[&viewport].level;
    assert!(!dimmed(&d, viewport));
    assert_eq!(level(&d), Some(egui::WindowLevel::AlwaysOnTop));

    d.input_blocked = true;
    d.step(vec![]);
    assert!(dimmed(&d, viewport), "drawn dimmed");
    assert_eq!(level(&d), Some(egui::WindowLevel::Normal), "the main window can cover the palette");
    let out = d.window_click(id, tab);
    assert!(out.actions.is_empty(), "{:?}", out.actions);
    d.window_click(id, counter);
    assert_eq!(d.content.count, 0, "the body takes no click");
    let out = d.window_step(id, vec![key.clone(), Event::Text("s".into())]);
    assert!(out.forwarded_events.is_empty(), "{:?}", out.forwarded_events);
    // A key released while blocked still reaches the app, which may have seen it go down.
    let release = Event::Key { key: Key::S, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::COMMAND };
    let out = d.window_step(id, vec![release.clone()]);
    assert_eq!(out.forwarded_events, vec![release]);
    // The bar starts no move, of the dock area's or the window system's.
    let sent = d.host().commands.len();
    d.window_hold(id, pos2(40.0, 9.0), pos2(140.0, 59.0));
    assert!(d.ctx.data(|data| data.get_temp::<WindowDrag>(d.id.with("window-drag"))).is_none());
    assert!(
        !d.host().commands[sent..].iter().any(|(v, c)| *v == viewport && matches!(c, ViewportCommand::StartDrag | ViewportCommand::OuterPosition(_)))
    );
    let out = d.window_release(id, pos2(140.0, 59.0));
    assert!(out.actions.is_empty(), "{:?}", out.actions);
    // The OS close request is cancelled.
    d.host().windows.get_mut(&viewport).unwrap().close = true;
    let out = d.step(vec![]);
    assert!(!out.actions.iter().any(|a| matches!(a, Action::CloseWindow { .. })), "{:?}", out.actions);
    assert!(d.host().commands.contains(&(viewport, ViewportCommand::CancelClose)));

    // Unblocked, the same input works again.
    d.input_blocked = false;
    d.step(vec![]);
    assert!(!dimmed(&d, viewport));
    assert_eq!(level(&d), Some(egui::WindowLevel::AlwaysOnTop));
    d.window_click(id, counter);
    assert_eq!(d.content.count, 1);
    let out = d.window_click(id, tab);
    assert!(out.actions.contains(&Action::Activate { panel: 6 }), "{:?}", out.actions);
    let out = d.window_step(id, vec![key.clone()]);
    assert_eq!(out.forwarded_events, vec![key]);
}

#[test]
/// A blocked native window rejects typing but preserves native control-event order: IME reset
/// completion and focus loss reach both event and raw-event readers, while a shortcut release is
/// still forwarded to the host.
fn a_blocked_native_window_preserves_ordered_ime_reset_completions() {
    let mut d = blocked_driver();
    let id = id_of(&d);
    let viewport = d.viewport(id);
    d.content.input_probe = Some(5);
    d.input_blocked = true;
    let request_id = d.ctx.request_ime_reset(viewport).unwrap();
    let completed = Event::ImeResetCompleted { request_id };
    let focus = Event::WindowFocused(false);
    let release = Event::Key { key: Key::S, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::COMMAND };
    let out = d.window_step(id, vec![Event::Text("blocked".into()), completed.clone(), release.clone(), focus.clone()]);
    assert_eq!(d.content.events, vec![completed.clone(), focus.clone()], "panel input keeps control order while rejecting typing");
    assert_eq!(d.content.raw_events, vec![completed, focus], "raw readers receive the same native controls");
    assert_eq!(out.forwarded_events, vec![release], "shortcut releases still reach the host");
    assert!(d.content.typed.is_empty());
}

#[test]
fn a_blocked_palette_stays_shown_and_behind_the_main_window_when_the_app_returns() {
    use egui::{ViewportId, WindowLevel};
    let mut d = blocked_driver();
    let palette = d.viewport(id_of(&d));
    d.input_blocked = true;
    d.step(vec![]);
    d.host().commands.clear();
    let focus = |d: &mut DockDriver<u32, Content>, window: Option<ViewportId>| {
        d.host().focus = window;
        for _ in 0..20 {
            d.step(vec![]);
        }
    };
    // Away and back, it stays shown at the normal level: showing it or putting it on top could
    // raise it over the modal, and showing it would, on macOS, give it focus.
    focus(&mut d, None);
    assert!(d.host().windows[&palette].visible);
    focus(&mut d, Some(palette));
    let host = d.host();
    assert_eq!(host.windows[&palette].level, Some(WindowLevel::Normal));
    assert!(!host.commands.iter().any(|(v, c)| *v == palette && matches!(c, ViewportCommand::Visible(_))), "{:?}", host.commands);
    // Activated through the palette, the main window comes in front of it, with no viewers.
    let levels: Vec<_> = host.commands.iter().filter(|(_, c)| matches!(c, ViewportCommand::WindowLevel(_))).cloned().collect();
    let expected = if cfg!(target_os = "macos") {
        Vec::new()
    } else {
        [WindowLevel::AlwaysOnTop, WindowLevel::Normal].map(|l| (ViewportId::ROOT, ViewportCommand::WindowLevel(l))).to_vec()
    };
    assert_eq!(levels, expected);
}

/// A menu's key equivalent, as an app's menu glue reports it in the main window's input.
fn menu_key(key: Key) -> Vec<Event> {
    let press = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::COMMAND };
    vec![press(true), press(false)]
}

#[test]
fn menu_keys_go_to_the_focused_text_field_of_the_native_window_with_the_keyboard() {
    let has = |events: &[Event], key: Key| events.iter().any(|e| matches!(e, Event::Key { key: k, .. } if *k == key));
    let pasted = |d: &DockDriver<u32, Content>, viewport| d.host().commands.contains(&(viewport, ViewportCommand::RequestPaste));
    let mut d = driver(layout(true));
    let viewport = d.viewport(id_of(&d));
    (d.content.text, d.content.focus_text) = (Some(5), true);
    d.host().focus = Some(viewport);
    d.settle();
    assert_eq!(super::typing_window(&d.ctx), Some(viewport));
    // Between frames, where an app's `raw_input_hook` asks, the answer is the last frame's.
    d.content.blur_text = true;
    d.step(vec![]);
    assert_eq!(super::typing_window(&d.ctx), None, "right after the field loses focus");
    d.content.focus_text = true;
    d.step(vec![]);
    assert_eq!(super::typing_window(&d.ctx), Some(viewport), "right after the field takes focus");
    d.step(menu_key(Key::V));
    assert!(!has(&d.content.main_keys, Key::V), "{:?}", d.content.main_keys);
    assert!(pasted(&d, viewport), "the field's window asks for the clipboard text");
    d.step(menu_key(Key::C));
    assert!(!has(&d.content.main_keys, Key::C));
    assert!(d.content.field_keys.contains(&Event::Copy), "{:?}", d.content.field_keys);
    // Saving is the app's, from any window.
    d.step(menu_key(Key::S));
    assert!(has(&d.content.main_keys, Key::S));
    assert!(!has(&d.content.field_keys, Key::S));

    // Under a block the window's field is inert: the main window keeps every key.
    d.input_blocked = true;
    d.settle();
    assert_eq!(super::typing_window(&d.ctx), None);
    d.host().commands.clear();
    d.step(menu_key(Key::V));
    assert!(has(&d.content.main_keys, Key::V));
    assert!(!pasted(&d, viewport));

    // With no focused field, nothing moves.
    let mut d = driver(layout(true));
    let viewport = d.viewport(id_of(&d));
    d.content.text = Some(5);
    d.host().focus = Some(viewport);
    d.settle();
    d.step(menu_key(Key::V));
    assert!(has(&d.content.main_keys, Key::V));
    assert!(d.content.field_keys.is_empty(), "{:?}", d.content.field_keys);
    assert!(!pasted(&d, viewport));
}

#[test]
fn a_focused_field_and_a_held_press_stay_inert_across_a_block() {
    let mut d = blocked_driver();
    let id = id_of(&d);
    d.content.text = Some(5);
    d.content.focus_text = true;
    d.step(vec![]);
    d.window_step(id, vec![Event::Text("a".into())]);
    assert_eq!(d.content.typed, "a");
    // The field loses focus when the block starts, and stays without it after.
    d.input_blocked = true;
    d.step(vec![]);
    let out = d.window_step(id, vec![Event::Text("b".into())]);
    assert!(out.forwarded_events.is_empty(), "{:?}", out.forwarded_events);
    d.input_blocked = false;
    d.step(vec![]);
    let out = d.window_step(id, vec![Event::Text("c".into())]);
    assert_eq!(d.content.typed, "a");
    assert_eq!(out.forwarded_events, vec![Event::Text("c".into())]);

    // A press made while blocked and released after is no click.
    let counter = d.content.counter_rect.unwrap().center();
    d.input_blocked = true;
    d.step(vec![]);
    d.window_step(id, vec![Event::PointerMoved(counter), button(counter, true)]);
    d.input_blocked = false;
    d.step(vec![]);
    d.window_step(id, vec![button(counter, false)]);
    assert_eq!(d.content.count, 0);
}

#[test]
fn blocking_input_hands_focus_from_a_native_window_to_the_main_window() {
    let focus_main = |d: &DockDriver<u32, Content>| {
        d.host().commands.iter().filter(|(v, c)| *v == egui::ViewportId::ROOT && matches!(c, ViewportCommand::Focus)).count()
    };
    let steps = |d: &mut DockDriver<u32, Content>, n: usize| {
        for _ in 0..n {
            d.step(vec![]);
        }
    };
    let focused_float = || {
        let mut d = blocked_driver();
        let viewport = d.viewport(id_of(&d));
        d.host().focus = Some(viewport);
        d.step(vec![]);
        d.input_blocked = true;
        d
    };
    // A dialog opened from a floating panel needs the keys there, in the main window: once,
    // after a short wait.
    let mut d = focused_float();
    steps(&mut d, 2);
    assert_eq!(focus_main(&d), 0, "not yet: an OS dialog may still take focus");
    steps(&mut d, 30);
    assert_eq!(focus_main(&d), 1);
    // An OS dialog took focus meanwhile: no window of the area has it, so nothing is sent.
    let mut d = focused_float();
    d.step(vec![]);
    d.host().focus = None;
    steps(&mut d, 30);
    assert_eq!(focus_main(&d), 0);
    // Blocking ended before the wait was over.
    let mut d = focused_float();
    d.step(vec![]);
    d.input_blocked = false;
    steps(&mut d, 30);
    assert_eq!(focus_main(&d), 0);
    // With the main window focused already, nothing is sent.
    let mut d = blocked_driver();
    d.input_blocked = true;
    steps(&mut d, 30);
    assert_eq!(focus_main(&d), 0);
}

#[test]
fn blocking_input_cancels_a_drag_in_progress() {
    // A tab drag a native window owns.
    let mut d = blocked_driver();
    let id = id_of(&d);
    let over = d.window_local(id, ORIGIN + vec2(600.0, 300.0)).unwrap();
    d.window_hold(id, pos2(15.0, TAB_ROW), over);
    assert!(super::is_dragging::<u32>(&d.ctx, d.id));
    d.input_blocked = true;
    d.step(vec![]);
    assert!(!super::is_dragging::<u32>(&d.ctx, d.id));
    let out = d.window_release(id, over);
    assert!(!out.actions.iter().any(|a| matches!(a, Action::Move { .. } | Action::Float { .. })), "{:?}", out.actions);

    // A bar drag.
    let mut d = blocked_driver();
    let id = id_of(&d);
    d.window_hold(id, pos2(40.0, 9.0), pos2(140.0, 59.0));
    assert!(d.ctx.data(|data| data.get_temp::<WindowDrag>(d.id.with("window-drag"))).is_some());
    d.input_blocked = true;
    d.step(vec![]);
    assert!(d.ctx.data(|data| data.get_temp::<WindowDrag>(d.id.with("window-drag"))).is_none());
}

#[test]
fn replacing_the_layout_or_cancel_drag_ends_a_bar_drag() {
    let moving = |d: &DockDriver<u32, Content>| d.ctx.data(|data| data.get_temp::<WindowDrag>(d.id.with("window-drag")));
    for replace in [false, true] {
        let mut d = driver(layout(true));
        let id = id_of(&d);
        d.window_step(id, vec![Event::PointerMoved(pos2(40.0, 9.0)), button(pos2(40.0, 9.0), true)]);
        d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 19.0))]);
        assert!(moving(&d).is_some());
        d.step(vec![]);
        if replace {
            // The same window where the OS has it, beside swapped frames.
            let mut replacement = layout(true);
            replacement.root = Some(Node::Split {
                axis: SplitAxis::Horizontal,
                size: SplitSize::Ratio(0.5),
                first: Box::new(tabs(&[3, 4])),
                second: Box::new(tabs(&[1, 2])),
            });
            replacement.floating[0] = Floating { native: window(&d, 5).native, ..replacement.floating[0].clone().with_id(id) };
            d.layout = replacement;
        } else {
            super::cancel_drag::<u32>(&d.ctx, d.id);
        }
        d.settle();
        assert_eq!(moving(&d), None);
        // The OS window goes where the layout has it.
        let before = os_window(&d, 5).unwrap().0;
        let native = window(&d, 5).native.unwrap();
        assert_eq!(before.min, pos2(native[0], native[1]));
        d.window_step(id, vec![Event::PointerMoved(pos2(90.0, 39.0))]);
        d.window_step(id, vec![Event::PointerMoved(pos2(120.0, 59.0))]);
        assert_eq!(moving(&d), None);
        assert_eq!(os_window(&d, 5).unwrap().0, before);
        let out = d.window_step(id, vec![button(pos2(120.0, 59.0), false)]);
        assert!(!out.actions.iter().any(|a| matches!(a, Action::Move { .. } | Action::MoveWindow { .. })), "{:?}", out.actions);
    }
}

#[test]
fn replacing_the_layout_ends_a_divider_drag_in_a_native_window() {
    let resizes = |out: &super::DockOutput<u32>| out.actions.iter().filter(|a| matches!(a, Action::ResizeWindowSplit { .. })).count();
    let split = |d: &DockDriver<u32, Content>| match &window(d, 5).root {
        Node::Split { size, .. } => *size,
        root => panic!("{root:?}"),
    };
    for replace in [false, true] {
        let mut start = layout(false);
        let mut float = Floating::single(vec![5], 0, [300.0, 80.0, 240.0, 300.0]);
        float.root =
            Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::Ratio(0.5), first: Box::new(tabs(&[5])), second: Box::new(tabs(&[6])) };
        start.floating.push(float);
        let mut d = driver(start);
        d.settle();
        let id = id_of(&d);
        let out = d.window_hold(id, pos2(120.0, 150.0), pos2(150.0, 150.0));
        assert!(resizes(&out) > 0, "{:?}", out.actions);
        if replace {
            d.layout.root = Some(Node::Split {
                axis: SplitAxis::Horizontal,
                size: SplitSize::Ratio(0.5),
                first: Box::new(tabs(&[3, 4])),
                second: Box::new(tabs(&[1, 2])),
            });
        }
        let before = split(&d);
        let mut moved = 0;
        for x in [160.0, 170.0, 180.0] {
            moved += resizes(&d.window_step(id, vec![Event::PointerMoved(pos2(x, 150.0))]));
        }
        moved += resizes(&d.window_release(id, pos2(180.0, 150.0)));
        if replace {
            assert_eq!(moved, 0);
            assert_eq!(split(&d), before);
        } else {
            assert!(moved > 0);
            assert_ne!(split(&d), before);
        }
    }
}

/// [`layout`] with window `[5, 6]` saved as a native window at `NATIVE`.
fn saved_native() -> Layout<u32> {
    let mut layout = layout(true);
    layout.floating[0].native = Some(NATIVE);
    layout
}

const NATIVE: [f32; 4] = [700.0, 400.0, 240.0, 300.0];

#[test]
fn with_native_windows_off_a_float_stays_in_the_app_window_and_keeps_its_native_rect() {
    let mut d = DockDriver::new(saved_native(), Content::default());
    d.native_windows = false;
    let mut d = d.with_native_host(ORIGIN);
    assert!(d.host().windows.is_empty() && d.host().commands.is_empty());
    assert!(d.hit(&Hit::Title(5)).is_some(), "drawn in the main window");
    assert_eq!(window(&d, 5).native, Some(NATIVE));

    // Its tab docks again by drag, in the main window.
    let tab = d.hit(&Hit::Tab(5)).unwrap().center();
    let out = d.drag(tab, pos2(600.0, 300.0));
    assert!(matches!(out.actions.as_slice(), [Action::Move { panel: 5, anchor: 3 | 4, .. }]), "{:?}", out.actions);
    assert!(in_root(&d, 5));
    assert_eq!(window(&d, 6).native, Some(NATIVE));
    assert!(d.host().windows.is_empty() && d.host().commands.is_empty());

    // The same workspace opens as a native window where native windows are on.
    let mut d = driver(saved_native());
    d.settle();
    assert_eq!(os_window(&d, 5).map(|(rect, _)| rect), Some(Rect::from_min_size(pos2(700.0, 400.0), vec2(240.0, 300.0))));
}

#[test]
fn wayland_is_a_main_window_that_never_reported_a_position() {
    use super::render::Session;
    let mut d = driver(layout(false));
    d.host().no_position = true;
    d.step(vec![]);
    assert!(wayland(&d.ctx, Session::Wayland));
    assert!(wayland(&d.ctx, Session::Unknown), "a real window without a Wayland variable");
    assert!(!wayland(&d.ctx, Session::Other), "X11");
    // X11 (forced under a Wayland session) reports it; minimized, it reports none again.
    d.host().no_position = false;
    d.step(vec![]);
    assert!(!wayland(&d.ctx, Session::Wayland));
    d.host().no_position = true;
    d.step(vec![]);
    assert!(!wayland(&d.ctx, Session::Wayland));
    // A context with no real window (a headless test) and no display variables at all.
    let ctx = egui::Context::default();
    ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
    assert!(!wayland(&ctx, Session::Unknown));
}

#[test]
fn the_session_follows_winits_environment_rule() {
    use super::render::{Session, session};
    let with = |vars: &'static [&'static str]| move |name: &str| vars.contains(&name);
    assert_eq!(session(true, with(&["WAYLAND_DISPLAY", "DISPLAY"])), Session::Wayland);
    assert_eq!(session(true, with(&["WAYLAND_SOCKET"])), Session::Wayland);
    assert_eq!(session(true, with(&["DISPLAY"])), Session::Other);
    assert_eq!(session(true, with(&[])), Session::Unknown);
    assert_eq!(session(false, with(&["WAYLAND_DISPLAY"])), Session::Other);
}

/// A driver whose panel `source` has an app drag source and whose panels `sites` are drop sites.
fn app_drag(layout: Layout<u32>, source: u32, sites: &[u32]) -> DockDriver<u32, Content> {
    let mut d = driver(layout);
    (d.content.source, d.content.drop_sites, d.content.marks) = (Some(source), sites.to_vec(), Marks::Route(d.id));
    d.settle();
    d
}

#[allow(clippy::expect_used)]
fn source(d: &DockDriver<u32, Content>) -> Pos2 {
    d.content.source_rect.expect("source drawn").center()
}

#[test]
fn an_app_drag_from_the_main_window_reaches_a_native_windows_panel() {
    // Panel 3's frame lies under window [5, 6], which takes the drop.
    let mut d = app_drag(layout(true), 1, &[3, 5]);
    let id = window(&d, 5).id;
    let local = pos2(120.0, 200.0);
    let point = d.host().windows[&d.viewport(id)].rect.min + local.to_vec2() - ORIGIN.to_vec2();
    d.hold(source(&d), point);
    assert!(d.content.hovers.contains(&(5, local)), "{:?}", d.content.hovers);
    d.release(point);
    d.step(vec![]);
    assert_eq!(d.content.drops, vec![(5, local)]);
    assert!(!d.content.hovers.iter().any(|(panel, _)| *panel == 3), "{:?}", d.content.hovers);
}

#[test]
fn a_stop_withdraws_a_routed_release_no_window_read_but_not_the_owners() {
    let mut d = app_drag(layout(true), 5, &[1]);
    let id = window(&d, 5).id;
    let local = d.window_local(id, ORIGIN + vec2(100.0, 300.0)).unwrap();
    d.window_hold(id, source(&d), local);
    // The release is queued for the main window's next pass, and the drag is cancelled first.
    d.window_release(id, local);
    super::cancel_drag::<u32>(&d.ctx, d.id);
    d.step(vec![]);
    d.step(vec![]);
    assert_eq!(d.content.drops, vec![]);
    // The owner still gets its own release.
    assert_eq!(d.content.source_stopped, 1);
}

#[test]
fn a_plain_egui_drag_over_a_native_window_keeps_its_moves_and_is_not_routed() {
    let mut d = app_drag(layout(true), 1, &[5]);
    d.content.marks = Marks::Nothing;
    let id = window(&d, 5).id;
    let over = d.host().windows[&d.viewport(id)].rect.center() - ORIGIN.to_vec2();
    let from = source(&d);
    d.hold(from, over);
    d.move_to(over + vec2(20.0, 10.0));
    let moved = over + vec2(20.0, 10.0) - from;
    assert!((d.content.source_delta - moved).length() < 0.01, "{:?}, not {moved:?}", d.content.source_delta);
    d.release(over);
    d.step(vec![]);
    assert!(d.content.hovers.is_empty(), "{:?}", d.content.hovers);
    assert!(d.content.drops.is_empty(), "{:?}", d.content.drops);
    assert_eq!(d.content.source_stopped, 1);
}

#[test]
fn an_egui_payload_reaches_a_window_drawn_before_the_owner() {
    // The main window draws before the native window that owns the drag.
    let mut d = app_drag(layout(true), 5, &[1]);
    d.content.marks = Marks::Payload;
    let id = window(&d, 5).id;
    let local = d.window_local(id, ORIGIN + vec2(100.0, 300.0)).unwrap();
    d.window_hold(id, source(&d), local);
    d.window_release(id, local);
    d.step(vec![]);
    d.step(vec![]);
    assert_eq!(d.content.payload_drops, vec![1]);
    assert!(!egui::DragAndDrop::has_any_payload(&d.ctx));
}

#[test]
fn an_owner_that_discards_its_release_pass_keeps_the_payload_for_the_target() {
    let mut d = app_drag(layout(true), 5, &[1]);
    d.content.marks = Marks::Payload;
    let id = window(&d, 5).id;
    let local = d.window_local(id, ORIGIN + vec2(100.0, 300.0)).unwrap();
    d.window_hold(id, source(&d), local);
    // The owner's pass that holds its release runs twice.
    d.content.discard_next = true;
    d.window_release(id, local);
    d.step(vec![]);
    d.step(vec![]);
    assert_eq!(d.content.payload_drops, vec![1]);
}

#[test]
fn pointer_events_after_a_held_release_wait_behind_it() {
    let mut d = app_drag(layout(true), 1, &[5]);
    let id = window(&d, 5).id;
    let point = d.host().windows[&d.viewport(id)].rect.min + vec2(120.0, 200.0) - ORIGIN.to_vec2();
    d.hold(source(&d), point);
    let delta = d.content.source_delta;
    // The release over the native window, then the pointer back on the source and a new press,
    // in one input.
    let at = source(&d);
    d.step(vec![button(point, false), Event::PointerMoved(at), button(at, true)]);
    assert_eq!(d.content.source_delta, delta, "no move after the release reaches the source's drag");
    d.step(vec![]);
    assert!(d.ctx.input(|i| i.pointer.primary_down()), "the new press outlives the held release");
}

#[test]
fn the_owners_drag_stops_a_pass_after_its_release_over_another_window() {
    let mut d = app_drag(layout(true), 1, &[5]);
    let id = window(&d, 5).id;
    let local = pos2(120.0, 200.0);
    let point = d.host().windows[&d.viewport(id)].rect.min + local.to_vec2() - ORIGIN.to_vec2();
    d.hold(source(&d), point);
    d.release(point);
    assert_eq!((d.content.drops.clone(), d.content.source_stopped), (vec![(5, local)], 0));
    // The owner saw the pointer go, and holds its release and the pointer's going once.
    assert_eq!(d.ctx.input(|i| i.pointer.hover_pos()), None);
    let held = super::render::held_events(&d.ctx, egui::ViewportId::ROOT);
    assert!(matches!(held.as_slice(), [Event::PointerButton { pressed: false, .. }, Event::PointerGone]), "{held:?}");
    d.step(vec![]);
    assert_eq!(d.content.source_stopped, 1);
    assert_eq!(d.ctx.dragged_id(), None);
}

#[test]
fn a_press_right_after_a_routed_release_outlives_the_held_release() {
    let mut d = app_drag(layout(true), 1, &[5]);
    let id = window(&d, 5).id;
    let point = d.host().windows[&d.viewport(id)].rect.min + vec2(120.0, 200.0) - ORIGIN.to_vec2();
    d.hold(source(&d), point);
    d.release(point);
    // The owner's held release comes before a press in its next input.
    let at = source(&d);
    d.step(vec![Event::PointerMoved(at), button(at, true)]);
    assert!(d.ctx.input(|i| i.pointer.primary_down()));
}

#[test]
fn a_routed_release_clicks_nothing_in_the_target() {
    let mut d = app_drag(layout(true), 1, &[5]);
    d.content.counter = Some(5);
    d.step(vec![]);
    let id = window(&d, 5).id;
    let button = d.content.counter_rect.unwrap().center();
    let point = d.host().windows[&d.viewport(id)].rect.min + button.to_vec2() - ORIGIN.to_vec2();
    d.hold(source(&d), point);
    d.release(point);
    d.step(vec![]);
    assert_eq!(d.content.drops, vec![(5, button)]);
    assert_eq!(d.content.count, 0);
}

#[test]
fn an_app_drag_from_a_native_window_reaches_the_main_window() {
    let mut d = app_drag(layout(true), 5, &[1]);
    let id = window(&d, 5).id;
    // Frame [1, 2], left of the native window on the desktop.
    let point = pos2(100.0, 300.0);
    let local = d.window_local(id, ORIGIN + point.to_vec2()).unwrap();
    d.window_hold(id, source(&d), local);
    d.window_release(id, local);
    d.step(vec![]);
    d.step(vec![]);
    assert_eq!(d.content.drops, vec![(1, point)]);
    assert!(d.content.hovers.contains(&(1, point)), "{:?}", d.content.hovers);
}

#[test]
fn an_app_drag_between_native_windows_drops_in_the_front_one() {
    // Window [7] overlaps window [5, 6] at (550, 280) on the desktop; window [8] is apart.
    let mut layout = layout(true);
    layout.floating.push(Floating::single(vec![7], 0, [350.0, 150.0, 240.0, 150.0]));
    layout.floating.push(Floating::single(vec![8], 0, [20.0, 420.0, 240.0, 150.0]));
    let overlap = pos2(550.0, 280.0);
    for front in [7, 5] {
        let mut d = app_drag(layout.clone(), 8, &[5, 7]);
        let viewport = d.viewport(window(&d, front).id);
        d.host().focus = Some(viewport);
        d.step(vec![]);
        d.host().focus = Some(egui::ViewportId::ROOT);
        d.step(vec![]);
        let (owner, target) = (window(&d, 8).id, window(&d, front).id);
        let local = d.window_local(owner, overlap).unwrap();
        d.window_hold(owner, source(&d), local);
        d.window_release(owner, local);
        d.step(vec![]);
        d.step(vec![]);
        assert_eq!(d.content.drops, vec![(front, d.window_local(target, overlap).unwrap())], "front {front}");
    }
}

#[test]
fn an_app_drag_released_outside_every_window_drops_nowhere() {
    let mut d = app_drag(layout(true), 1, &[5]);
    let id = window(&d, 5).id;
    let over = d.host().windows[&d.viewport(id)].rect.center() - ORIGIN.to_vec2();
    d.hold(source(&d), over);
    assert!(d.content.hovers.iter().any(|(panel, _)| *panel == 5), "{:?}", d.content.hovers);
    // Left of the main window, over no window of the app: the native window sees the pointer go.
    let out = pos2(-60.0, 120.0);
    d.move_to(out);
    d.content.hovers.clear();
    d.move_to(out);
    d.release(out);
    d.step(vec![]);
    assert!(d.content.hovers.is_empty(), "{:?}", d.content.hovers);
    assert!(d.content.drops.is_empty(), "{:?}", d.content.drops);
}

/// Hold an app drag from panel 1 over window [5, 6] and release it there.
fn drag_onto_window(d: &mut DockDriver<u32, Content>) {
    let id = window(d, 5).id;
    let over = d.host().windows[&d.viewport(id)].rect.center() - ORIGIN.to_vec2();
    d.hold(source(d), over);
    d.release(over);
    d.step(vec![]);
    d.step(vec![]);
}

#[test]
fn no_app_drag_is_routed_where_windows_report_no_positions() {
    let mut d = app_drag(layout(true), 1, &[5]);
    d.host().no_position = true;
    d.step(vec![]);
    drag_onto_window(&mut d);
    assert!(d.content.hovers.is_empty(), "{:?}", d.content.hovers);
    assert!(d.content.drops.is_empty(), "{:?}", d.content.drops);
}

#[test]
fn a_blocked_native_window_gets_no_app_drag() {
    let mut d = app_drag(layout(true), 1, &[5]);
    d.input_blocked = true;
    d.settle();
    drag_onto_window(&mut d);
    assert!(d.content.hovers.is_empty(), "{:?}", d.content.hovers);
    assert!(d.content.drops.is_empty(), "{:?}", d.content.drops);
}

#[test]
fn escape_and_cancel_drag_end_the_routing_of_an_app_drag() {
    for cancel in [false, true] {
        let mut d = app_drag(layout(true), 1, &[5]);
        let id = window(&d, 5).id;
        let over = d.host().windows[&d.viewport(id)].rect.center() - ORIGIN.to_vec2();
        d.hold(source(&d), over);
        assert!(!d.content.hovers.is_empty(), "cancel {cancel}");
        if cancel {
            super::cancel_drag::<u32>(&d.ctx, d.id);
            d.step(vec![]);
        } else {
            d.key(Key::Escape);
        }
        d.content.hovers.clear();
        d.move_to(over);
        d.release(over);
        d.step(vec![]);
        assert!(d.content.hovers.is_empty(), "cancel {cancel}: {:?}", d.content.hovers);
        assert!(d.content.drops.is_empty(), "cancel {cancel}: {:?}", d.content.drops);
    }
}

#[test]
/// Dragging a tab onto a native window's body emits a tab `Move` rather than
/// an app body drag and leaves the content's drops empty.
fn a_tab_drag_onto_a_native_window_is_no_app_drag() {
    let mut d = app_drag(layout(true), 1, &[5]);
    let id = window(&d, 5).id;
    let over = d.host().windows[&d.viewport(id)].rect.center() - ORIGIN.to_vec2();
    let tab = d.hit(&Hit::Tab(1)).unwrap().center();
    let out = d.drag(tab, over);
    d.step(vec![]);
    assert!(matches!(out.actions.as_slice(), [Action::Move { panel: 1, anchor: 5 | 6, .. }]), "{:?}", out.actions);
    assert!(d.content.drops.is_empty(), "{:?}", d.content.drops);
}

/// `cancel_drag` from an app's per-area native input hook (`DockArea::native_input_hook`)
/// runs while egui holds the `NativeInput` plugin lock. It must not try to lock that plugin
/// again: in a debug build the original deadlocks, and egui's debug mutex panics after its
/// 10 s deadlock bound.
#[test]
fn cancel_drag_from_a_native_input_hook_returns() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let mut d = driver(layout(true)).with_native_input_hook(move |ctx, _input| {
        if flag.swap(false, Ordering::SeqCst) {
            // `driver` uses this area id.
            super::cancel_drag::<u32>(ctx, egui::Id::unique("dock-driver"));
        }
    });
    let id = id_of(&d);
    d.settle();

    // The hook runs for the native window's input, inside `NativeInput::input_hook`.
    cancel.store(true, Ordering::SeqCst);
    d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 150.0))]);
    assert!(!cancel.load(Ordering::SeqCst), "the hook did not run");
}

/// `send_keys` from the per-area native input hook, inside `Plugin::input_hook`, returns
/// without locking the plugin again, and the keys reach that window's next input.
#[test]
fn send_keys_from_a_native_input_hook_returns_and_delivers() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let send = Arc::new(AtomicBool::new(false));
    let flag = send.clone();
    let mut d = driver(layout(true)).with_native_input_hook(move |ctx, input| {
        if flag.swap(false, Ordering::SeqCst) {
            super::send_keys(ctx, input.viewport_id, vec![Event::Copy]);
        }
    });
    let id = id_of(&d);
    d.content.text = Some(5);
    d.settle();
    send.store(true, Ordering::SeqCst);
    d.window_step(id, vec![]);
    assert!(!send.load(Ordering::SeqCst), "the hook did not run");
    d.step(vec![]);
    d.window_step(id, vec![]);
    assert!(d.content.field_keys.contains(&Event::Copy), "{:?}", d.content.field_keys);
}

/// An app drag with a release queued for the main window: `cancel_drag` from the per-area
/// native input hook (inside `Plugin::input_hook`, with egui holding the `NativeInput` plugin
/// lock) returns without deadlocking, withdraws that release, and leaves the owner's own
/// release.
#[test]
fn cancel_drag_from_a_native_input_hook_withdraws_a_queued_release() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let mut d = app_drag(layout(true), 5, &[1]).with_native_input_hook(move |ctx, _input| {
        if flag.swap(false, Ordering::SeqCst) {
            // `driver` uses this area id.
            super::cancel_drag::<u32>(ctx, egui::Id::unique("dock-driver"));
        }
    });
    let id = window(&d, 5).id;
    // The release is over the main window: it is queued for the main window's next pass.
    let local = d.window_local(id, ORIGIN + vec2(100.0, 300.0)).unwrap();
    d.window_hold(id, source(&d), local);
    cancel.store(true, Ordering::SeqCst);
    d.window_release(id, local);
    assert!(!cancel.load(Ordering::SeqCst), "the hook did not run");
    assert!(d.content.drops.is_empty(), "{:?}", d.content.drops);
    assert_eq!(d.content.source_stopped, 0, "the owner's own release is still held");
    d.step(vec![]);
    d.step(vec![]);
    assert!(d.content.drops.is_empty(), "{:?}", d.content.drops);
    assert_eq!(d.content.source_stopped, 1, "the owner still gets its own release");
    assert!(super::render::held_events(&d.ctx, egui::ViewportId::ROOT).is_empty());
    assert!(super::render::held_events(&d.ctx, d.viewport(id)).is_empty());
}

/// A main-window owner that asks for its release pass to be discarded still gets its release
/// exactly once, and the native target reads the payload exactly once.
#[test]
fn a_root_owner_that_discards_its_release_pass_drops_the_payload_once() {
    let mut d = app_drag(layout(true), 1, &[5]);
    d.content.marks = Marks::Payload;
    let id = window(&d, 5).id;
    let local = pos2(120.0, 200.0);
    let point = d.host().windows[&d.viewport(id)].rect.min + local.to_vec2() - ORIGIN.to_vec2();
    d.hold(source(&d), point);

    d.content.discard_next = true;
    d.release(point);
    assert_eq!(d.content.drops, vec![(5, local)]);
    assert_eq!(d.content.payload_drops, vec![5]);
    // The owner's release may arrive in the same frame's rerun.
    assert_eq!(d.content.source_stopped, 1);
    assert_eq!(d.content.source_releases, 1, "the owner receives its release once");

    d.step(vec![]);
    assert_eq!(d.content.drops, vec![(5, local)], "the target drops once");
    assert_eq!(d.content.payload_drops, vec![5], "the payload drops once");
    assert_eq!(d.content.source_stopped, 1, "the owner gets its release once");
    assert_eq!(d.content.source_releases, 1, "the owner receives its release once");
    assert!(!egui::DragAndDrop::has_any_payload(&d.ctx));
    assert!(super::render::held_events(&d.ctx, egui::ViewportId::ROOT).is_empty());
}

/// A native target that asks for its release pass to be discarded reads the release and the
/// payload once; its rerun has no events, and the owner's held release follows a pass later.
#[test]
fn a_target_that_discards_its_release_pass_drops_the_payload_once() {
    let mut d = app_drag(layout(true), 1, &[5]);
    d.content.marks = Marks::Payload;
    d.content.discard_drop = true;
    let id = window(&d, 5).id;
    let local = pos2(120.0, 200.0);
    let point = d.host().windows[&d.viewport(id)].rect.min + local.to_vec2() - ORIGIN.to_vec2();
    d.hold(source(&d), point);

    d.release(point);
    assert_eq!(d.content.drops, vec![(5, local)]);
    assert_eq!(d.content.payload_drops, vec![5]);
    assert_eq!(d.content.source_stopped, 0, "the owner's release is held for a later pass");
    assert_eq!(d.content.source_releases, 0, "the owner's release is held for a later pass");

    d.step(vec![]);
    assert_eq!(d.content.drops, vec![(5, local)], "the target drops once");
    assert_eq!(d.content.payload_drops, vec![5], "the payload drops once");
    assert_eq!(d.content.source_stopped, 1, "the owner gets its release once");
    assert_eq!(d.content.source_releases, 1, "the owner receives its release once");
    assert!(!egui::DragAndDrop::has_any_payload(&d.ctx));
    assert!(super::render::held_events(&d.ctx, egui::ViewportId::ROOT).is_empty());
}

/// Several discarded passes in one frame neither duplicate nor lose the routed release: the
/// target's drop and payload read, and the owner's release, happen exactly once.
#[test]
fn repeated_discards_deliver_the_release_and_drop_once() {
    let mut d = app_drag(layout(true), 1, &[5]);
    d.content.marks = Marks::Payload;
    let id = window(&d, 5).id;
    let local = pos2(120.0, 200.0);
    let point = d.host().windows[&d.viewport(id)].rect.min + local.to_vec2() - ORIGIN.to_vec2();
    d.hold(source(&d), point);

    d.ctx.memory_mut(|memory| memory.options.max_passes = std::num::NonZeroUsize::new(4).unwrap());
    d.content.discard_passes = 3;
    d.release(point);
    assert_eq!(d.content.drops, vec![(5, local)], "the target drops once");
    assert_eq!(d.content.payload_drops, vec![5], "the payload drops once");
    assert_eq!(d.content.source_stopped, 1, "the owner gets its release once");
    assert_eq!(d.content.source_releases, 1, "the owner receives its release once");

    // The frame ran its fourth, completing pass here; the release and payload are gone.
    d.step(vec![]);
    assert_eq!(d.content.drops, vec![(5, local)]);
    assert_eq!(d.content.payload_drops, vec![5]);
    assert_eq!(d.content.source_stopped, 1);
    assert_eq!(d.content.source_releases, 1);
    assert!(!egui::DragAndDrop::has_any_payload(&d.ctx));
    assert!(super::render::held_events(&d.ctx, egui::ViewportId::ROOT).is_empty());
}

#[test]
fn a_native_window_fades_only_while_its_bar_drag_highlights_a_drop_target() {
    let bar_to = |d: &mut DockDriver<u32, Content>, to: Pos2| {
        let id = window(d, 5).id;
        let local = d.window_local(id, ORIGIN + to.to_vec2()).unwrap();
        d.window_step(id, vec![Event::PointerMoved(local)]);
        drag_opacity(&d.ctx, d.id)
    };
    let grab = |d: &mut DockDriver<u32, Content>| {
        let id = window(d, 5).id;
        d.window_step(id, vec![Event::PointerMoved(pos2(40.0, 9.0)), button(pos2(40.0, 9.0), true)]);
        d.window_step(id, vec![Event::PointerMoved(pos2(60.0, 19.0))]);
        id
    };
    // A bar that only moves its window highlights nothing.
    let mut d = driver(layout(true)).with_style(|s| {
        s.command_press_floats = true;
        s.title_bar_docks = false;
    });
    grab(&mut d);
    assert_eq!(bar_to(&mut d, pos2(760.0, 300.0)), 1.0, "a move that cannot dock");

    let mut d = driver(layout(true));
    let id = grab(&mut d);
    // Past the main window's right side: the desktop.
    assert_eq!(bar_to(&mut d, pos2(1000.0, 300.0)), 1.0, "over empty space");
    // The right edge zone of frame [3, 4].
    assert_eq!(bar_to(&mut d, pos2(760.0, 300.0)), 0.75, "over a drop target");
    // The middle of the frame is no target for a whole window.
    assert_eq!(bar_to(&mut d, pos2(600.0, 300.0)), 1.0, "after leaving the target");
    assert_eq!(bar_to(&mut d, pos2(760.0, 300.0)), 0.75);
    // With Cmd/Ctrl held the drop floats and nothing is highlighted.
    d.modifiers = Modifiers::COMMAND;
    assert_eq!(bar_to(&mut d, pos2(760.0, 300.0)), 1.0, "a floating drop");
    d.modifiers = Modifiers::NONE;
    assert_eq!(bar_to(&mut d, pos2(760.0, 300.0)), 0.75);
    d.window_step(id, vec![button(pos2(40.0, 9.0), false)]);
    assert_eq!(drag_opacity(&d.ctx, d.id), 1.0, "after the drop");
}

/// A driver whose window system moves a native window for a bar drag, as AppKit does.
fn os_driver(layout: Layout<u32>) -> DockDriver<u32, Content> {
    let d = driver(layout);
    d.host().moves_windows = true;
    d
}

/// Where window [5, 6]'s bar is pressed, in its coordinates.
const GRAB: Pos2 = pos2(40.0, 9.0);

/// Press window [5, 6]'s bar.
fn os_press(d: &mut DockDriver<u32, Content>) -> u64 {
    let id = id_of(d);
    d.window_step(id, vec![Event::PointerMoved(GRAB), button(GRAB, true)]);
    id
}

fn bar_dragging(d: &DockDriver<u32, Content>) -> bool {
    d.ctx.data(|data| data.get_temp::<WindowDrag>(d.id.with("window-drag"))).is_some()
}

#[test]
fn the_window_system_moves_a_native_window_for_its_bar_drag() {
    let mut d = os_driver(layout(true));
    let id = id_of(&d);
    let viewport = d.viewport(id);
    let before = moves(&d, viewport).len();
    let sent = d.host().commands.len();
    os_press(&mut d);
    assert!(d.host().commands[sent..].iter().any(|(v, c)| *v == viewport && *c == ViewportCommand::StartDrag), "on the press");
    // Over the right edge zone of frame [3, 4]. The window reports its place where the drag
    // began, so the target is found from the cursor.
    let target = ORIGIN + vec2(760.0, 300.0);
    os_bar_drag_on(&mut d, id, target);
    assert_eq!(d.host().windows[&viewport].rect.min, target - GRAB.to_vec2());
    assert_eq!(drag_opacity(&d.ctx, d.id), 0.75, "over a drop target");
    // Escape does not cancel it.
    d.window_step(id, vec![escape()]);
    assert!(bar_dragging(&d));
    assert_eq!(d.host().windows[&viewport].rect.min, target - GRAB.to_vec2());
    // Released on the desktop right of the main window, it stays where the cursor left it.
    let desktop = ORIGIN + vec2(1000.0, 300.0);
    d.window_pointer(id, desktop);
    assert_eq!(drag_opacity(&d.ctx, d.id), 1.0, "over empty space");
    let out = d.window_release(id, GRAB);
    assert_eq!(window_move(&out), None, "{:?}", out.actions);
    d.settle();
    let to = desktop - GRAB.to_vec2();
    assert_eq!(window(&d, 5).native, Some([to.x, to.y, 240.0, 300.0]));
    assert_eq!(moves(&d, viewport).len(), before, "the app moved the window");
}

/// Move the cursor of the bar drag of window `id` to the desktop point `to` in two steps.
fn os_bar_drag_on(d: &mut DockDriver<u32, Content>, id: u64, to: Pos2) {
    let from = d.host().windows[&d.viewport(id)].rect.min + GRAB.to_vec2();
    d.window_pointer(id, from.lerp(to, 0.5));
    d.window_pointer(id, to);
}

#[test]
fn a_bar_drag_the_window_system_moves_ends_once_on_the_first_release_reported() {
    // The window system reports the release a frame before egui: the window docks then.
    let mut d = os_driver(layout(true));
    let id = os_press(&mut d);
    os_bar_drag_on(&mut d, id, ORIGIN + vec2(760.0, 300.0));
    d.host().held = false;
    let out = d.step(vec![]);
    assert!(matches!(window_move(&out), Some(Target::At { anchor: 3 | 4, placement: Placement::Split(super::Zone::Right) })), "{:?}", out.actions);
    assert!(in_root(&d, 5) && in_root(&d, 6));

    // With Cmd held it only moves, and egui's release without Cmd a frame later docks nothing.
    let mut d = os_driver(layout(true));
    let id = os_press(&mut d);
    let viewport = d.viewport(id);
    d.modifiers = Modifiers::COMMAND;
    let target = ORIGIN + vec2(760.0, 300.0);
    os_bar_drag_on(&mut d, id, target);
    assert_eq!(drag_opacity(&d.ctx, d.id), 1.0, "a floating drop");
    d.host().held = false;
    let out = d.step(vec![]);
    assert_eq!(window_move(&out), None, "{:?}", out.actions);
    assert!(!bar_dragging(&d));
    d.modifiers = Modifiers::NONE;
    let sent = d.host().commands.len();
    let out = d.window_release(id, GRAB);
    assert_eq!(window_move(&out), None, "{:?}", out.actions);
    assert!(!bar_dragging(&d));
    assert!(
        !d.host().commands[sent..].iter().any(|(v, c)| *v == viewport && matches!(c, ViewportCommand::StartDrag | ViewportCommand::OuterPosition(_)))
    );
    d.settle();
    let to = target - GRAB.to_vec2();
    assert_eq!(window(&d, 5).native.map(|r| [r[0], r[1]]), Some([to.x, to.y]));

    // A click on the bar over the main window docks and moves nothing.
    let mut d = os_driver(layout(true));
    let native = window(&d, 5).native;
    let id = os_press(&mut d);
    let out = d.window_release(id, GRAB);
    assert!(out.actions.is_empty(), "{:?}", out.actions);
    d.settle();
    assert_eq!(window(&d, 5).native, native);
}
