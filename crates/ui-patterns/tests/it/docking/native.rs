//! Native dock windows: when a float gets an OS window, what the dock records when the OS moves it, the OS close, and
//! docking the whole window by dragging its title row.

use craft_behavior::dock::{Action, Floating, Layout, Node, Permissions, Workspace};
use craft_behavior::placement::Placement;
use craft_host::target::{Backend, Caps, Os, ShellInfo};
use craft_ui::contract::{Bind, BodyCx};
use craft_ui::docking::{DockConfig, DockPanels, PanelSpec};
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::drag;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::trial;
use craft_ui::testing::windows::FakeSystem;
use craft_ui_test_kit::windows::Windows;
use egui::{Event, Key, Modifiers, Pos2, Rect, Ui, Vec2, ViewportCommand, ViewportEvent, ViewportId, ViewportInfo};
use std::sync::Arc;

/// The root window and the palette overlap; the palette is the larger.
const ROOT_AT: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(300.0, 300.0) };
const PALETTE_AT: Rect = Rect { min: Pos2::new(200.0, 150.0), max: Pos2::new(600.0, 450.0) };
const PALETTE_TITLE: &str = "Tools";

/// Palette panels with the given rights.
struct Palettes(Permissions);

impl DockPanels<u32> for Palettes {
    fn spec(&self, _: &u32) -> PanelSpec {
        PanelSpec::new(PALETTE_TITLE)
    }

    fn permissions(&self, _: &u32) -> Permissions {
        self.0
    }

    fn body(&mut self, _: &u32, _: &mut Ui, _: &mut BodyCx<'_>) {}
}

/// The dock records a move the OS made to its native window, and nothing while the OS reports what the model holds.
#[trial]
fn the_dock_records_where_the_os_put_its_native_window() {
    let windows = Windows::new();
    windows.root_info(info(true, ROOT_AT));
    let mut window = Floating::single(vec![1], 0, Rect::from_min_size(Pos2::new(40.0, 40.0), Vec2::new(300.0, 200.0)));
    let mut saved = Placement::new(PALETTE_AT.size().into(), 1.0);
    saved.position_px = Some([200, 150]);
    window.native = Some(saved);
    let mut workspace = Workspace::new(Layout::new(None, vec![window]));
    let run = |workspace: &mut Workspace<u32>| {
        let mut actions = Vec::new();
        windows.frame(vec![], vec![], |ui| {
            let config = DockConfig::default().native_windows(true);
            let shown = DockArea::new("Native dock", Bind::bound(workspace), &mut Palettes(Permissions::ALL), config).show(ui);
            actions.extend(shown.actions);
        });
        actions
    };
    windows.child_info(info(true, PALETTE_AT));
    for _ in 0..3 {
        run(&mut workspace);
    }
    let rest = run(&mut workspace);
    assert!(rest.iter().all(|action| !matches!(action, Action::SetNative { .. })), "at rest: {rest:?}");
    let moved = PALETTE_AT.translate(Vec2::new(-120.0, 40.0));
    windows.child_info(info(true, moved));
    let after = run(&mut workspace);
    let recorded = |action: &Action<u32>| matches!(action, Action::SetNative { placement: Some(p), .. } if p.position_px == Some([80, 190]));
    assert!(after.iter().any(recorded), "the OS move is recorded: {after:?}");
}

/// A float made in the app window, never shown in an OS window: the dock opens it native over its place there, where
/// the host opens OS windows and reports where they are, and records where the OS put it. On Wayland, which reports
/// no positions, it shows in the app window.
#[trial]
fn a_float_opens_an_os_window_where_the_host_reports_window_positions() {
    let opened = |windows: Windows| {
        windows.root_info(info(true, ROOT_AT));
        windows.child_info(info(true, PALETTE_AT));
        let mut workspace = Workspace::new(Layout::new(None, vec![float()]));
        let (mut child, mut recorded) = (false, false);
        for _ in 0..3 {
            let out = windows.frame(vec![], vec![], |ui| {
                let shown =
                    DockArea::new("Native dock", Bind::bound(&mut workspace), &mut Palettes(Permissions::ALL), DockConfig::default()).show(ui);
                recorded |= shown.actions.iter().any(|action| matches!(action, Action::SetNative { .. }));
            });
            child |= out.builders.iter().any(|(viewport, _)| *viewport != ViewportId::ROOT);
        }
        (child, recorded)
    };
    assert_eq!(opened(Windows::new()), (true, true), "an OS window opens and its placement is recorded");
    let wayland = ShellInfo::default().backend(Backend::Wayland);
    assert_eq!(opened(Windows::with(InstallSpec::new(wayland, Arc::new(FakeSystem::new())))), (false, false), "Wayland: in the app window");
}

/// A native dock window's own title row is its only title bar, on macOS and elsewhere, with no frame decision
/// installed: the OS title bar and its buttons are hidden.
#[trial]
fn a_native_dock_window_shows_no_os_title_bar() {
    let builder = |os: Os| {
        let windows = Windows::with(InstallSpec::new(ShellInfo::default().os(os).caps(Caps::native(os)), Arc::new(FakeSystem::new())));
        windows.root_info(info(true, ROOT_AT));
        windows.child_info(info(true, PALETTE_AT));
        let mut workspace = Workspace::new(Layout::new(None, vec![float()]));
        let out = windows.frame(vec![], vec![], |ui| {
            DockArea::new("Native dock", Bind::bound(&mut workspace), &mut Palettes(Permissions::ALL), DockConfig::default()).show(ui);
        });
        out.builders.into_iter().find(|(viewport, _)| *viewport != ViewportId::ROOT).map(|(_, builder)| builder).expect("an OS window opens")
    };
    let mac = builder(Os::Mac);
    assert_eq!((mac.titlebar_shown, mac.titlebar_buttons_shown, mac.fullsize_content_view), (Some(false), Some(false), Some(true)), "macOS");
    for os in [Os::Linux, Os::Windows] {
        assert_eq!(builder(os).decorations, Some(false), "{os:?}");
    }
}

/// A native float's OS window opens at the float's own size, not at what its content measured.
#[trial]
fn a_native_float_opens_at_its_own_size() {
    let windows = Windows::new();
    windows.root_info(info(true, ROOT_AT));
    let mut workspace = Workspace::new(Layout::new(None, vec![float()]));
    let mut sizes = Vec::new();
    for _ in 0..3 {
        let out = windows.frame(vec![], vec![], |ui| {
            DockArea::new("Native dock", Bind::bound(&mut workspace), &mut Palettes(Permissions::ALL), DockConfig::default()).show(ui);
        });
        sizes.extend(out.builders.into_iter().filter(|(viewport, _)| *viewport != ViewportId::ROOT).map(|(_, builder)| builder.inner_size));
    }
    let size = float().rect.size();
    assert_eq!(sizes.first(), Some(&Some(size)), "{size:?} asked, {sizes:?} opened");
    assert!(sizes.iter().flatten().all(|shown| *shown == size), "{size:?} asked, {sizes:?} opened");
}

/// The OS close of a native float closes its window when every panel may close, and leaves it open otherwise.
#[trial]
fn the_os_close_of_a_native_float_closes_it_when_its_panels_may_close() {
    let closed = |rights: Permissions| {
        let windows = Windows::new();
        windows.root_info(info(true, ROOT_AT));
        windows.child_info(info(true, PALETTE_AT));
        let mut workspace = Workspace::new(Layout::new(None, vec![float()]));
        let run = |workspace: &mut Workspace<u32>| {
            let mut actions = Vec::new();
            windows.frame(vec![], vec![], |ui| {
                actions.extend(DockArea::new("Native dock", Bind::bound(workspace), &mut Palettes(rights), DockConfig::default()).show(ui).actions);
            });
            actions.iter().any(|action| matches!(action, Action::CloseWindow { .. }))
        };
        for _ in 0..3 {
            run(&mut workspace);
        }
        windows.child_info(ViewportInfo { events: vec![ViewportEvent::Close], ..info(true, PALETTE_AT) });
        let mut closed = run(&mut workspace);
        windows.child_info(info(true, PALETTE_AT));
        for _ in 0..2 {
            closed |= run(&mut workspace);
        }
        closed
    };
    assert!(closed(Permissions::ALL), "a closable palette's window closes");
    assert!(!closed(Permissions::ALL.closable(false)), "a window with a panel that may not close stays");
}

/// The main window, one docked group filling it, and a native palette window away from it.
const MAIN_AT: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(900.0, 700.0) };
const FLOAT_AT: Rect = Rect { min: Pos2::new(1000.0, 150.0), max: Pos2::new(1400.0, 450.0) };
/// A point of the palette's title row, clear of the lights and the actions, in the palette's coordinates.
const ROW_AT: Pos2 = Pos2::new(150.0, 10.0);
/// A desktop point over the middle of the main window's group.
const OVER_GROUP: Pos2 = Pos2::new(550.0, 400.0);

/// A dock of one docked group and one native palette.
struct NativeDock {
    windows: Windows,
    workspace: Workspace<u32>,
}

impl NativeDock {
    fn on(os: Os, system: Arc<FakeSystem>) -> Self {
        let shell = ShellInfo::default().os(os).caps(Caps::native(os));
        let windows = Windows::with(InstallSpec::new(shell, system));
        windows.root_info(info(true, MAIN_AT));
        windows.child_info(info(true, FLOAT_AT));
        let palette = Floating::single(vec![1], 0, Rect::from_min_size(Pos2::new(40.0, 40.0), FLOAT_AT.size()));
        let mut dock = NativeDock { windows, workspace: Workspace::new(Layout::new(Some(Node::tabs(vec![2], 0)), vec![palette])) };
        for _ in 0..3 {
            dock.frame(vec![]);
        }
        dock
    }

    /// One frame with `child` events for the palette's window: the dock's actions and the palette's window commands.
    fn frame(&mut self, child: Vec<Event>) -> (Vec<Action<u32>>, Vec<ViewportCommand>) {
        let mut actions = Vec::new();
        let workspace = &mut self.workspace;
        let out = self.windows.frame(vec![], child, |ui| {
            actions.extend(
                DockArea::new("Native dock", Bind::bound(workspace), &mut Palettes(Permissions::ALL), DockConfig::default()).show(ui).actions,
            );
        });
        let commands = out.commands.into_iter().filter(|(viewport, _)| *viewport != ViewportId::ROOT).map(|(_, command)| command).collect();
        (actions, commands)
    }
}

fn docks_window(actions: &[Action<u32>]) -> bool {
    actions.iter().any(|action| matches!(action, Action::MoveWindow { .. }))
}

fn outer_positions(commands: &[ViewportCommand]) -> Vec<Pos2> {
    commands.iter().filter_map(|command| if let ViewportCommand::OuterPosition(at) = command { Some(*at) } else { None }).collect()
}

/// Where the app moves windows itself (X11): a drag of the palette's title row moves the window so the pressed point
/// stays under the pointer, marks the main window's group under it, and the release docks the whole window there.
/// Escape puts the window back and docks nothing.
#[trial]
fn dragging_a_native_palette_by_its_title_row_docks_it_where_the_app_moves_windows() {
    // From the title row to the main window's group, in three moves the window follows.
    let step = (OVER_GROUP - (FLOAT_AT.min + ROW_AT.to_vec2())) / 3.0;
    let drag_over_group = |dock: &mut NativeDock| {
        let mut to = Vec::new();
        for events in [vec![moved(ROW_AT)], vec![press(ROW_AT)]] {
            dock.frame(events);
        }
        for _ in 0..3 {
            to.extend(outer_positions(&dock.frame(vec![moved(ROW_AT + step)]).1));
        }
        for _ in 0..2 {
            dock.frame(vec![]);
        }
        to
    };

    let mut dock = NativeDock::on(Os::Linux, Arc::new(FakeSystem::new()));
    let to = drag_over_group(&mut dock);
    assert_eq!(to.len(), 3, "the window follows each move: {to:?}");
    assert_eq!(to.last().map(|at| *at + ROW_AT.to_vec2()), Some(OVER_GROUP), "the pressed point stays under the pointer");
    assert!(drag::mark(&dock.windows.ctx).is_some(), "the group under the pointer shows the drop mark");
    let mut docked = false;
    for events in [vec![release(ROW_AT)], vec![], vec![]] {
        docked |= docks_window(&dock.frame(events).0);
    }
    assert!(docked, "the release docks the whole window");

    let mut dock = NativeDock::on(Os::Linux, Arc::new(FakeSystem::new()));
    drag_over_group(&mut dock);
    let (actions, commands) = dock.frame(vec![key_press(Key::Escape, Modifiers::NONE)]);
    assert_eq!(outer_positions(&commands).last(), Some(&FLOAT_AT.min), "Escape puts the window back");
    let mut docked = docks_window(&actions);
    for events in [vec![release(ROW_AT)], vec![], vec![]] {
        docked |= docks_window(&dock.frame(events).0);
    }
    assert!(!docked, "a cancelled move docks nothing");
}

/// Where the OS moves windows and the app reads the system pointer and button meanwhile (macOS): the title row's press
/// hands the move to the OS in its own pass, the system pointer over the main window's group marks it (Cmd keeps the
/// drop floating), and the window docks when the system tells the button is up, not on the first pass after the
/// hand-off. A click on the row docks nothing.
#[trial]
fn dragging_a_native_palette_by_its_title_row_docks_it_where_the_os_moves_windows() {
    let system = Arc::new(FakeSystem::new());
    let mut dock = NativeDock::on(Os::Mac, system.clone());
    let pressed_at = FLOAT_AT.min + ROW_AT.to_vec2();
    system.set_pointer(Some([pressed_at.x, pressed_at.y]));
    system.set_primary_held(Some(true));
    dock.frame(vec![moved(ROW_AT)]);
    let (_, commands) = dock.frame(vec![press(ROW_AT)]);
    assert!(commands.contains(&ViewportCommand::StartDrag), "the press hands the move to the OS: {commands:?}");
    system.set_primary_held(Some(false));
    let mut clicked = false;
    for _ in 0..3 {
        clicked |= docks_window(&dock.frame(vec![]).0);
    }
    assert!(!clicked && drag::mark(&dock.windows.ctx).is_none(), "a click on the row docks nothing");
    system.set_primary_held(Some(true));
    dock.frame(vec![press(ROW_AT)]);
    dock.frame(vec![]);
    system.set_pointer(Some([OVER_GROUP.x, OVER_GROUP.y]));
    let mut docked = false;
    for _ in 0..3 {
        docked |= docks_window(&dock.frame(vec![]).0);
    }
    assert!(drag::mark(&dock.windows.ctx).is_some(), "the group under the system pointer shows the drop mark");
    system.set_command_held(Some(true));
    for _ in 0..2 {
        docked |= docks_window(&dock.frame(vec![]).0);
    }
    assert!(drag::mark(&dock.windows.ctx).is_none(), "Cmd keeps the drop floating");
    system.set_command_held(Some(false));
    for _ in 0..2 {
        docked |= docks_window(&dock.frame(vec![]).0);
    }
    assert!(!docked, "nothing docks while the system holds the button");
    system.set_primary_held(Some(false));
    for _ in 0..3 {
        docked |= docks_window(&dock.frame(vec![]).0);
    }
    assert!(docked, "the button's release docks the whole window");
}

fn placing(commands: &[ViewportCommand]) -> Vec<&ViewportCommand> {
    commands.iter().filter(|command| matches!(command, ViewportCommand::OuterPosition(_) | ViewportCommand::InnerSize(_))).collect()
}

/// The OS resizes a native palette live from its edge and then moves it, as AppKit does on macOS, while the dock model
/// records each place a pass after the OS reported it. The app sends the window none of those places back: each would
/// be a pass old, and the two writers would set the window to two places in turn for ever. At rest it stays where the
/// OS left it.
#[trial]
fn a_native_float_the_os_resizes_or_moves_gets_no_place_back() {
    let mut dock = NativeDock::on(Os::Mac, Arc::new(FakeSystem::new()));
    let mut sent = Vec::new();
    let mut at = FLOAT_AT;
    for step in 1..=4 {
        at = Rect::from_min_size(FLOAT_AT.min, FLOAT_AT.size() + Vec2::new(2.0, 8.0) * step as f32);
        dock.windows.child_info(info(true, at));
        sent.extend(dock.frame(vec![]).1);
    }
    for step in 1..=3 {
        at = at.translate(Vec2::new(-30.0, 12.0) * step as f32);
        dock.windows.child_info(info(true, at));
        sent.extend(dock.frame(vec![]).1);
    }
    for _ in 0..3 {
        sent.extend(dock.frame(vec![]).1);
    }
    assert!(placing(&sent).is_empty(), "the OS's places come back: {:?}", placing(&sent));
    let recorded = dock.workspace.layout.floating[0].native.clone().expect("the model records the window");
    assert_eq!(Vec2::from(recorded.normal.size), at.size(), "the model holds the OS's last size");
}

/// An app change of a native palette's place (an undo, a reloaded layout) moves and sizes its window, once.
#[trial]
fn an_app_change_of_a_native_floats_place_moves_its_window_once() {
    let mut dock = NativeDock::on(Os::Mac, Arc::new(FakeSystem::new()));
    let to = Rect::from_min_size(Pos2::new(1100.0, 200.0), Vec2::new(320.0, 260.0));
    let mut placement = Placement::new(to.size().into(), 1.0);
    placement.position_px = Some([to.min.x as i32, to.min.y as i32]);
    dock.workspace.apply(Action::set_native(1, Some(placement)), false, |_| Permissions::ALL).expect("the model takes the place");
    let mut sent = Vec::new();
    for _ in 0..3 {
        sent.extend(dock.frame(vec![]).1);
    }
    let placing = placing(&sent);
    assert_eq!(placing, [&ViewportCommand::OuterPosition(to.min), &ViewportCommand::InnerSize(to.size())], "one move and one size");
}

/// A palette float made in the app window.
fn float() -> Floating<u32> {
    Floating::single(vec![1], 0, Rect::from_min_size(Pos2::new(40.0, 40.0), Vec2::new(300.0, 200.0)))
}

fn info(focused: bool, rect: Rect) -> ViewportInfo {
    ViewportInfo {
        focused: Some(focused),
        inner_rect: Some(rect),
        outer_rect: Some(rect),
        native_pixels_per_point: Some(1.0),
        ..ViewportInfo::default()
    }
}
