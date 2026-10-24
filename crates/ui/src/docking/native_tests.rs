//! Floating windows in their own OS windows (feature `native-windows`), drawn by the driver's
//! stand-in for OS windows.

use egui::{Color32, Event, Key, Modifiers, PointerButton, Pos2, Rect, ViewportCommand, epaint::ClippedShape, pos2, vec2};

use super::render::{APPKIT_CORNER, DragPointer, MACOS_CLOSE, WindowDrag, appkit_resize_areas, close_rects, floats_shown, resize_bands};
use super::testing::DockDriver;
use super::{Action, DockContent, Floating, Hit, Layout, Monitor, Node, Permissions, Placement, Target, WindowKind, Workspace};
use crate::layout::{SplitAxis, SplitSize};

/// Desktop position of the main window's content.
const ORIGIN: Pos2 = pos2(100.0, 50.0);

#[derive(Default)]
struct Content {
    /// Panels that may not close.
    fixed: Vec<u32>,
    /// Panels that stay in-window.
    in_window: Vec<u32>,
    /// Viewer panels: their windows are normal windows.
    normal: Vec<u32>,
    /// The panel with a text field, what it holds, and whether it takes focus next frame.
    text: Option<u32>,
    typed: String,
    focus_text: bool,
    /// The panel with a button that takes focus.
    focused_button: Option<u32>,
}

impl DockContent<u32> for Content {
    fn body(&mut self, ui: &mut egui::Ui, panel: &u32) {
        ui.label(format!("panel {panel}"));
        if self.text == Some(*panel) {
            let field = ui.text_edit_singleline(&mut self.typed);
            if std::mem::take(&mut self.focus_text) {
                field.request_focus();
            }
        }
        if self.focused_button == Some(*panel) {
            ui.button("OK").request_focus();
        }
    }

    fn logical_label(&self, panel: &u32) -> String {
        format!("Panel {panel}")
    }

    fn permissions(&self, panel: &u32) -> Permissions {
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
fn floating_windows_hide_while_the_app_is_inactive() {
    assert!(floats_shown(true, 10.0, 10.0));
    assert!(floats_shown(false, 10.2, 10.0));
    assert!(!floats_shown(false, 10.3, 10.0));

    let mut d = driver(layout(true));
    let viewport = d.viewport(window(&d, 5).id);
    let hides =
        |d: &DockDriver<u32, Content>| d.host().commands.iter().filter(|(v, c)| *v == viewport && *c == ViewportCommand::Visible(false)).count();
    // Focus moving to the floating window keeps it.
    d.host().focus = None;
    d.step(vec![]);
    d.host().focus = Some(viewport);
    for _ in 0..30 {
        d.step(vec![]);
    }
    assert_eq!(hides(&d), 0);
    // Another application: hidden after the delay, shown again on return.
    d.host().focus = None;
    for _ in 0..10 {
        d.step(vec![]);
    }
    assert_eq!(hides(&d), 0, "not yet");
    for _ in 0..10 {
        d.step(vec![]);
    }
    assert_eq!(hides(&d), 1);
    assert!(!d.host().windows[&viewport].visible);
    d.host().focus = Some(egui::ViewportId::ROOT);
    d.step(vec![]);
    assert!(d.host().windows[&viewport].visible);
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
    // Another application: only the palette hides, and the others get no visibility command.
    d.host().focus = None;
    for _ in 0..20 {
        d.step(vec![]);
    }
    let shown = |d: &DockDriver<u32, Content>| [viewer, palette, mixed].map(|v| d.host().windows[&v].visible);
    assert_eq!(shown(&d), [true, false, true]);
    let told = |d: &DockDriver<u32, Content>, v| d.host().commands.iter().any(|(w, c)| *w == v && matches!(c, ViewportCommand::Visible(_)));
    assert!(!told(&d, viewer) && !told(&d, mixed));
    d.host().focus = Some(egui::ViewportId::ROOT);
    d.step(vec![]);
    assert_eq!(shown(&d), [true, true, true]);
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
    // An in-window window writes no id, as older builds did.
    assert!(!serde_json::to_string(&Floating::single(vec![1u32], 0, [0.0, 0.0, 9.0, 9.0])).unwrap().contains("id"));
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
    // The right edge zone of frame [3, 4], 25 pt in, clear of the new-dock band along the window's edge.
    let mut d = driver(layout(true));
    let out = drag_bar_to(&mut d, 5, pos2(775.0, 300.0));
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
    // Inside a corner's L, the bar moves the window.
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
    let (moved, resize, _) = press_and_drag(pos2(10.0, 10.0), vec2(20.0, 20.0));
    assert_eq!((moved, resize), (rect(420.0, 150.0, 660.0, 450.0), None));
}

#[test]
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
            let response = TitleBar::new(egui::Id::new("bar"), "Move").title("Panel").close("Close").close_at(hit, glyph).show(ui, bar, &style);
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
