//! Native dock windows: when a float gets an OS window, what the dock records when the OS moves it, the OS close, and
//! docking the whole window by dragging its title row. A context menu opened in a palette places its popup window as
//! it places the menu in the main window; the palette's own menus keep their place and take the arrow keys in theirs.

use craft_behavior::Intent;
use craft_behavior::dock::{Action, Column, DockEdge, Floating, Group, Layout, Node, Permissions, Workspace};
use craft_behavior::placement::Placement;
use craft_host::target::{Backend, Caps, Os, ShellInfo};
use craft_ui::buttons::Button;
use craft_ui::contract::Role;
use craft_ui::contract::{Bind, BodyCx, DropMark, PlainData, Show as _, ShowContainer as _, SizeLimits, Tracked};
use craft_ui::display::Text;
use craft_ui::docking::{DockConfig, DockGrid, DockPanels, GroupAt, PanelSpec, PanelSpecs};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::patterns::dock::DockArea;
use craft_ui::popover::DropVeil;
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::env::RootEnv;
use craft_ui::runtime::{drag, keys};
use craft_ui::testing::access::{AccessTree, focused_key};
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::{gone, moved, press, release, secondary};
use craft_ui::testing::trial;
use craft_ui::testing::windows::{FakeSystem, HeldButtons};
use craft_ui::text::NumberField;
use craft_ui_test_kit::windows::{AppKitWindow, FilesOver, Out, Windows};
use egui::{
    Event, HoveredFile, Key, Modifiers, Pos2, Rect, Shape, Ui, Vec2, ViewportBuilder, ViewportCommand, ViewportEvent, ViewportId, ViewportInfo,
};
use std::sync::Arc;

/// The root window and the palette overlap; the palette is the larger.
const ROOT_AT: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(300.0, 300.0) };
const PALETTE_AT: Rect = Rect { min: Pos2::new(200.0, 150.0), max: Pos2::new(600.0, 450.0) };
const PALETTE_TITLE: &str = "Tools";

/// The palettes' specs: palettes named `PALETTE_TITLE` with `rights`. `least` keeps them no smaller than
/// `PALETTE_LEAST`; `stepped` lays panel 1's content in a grid, as a toolbar's that never collapses, beside a canvas.
#[derive(Clone, Copy, PlainData)]
struct Specs {
    rights: Permissions,
    least: bool,
    stepped: bool,
}

impl Specs {
    /// Palettes with `rights`.
    fn with(rights: Permissions) -> Tracked<Specs> {
        Tracked::new(Specs { rights, least: false, stepped: false })
    }
}

impl PanelSpecs<u32> for Specs {
    fn spec(&self, panel: &u32) -> PanelSpec {
        match (self.stepped, *panel) {
            (true, 1) => PanelSpec::new(PALETTE_TITLE).grid(tools_grid()),
            (true, _) => PanelSpec::new("Canvas"),
            _ if self.least => PanelSpec::new(PALETTE_TITLE).limits(SizeLimits::at_least(PALETTE_LEAST)),
            _ => PanelSpec::new(PALETTE_TITLE),
        }
    }

    fn permissions(&self, _: &u32) -> Permissions {
        if self.stepped { Permissions::ALL.collapses(false) } else { self.rights }
    }
}

/// Palette panels with empty bodies.
struct Palettes;

impl DockPanels<u32> for Palettes {
    fn body(&mut self, _: &u32, _: &mut Ui, _: &mut BodyCx<'_>) {}
}

/// Palette panels whose body records, in each pass of its window, whether the window host knows the app moves that
/// window.
struct MoveProbe(Vec<bool>);

impl DockPanels<u32> for MoveProbe {
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
    let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![window])));
    let mut probe = MoveProbe(Vec::new());
    windows.frame(vec![], vec![], |ui| {
        let config = DockConfig::default().native_windows(true);
        let _ = DockArea::new("Native dock", Bind::bound(&mut workspace), &Specs::with(Permissions::ALL), &mut probe, config).show(ui);
    });
    assert!(!probe.0.is_empty(), "the window's first frame runs the panel's body");
    assert!(probe.0.iter().all(|known| *known), "every pass of the window knows the app moves it: {:?}", probe.0);
}

/// A dock restored with a native float in the second pass of the first frame, as an app that lays out its window in
/// the first pass and shows its dock in the next: the float's window paints during the frame's run, before the run's
/// output reaches the renderer the windows share, so its texture changes must come after the first pass's
/// allocations (the font atlas). The kit's renderer panics on a change to a texture it has not allocated.
#[trial]
fn a_dock_restored_with_a_native_float_in_a_later_pass_of_the_first_frame_paints() {
    let windows = Windows::new();
    windows.root_info(info(true, ROOT_AT));
    windows.child_info(info(true, PALETTE_AT));
    let mut window = float();
    window.native = Some(Placement::new(PALETTE_AT.size().into(), 1.0));
    let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![window])));
    let mut probe = MoveProbe(Vec::new());
    let mut passes = 0;
    windows.frame(vec![], vec![], |ui| {
        passes += 1;
        if passes == 1 {
            ui.label("Starting");
            ui.ctx().request_discard("the dock shows in the next pass");
            return;
        }
        let config = DockConfig::default().native_windows(true);
        let _ = DockArea::new("Native dock", Bind::bound(&mut workspace), &Specs::with(Permissions::ALL), &mut probe, config).show(ui);
    });
    assert_eq!(passes, 2, "the first frame runs two passes");
    assert!(!probe.0.is_empty(), "the float's window runs its pass in the second one");
}

/// The dock records a move the OS made to its native window, and nothing while the OS reports what the model holds.
#[trial]
fn the_dock_records_where_the_os_put_its_native_window() {
    let windows = Windows::new();
    windows.root_info(info(true, ROOT_AT));
    let mut window = Floating::single(100, vec![1], 0, Rect::from_min_size(Pos2::new(40.0, 40.0), Vec2::new(300.0, 200.0)));
    let mut saved = Placement::new(PALETTE_AT.size().into(), 1.0);
    saved.position_px = Some([200, 150]);
    window.native = Some(saved);
    let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![window])));
    let run = |workspace: &mut Arc<Workspace<u32>>| {
        let mut actions = Vec::new();
        windows.frame(vec![], vec![], |ui| {
            let config = DockConfig::default().native_windows(true);
            let shown = DockArea::new("Native dock", Bind::bound(workspace), &Specs::with(Permissions::ALL), &mut Palettes, config).show(ui);
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
        let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![float()])));
        let (mut child, mut recorded) = (false, false);
        for _ in 0..3 {
            let out = windows.frame(vec![], vec![], |ui| {
                let shown =
                    DockArea::new("Native dock", Bind::bound(&mut workspace), &Specs::with(Permissions::ALL), &mut Palettes, DockConfig::default())
                        .show(ui);
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
        let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![float()])));
        let out = windows.frame(vec![], vec![], |ui| {
            DockArea::new("Native dock", Bind::bound(&mut workspace), &Specs::with(Permissions::ALL), &mut Palettes, DockConfig::default()).show(ui);
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
    let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![float()])));
    let mut sizes = Vec::new();
    for _ in 0..3 {
        let out = windows.frame(vec![], vec![], |ui| {
            DockArea::new("Native dock", Bind::bound(&mut workspace), &Specs::with(Permissions::ALL), &mut Palettes, DockConfig::default()).show(ui);
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
        let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![float()])));
        let run = |workspace: &mut Arc<Workspace<u32>>| {
            let mut actions = Vec::new();
            windows.frame(vec![], vec![], |ui| {
                actions.extend(
                    DockArea::new("Native dock", Bind::bound(workspace), &Specs::with(rights), &mut Palettes, DockConfig::default()).show(ui).actions,
                );
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
    workspace: Arc<Workspace<u32>>,
    stepped: bool,
    specs: Tracked<Specs>,
    /// The palette window's builder as the integration keeps it: each frame's builder patched onto it, which sends the
    /// window only the limits a builder carries.
    builder: Option<ViewportBuilder>,
    /// The palette's window: the first child window the dock opened.
    palette: Option<ViewportId>,
    /// The tree the palette window's pass produced in the last frame.
    palette_tree: Option<AccessTree>,
    /// The shapes the palette window's pass painted in the last frame.
    palette_shapes: Vec<Shape>,
    /// Whether Auto-Collapse Iconic Panels is on in the frames of a collapsed palette.
    auto_collapse: bool,
    /// The panel the next frame asks the dock to raise (`DockArea::raise`).
    raise: Option<u32>,
}

impl NativeDock {
    fn on(os: Os, system: Arc<FakeSystem>) -> Self {
        let palette = Floating::single(101, vec![1], 0, Rect::from_min_size(Pos2::new(40.0, 40.0), FLOAT_AT.size()));
        Self::with(os, system, palette, false)
    }

    /// The dock with a stepped palette of one step ([`SteppedPalette`]).
    fn stepped(os: Os, system: Arc<FakeSystem>) -> Self {
        let column = Column::new(vec![Group::new(vec![1], 0)]).with_steps(1);
        Self::with(os, system, Floating::new(102, column, Rect::from_min_size(Pos2::new(40.0, 40.0), FLOAT_AT.size())), true)
    }

    fn with(os: Os, system: Arc<FakeSystem>, palette: Floating<u32>, stepped: bool) -> Self {
        let shell = ShellInfo::default().os(os).caps(Caps::native(os));
        let windows = Windows::with(InstallSpec::new(shell, system));
        windows.root_info(info(true, MAIN_AT));
        windows.child_info(info(true, FLOAT_AT));
        let workspace = Arc::new(Workspace::new(Layout::new(Some(Node::tabs(vec![2], 0)), vec![palette])));
        let specs = Tracked::new(Specs { rights: Permissions::ALL, least: false, stepped });
        let mut dock = NativeDock {
            windows,
            workspace,
            stepped,
            specs,
            builder: None,
            palette: None,
            palette_tree: None,
            palette_shapes: Vec::new(),
            auto_collapse: false,
            raise: None,
        };
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
        let (mut palettes, mut stepped) = (Palettes, SteppedPalette);
        let panels: &mut dyn DockPanels<u32> = if self.stepped { &mut stepped } else { &mut palettes };
        let specs = &self.specs;
        let raise = self.raise.take();
        let out = self.windows.frame(root, child, |ui| {
            let mut area = DockArea::new("Native dock", Bind::bound(workspace), specs, &mut *panels, DockConfig::default());
            if let Some(panel) = raise {
                area = area.raise(panel);
            }
            actions.extend(area.show(ui).actions);
        });
        if self.palette.is_none() {
            self.palette = out.builders.iter().map(|(viewport, _)| *viewport).find(|viewport| *viewport != ViewportId::ROOT);
        }
        let palette = self.palette;
        self.palette_tree = out.trees.into_iter().find(|(viewport, _)| Some(*viewport) == palette).map(|(_, tree)| tree);
        self.palette_shapes = out.shapes.into_iter().find(|(viewport, _)| Some(*viewport) == palette).map(|(_, shapes)| shapes).unwrap_or_default();
        for (_, builder) in out.builders.into_iter().filter(|(viewport, _)| Some(*viewport) == palette) {
            match self.builder.as_mut() {
                Some(kept) => {
                    let _ = kept.patch(builder);
                }
                None => self.builder = Some(builder),
            }
        }
        let commands = out.commands.into_iter().filter(|(viewport, _)| Some(*viewport) == palette).map(|(_, command)| command).collect();
        (actions, commands)
    }
}

/// `DockArea::raise` of a panel in a native palette focuses the palette's OS window once, in the pass it is given in;
/// a pass without it sends nothing.
#[trial]
fn a_raised_native_palette_gets_the_os_focus_once() {
    let mut dock = NativeDock::on(Os::Linux, Arc::new(FakeSystem::new()));
    let focus = |commands: &[ViewportCommand]| commands.iter().filter(|command| matches!(command, ViewportCommand::Focus)).count();
    let (_, settled) = dock.frame(vec![]);
    assert_eq!(focus(&settled), 0, "a pass without a raise focuses nothing");
    dock.raise = Some(1);
    let (_, raised) = dock.frame(vec![]);
    assert_eq!(focus(&raised), 1, "the raised palette's window gets the focus");
    let (_, after) = dock.frame(vec![]);
    assert_eq!(focus(&after), 0, "the raise is a request of its pass");
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

/// A palette moved by its title row over the main window's group lies over the mark the group shows: its own window
/// paints the mark too, at the same desktop place by the windows' inner rects, so the mark shows over the palette and
/// not only through its faded window.
#[trial]
fn a_native_palette_moved_over_a_drop_target_paints_the_mark_over_itself() {
    let mut dock = NativeDock::on(Os::Linux, Arc::new(FakeSystem::new()));
    let step = (OVER_GROUP - (FLOAT_AT.min + ROW_AT.to_vec2())) / 3.0;
    for events in [vec![moved(ROW_AT)], vec![press(ROW_AT)]] {
        dock.frame(events);
    }
    for _ in 0..3 {
        dock.frame(vec![moved(ROW_AT + step)]);
    }
    dock.frame(vec![]);
    let area = match drag::mark(&dock.windows.ctx) {
        Some(DropMark::Region { area, .. } | DropMark::Into(area)) => area,
        mark => panic!("the main window's group marks the region the palette takes: {mark:?}"),
    };
    dock.frame(vec![]);
    let palette = dock.palette.expect("the palette has its window");
    let inner = dock.windows.facts(palette).inner_rect.expect("the palette reports its inner rect");
    let mapped = area.translate(MAIN_AT.min - inner.min);
    let shows = dock.palette_shapes.iter().any(
        |shape| matches!(shape, Shape::Rect(rect) if (rect.rect.min - mapped.min).length() < 0.5 && (rect.rect.max - mapped.max).length() < 0.5),
    );
    assert!(shows, "the palette's window paints the mark at {mapped:?}, where the main window's {area:?} lies under it");
    for events in [vec![key_press(Key::Escape, Modifiers::NONE)], vec![release(ROW_AT)], vec![]] {
        dock.frame(events);
    }
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
    system.set_buttons_held(Some(HeldButtons::PRIMARY));
    dock.frame(vec![moved(ROW_AT)]);
    let (_, commands) = dock.frame(vec![press(ROW_AT)]);
    assert!(commands.contains(&ViewportCommand::StartDrag), "the press hands the move to the OS: {commands:?}");
    system.set_buttons_held(Some(HeldButtons::NONE));
    let mut clicked = false;
    for _ in 0..3 {
        clicked |= docks_window(&dock.frame(vec![]).0);
    }
    assert!(!clicked && drag::mark(&dock.windows.ctx).is_none(), "a click on the row docks nothing");
    system.set_buttons_held(Some(HeldButtons::PRIMARY));
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
    system.set_buttons_held(Some(HeldButtons::NONE));
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
    system.set_buttons_held(Some(HeldButtons::PRIMARY));
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
    system.set_buttons_held(Some(HeldButtons::NONE));
    for _ in 0..3 {
        docked |= docks_window(&dock.frame(vec![]).0);
    }
    assert!(docked, "the release docks the whole window");
}

/// The steps of the stepped palette's column.
fn palette_steps(dock: &NativeDock) -> Option<u16> {
    dock.workspace.layout().floating[0].column.steps
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
        for events in [vec![moved(grip)], vec![press(grip)], vec![moved(grip + Vec2::new(1.6 * (grid.width(2) - grid.width(1)), 0.0))], vec![]] {
            sent.extend(placed_frame(&mut dock, events, &mut at));
        }
        assert_eq!(palette_steps(&dock), Some(2), "{os:?}: a drag of 1.6 steps sets the whole step it reached");
        assert_eq!(at.width(), grid.width(2), "{os:?}: the window takes the width of its steps while the drag runs");
        let end = grip + Vec2::new(1.6 * (grid.width(2) - grid.width(1)), 0.0);
        for events in [vec![release(end)], vec![], vec![]] {
            sent.extend(placed_frame(&mut dock, events, &mut at));
        }
        assert_eq!(palette_steps(&dock), Some(2), "{os:?}: the release keeps the steps");

        let right = at.right();
        let grip = Pos2::new(1.0, at.height() / 2.0);
        let end = grip - Vec2::new(1.6 * (grid.width(2) - grid.width(1)), 0.0);
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
    let recorded = dock.workspace.layout().floating[0].native.clone().expect("the model records the window");
    assert_eq!(Vec2::from(recorded.normal.size), at.size(), "the model holds the OS's last size");
}

/// An app change of a native palette's place (an undo, a reloaded layout) moves and sizes its window, once, and the
/// window lands at that place: on Linux by one move and one size; on macOS, where AppKit keeps a resized window's
/// bottom-left corner, the move goes out again after the size.
#[trial]
fn an_app_change_of_a_native_floats_place_moves_its_window_once() {
    for os in [Os::Linux, Os::Mac] {
        let mut dock = NativeDock::on(os, Arc::new(FakeSystem::new()));
        let to = Rect::from_min_size(Pos2::new(1100.0, 200.0), Vec2::new(320.0, 260.0));
        let mut placement = Placement::new(to.size().into(), 1.0);
        placement.position_px = Some([to.min.x as i32, to.min.y as i32]);
        Arc::make_mut(&mut dock.workspace)
            .apply(Action::set_native(1, Some(placement)), false, |_| Permissions::ALL)
            .expect("the model takes the place");
        let (mut at, mut window) = (FLOAT_AT, AppKitWindow::new(FLOAT_AT, None));
        let mut sent = Vec::new();
        for _ in 0..3 {
            let commands = dock.frame(vec![]).1;
            window.run(&commands);
            at = if os == Os::Mac { window.rect } else { placed_by(&commands, at) };
            dock.windows.child_info(info(true, at));
            sent.push(placing(&commands).into_iter().cloned().collect::<Vec<_>>());
        }
        let expected = match os {
            Os::Mac => vec![ViewportCommand::OuterPosition(to.min), ViewportCommand::InnerSize(to.size()), ViewportCommand::OuterPosition(to.min)],
            _ => vec![ViewportCommand::OuterPosition(to.min), ViewportCommand::InnerSize(to.size())],
        };
        assert_eq!(sent.concat(), expected, "{os:?}: one move and one size");
        assert_eq!(sent.iter().filter(|frame| !frame.is_empty()).count(), 1, "{os:?}: in one frame");
        assert_eq!(at, to, "{os:?}: the window lands at the place");
    }
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
    // AppKit follows the window commands of each frame, within the largest size the previous frames left in force,
    // and the window reports where it is.
    let follow = |dock: &mut NativeDock, at: &mut Rect, frames: usize| {
        let mut sent = Vec::new();
        for _ in 0..frames {
            let mut window = AppKitWindow::new(*at, dock.builder.as_ref().and_then(|builder| builder.max_inner_size));
            let commands = dock.frame(vec![]).1;
            window.run(&commands);
            *at = window.rect;
            dock.windows.child_info(info(true, *at));
            sent.extend(commands);
        }
        sent
    };
    let mut at = FLOAT_AT;
    Arc::make_mut(&mut dock.workspace).apply(Action::set_iconic(1, true), false, |_| Permissions::ALL).expect("the column collapses");
    follow(&mut dock, &mut at, 3);
    let strip = at;
    assert!(strip.width() < expanded.x, "the OS window shows the strip: {strip:?}");
    let recorded = |dock: &NativeDock| dock.workspace.layout().floating[0].native.clone().expect("the model records the window");
    assert_eq!(Vec2::from(recorded(&dock).normal.size), expanded, "collapsed, the model keeps the expanded size");

    // The OS moves the strip over the start half of the main window.
    let dropped = Rect::from_min_size(Pos2::new(MAIN_AT.min.x + 50.0, strip.min.y), strip.size());
    at = dropped;
    dock.windows.child_info(info(true, at));
    let sent = follow(&mut dock, &mut at, 4);
    assert!(placing(&sent).is_empty(), "the strip stays where it was dropped: {:?}", placing(&sent));
    assert_eq!(at, dropped);
    assert_eq!(dock.workspace.layout().floating[0].edge, Some(DockEdge::Start), "it keeps to the edge nearer the strip");
    assert_eq!(Vec2::from(recorded(&dock).normal.size), expanded, "moved, the model keeps the expanded size");

    Arc::make_mut(&mut dock.workspace).apply(Action::set_iconic(1, false), false, |_| Permissions::ALL).expect("the column expands");
    let sent = follow(&mut dock, &mut at, 3);
    assert!(sent.contains(&ViewportCommand::InnerSize(expanded)), "the window takes its expanded size back: {:?}", placing(&sent));
    assert_eq!(at.min, dropped.min, "it opens from the strip's place, against the start edge, at the strip's top");
    assert_eq!(at.size(), expanded, "the OS window grows although the strip's limits capped it when the size was sent");
    let max = dock.builder.as_ref().and_then(|builder| builder.max_inner_size);
    assert!(max.is_none_or(|max| max.x >= expanded.x && max.y >= expanded.y), "the OS lets it take that size: max {max:?}");
}

/// On macOS a native palette collapsed to icons is a window AppKit resizes, in its width only: it opens resizable with
/// its height fixed to the strip's, the app stands no grips of its own at its sides, and a width the OS gives it is the
/// column's iconic width while the model keeps the expanded size; the app sends no size back.
#[trial]
fn a_collapsed_native_palette_on_macos_is_resized_by_appkit_in_its_width_only() {
    let mut dock = NativeDock::on(Os::Mac, Arc::new(FakeSystem::new()));
    let mut at = FLOAT_AT;
    // AppKit follows the window commands of each frame, and the window reports where it is.
    let follow = |dock: &mut NativeDock, at: &mut Rect, events: Vec<Event>| {
        let mut window = AppKitWindow::new(*at, dock.builder.as_ref().and_then(|builder| builder.max_inner_size));
        let commands = dock.frame(events).1;
        window.run(&commands);
        *at = window.rect;
        dock.windows.child_info(info(true, *at));
        commands
    };
    Arc::make_mut(&mut dock.workspace).apply(Action::set_iconic(1, true), false, |_| Permissions::ALL).expect("the column collapses");
    for _ in 0..3 {
        follow(&mut dock, &mut at, vec![]);
    }
    let strip = at;
    let builder = dock.builder.clone().expect("the palette's window opened");
    assert_eq!(builder.resizable, Some(true), "the collapsed window is resizable");
    let (least, most) = (builder.min_inner_size.expect("a least size"), builder.max_inner_size.expect("a most size"));
    assert!(least.y == most.y && (least.y - strip.height()).abs() <= 0.5, "the height is fixed to the strip's: {least:?}..{most:?}, {strip:?}");
    assert!(least.x < most.x, "the width resizes: {least:?}..{most:?}");

    // A press and drag at the strip's free side, in its title row, clear of the double arrow at the end it keeps to:
    // no app grip stands there, so nothing asks the OS for a resize.
    let end = strip.max.x >= FLOAT_AT.max.x - 1.0 && strip.min.x > FLOAT_AT.min.x + 1.0;
    let side = Pos2::new(if end { 2.0 } else { strip.width() - 2.0 }, 4.0);
    let mut sent = Vec::new();
    for events in [vec![moved(side)], vec![press(side)], vec![moved(side + Vec2::new(0.0, 1.0))], vec![release(side)], vec![]] {
        sent.extend(follow(&mut dock, &mut at, events));
    }
    assert!(!sent.iter().any(|command| matches!(command, ViewportCommand::BeginResize(_))), "the app stands no grip: {sent:?}");

    // AppKit widens the window.
    let width = strip.width() + 60.0;
    at = Rect::from_min_size(at.min, Vec2::new(width, at.height()));
    dock.windows.child_info(info(true, at));
    let mut sent = Vec::new();
    for _ in 0..3 {
        sent.extend(follow(&mut dock, &mut at, vec![]));
    }
    let palette = &dock.workspace.layout().floating[0];
    assert!(palette.column.iconic_width.is_some_and(|kept| (kept - width).abs() <= 0.5), "the column keeps the width: {:?}", palette.column);
    let recorded = palette.native.clone().expect("the model records the window");
    assert_eq!(Vec2::from(recorded.normal.size), FLOAT_AT.size(), "the model keeps the expanded size");
    assert!(!sent.iter().any(|command| matches!(command, ViewportCommand::InnerSize(_))), "no size goes back: {sent:?}");
    assert_eq!(at.width(), width, "the window keeps the width the OS gave it");
}

/// A native palette whose every group is minimized shrinks to its title row and tab strip at its place and width.
/// The OS resizes its width and never its height: AppKit takes the height as fixed, and the app sends no size back
/// for the width the OS gave it. The model keeps the expanded height throughout, and restored, the window takes that
/// height back at the width it was given.
#[trial]
fn a_minimized_native_palette_keeps_its_width_resizable_and_comes_back_at_its_height() {
    for os in [Os::Mac, Os::Windows] {
        let mut dock = NativeDock::on(os, Arc::new(FakeSystem::new()));
        let expanded = FLOAT_AT.size();
        let mut at = FLOAT_AT;
        // The OS follows the window commands of each frame, within the limits in force, and the window reports where it is:
        // AppKit keeps a resized window's bottom-left corner, Windows its top-left corner.
        let follow = |dock: &mut NativeDock, at: &mut Rect, frames: usize| {
            let mut sent = Vec::new();
            for _ in 0..frames {
                let mut max = dock.builder.as_ref().and_then(|builder| builder.max_inner_size);
                let mut window = AppKitWindow::new(*at, max);
                let commands = dock.frame(vec![]).1;
                if os == Os::Mac {
                    window.run(&commands);
                    *at = window.rect;
                }
                for command in commands.iter().filter(|_| os != Os::Mac) {
                    match command {
                        ViewportCommand::MaxInnerSize(limit) => max = Some(*limit),
                        ViewportCommand::InnerSize(size) => *at = Rect::from_min_size(at.min, max.map_or(*size, |max| size.min(max))),
                        ViewportCommand::OuterPosition(to) => *at = Rect::from_min_size(*to, at.size()),
                        _ => {}
                    }
                }
                dock.windows.child_info(info(true, *at));
                sent.extend(commands);
            }
            sent
        };
        let recorded =
            |dock: &NativeDock| Vec2::from(dock.workspace.layout().floating[0].native.clone().expect("the model records the window").normal.size);
        Arc::make_mut(&mut dock.workspace).apply(Action::set_minimized(1, true), false, |_| Permissions::ALL).expect("the group minimizes");
        follow(&mut dock, &mut at, 3);
        let strips = at;
        assert!(strips.height() < expanded.y / 4.0, "{os:?}: the window shrinks to its title row and strip: {strips:?}");
        if let Err(fit) = fits_exactly(dock.palette_tree.as_ref().expect("the palette ran a pass"), strips.size(), Role::TAB_LIST) {
            panic!("{os:?}: the minimized window fits its title row and strip: {fit}");
        }
        assert_eq!((strips.min, strips.width()), (FLOAT_AT.min, expanded.x), "{os:?}: at its place and width");
        assert_eq!(recorded(&dock), expanded, "{os:?}: minimized, the model keeps the expanded size");
        let builder = dock.builder.clone().expect("the palette's window opened");
        let (least, most) = (builder.min_inner_size.expect("a least size"), builder.max_inner_size.expect("a most size"));
        assert!(least.y == most.y && (least.y - strips.height()).abs() <= 0.5, "{os:?}: the height is fixed: {least:?}..{most:?}, {strips:?}");
        assert!(least.x < most.x, "{os:?}: the width resizes: {least:?}..{most:?}");
        if os == Os::Mac {
            assert_eq!(builder.resizable, Some(true), "AppKit resizes the minimized window");
        }

        // The OS widens the window.
        let width = expanded.x + 60.0;
        at = Rect::from_min_size(at.min, Vec2::new(width, at.height()));
        dock.windows.child_info(info(true, at));
        let sent = follow(&mut dock, &mut at, 3);
        assert!(!sent.iter().any(|command| matches!(command, ViewportCommand::InnerSize(_))), "{os:?}: no size goes back: {sent:?}");
        assert_eq!(recorded(&dock), Vec2::new(width, expanded.y), "{os:?}: the model takes the width and keeps the expanded height");

        Arc::make_mut(&mut dock.workspace).apply(Action::set_minimized(1, false), false, |_| Permissions::ALL).expect("the group restores");
        follow(&mut dock, &mut at, 3);
        assert_eq!(at, Rect::from_min_size(FLOAT_AT.min, Vec2::new(width, expanded.y)), "{os:?}: restored, the window takes its height back");
    }
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
    Arc::make_mut(&mut dock.workspace).apply(Action::set_iconic(1, true), false, |_| Permissions::ALL).expect("the column collapses");
    for _ in 0..3 {
        frame(&mut dock, &mut at, vec![]);
    }
    assert!(dock.workspace.layout().floating[0].column.iconic, "the column is collapsed");
    assert!(at.width() < FLOAT_AT.width(), "the OS window shows the strip: {at:?}");

    // The double arrow is the title row's square at the end the strip keeps to: a point just inside its outer corner.
    let end = at.max.x >= FLOAT_AT.max.x - 1.0 && at.min.x > FLOAT_AT.min.x + 1.0;
    let arrow = Pos2::new(if end { at.width() - 4.0 } else { 4.0 }, 4.0);
    let mut actions = Vec::new();
    for events in [vec![moved(arrow)], vec![press(arrow)], vec![release(arrow)], vec![], vec![]] {
        actions.extend(frame(&mut dock, &mut at, events));
    }
    assert!(!dock.workspace.layout().floating[0].column.iconic, "the click expands the column: {actions:?}");
    assert_eq!(at.size(), FLOAT_AT.size(), "the OS window takes its expanded size");
}

/// The display scale of a MacBook's built-in display.
const RETINA: f32 = 2.0;

/// A native palette on a Retina Mac whose window AppKit runs, after its first frames.
fn appkit_palette(system: &Arc<FakeSystem>) -> (NativeDock, AppKitWindow) {
    let mut dock = NativeDock::on(Os::Mac, Arc::clone(system));
    dock.windows.root_info(scaled(true, MAIN_AT, RETINA));
    let mut window = AppKitWindow::new(FLOAT_AT, None);
    dock.windows.child_info(scaled(true, window.rect, RETINA));
    for _ in 0..2 {
        appkit_frame(&mut dock, &mut window, vec![]);
    }
    (dock, window)
}

/// One frame with `child` events for the palette's window, whose commands AppKit runs; the window then reports where
/// it is.
fn appkit_frame(dock: &mut NativeDock, window: &mut AppKitWindow, child: Vec<Event>) -> (Vec<Action<u32>>, Vec<ViewportCommand>) {
    let (actions, commands) = dock.frame(child);
    window.run(&commands);
    dock.windows.child_info(scaled(true, window.rect, RETINA));
    (actions, commands)
}

/// A click at `at` in the palette's window, with the system pointer and button as macOS tells them, and the frames
/// after it.
fn appkit_click(dock: &mut NativeDock, system: &FakeSystem, window: &mut AppKitWindow, at: Pos2) -> Vec<Action<u32>> {
    let desktop = window.rect.min + at.to_vec2();
    system.set_pointer(Some([desktop.x, desktop.y]));
    let mut actions = appkit_frame(dock, window, vec![moved(at)]).0;
    system.set_buttons_held(Some(HeldButtons::PRIMARY));
    actions.extend(appkit_frame(dock, window, vec![press(at)]).0);
    system.set_buttons_held(Some(HeldButtons::NONE));
    actions.extend(appkit_frame(dock, window, vec![release(at)]).0);
    for _ in 0..3 {
        actions.extend(appkit_frame(dock, window, vec![]).0);
    }
    actions
}

/// The rect of the palette's button whose name starts with `name`, in its window's points.
fn palette_button(dock: &NativeDock, name: &str) -> Rect {
    let tree = dock.palette_tree.as_ref().expect("the palette ran a pass");
    tree.keys()
        .into_iter()
        .find(|key| tree.role_of(*key) == Some(Role::BUTTON) && tree.name_of(*key).is_some_and(|shown| shown.starts_with(name)))
        .and_then(|key| tree.bounds_of(key))
        .unwrap_or_else(|| panic!("the palette shows a button named {name}…: {:?}", nodes(tree)))
}

/// On a Retina Mac, clicks on the double arrow collapse a native palette to its strip of icons and expand it again,
/// as AppKit runs the window commands: the strip shows at the window's top against the edge the window keeps to, and
/// expanded, the window takes its size back at its place.
#[trial]
fn the_double_arrow_collapses_and_expands_a_native_palette_on_macos() {
    let system = Arc::new(FakeSystem::new());
    let (mut dock, mut window) = appkit_palette(&system);
    let collapse = palette_button(&dock, "Collapse").center();
    let actions = appkit_click(&mut dock, &system, &mut window, collapse);
    let strip = window.rect;
    assert!(dock.workspace.layout().floating[0].column.iconic, "the click collapses the column: {actions:?}");
    assert!(strip.width() < FLOAT_AT.width() && strip.height() < FLOAT_AT.height(), "the window shows the strip: {strip:?}");
    assert!((strip.max.x - FLOAT_AT.max.x).abs() <= 0.5, "the strip keeps to the window's end edge: {strip:?} from {FLOAT_AT:?}");
    assert!((strip.min.y - FLOAT_AT.min.y).abs() <= 0.5, "the strip keeps the window's top: {strip:?} from {FLOAT_AT:?}");

    let expand = palette_button(&dock, "Expand").center();
    let actions = appkit_click(&mut dock, &system, &mut window, expand);
    assert!(!dock.workspace.layout().floating[0].column.iconic, "the click expands the column: {actions:?}");
    assert_eq!(window.rect.size(), FLOAT_AT.size(), "the window takes its expanded size");
    assert!((window.rect.min - FLOAT_AT.min).length() <= 0.5, "at its place: {:?} from {FLOAT_AT:?}", window.rect);
}

/// A native palette on a Retina Mac collapsed to icons by its double arrow.
fn collapsed_appkit_palette(system: &Arc<FakeSystem>) -> (NativeDock, AppKitWindow) {
    let (mut dock, mut window) = appkit_palette(system);
    let collapse = palette_button(&dock, "Collapse").center();
    appkit_click(&mut dock, system, &mut window, collapse);
    assert!(dock.workspace.layout().floating[0].column.iconic, "the click collapses the column");
    (dock, window)
}

/// Whether `actions` dock a window or a group of it.
fn docks(actions: &[Action<u32>]) -> bool {
    actions.iter().any(|action| matches!(action, Action::MoveWindow { .. } | Action::MoveGroup { .. }))
}

/// Drags the collapsed palette from `from`, a point of its window, until the system pointer is over the main window's
/// group, as macOS runs a drag: the press holds the pointer in the palette's window, and AppKit moves the window
/// itself when the press hands it the move. Returns the commands of the press's frame and whether anything docked.
fn appkit_drag_over_group(dock: &mut NativeDock, system: &FakeSystem, window: &mut AppKitWindow, from: Pos2) -> (Vec<ViewportCommand>, bool) {
    let pressed_at = window.rect.min + from.to_vec2();
    system.set_pointer(Some([pressed_at.x, pressed_at.y]));
    system.set_buttons_held(Some(HeldButtons::PRIMARY));
    appkit_frame(dock, window, vec![moved(from)]);
    let (actions, pressed) = appkit_frame(dock, window, vec![press(from)]);
    let mut docked = docks(&actions);
    let handed = pressed.contains(&ViewportCommand::StartDrag);
    let step = (OVER_GROUP - pressed_at) / 3.0;
    for at in 1..=3 {
        let pointer = pressed_at + step * at as f32;
        system.set_pointer(Some([pointer.x, pointer.y]));
        if handed {
            // AppKit keeps the pressed point under the pointer and sends the app no pointer events.
            window.rect = Rect::from_min_size(pointer - from.to_vec2(), window.rect.size());
            dock.windows.child_info(scaled(true, window.rect, RETINA));
            docked |= docks(&appkit_frame(dock, window, vec![]).0);
        } else {
            docked |= docks(&appkit_frame(dock, window, vec![moved(pointer - window.rect.min.to_vec2())]).0);
        }
    }
    for _ in 0..2 {
        docked |= docks(&appkit_frame(dock, window, vec![]).0);
    }
    (pressed, docked)
}

/// On a Retina Mac a native palette collapsed to icons moves by its title row as an expanded one does: the press hands
/// the move to the OS, the system pointer over the main window's group marks it, and the button's release docks the
/// whole window there.
#[trial]
fn dragging_a_collapsed_native_palette_by_its_title_row_docks_it_on_macos() {
    let system = Arc::new(FakeSystem::new());
    let (mut dock, mut window) = collapsed_appkit_palette(&system);
    let arrow = palette_button(&dock, "Expand");
    // The band: the title row beside the double arrow, half way to the strip's other side.
    let width = window.rect.width();
    let band = Pos2::new(if arrow.center().x > width / 2.0 { arrow.min.x / 2.0 } else { (arrow.max.x + width) / 2.0 }, arrow.center().y);
    let (pressed, docked) = appkit_drag_over_group(&mut dock, &system, &mut window, band);
    assert!(pressed.contains(&ViewportCommand::StartDrag), "the press hands the move to the OS: {pressed:?}");
    assert!(drag::mark(&dock.windows.ctx).is_some(), "the group under the system pointer shows the drop mark");
    assert!(!docked, "nothing docks while the system holds the button");
    system.set_buttons_held(Some(HeldButtons::NONE));
    let mut docked = false;
    for _ in 0..3 {
        docked |= docks(&appkit_frame(&mut dock, &mut window, vec![]).0);
    }
    assert!(docked, "the button's release docks the whole window");
}

/// On a Retina Mac the grip of a native palette's icon group, collapsed to icons, drags the group: over the main
/// window's group it shows the drop mark, and the release docks it there.
#[trial]
fn dragging_the_group_grip_of_a_collapsed_native_palette_docks_it_on_macos() {
    let system = Arc::new(FakeSystem::new());
    let (mut dock, mut window) = collapsed_appkit_palette(&system);
    let grip = palette_button(&dock, "Move").center();
    let (pressed, docked) = appkit_drag_over_group(&mut dock, &system, &mut window, grip);
    assert!(!pressed.contains(&ViewportCommand::StartDrag), "the grip does not move the window: {pressed:?}");
    assert!(drag::mark(&dock.windows.ctx).is_some(), "the group under the pointer shows the drop mark");
    assert!(!docked, "nothing docks while the button is held");
    let release_at = OVER_GROUP - window.rect.min.to_vec2();
    system.set_buttons_held(Some(HeldButtons::NONE));
    let mut docked = docks(&appkit_frame(&mut dock, &mut window, vec![release(release_at)]).0);
    for _ in 0..2 {
        docked |= docks(&appkit_frame(&mut dock, &mut window, vec![]).0);
    }
    assert!(docked, "the release docks the group");
}

/// Panels whose palette panel's body records, in each of its real runs, the viewport it runs in.
#[derive(Default)]
struct BodyProbe(Vec<ViewportId>);

impl DockPanels<u32> for BodyProbe {
    fn body(&mut self, panel: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let ctx = ui.ctx();
        if *panel == 1 && !craft_ui::testing::is_measuring(ctx) {
            self.0.push(ctx.viewport_id());
        }
    }
}

/// The scale of the display the collapsed palette trials run on: Windows at 125 %, where the issue was reported.
const COLLAPSED_SCALE: f32 = 1.25;

/// A native palette collapsed to icons on Windows at 125 %, and the rect its OS window shows the strip at.
fn collapsed_palette() -> (NativeDock, Rect) {
    let mut dock = NativeDock::on(Os::Windows, Arc::new(FakeSystem::new()));
    // Both windows report the display at 125 % before the column collapses.
    dock.windows.root_info(scaled(true, MAIN_AT, COLLAPSED_SCALE));
    let mut at = FLOAT_AT;
    dock.windows.child_info(scaled(true, at, COLLAPSED_SCALE));
    collapsed_frame(&mut dock, &mut BodyProbe::default(), &mut at, vec![], vec![]);
    Arc::make_mut(&mut dock.workspace).apply(Action::set_iconic(1, true), false, |_| Permissions::ALL).expect("the column collapses");
    for _ in 0..4 {
        collapsed_frame(&mut dock, &mut BodyProbe::default(), &mut at, vec![], vec![]);
    }
    (dock, at)
}

/// One frame of `dock` with `panels`, `root` events for the main window and `child` events for the palette's window,
/// whose OS window follows its commands at `at`: the frame's output.
fn collapsed_frame(dock: &mut NativeDock, panels: &mut dyn DockPanels<u32>, at: &mut Rect, root: Vec<Event>, child: Vec<Event>) -> Out {
    let (workspace, specs, auto_collapse) = (&mut dock.workspace, &dock.specs, dock.auto_collapse);
    let out = dock.windows.frame(root, child, |ui| {
        let area = DockArea::new("Native dock", Bind::bound(workspace), specs, &mut *panels, DockConfig::default());
        let _ = if auto_collapse { area.auto_collapse() } else { area }.show(ui);
    });
    let palette = dock.palette;
    let commands: Vec<ViewportCommand> =
        out.commands.iter().filter(|(viewport, _)| Some(*viewport) == palette).map(|(_, command)| command.clone()).collect();
    *at = placed_by(&commands, *at);
    dock.windows.child_info(scaled(true, *at, COLLAPSED_SCALE));
    out
}

/// The windows the frame showed other than the main window and the palette's: its popup windows.
fn popups(dock: &NativeDock, out: &Out) -> Vec<ViewportId> {
    out.builders.iter().map(|(viewport, _)| *viewport).filter(|viewport| *viewport != ViewportId::ROOT && Some(*viewport) != dock.palette).collect()
}

/// The rect of `viewport`'s window in desktop points, as the window system reports it.
fn window_rect(dock: &NativeDock, viewport: ViewportId) -> Rect {
    dock.windows.facts(viewport).inner_rect.expect("the window reports its rect")
}

/// The rect, in its window's points, of the node of `tree` named `name` whose role is `role`.
fn node_rect(tree: &AccessTree, name: &str, role: Role) -> Option<Rect> {
    tree.keys().into_iter().find(|key| tree.name_of(*key) == Some(name) && tree.role_of(*key) == Some(role)).and_then(|key| tree.bounds_of(key))
}

/// How far a node's edge may stray from the window's for float rounding: far under a pixel at any scale.
const FIT_ROUNDING: f32 = 0.05;

/// Whether a window `size` big fits what its pass shows in `tree` exactly: every node lies inside it, and the lowest
/// node of `role` ends at its bottom, within float rounding.
fn fits_exactly(tree: &AccessTree, size: Vec2, role: Role) -> Result<(), String> {
    let window = Rect::from_min_size(Pos2::ZERO, size);
    let outside: Vec<Rect> =
        tree.keys().into_iter().filter_map(|key| tree.bounds_of(key)).filter(|rect| !window.expand(FIT_ROUNDING).contains_rect(*rect)).collect();
    let bottom = tree.with_role(role).into_iter().filter_map(|key| tree.bounds_of(key)).map(|rect| rect.max.y).fold(f32::MIN, f32::max);
    if !outside.is_empty() {
        return Err(format!("nodes past the {size:?} window: {outside:?}"));
    }
    if (bottom - size.y).abs() > FIT_ROUNDING {
        return Err(format!("the content ends at {bottom}, the {size:?} window at {}", size.y));
    }
    Ok(())
}

/// The role and name of each named node of `tree`.
fn nodes(tree: &AccessTree) -> Vec<(Option<Role>, String)> {
    let mut nodes: Vec<(Option<Role>, String)> =
        tree.keys().into_iter().filter_map(|key| tree.name_of(key).map(|name| (tree.role_of(key), name.to_owned()))).collect();
    nodes.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    nodes
}

/// The tree the frame's pass of `viewport` produced.
fn tree_of_viewport(out: &Out, viewport: ViewportId) -> &AccessTree {
    &out.trees.iter().find(|(shown, _)| *shown == viewport).expect("the viewport ran a pass").1
}

/// Opens the flyout of the collapsed palette's panel by a click on its icon; returns the flyout's popup window.
fn open_flyout(dock: &mut NativeDock, panels: &mut dyn DockPanels<u32>, at: &mut Rect) -> ViewportId {
    let out = collapsed_frame(dock, panels, at, vec![], vec![]);
    let palette = dock.palette.expect("the palette's window opened");
    let icon = node_rect(tree_of_viewport(&out, palette), PALETTE_TITLE, Role::BUTTON).expect("the strip shows the panel's icon").center();
    for events in [vec![moved(icon)], vec![press(icon)], vec![release(icon)]] {
        collapsed_frame(dock, panels, at, vec![], events);
    }
    let out = collapsed_frame(dock, panels, at, vec![], vec![]);
    let opened = popups(dock, &out);
    assert_eq!(opened.len(), 1, "the click opens one popup window: {opened:?}");
    opened[0]
}

/// A click on the icon of a native palette collapsed to icons opens the panel's flyout in a popup OS window of its
/// own, beside the strip against its side away from the edge it keeps to, in desktop points: a palette-level window
/// with no taskbar entry that takes the focus. The flyout's body runs in the popup's pass only, as often in each
/// frame, never in the palette's.
#[trial]
fn a_click_on_an_icon_of_a_collapsed_native_palette_opens_its_flyout_in_an_os_window_beside_it() {
    let (mut dock, mut at) = collapsed_palette();
    let mut probe = BodyProbe::default();
    let palette = dock.palette.expect("the palette's window opened");
    let strip = nodes(tree_of_viewport(&collapsed_frame(&mut dock, &mut probe, &mut at, vec![], vec![]), palette));
    let popup = open_flyout(&mut dock, &mut probe, &mut at);
    let flyout = window_rect(&dock, popup);
    let end = at.max.x >= FLOAT_AT.max.x - 1.0;
    let outer = if end { (flyout.max.x - at.min.x).abs() } else { (flyout.min.x - at.max.x).abs() };
    assert!(outer <= 0.5, "the flyout stands against the strip's side away from its edge: {flyout:?} beside {at:?}");
    assert!(flyout.min.y >= at.min.y - 0.5 && flyout.min.y < at.max.y, "the flyout's top is level with the strip: {flyout:?}, {at:?}");
    let out = collapsed_frame(&mut dock, &mut probe, &mut at, vec![], vec![]);
    let builder = &out.builders.iter().find(|(viewport, _)| *viewport == popup).expect("the popup shows").1;
    assert_eq!(builder.window_level, Some(egui::WindowLevel::AlwaysOnTop), "the popup stands as a palette of the active app");
    assert_eq!(builder.taskbar, Some(false), "the popup has no taskbar entry");
    assert_ne!(builder.active, Some(false), "the flyout takes the focus");
    assert_eq!(nodes(tree_of_viewport(&out, palette)), strip, "the palette's window shows none of the flyout");
    assert!(!tree_of_viewport(&out, popup).keys().is_empty(), "the popup window shows the flyout");
    let runs = |probe: &mut BodyProbe, dock: &mut NativeDock, at: &mut Rect| {
        probe.0.clear();
        collapsed_frame(dock, probe, at, vec![], vec![]);
        std::mem::take(&mut probe.0)
    };
    let (first, second) = (runs(&mut probe, &mut dock, &mut at), runs(&mut probe, &mut dock, &mut at));
    assert!(!first.is_empty() && first.iter().all(|viewport| *viewport == popup), "the body runs in the popup's pass: {first:?}");
    assert_eq!(first.len(), second.len(), "the body runs as often in each frame");
}

/// The pointer rests on the double arrow of a collapsed native palette past the tooltip delay: the frame that follows.
fn hovered_arrow(dock: &mut NativeDock, at: &mut Rect) -> (Out, Rect) {
    let end = at.max.x >= FLOAT_AT.max.x - 1.0 && at.min.x > FLOAT_AT.min.x + 1.0;
    let arrow = Pos2::new(if end { at.width() - 4.0 } else { 4.0 }, 4.0);
    collapsed_frame(dock, &mut BodyProbe::default(), at, vec![], vec![moved(arrow)]);
    // The tooltip shows after its delay, at 60 frames a second.
    let frames = (craft_ui::contract::tooltip::DELAY * 60.0).ceil() as usize + 2;
    for _ in 0..frames {
        collapsed_frame(dock, &mut BodyProbe::default(), at, vec![], vec![]);
    }
    let out = collapsed_frame(dock, &mut BodyProbe::default(), at, vec![], vec![]);
    let palette = dock.palette.expect("the palette's window opened");
    let name = tree_of_viewport(&out, palette).keys().into_iter().find_map(|key| {
        let tree = tree_of_viewport(&out, palette);
        tree.name_of(key)
            .filter(|name| name.starts_with("Expand"))
            .filter(|_| tree.role_of(key) == Some(Role::BUTTON))
            .and_then(|_| tree.bounds_of(key))
    });
    let arrow_rect = name.expect("the title row shows the double arrow");
    (out, arrow_rect)
}

/// A native palette collapsed to icons paints the icons of its panels and no text cut by its window, also while the
/// pointer rests on its double arrow: the arrow's tooltip shows outside the strip, not wrapped inside it, and the
/// window fits the title row, its rule and the icons exactly.
#[trial]
fn a_collapsed_native_palette_paints_its_icons_and_no_clipped_text() {
    let (mut dock, mut at) = collapsed_palette();
    let (out, _) = hovered_arrow(&mut dock, &mut at);
    let palette = dock.palette.expect("the palette's window opened");
    let tree = tree_of_viewport(&out, palette);
    let window = Rect::from_min_size(Pos2::ZERO, at.size());
    let icon = node_rect(tree, PALETTE_TITLE, Role::BUTTON).expect("the strip shows the panel's icon");
    assert!(window.contains(icon.center()), "the icon shows in the strip: {icon:?} in {window:?}");
    assert!(tree.with_role(Role::TOOLTIP).is_empty(), "no tooltip shows inside the strip");
    if let Err(fit) = fits_exactly(tree, at.size(), Role::BUTTON) {
        panic!("the strip's window fits its icons: {fit}");
    }
}

/// A native palette on a monitor at 1x while the main window is on one at 2x (a laptop with an external display):
/// collapsed to icons, its window fits the title row, the rule under it and the icons exactly at its own scale.
#[trial]
fn a_collapsed_native_palette_on_a_1x_monitor_beside_a_2x_main_window_fits_its_icons_exactly() {
    let mut dock = NativeDock::on(Os::Mac, Arc::new(FakeSystem::new()));
    dock.windows.root_info(scaled(true, MAIN_AT, 2.0));
    let mut at = FLOAT_AT;
    dock.windows.child_info(scaled(true, at, 1.0));
    Arc::make_mut(&mut dock.workspace).apply(Action::set_iconic(1, true), false, |_| Permissions::ALL).expect("the column collapses");
    for _ in 0..4 {
        let commands = dock.frame(vec![]).1;
        at = placed_by(&commands, at);
        dock.windows.child_info(scaled(true, at, 1.0));
    }
    let tree = dock.palette_tree.as_ref().expect("the palette ran a pass");
    if let Err(fit) = fits_exactly(tree, at.size(), Role::BUTTON) {
        panic!("the strip's window fits its icons at its own scale: {fit}");
    }
}

/// The tooltip of a collapsed native palette's double arrow opens in a popup OS window below the arrow, in desktop
/// points, on one line: the window takes no focus and lets the pointer through to the palette.
#[trial]
fn a_tooltip_from_a_native_palette_opens_in_a_popup_os_window() {
    let (mut dock, mut at) = collapsed_palette();
    let (out, arrow) = hovered_arrow(&mut dock, &mut at);
    let shown = popups(&dock, &out);
    let tip = shown
        .iter()
        .copied()
        .find(|viewport| !tree_of_viewport(&out, *viewport).with_role(Role::TOOLTIP).is_empty())
        .expect("the tooltip shows in a popup window");
    let builder = &out.builders.iter().find(|(viewport, _)| *viewport == tip).expect("the popup shows").1;
    assert_eq!(builder.active, Some(false), "the tooltip's window takes no focus");
    assert_eq!(builder.mouse_passthrough, Some(true), "the pointer passes through the tooltip's window");
    let rect = window_rect(&dock, tip);
    assert!(rect.min.y >= at.min.y + arrow.max.y - 0.5, "the tooltip stands below the arrow: {rect:?}, arrow {arrow:?} in {at:?}");
    assert!(rect.width() > at.width() && rect.height() < rect.width(), "the tooltip's text runs on one line, wider than the strip: {rect:?}");
}

/// With Auto-Collapse Iconic Panels on, a press in the main window closes the flyout a collapsed native palette opened
/// in a popup window; with it off the flyout stays open.
#[trial]
fn a_press_in_the_main_window_closes_a_native_palettes_flyout_with_auto_collapse() {
    for auto_collapse in [true, false] {
        let (mut dock, mut at) = collapsed_palette();
        dock.auto_collapse = auto_collapse;
        let mut probe = BodyProbe::default();
        let popup = open_flyout(&mut dock, &mut probe, &mut at);
        let over = OVER_GROUP - MAIN_AT.min.to_vec2();
        for root in [vec![moved(over)], vec![press(over)], vec![release(over)]] {
            collapsed_frame(&mut dock, &mut probe, &mut at, root, vec![]);
        }
        // The flyout fades out, then its window closes.
        let closed = (0..60).any(|_| {
            let out = collapsed_frame(&mut dock, &mut probe, &mut at, vec![], vec![]);
            !popups(&dock, &out).contains(&popup)
        });
        assert_eq!(closed, auto_collapse, "auto-collapse {auto_collapse}: the flyout's window closed only with it on");
    }
}

/// Palettes no smaller than `PALETTE_LEAST`.
struct Least;

const PALETTE_LEAST: Vec2 = Vec2::new(240.0, 160.0);

impl DockPanels<u32> for Least {
    fn body(&mut self, _: &u32, _: &mut Ui, _: &mut BodyCx<'_>) {}
}

/// A model place smaller than the palette's limits sizes its OS window within them: the window is never asked for a
/// size the OS would clamp, so the size the model records is the one the window has.
#[trial]
fn a_native_palette_is_never_sized_below_its_limits() {
    let windows = Windows::new();
    windows.root_info(info(true, MAIN_AT));
    windows.child_info(info(true, FLOAT_AT));
    let mut window = Floating::single(103, vec![1], 0, Rect::from_min_size(Pos2::new(40.0, 40.0), FLOAT_AT.size()));
    let mut saved = Placement::new(FLOAT_AT.size().into(), 1.0);
    saved.position_px = Some([FLOAT_AT.min.x as i32, FLOAT_AT.min.y as i32]);
    window.native = Some(saved);
    let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![window])));
    let run = |workspace: &mut Arc<Workspace<u32>>| {
        let out = windows.frame(vec![], vec![], |ui| {
            let _ = DockArea::new(
                "Native dock",
                Bind::bound(workspace),
                &Tracked::new(Specs { rights: Permissions::ALL, least: true, stepped: false }),
                &mut Least,
                DockConfig::default().native_windows(true),
            )
            .show(ui);
        });
        out.commands.into_iter().filter(|(viewport, _)| *viewport != ViewportId::ROOT).map(|(_, command)| command).collect::<Vec<_>>()
    };
    for _ in 0..3 {
        run(&mut workspace);
    }
    let mut small = Placement::new([80.0, 60.0], 1.0);
    small.position_px = Some([FLOAT_AT.min.x as i32, FLOAT_AT.min.y as i32]);
    Arc::make_mut(&mut workspace).apply(Action::set_native(1, Some(small)), false, |_| Permissions::ALL).expect("the model takes the place");
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
    let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![float()])));
    let mut child = false;
    let mut run = |workspace: &mut Arc<Workspace<u32>>, events: Vec<Event>| {
        let out = windows.frame(events, vec![], |ui| {
            let _ =
                DockArea::new("Native dock", Bind::bound(workspace), &Specs::with(Permissions::ALL), &mut Palettes, DockConfig::default()).show(ui);
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
    let moved_to = workspace.layout().floating[0].rect.min;
    assert!(
        (moved_to - (start.min + by)).length() <= by.length() / 2.0,
        "the drag moved the float with the pointer: {moved_to:?} from {:?}",
        start.min
    );
}

/// Palettes whose body is a veil over the window: a target inside the window that takes dropped files.
struct Veiled;

impl DockPanels<u32> for Veiled {
    fn body(&mut self, _: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let _ = DropVeil::new("Drop").show_with_files(ui, |_, _| {});
    }
}

/// Files dropped on a native float's own OS window reach `DockAreaOutput::window_files` once, unless a target inside
/// that window took them. The main window, whose pass runs first, sees none of them, as the OS sends them only to the
/// window under the pointer: it must not end the drag of files over the float's window.
#[trial]
fn a_file_dropped_on_a_native_float_reaches_window_files_once() {
    let reached = |veiled: bool| {
        let windows = Windows::new();
        windows.root_info(info(true, ROOT_AT));
        windows.child_info(info(true, PALETTE_AT));
        let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![float()])));
        let mut frame = |hovered: Vec<HoveredFile>, dropped: Vec<String>| {
            let mut files = 0;
            windows.frame_with_files(vec![], vec![], FilesOver::Child, hovered, dropped, |ui| {
                let config = DockConfig::default();
                let shown = if veiled {
                    DockArea::new("Native dock", Bind::bound(&mut workspace), &Specs::with(Permissions::ALL), &mut Veiled, config).show(ui)
                } else {
                    DockArea::new("Native dock", Bind::bound(&mut workspace), &Specs::with(Permissions::ALL), &mut Palettes, config).show(ui)
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

/// Palettes that record where each group showed, by its panels, in the window it showed in.
#[derive(Default)]
struct GroupRects(Vec<(Vec<u32>, Rect)>);

impl DockPanels<u32> for GroupRects {
    fn body(&mut self, _: &u32, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn group_view(&mut self, panels: &[u32], _: usize, _: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if let GroupAt::Show(rect) = at {
            self.0.retain(|(shown, _)| shown != panels);
            self.0.push((panels.to_vec(), rect));
        }
        None
    }
}

/// A file dropped on a native float of two groups names the shown panel of the group under the drop, once.
#[trial]
fn a_file_dropped_on_a_native_float_names_the_group_under_it() {
    let dropped_on = |lower: bool| {
        let windows = Windows::new();
        windows.root_info(info(true, ROOT_AT));
        windows.child_info(info(true, PALETTE_AT));
        let column = Column::new(vec![Group::new(vec![1], 0), Group::new(vec![2], 0)]);
        let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![Floating::new(105, column, PALETTE_AT)])));
        let mut panels = GroupRects::default();
        let specs = Specs::with(Permissions::ALL);
        let mut frame = |child: Vec<Event>, hovered: Vec<HoveredFile>, dropped: Vec<String>, panels: &mut GroupRects| {
            let mut reached = Vec::new();
            windows.frame_with_files(vec![], child, FilesOver::Child, hovered, dropped, |ui| {
                let area = DockArea::new("Native dock", Bind::bound(&mut workspace), &specs, panels, DockConfig::default());
                reached.extend(area.show(ui).window_files.into_iter().map(|files| files.panel));
            });
            reached
        };
        for _ in 0..4 {
            frame(vec![], vec![], vec![], &mut panels);
        }
        let group = |panel: u32| panels.0.iter().find(|(shown, _)| shown == &[panel]).map(|(_, rect)| *rect).expect("the group showed");
        let at = group(if lower { 2 } else { 1 }).center();
        let mut reached = frame(vec![moved(at)], vec![HoveredFile::default()], vec![], &mut panels);
        reached.extend(frame(vec![moved(at)], vec![], vec!["a.png".to_owned()], &mut panels));
        for _ in 0..2 {
            reached.extend(frame(vec![moved(at)], vec![], vec![], &mut panels));
        }
        reached
    };
    assert_eq!(dropped_on(true), vec![2], "a drop on the lower group names its panel once");
    assert_eq!(dropped_on(false), vec![1], "a drop on the upper group names its panel once");
}

/// A palette whose panel holds an Opacity field, then reads the keys its OS window's pass left, as an app passes a
/// panel window's leftover Enter and Escape on to its main window: how many of each it read.
#[derive(Default)]
struct FieldPalette {
    opacity: Option<f64>,
    submits: usize,
    cancels: usize,
}

impl DockPanels<u32> for FieldPalette {
    fn body(&mut self, _: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        NumberField::new("Opacity", Bind::bound(&mut self.opacity)).show(ui);
        if ui.ctx().viewport_id() != ViewportId::ROOT {
            self.submits += usize::from(keys::with_window_intents(ui.ctx(), false, |window| window.take(Intent::Submit)));
            self.cancels += usize::from(keys::with_window_intents(ui.ctx(), false, |window| window.take(Intent::Cancel)));
        }
    }
}

/// Enter and Escape in a field of a native palette are the field's, with `RootEnv::escape_leaves_fields` off or on:
/// neither reaches the palette window's leftover keys. A press on the field once it has the focus, which egui gives the
/// field's text widget, is no press on the group's empty body, which would hand the focus to the panel's tab. Once
/// Escape has taken the focus from the field, Enter is the window's.
#[trial]
fn enter_and_escape_in_a_native_palettes_field_stay_the_fields() {
    for leaves in [false, true] {
        let windows = Windows::new();
        windows.root_info(info(true, ROOT_AT));
        windows.child_info(info(true, PALETTE_AT));
        let mut root = RootEnv::new();
        root.escape_leaves_fields = leaves;
        windows.root_env(root);
        let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![float()])));
        let mut palette = FieldPalette { opacity: Some(50.0), ..FieldPalette::default() };
        let mut frame = |palette: &mut FieldPalette, child: Vec<Event>| {
            windows.frame(vec![], child, |ui| {
                let _ =
                    DockArea::new("Native dock", Bind::bound(&mut workspace), &Specs::with(Permissions::ALL), &mut *palette, DockConfig::default())
                        .show(ui);
            })
        };
        let mut out = frame(&mut palette, vec![]);
        for _ in 0..3 {
            out = frame(&mut palette, vec![]);
        }
        let child = out.trees.iter().find(|(viewport, _)| *viewport != ViewportId::ROOT).expect("the palette has an OS window");
        let tree = &child.1;
        let field = tree
            .keys()
            .into_iter()
            .find(|key| tree.name_of(*key) == Some("Opacity") && tree.role_of(*key) != Some(Role::LABEL))
            .and_then(|key| tree.bounds_of(key))
            .expect("the palette shows the field")
            .center();
        for key in [Key::Enter, Key::Escape] {
            for events in [vec![moved(field), press(field)], vec![release(field)], vec![], vec![]] {
                frame(&mut palette, events);
            }
            for events in [vec![key_press(key, Modifiers::NONE)], vec![key_release(key, Modifiers::NONE)], vec![], vec![]] {
                frame(&mut palette, events);
            }
            assert_eq!((palette.submits, palette.cancels), (0, 0), "leaves {leaves}: {key:?} in the field is the field's");
        }
        if leaves {
            for events in [vec![key_press(Key::Enter, Modifiers::NONE)], vec![key_release(Key::Enter, Modifiers::NONE)], vec![]] {
                frame(&mut palette, events);
            }
            assert_eq!(palette.submits, 1, "an Enter after Escape left the field is the window's");
        }
    }
}

/// What panel 1's body holds that takes an Escape pressed in its window.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Taker {
    Nothing,
    /// A field, which a click focuses.
    Field,
    /// A button whose context menu a secondary click opens.
    Menu,
    /// App code that reads the window's leftover Escape.
    App,
}

/// Panels of which panel 1 holds `taker`.
struct EscapePalette {
    taker: Taker,
    opacity: Option<f64>,
}

impl DockPanels<u32> for EscapePalette {
    fn body(&mut self, panel: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        match self.taker {
            _ if *panel != 1 => {}
            Taker::Nothing => {}
            Taker::Field => {
                NumberField::new("Opacity", Bind::bound(&mut self.opacity)).show(ui);
            }
            Taker::Menu => {
                let target = Button::new("Target").show(ui);
                let _ = Menu::context("Target menu", &target).show(ui, |ui, _| {
                    MenuItem::new("Row").show(ui);
                });
            }
            Taker::App => {
                keys::with_window_intents(ui.ctx(), false, |window| window.take(Intent::Cancel));
            }
        }
    }
}

/// An Escape pressed in a native palette that holds `taker`, with the field focused or the menu open: how many of the
/// main window's Cancels two readers found that the app runs before its dock, one after the other.
fn main_window_cancels(taker: Taker) -> (usize, usize) {
    let mut dock = NativeDock::on(Os::Windows, Arc::new(FakeSystem::new()));
    let palette = dock.palette.expect("the palette's window opened");
    let mut panels = EscapePalette { taker, opacity: Some(50.0) };
    let mut read = (0, 0);
    let mut frame = |child: Vec<Event>| {
        dock.windows.frame(vec![], child, |ui| {
            let cancel = |ui: &Ui| usize::from(keys::with_window_intents(ui.ctx(), false, |window| window.take(Intent::Cancel)));
            read.0 += cancel(ui);
            read.1 += cancel(ui);
            let _ = DockArea::new("Native dock", Bind::bound(&mut dock.workspace), &dock.specs, &mut panels, DockConfig::default()).show(ui);
        })
    };
    frame(vec![]);
    let out = frame(vec![]);
    let at = |name: &str| {
        let tree = tree_of_viewport(&out, palette);
        let part = tree.keys().into_iter().find(|key| tree.name_of(*key) == Some(name) && tree.role_of(*key) != Some(Role::LABEL));
        part.and_then(|key| tree.bounds_of(key)).expect("the palette shows it").center()
    };
    let opened = match taker {
        Taker::Field => {
            let field = at("Opacity");
            vec![vec![moved(field), press(field)], vec![release(field)]]
        }
        Taker::Menu => {
            let target = at("Target");
            vec![vec![moved(target)], vec![secondary(target, true)], vec![secondary(target, false)]]
        }
        Taker::Nothing | Taker::App => vec![],
    };
    let escape = [vec![key_press(Key::Escape, Modifiers::NONE)], vec![key_release(Key::Escape, Modifiers::NONE)], vec![], vec![]];
    for events in opened.into_iter().chain([vec![], vec![]]).chain(escape) {
        frame(events);
    }
    read
}

/// An Escape that nothing in a native palette takes is the main window's Cancel, once: the app reads it where it reads
/// the main window's own, and the read takes it.
#[trial]
fn an_escape_nothing_in_a_native_palette_takes_is_the_main_windows_cancel_once() {
    assert_eq!(main_window_cancels(Taker::Nothing), (1, 0), "the first read finds the Escape, the second nothing");
}

/// An Escape in a native palette that a focused field, an open menu or the app's own code there takes or keeps never
/// reaches the main window.
#[trial]
fn an_escape_taken_in_a_native_palette_never_reaches_the_main_window() {
    for taker in [Taker::Field, Taker::Menu, Taker::App] {
        assert_eq!(main_window_cancels(taker), (0, 0), "{taker:?}: the Escape stays in the palette's window");
    }
}

/// A palette float made in the app window.
fn float() -> Floating<u32> {
    Floating::single(104, vec![1], 0, Rect::from_min_size(Pos2::new(40.0, 40.0), Vec2::new(300.0, 200.0)))
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

/// How far down panel 1's body the context menu's target sits, and the menu's rows: in the native palette at
/// `FLOAT_AT`, the pointer on the target has about 300 points above it and below it in the windows' span, and the
/// menu is taller than either and shorter than the span.
const CONTEXT_DROP: f32 = 220.0;
const CONTEXT_ROWS: usize = 18;

/// A palette whose panel 1 holds a button with a context menu of `CONTEXT_ROWS` rows.
struct ContextTarget;

impl DockPanels<u32> for ContextTarget {
    fn body(&mut self, panel: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        if *panel != 1 {
            return;
        }
        ui.add_space(CONTEXT_DROP);
        let target = Button::new("Target").show(ui);
        let _ = Menu::context("Target menu", &target).show(ui, |ui, _| {
            for at in 0..CONTEXT_ROWS {
                MenuItem::new(format!("Row {at}")).show(ui);
            }
        });
    }
}

/// A context menu opened in a native palette shows in a popup OS window; too tall for the room on either side of the
/// pointer but not for the room the popup has, it covers the pointer and shows every row, as it does in the main window.
#[trial]
fn a_context_menu_from_a_native_palette_too_tall_for_either_side_covers_the_pointer_in_its_popup_window() {
    let mut dock = NativeDock::on(Os::Windows, Arc::new(FakeSystem::new()));
    dock.windows.root_info(scaled(true, MAIN_AT, COLLAPSED_SCALE));
    let mut at = FLOAT_AT;
    dock.windows.child_info(scaled(true, at, COLLAPSED_SCALE));
    let mut panels = ContextTarget;
    let out = collapsed_frame(&mut dock, &mut panels, &mut at, vec![], vec![]);
    let palette = dock.palette.expect("the palette's window opened");
    let target = node_rect(tree_of_viewport(&out, palette), "Target", Role::BUTTON).expect("the palette shows the target").center();
    for events in [vec![moved(target)], vec![secondary(target, true)], vec![secondary(target, false)]] {
        collapsed_frame(&mut dock, &mut panels, &mut at, vec![], events);
    }
    let mut out = collapsed_frame(&mut dock, &mut panels, &mut at, vec![], vec![]);
    for _ in 0..4 {
        out = collapsed_frame(&mut dock, &mut panels, &mut at, vec![], vec![]);
    }
    let menu = popups(&dock, &out)
        .into_iter()
        .find(|viewport| !tree_of_viewport(&out, *viewport).with_role(Role::MENU).is_empty())
        .expect("the menu shows in a popup window");
    let rect = window_rect(&dock, menu);
    let pointer = at.min + target.to_vec2();
    let room = MAIN_AT.union(at);
    assert!(
        rect.height() > pointer.y - room.top() && rect.height() > room.bottom() - pointer.y,
        "the menu {rect:?} fits on neither side of {pointer:?}"
    );
    assert!(room.contains_rect(rect.shrink(0.5)), "the menu {rect:?} stays inside the windows' span {room:?}");
    assert!(rect.contains(pointer), "the menu {rect:?} covers the pointer at {pointer:?}");
    let tree = tree_of_viewport(&out, menu);
    let window = Rect::from_min_size(Pos2::ZERO, rect.size()).expand(FIT_ROUNDING);
    for row in [0, CONTEXT_ROWS - 1] {
        let shown = node_rect(tree, &format!("Row {row}"), Role::MENU_ITEM).unwrap_or_else(|| panic!("row {row} shows"));
        assert!(window.contains_rect(shown), "row {row} {shown:?} lies whole in the menu's window {window:?}");
    }
}

/// Opens the menu a secondary click at `at`, a point of the palette's window, asks for; returns its popup window.
fn open_menu(dock: &mut NativeDock, rect: &mut Rect, at: Pos2) -> ViewportId {
    for events in [vec![moved(at)], vec![secondary(at, true)], vec![secondary(at, false)], vec![], vec![]] {
        collapsed_frame(dock, &mut Palettes, rect, vec![], events);
    }
    let out = collapsed_frame(dock, &mut Palettes, rect, vec![], vec![]);
    let opened = popups(dock, &out).into_iter().find(|viewport| !tree_of_viewport(&out, *viewport).with_role(Role::MENU).is_empty());
    opened.expect("the menu shows in a popup window")
}

/// The arrow keys move the focus in a native palette's tab menu, and Enter runs the row they reach: Dock Panel, which
/// is not the first row, docks the panel. Opened again after Escape, the menu starts as it opened first.
#[trial]
fn the_arrow_keys_reach_a_row_of_a_native_palettes_tab_menu() {
    let mut dock = NativeDock::on(Os::Windows, Arc::new(FakeSystem::new()));
    let mut at = FLOAT_AT;
    dock.windows.root_info(scaled(true, MAIN_AT, COLLAPSED_SCALE));
    dock.windows.child_info(scaled(true, at, COLLAPSED_SCALE));
    let out = collapsed_frame(&mut dock, &mut Palettes, &mut at, vec![], vec![]);
    let palette = dock.palette.expect("the palette's window opened");
    let tab = node_rect(tree_of_viewport(&out, palette), PALETTE_TITLE, Role::TAB).expect("the palette shows the panel's tab").center();
    // A key in the menu's window, and the row focused in the frame after it.
    let key = |dock: &mut NativeDock, at: &mut Rect, menu: ViewportId, key: Key| -> Option<String> {
        dock.windows.send(menu, vec![key_press(key, Modifiers::NONE)]);
        collapsed_frame(dock, &mut Palettes, at, vec![], vec![]);
        dock.windows.send(menu, vec![key_release(key, Modifiers::NONE)]);
        collapsed_frame(dock, &mut Palettes, at, vec![], vec![]);
        let out = collapsed_frame(dock, &mut Palettes, at, vec![], vec![]);
        let tree = &out.trees.iter().find(|(shown, _)| *shown == menu)?.1;
        tree.name_of(focused_key(tree)).map(str::to_owned)
    };
    let menu = open_menu(&mut dock, &mut at, tab);
    let first = key(&mut dock, &mut at, menu, Key::ArrowDown);
    let second = key(&mut dock, &mut at, menu, Key::ArrowDown);
    assert_ne!(first, second, "the second arrow moves the focus on");
    key(&mut dock, &mut at, menu, Key::Escape);
    for _ in 0..30 {
        collapsed_frame(&mut dock, &mut Palettes, &mut at, vec![], vec![]);
    }
    let out = collapsed_frame(&mut dock, &mut Palettes, &mut at, vec![], vec![]);
    assert!(!popups(&dock, &out).contains(&menu), "Escape closed the menu");
    let menu = open_menu(&mut dock, &mut at, tab);
    let mut row = key(&mut dock, &mut at, menu, Key::ArrowDown);
    assert_eq!(row, first, "the menu starts at the same row again");
    for _ in 0..4 {
        if row.as_deref() == Some("Dock Panel") {
            break;
        }
        row = key(&mut dock, &mut at, menu, Key::ArrowDown);
    }
    key(&mut dock, &mut at, menu, Key::Enter);
    let layout = dock.workspace.layout();
    assert!(layout.floating.is_empty(), "Enter ran {row:?}, which docks the panel: {layout:?}");
}

/// The menu a secondary click on a native palette's title row hangs at the pointer stays there while the pointer
/// leaves the palette for the menu's window and comes back, and hangs at the new place of a second secondary click.
#[trial]
fn a_native_palettes_title_row_menu_stays_where_it_opened() {
    let mut dock = NativeDock::on(Os::Windows, Arc::new(FakeSystem::new()));
    let mut at = FLOAT_AT;
    dock.windows.root_info(scaled(true, MAIN_AT, COLLAPSED_SCALE));
    dock.windows.child_info(scaled(true, at, COLLAPSED_SCALE));
    let menu = open_menu(&mut dock, &mut at, ROW_AT);
    let opened = window_rect(&dock, menu);
    for events in [vec![gone()], vec![], vec![moved(ROW_AT + Vec2::new(-60.0, 80.0))], vec![]] {
        collapsed_frame(&mut dock, &mut Palettes, &mut at, vec![], events);
        assert_eq!(window_rect(&dock, menu), opened, "the menu stays where it opened");
    }
    let menu = open_menu(&mut dock, &mut at, ROW_AT - Vec2::new(40.0, 0.0));
    let moved = window_rect(&dock, menu);
    assert!(moved.min.x < opened.min.x - 1.0, "it hangs at the second click: {moved:?}, first {opened:?}");
}
