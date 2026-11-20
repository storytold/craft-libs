//! Native dock windows: when a float gets an OS window, what the dock records when the OS moves it, the OS close, and
//! docking the whole window by dragging its title row.

use craft_behavior::dock::{Action, Column, DockEdge, Floating, Group, Layout, Node, Permissions, Workspace};
use craft_behavior::placement::Placement;
use craft_host::target::{Backend, Caps, Os, ShellInfo};
use craft_ui::contract::{Bind, BodyCx, DropMark, Show as _, ShowContainer as _, SizeLimits};
use craft_ui::display::Text;
use craft_ui::docking::{DockConfig, DockGrid, DockPanels, PanelSpec};
use craft_ui::patterns::dock::DockArea;
use craft_ui::popover::DropVeil;
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::drag;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::trial;
use craft_ui::testing::windows::FakeSystem;
use craft_ui_test_kit::windows::Windows;
use egui::{Event, HoveredFile, Key, Modifiers, Pos2, Rect, Ui, Vec2, ViewportBuilder, ViewportCommand, ViewportEvent, ViewportId, ViewportInfo};
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

/// Palette panels whose body records, in each pass of its window, whether the window host knows the app moves that
/// window.
struct MoveProbe(Vec<bool>);

impl DockPanels<u32> for MoveProbe {
    fn spec(&self, _: &u32) -> PanelSpec {
        PanelSpec::new(PALETTE_TITLE)
    }

    fn permissions(&self, _: &u32) -> Permissions {
        Permissions::ALL
    }

    fn body(&mut self, _: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let ctx = ui.ctx();
        self.0.push(craft_ui::testing::windows::moved_by_app(ctx, ctx.viewport_id()));
    }
}

/// A dock window's OS window is opened as one the app moves: its first pass knows it before the pass ends, where the
/// Windows hit test is routed, so the system never takes the title row's band for its own move loop.
#[trial]
fn a_native_dock_window_is_moved_by_the_app_from_its_first_pass() {
    let windows = Windows::new();
    windows.root_info(info(true, ROOT_AT));
    windows.child_info(info(true, PALETTE_AT));
    let mut window = float();
    let mut saved = Placement::new(PALETTE_AT.size().into(), 1.0);
    saved.position_px = Some([PALETTE_AT.min.x as i32, PALETTE_AT.min.y as i32]);
    window.native = Some(saved);
    let mut workspace = Workspace::new(Layout::new(None, vec![window]));
    let mut probe = MoveProbe(Vec::new());
    windows.frame(vec![], vec![], |ui| {
        let config = DockConfig::default().native_windows(true);
        let _ = DockArea::new("Native dock", Bind::bound(&mut workspace), &mut probe, config).show(ui);
    });
    assert!(!probe.0.is_empty(), "the window's first frame runs the panel's body");
    assert!(probe.0.iter().all(|known| *known), "every pass of the window knows the app moves it: {:?}", probe.0);
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

/// The grid of the stepped palette's content, as a toolbar's.
fn tools_grid() -> DockGrid {
    DockGrid::new(40.0, 32.0, 6)
}

/// A palette whose content lays out in a grid (a toolbar): its column has steps and its window fits its content.
struct SteppedPalette;

impl DockPanels<u32> for SteppedPalette {
    fn spec(&self, panel: &u32) -> PanelSpec {
        if *panel == 1 { PanelSpec::new(PALETTE_TITLE).grid(tools_grid()) } else { PanelSpec::new("Canvas") }
    }

    fn permissions(&self, _: &u32) -> Permissions {
        Permissions::ALL.collapses(false)
    }

    fn body(&mut self, panel: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        if *panel == 1 {
            Text::new("One").show(ui);
            Text::new("Two").show(ui);
        }
    }
}

/// A dock of one docked group and one native palette.
struct NativeDock {
    windows: Windows,
    workspace: Workspace<u32>,
    stepped: bool,
    /// The palette window's builder as the integration keeps it: each frame's builder patched onto it, which sends the
    /// window only the limits a builder carries.
    builder: Option<ViewportBuilder>,
}

impl NativeDock {
    fn on(os: Os, system: Arc<FakeSystem>) -> Self {
        let palette = Floating::single(vec![1], 0, Rect::from_min_size(Pos2::new(40.0, 40.0), FLOAT_AT.size()));
        Self::with(os, system, palette, false)
    }

    /// The dock with a stepped palette of one step ([`SteppedPalette`]).
    fn stepped(os: Os, system: Arc<FakeSystem>) -> Self {
        let column = Column::new(vec![Group::new(vec![1], 0)]).with_steps(1);
        Self::with(os, system, Floating::new(column, Rect::from_min_size(Pos2::new(40.0, 40.0), FLOAT_AT.size())), true)
    }

    fn with(os: Os, system: Arc<FakeSystem>, palette: Floating<u32>, stepped: bool) -> Self {
        let shell = ShellInfo::default().os(os).caps(Caps::native(os));
        let windows = Windows::with(InstallSpec::new(shell, system));
        windows.root_info(info(true, MAIN_AT));
        windows.child_info(info(true, FLOAT_AT));
        let workspace = Workspace::new(Layout::new(Some(Node::tabs(vec![2], 0)), vec![palette]));
        let mut dock = NativeDock { windows, workspace, stepped, builder: None };
        for _ in 0..3 {
            dock.frame(vec![]);
        }
        dock
    }

    /// One frame with `child` events for the palette's window: the dock's actions and the palette's window commands.
    fn frame(&mut self, child: Vec<Event>) -> (Vec<Action<u32>>, Vec<ViewportCommand>) {
        self.frame_with(vec![], child)
    }

    /// One frame with `root` events for the main window and `child` events for the palette's window.
    fn frame_with(&mut self, root: Vec<Event>, child: Vec<Event>) -> (Vec<Action<u32>>, Vec<ViewportCommand>) {
        let mut actions = Vec::new();
        let workspace = &mut self.workspace;
        let (mut palettes, mut stepped) = (Palettes(Permissions::ALL), SteppedPalette);
        let panels: &mut dyn DockPanels<u32> = if self.stepped { &mut stepped } else { &mut palettes };
        let out = self.windows.frame(root, child, |ui| {
            actions.extend(DockArea::new("Native dock", Bind::bound(workspace), &mut *panels, DockConfig::default()).show(ui).actions);
        });
        for (_, builder) in out.builders.into_iter().filter(|(viewport, _)| *viewport != ViewportId::ROOT) {
            match self.builder.as_mut() {
                Some(kept) => {
                    let _ = kept.patch(builder);
                }
                None => self.builder = Some(builder),
            }
        }
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

/// Where the app moves windows itself and the platform tells the system pointer (Windows): the window follows the
/// system pointer, not the pass's pointer event, which can be an older sample than the window's current place (the
/// window moves between passes). Measured from such an event the window lands a step behind the pointer each pass and
/// stays behind after the release; from the system pointer the pressed point stays under the pointer, also at the
/// release that comes a move after the last pass.
#[trial]
fn a_native_palette_the_app_moves_follows_the_system_pointer_not_a_stale_event() {
    let system = Arc::new(FakeSystem::new());
    let mut dock = NativeDock::on(Os::Linux, Arc::clone(&system));
    let mut window = FLOAT_AT;
    let mut desktop = FLOAT_AT.min + ROW_AT.to_vec2();
    system.set_pointer(Some([desktop.x, desktop.y]));
    for events in [vec![moved(ROW_AT)], vec![press(ROW_AT)]] {
        window = Rect::from_min_size(*outer_positions(&dock.frame(events).1).last().unwrap_or(&window.min), window.size());
    }
    // Away from the main window, so the release docks nothing.
    let step = Vec2::new(40.0, 25.0);
    let mut moves = 0;
    for _ in 0..4 {
        let older = desktop;
        desktop += step;
        system.set_pointer(Some([desktop.x, desktop.y]));
        // The pass's event is the previous sample, relative to where the window is now. The first one is still the
        // pressed point, so the drag starts a pass later.
        let event = moved(older - window.min.to_vec2());
        if let Some(to) = outer_positions(&dock.frame(vec![event]).1).last() {
            window = Rect::from_min_size(*to, window.size());
            moves += 1;
            assert_eq!(window.min + ROW_AT.to_vec2(), desktop, "the pressed point stays under the system pointer");
        }
    }
    assert_eq!(moves, 3, "the window follows every pass once the drag has started");
    // The pointer moves once more before the release reaches the app.
    desktop += step;
    system.set_pointer(Some([desktop.x, desktop.y]));
    for events in [vec![release(desktop - window.min.to_vec2())], vec![], vec![]] {
        if let Some(to) = outer_positions(&dock.frame(events).1).last() {
            window = Rect::from_min_size(*to, window.size());
        }
    }
    assert_eq!(window.min + ROW_AT.to_vec2(), desktop, "the window lands where the pointer let go");
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

/// A point near the main window's start edge, in its own coordinates: away from the middle of its group.
const STALE: Pos2 = Pos2::new(5.0, 300.0);

/// The main window keeps the place it saw the pointer last when the palette over it takes the pointer, and may never
/// learn that the pointer left: AppKit sends no exit for a window of the app laid over it, and the press holds the
/// pointer in the palette's window. A drag of the palette resolves the main window's drops at the system pointer all
/// the same, never at that place.
#[trial]
fn a_palette_drag_resolves_the_main_windows_drops_at_the_system_pointer_not_where_it_saw_the_pointer_last() {
    let system = Arc::new(FakeSystem::new());
    let mut dock = NativeDock::on(Os::Mac, system.clone());
    dock.frame_with(vec![moved(STALE)], vec![]);
    let pressed_at = FLOAT_AT.min + ROW_AT.to_vec2();
    system.set_pointer(Some([pressed_at.x, pressed_at.y]));
    system.set_primary_held(Some(true));
    dock.frame(vec![moved(ROW_AT)]);
    dock.frame(vec![press(ROW_AT)]);
    dock.frame(vec![]);
    system.set_pointer(Some([OVER_GROUP.x, OVER_GROUP.y]));
    let mut docked = false;
    for _ in 0..3 {
        docked |= docks_window(&dock.frame(vec![]).0);
    }
    let mark = drag::mark(&dock.windows.ctx);
    // The group fills the main window: a drop in its middle joins it, so the mark is the whole group.
    let group = Rect::from_min_size(Pos2::ZERO, MAIN_AT.size());
    assert!(
        matches!(mark, Some(DropMark::Region { area, .. }) if area == group),
        "the middle of the group under the system pointer is marked, the whole group: {mark:?}"
    );
    system.set_primary_held(Some(false));
    for _ in 0..3 {
        docked |= docks_window(&dock.frame(vec![]).0);
    }
    assert!(docked, "the release docks the whole window");
}

/// The steps of the stepped palette's column.
fn palette_steps(dock: &NativeDock) -> Option<u16> {
    dock.workspace.layout.floating[0].column.steps
}

/// One frame of `dock` with `child` events, after which the palette's window is where the app sent it, as the OS
/// puts it: its commands, with `at` the window's rect.
fn placed_frame(dock: &mut NativeDock, child: Vec<Event>, at: &mut Rect) -> Vec<ViewportCommand> {
    let (_, commands) = dock.frame(child);
    for command in &commands {
        match command {
            ViewportCommand::OuterPosition(to) => *at = Rect::from_min_size(*to, at.size()),
            ViewportCommand::InnerSize(size) => *at = Rect::from_min_size(at.min, *size),
            _ => {}
        }
    }
    dock.windows.child_info(info(true, *at));
    commands
}

/// A native palette whose column has steps (a toolbar) is sized by the app, not the OS: a drag of its side sets whole
/// steps and the window takes their width at once, a drag of its left side keeps its right edge, and the OS is never
/// asked to resize it. The drag holds on where the OS resizes undecorated windows (Windows): a size change the app
/// makes under the held press ends no press.
#[trial]
fn a_native_stepped_palette_resizes_in_whole_steps_that_the_app_runs() {
    let grid = tools_grid();
    for os in [Os::Mac, Os::Windows] {
        let mut dock = NativeDock::stepped(os, Arc::new(FakeSystem::new()));
        let mut at = FLOAT_AT;
        let mut sent = Vec::new();
        for _ in 0..3 {
            sent.extend(placed_frame(&mut dock, vec![], &mut at));
        }
        assert_eq!(at.width(), grid.width(1), "{os:?}: the window is as wide as its one step");

        // The right grip straddles the window's right edge.
        let grip = Pos2::new(at.width() - 1.0, at.height() / 2.0);
        for events in [vec![moved(grip)], vec![press(grip)], vec![moved(grip + Vec2::new(1.6 * grid.step, 0.0))], vec![]] {
            sent.extend(placed_frame(&mut dock, events, &mut at));
        }
        assert_eq!(palette_steps(&dock), Some(2), "{os:?}: a drag of 1.6 steps sets the whole step it reached");
        assert_eq!(at.width(), grid.width(2), "{os:?}: the window takes the width of its steps while the drag runs");
        let end = grip + Vec2::new(1.6 * grid.step, 0.0);
        for events in [vec![release(end)], vec![], vec![]] {
            sent.extend(placed_frame(&mut dock, events, &mut at));
        }
        assert_eq!(palette_steps(&dock), Some(2), "{os:?}: the release keeps the steps");

        let right = at.right();
        let grip = Pos2::new(1.0, at.height() / 2.0);
        let end = grip - Vec2::new(1.6 * grid.step, 0.0);
        for events in [vec![moved(grip)], vec![press(grip)], vec![moved(end)], vec![], vec![release(end)], vec![], vec![]] {
            sent.extend(placed_frame(&mut dock, events, &mut at));
        }
        assert_eq!(palette_steps(&dock), Some(3), "{os:?}: a drag of the left side sets whole steps too");
        assert_eq!(at.width(), grid.width(3), "{os:?}: the window takes the width of its steps");
        assert!((at.right() - right).abs() <= 0.5, "{os:?}: the right edge stays: {right} then {at:?}");
        assert!(!sent.iter().any(|command| matches!(command, ViewportCommand::BeginResize(_))), "{os:?}: the OS never resizes the window");
    }
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

/// The size and place an OS window takes from `commands`, starting at `at`.
fn placed_by(commands: &[ViewportCommand], at: Rect) -> Rect {
    commands.iter().fold(at, |at, command| match command {
        ViewportCommand::OuterPosition(to) => Rect::from_min_size(*to, at.size()),
        ViewportCommand::InnerSize(size) => Rect::from_min_size(at.min, *size),
        _ => at,
    })
}

/// A native palette collapsed to icons keeps its expanded size in the model while its OS window shows the strip of
/// icons. Moved across the main window, the strip stays where it was dropped and keeps to the edge nearer it. Expanded
/// again, the window takes its expanded size back and the OS lets it: the strip's size limits are lifted.
#[trial]
fn a_native_palette_collapsed_moved_and_expanded_comes_back_at_its_size() {
    let mut dock = NativeDock::on(Os::Mac, Arc::new(FakeSystem::new()));
    let expanded = FLOAT_AT.size();
    // The OS follows the window commands of each frame, and the window reports where it is.
    let follow = |dock: &mut NativeDock, at: &mut Rect, frames: usize| {
        let mut sent = Vec::new();
        for _ in 0..frames {
            // The OS clamps a size to the maximum in force when it takes the command: the one the previous frames
            // left, then each limit command of this frame in order.
            let mut max = dock.builder.as_ref().and_then(|builder| builder.max_inner_size);
            let commands = dock.frame(vec![]).1;
            for command in &commands {
                match command {
                    ViewportCommand::MaxInnerSize(limit) => max = Some(*limit),
                    ViewportCommand::InnerSize(size) => {
                        let size = max.map_or(*size, |max| size.min(max));
                        *at = Rect::from_min_size(at.min, size);
                    }
                    ViewportCommand::OuterPosition(to) => *at = Rect::from_min_size(*to, at.size()),
                    _ => {}
                }
            }
            dock.windows.child_info(info(true, *at));
            sent.extend(commands);
        }
        sent
    };
    let mut at = FLOAT_AT;
    dock.workspace.apply(Action::set_iconic(1, true), false, |_| Permissions::ALL).expect("the column collapses");
    follow(&mut dock, &mut at, 3);
    let strip = at;
    assert!(strip.width() < expanded.x, "the OS window shows the strip: {strip:?}");
    let recorded = |dock: &NativeDock| dock.workspace.layout.floating[0].native.clone().expect("the model records the window");
    assert_eq!(Vec2::from(recorded(&dock).normal.size), expanded, "collapsed, the model keeps the expanded size");

    // The OS moves the strip over the start half of the main window.
    let dropped = Rect::from_min_size(Pos2::new(MAIN_AT.min.x + 50.0, strip.min.y), strip.size());
    at = dropped;
    dock.windows.child_info(info(true, at));
    let sent = follow(&mut dock, &mut at, 4);
    assert!(placing(&sent).is_empty(), "the strip stays where it was dropped: {:?}", placing(&sent));
    assert_eq!(at, dropped);
    assert_eq!(dock.workspace.layout.floating[0].edge, Some(DockEdge::Start), "it keeps to the edge nearer the strip");
    assert_eq!(Vec2::from(recorded(&dock).normal.size), expanded, "moved, the model keeps the expanded size");

    dock.workspace.apply(Action::set_iconic(1, false), false, |_| Permissions::ALL).expect("the column expands");
    let sent = follow(&mut dock, &mut at, 3);
    assert!(sent.contains(&ViewportCommand::InnerSize(expanded)), "the window takes its expanded size back: {:?}", placing(&sent));
    assert_eq!(at.min, dropped.min, "it opens from the strip's place, against the start edge");
    assert_eq!(at.size(), expanded, "the OS window grows although the strip's limits capped it when the size was sent");
    let max = dock.builder.as_ref().and_then(|builder| builder.max_inner_size);
    assert!(max.is_none_or(|max| max.x >= expanded.x && max.y >= expanded.y), "the OS lets it take that size: max {max:?}");
}

/// On a display at 125 % (Windows), a click on the double arrow of a native palette collapsed to icons expands its
/// column, and the OS window takes its expanded size.
#[trial]
fn a_click_on_the_double_arrow_of_a_collapsed_native_palette_expands_it() {
    const SCALE: f32 = 1.25;
    let mut dock = NativeDock::on(Os::Windows, Arc::new(FakeSystem::new()));
    dock.windows.root_info(scaled(true, MAIN_AT, SCALE));
    let mut at = FLOAT_AT;
    // The OS follows the window commands of each frame, and the window reports where it is.
    let frame = |dock: &mut NativeDock, at: &mut Rect, events: Vec<Event>| {
        let (actions, commands) = dock.frame(events);
        *at = placed_by(&commands, *at);
        dock.windows.child_info(scaled(true, *at, SCALE));
        actions
    };
    frame(&mut dock, &mut at, vec![]);
    dock.workspace.apply(Action::set_iconic(1, true), false, |_| Permissions::ALL).expect("the column collapses");
    for _ in 0..3 {
        frame(&mut dock, &mut at, vec![]);
    }
    assert!(dock.workspace.layout.floating[0].column.iconic, "the column is collapsed");
    assert!(at.width() < FLOAT_AT.width(), "the OS window shows the strip: {at:?}");

    // The double arrow is the title row's square at the end the strip keeps to: a point just inside its outer corner.
    let end = at.max.x >= FLOAT_AT.max.x - 1.0 && at.min.x > FLOAT_AT.min.x + 1.0;
    let arrow = Pos2::new(if end { at.width() - 4.0 } else { 4.0 }, 4.0);
    let mut actions = Vec::new();
    for events in [vec![moved(arrow)], vec![press(arrow)], vec![release(arrow)], vec![], vec![]] {
        actions.extend(frame(&mut dock, &mut at, events));
    }
    assert!(!dock.workspace.layout.floating[0].column.iconic, "the click expands the column: {actions:?}");
    assert_eq!(at.size(), FLOAT_AT.size(), "the OS window takes its expanded size");
}

/// Palettes no smaller than `PALETTE_LEAST`.
struct Least;

const PALETTE_LEAST: Vec2 = Vec2::new(240.0, 160.0);

impl DockPanels<u32> for Least {
    fn spec(&self, _: &u32) -> PanelSpec {
        PanelSpec::new(PALETTE_TITLE).limits(SizeLimits::at_least(PALETTE_LEAST))
    }

    fn body(&mut self, _: &u32, _: &mut Ui, _: &mut BodyCx<'_>) {}
}

/// A model place smaller than the palette's limits sizes its OS window within them: the window is never asked for a
/// size the OS would clamp, so the size the model records is the one the window has.
#[trial]
fn a_native_palette_is_never_sized_below_its_limits() {
    let windows = Windows::new();
    windows.root_info(info(true, MAIN_AT));
    windows.child_info(info(true, FLOAT_AT));
    let mut window = Floating::single(vec![1], 0, Rect::from_min_size(Pos2::new(40.0, 40.0), FLOAT_AT.size()));
    let mut saved = Placement::new(FLOAT_AT.size().into(), 1.0);
    saved.position_px = Some([FLOAT_AT.min.x as i32, FLOAT_AT.min.y as i32]);
    window.native = Some(saved);
    let mut workspace = Workspace::new(Layout::new(None, vec![window]));
    let run = |workspace: &mut Workspace<u32>| {
        let out = windows.frame(vec![], vec![], |ui| {
            let _ = DockArea::new("Native dock", Bind::bound(workspace), &mut Least, DockConfig::default().native_windows(true)).show(ui);
        });
        out.commands.into_iter().filter(|(viewport, _)| *viewport != ViewportId::ROOT).map(|(_, command)| command).collect::<Vec<_>>()
    };
    for _ in 0..3 {
        run(&mut workspace);
    }
    let mut small = Placement::new([80.0, 60.0], 1.0);
    small.position_px = Some([FLOAT_AT.min.x as i32, FLOAT_AT.min.y as i32]);
    workspace.apply(Action::set_native(1, Some(small)), false, |_| Permissions::ALL).expect("the model takes the place");
    let mut sent = Vec::new();
    for _ in 0..3 {
        sent.extend(run(&mut workspace));
    }
    let sizes: Vec<Vec2> =
        sent.iter().filter_map(|command| if let ViewportCommand::InnerSize(size) = command { Some(*size) } else { None }).collect();
    assert!(!sizes.is_empty(), "the window is sized to the model's place");
    assert!(sizes.iter().all(|size| size.x >= PALETTE_LEAST.x && size.y >= PALETTE_LEAST.y), "within the limits: {sizes:?}");
}

/// While the main window is full screen, an OS window opened over it would cover it, so a float shows in the app
/// window instead, at its own rect there, and a drag of its title row moves it.
#[trial]
fn a_float_shows_and_moves_in_the_app_window_while_the_main_window_is_full_screen() {
    let windows = Windows::new();
    windows.root_info(ViewportInfo { fullscreen: Some(true), ..info(true, MAIN_AT) });
    windows.child_info(info(true, FLOAT_AT));
    let start = float().rect;
    let mut workspace = Workspace::new(Layout::new(None, vec![float()]));
    let mut child = false;
    let mut run = |workspace: &mut Workspace<u32>, events: Vec<Event>| {
        let out = windows.frame(events, vec![], |ui| {
            let _ = DockArea::new("Native dock", Bind::bound(workspace), &mut Palettes(Permissions::ALL), DockConfig::default()).show(ui);
        });
        child |= out.builders.iter().any(|(viewport, _)| *viewport != ViewportId::ROOT);
    };
    for _ in 0..3 {
        run(&mut workspace, vec![]);
    }
    let row = start.min + ROW_AT.to_vec2();
    let by = Vec2::new(60.0, 40.0);
    run(&mut workspace, vec![moved(row)]);
    run(&mut workspace, vec![press(row)]);
    for step in 1..=3 {
        run(&mut workspace, vec![moved(row + by * step as f32 / 3.0)]);
    }
    run(&mut workspace, vec![release(row + by)]);
    for _ in 0..2 {
        run(&mut workspace, vec![]);
    }
    assert!(!child, "no OS window opens over the full-screen window");
    let moved_to = workspace.layout.floating[0].rect.min;
    assert!(
        (moved_to - (start.min + by)).length() <= by.length() / 2.0,
        "the drag moved the float with the pointer: {moved_to:?} from {:?}",
        start.min
    );
}

/// Palettes whose body is a veil over the window: a target inside the window that takes dropped files.
struct Veiled;

impl DockPanels<u32> for Veiled {
    fn spec(&self, _: &u32) -> PanelSpec {
        PanelSpec::new(PALETTE_TITLE)
    }

    fn body(&mut self, _: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let _ = DropVeil::new("Drop").show(ui, |_, _| {});
    }
}

/// Files dropped on a native float's own OS window reach `DockAreaOutput::window_files` once, unless a target inside
/// that window took them.
#[trial]
fn a_file_dropped_on_a_native_float_reaches_window_files_once() {
    let reached = |veiled: bool| {
        let windows = Windows::new();
        windows.root_info(info(true, ROOT_AT));
        windows.child_info(info(true, PALETTE_AT));
        let mut workspace = Workspace::new(Layout::new(None, vec![float()]));
        let mut frame = |hovered: Vec<HoveredFile>, dropped: Vec<String>| {
            let mut files = 0;
            windows.frame_with_files(vec![], vec![], hovered, dropped, |ui| {
                let config = DockConfig::default();
                let shown = if veiled {
                    DockArea::new("Native dock", Bind::bound(&mut workspace), &mut Veiled, config).show(ui)
                } else {
                    DockArea::new("Native dock", Bind::bound(&mut workspace), &mut Palettes(Permissions::ALL), config).show(ui)
                };
                files += shown.window_files.len();
            });
            files
        };
        for _ in 0..4 {
            frame(vec![], vec![]);
        }
        let mut total = frame(vec![HoveredFile::default()], vec![]);
        total += frame(vec![], vec!["a.png".to_owned()]);
        for _ in 0..2 {
            total += frame(vec![], vec![]);
        }
        total
    };
    assert_eq!(reached(false), 1, "a file no target took arrives once");
    assert_eq!(reached(true), 0, "a file a target in the window took does not");
}

/// A palette float made in the app window.
fn float() -> Floating<u32> {
    Floating::single(vec![1], 0, Rect::from_min_size(Pos2::new(40.0, 40.0), Vec2::new(300.0, 200.0)))
}

fn info(focused: bool, rect: Rect) -> ViewportInfo {
    scaled(focused, rect, 1.0)
}

/// The facts of a window at `rect` on a display of `pixels_per_point`.
fn scaled(focused: bool, rect: Rect, pixels_per_point: f32) -> ViewportInfo {
    ViewportInfo {
        focused: Some(focused),
        inner_rect: Some(rect),
        outer_rect: Some(rect),
        native_pixels_per_point: Some(pixels_per_point),
        ..ViewportInfo::default()
    }
}
