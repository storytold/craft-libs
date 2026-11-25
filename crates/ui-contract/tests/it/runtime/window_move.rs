//! The body step moves and resizes an OS window (16.3, 21.2): a move band acts only where the app draws the frame and
//! the window is not full screen (a maximized window's move is the platform's, which restores it), and grips only where
//! it is neither maximized nor full screen; a `Window` mount puts its grips on the window's edges; a double click on the
//! move band is reported once.

use craft_behavior::{Cursor, CursorIcon};
use craft_host::target::{Backend, Os, ShellInfo, WindowChrome, WindowInfo, WindowKind};
use craft_ui::contract::overlay::Resize;
use craft_ui::contract::{
    BodyCx, Common, Component, Container, ContainerProps, Cx, DragBand, HeaderRegion, Mount, Overlay, Props as _, Role, Show as _,
    ShowContainer as _, Sizing,
};
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::focus::FocusPolicy;
use craft_ui::runtime::windows::WindowSpec;
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::allocate;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::trial;
use craft_ui::testing::windows::FakeSystem;
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use craft_ui_test_kit::windows::Windows;
use craft_window::policy::{Desktop, DesktopKind, Runtime, TitleBarPref};
use egui::viewport::ResizeDirection;
use egui::{Event, Pos2, Rect, Response, Ui, Vec2, ViewportCommand, ViewportInfo, pos2, vec2};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

const HEADER: Vec2 = Vec2::new(100.0, 24.0);
/// The size of the root window in the windows kit.
const ROOT: Vec2 = Vec2::new(800.0, 600.0);

/// A container whose header is a block of a fixed size; it counts the passes in which its move band reported a
/// double click.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Bar<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    header: &'a Cell<Rect>,
    doubles: &'a Cell<u32>,
}

impl Component for Bar<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: BarStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Bar<'_> {
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        self.header.set(allocate(ui, HEADER));
        if cx.band_double_pressed() {
            self.doubles.set(self.doubles.get() + 1);
        }
    }
}

/// A frame source for one container: it keeps the header rect and the double click count.
struct Rig {
    header: Cell<Rect>,
    doubles: Cell<u32>,
    seen: RefCell<Seen>,
    props: ContainerProps,
}

impl Rig {
    fn new(mount: Mount) -> Self {
        let props = ContainerProps::new().mount(mount).sizing(Sizing::Content).header(HeaderRegion::new().band(DragBand::Move));
        Rig { header: Cell::new(Rect::NOTHING), doubles: Cell::new(0), seen: RefCell::new(Seen::default()), props }
    }

    fn show(&self, ui: &mut Ui) {
        Bar { common: Common::new(), props: self.props.clone(), header: &self.header, doubles: &self.doubles }.id_salt("win").show(ui, |ui, _| {
            Probe::new(&self.seen).size(vec2(40.0, 20.0)).show(ui);
        });
    }

    fn frame(&self, windows: &Windows, root: Vec<Event>, child: Vec<Event>) -> Vec<ViewportCommand> {
        windows.frame(root, child, |ui| self.show(ui)).commands.into_iter().map(|(_, command)| command).collect()
    }
}

/// Presses at `at`, drags by `by` in four steps and releases. `events` wraps each step for the viewport under test.
fn drag(at: Pos2, by: Vec2, mut frame: impl FnMut(Vec<Event>) -> Vec<ViewportCommand>) -> Vec<ViewportCommand> {
    let mut sent = frame(vec![moved(at)]);
    sent.extend(frame(vec![press(at)]));
    for step in 1..=4 {
        sent.extend(frame(vec![moved(at + by * (step as f32 / 4.0))]));
    }
    sent.extend(frame(vec![release(at + by)]));
    sent.extend(frame(vec![]));
    sent
}

fn resizes(sent: &[ViewportCommand]) -> Vec<ResizeDirection> {
    sent.iter()
        .filter_map(|command| match command {
            ViewportCommand::BeginResize(direction) => Some(*direction),
            _ => None,
        })
        .collect()
}

/// The root window with the frame drawn by the app, or by the OS.
fn root(chrome: WindowChrome) -> Windows {
    let shell = ShellInfo::default().window(WindowInfo::new(WindowKind::Main).chrome(chrome));
    Windows::with(InstallSpec::new(shell, Arc::new(FakeSystem::new())))
}

fn root_drag(chrome: WindowChrome, at: Pos2, by: Vec2) -> (Vec<ViewportCommand>, Rig) {
    let windows = root(chrome);
    let rig = Rig::new(Mount::window(Resize::GRIPS));
    for _ in 0..4 {
        rig.frame(&windows, vec![], vec![]);
    }
    (drag(at, by, |events| rig.frame(&windows, events, vec![])), rig)
}

/// A grip pulled out of the window resizes it: winit reports the pointer leaving the window (`PointerGone`) while the
/// button is held, before the press has passed slop, and the moves from outside still start the drag. The resize used
/// to start only when the pointer passed slop inside the window, so a quick outward pull on an edge did nothing.
#[trial]
fn a_grip_pulled_out_of_the_window_still_resizes_it() {
    let windows = root(WindowChrome::Integrated);
    let rig = Rig::new(Mount::window(Resize::GRIPS));
    for _ in 0..4 {
        rig.frame(&windows, vec![], vec![]);
    }
    let edge = pos2(2.0, ROOT.y / 2.0);
    let mut sent = rig.frame(&windows, vec![moved(edge)], vec![]);
    sent.extend(rig.frame(&windows, vec![press(edge)], vec![]));
    // The first move lands outside the window, within slop of the press; winit reports the leave with it.
    sent.extend(rig.frame(&windows, vec![moved(edge - vec2(3.0, 0.0)), Event::PointerGone], vec![]));
    for step in 2..=4 {
        sent.extend(rig.frame(&windows, vec![moved(edge - vec2(5.0 * step as f32, 0.0))], vec![]));
    }
    sent.extend(rig.frame(&windows, vec![release(edge - vec2(20.0, 0.0))], vec![]));
    sent.extend(rig.frame(&windows, vec![], vec![]));
    assert_eq!(resizes(&sent), [ResizeDirection::West], "a pull out of the left edge resizes from the west: {sent:?}");
}

#[trial]
fn a_window_mounts_edges_resize_the_window_and_the_childs_press_keeps_its_click() {
    let (sent, _) = root_drag(WindowChrome::Integrated, pos2(1.0, ROOT.y / 2.0), vec2(20.0, 0.0));
    assert_eq!(resizes(&sent), [ResizeDirection::West], "a drag on the left edge resizes from the west: {sent:?}");
    let (sent, _) = root_drag(WindowChrome::Native, pos2(1.0, ROOT.y / 2.0), vec2(20.0, 0.0));
    assert!(resizes(&sent).is_empty(), "the OS resizes a window it draws the frame of: {sent:?}");

    // The probe is the first thing in the body, under the left edge: a click on it is the probe's, not a resize.
    let windows = root(WindowChrome::Integrated);
    let rig = Rig::new(Mount::window(Resize::GRIPS));
    for _ in 0..4 {
        rig.frame(&windows, vec![], vec![]);
    }
    let on_probe = pos2(1.0, rig.header.get().bottom() + 5.0);
    let mut sent = rig.frame(&windows, vec![moved(on_probe)], vec![]);
    sent.extend(rig.frame(&windows, vec![press(on_probe)], vec![]));
    sent.extend(rig.frame(&windows, vec![release(on_probe)], vec![]));
    sent.extend(rig.frame(&windows, vec![], vec![]));
    assert_eq!(rig.seen.borrow().activations, 1, "the child under the edge keeps its click");
    assert!(resizes(&sent).is_empty(), "and the click starts no resize: {sent:?}");
}

/// Where the OS draws a resizable window's frame (macOS `Inset`), AppKit resizes the window from its edges: the app
/// registers no grips there, so a drag on an edge starts no resize of the app's, and over AppKit's resize areas the
/// cursor is the default, as the OS shows its own there.
#[trial]
fn where_appkit_resizes_a_window_the_app_registers_no_grips_and_leaves_the_cursor_to_it() {
    let edge = pos2(1.0, ROOT.y / 2.0);
    let (sent, _) = root_drag(WindowChrome::Inset, edge, vec2(20.0, 0.0));
    assert!(resizes(&sent).is_empty(), "AppKit resizes the window, the app's grips stand down: {sent:?}");
    let windows = root(WindowChrome::Inset);
    let rig = Rig::new(Mount::window(Resize::GRIPS));
    for _ in 0..4 {
        rig.frame(&windows, vec![], vec![]);
    }
    rig.frame(&windows, vec![moved(edge)], vec![]);
    let cursor = craft_ui::runtime::cursor::current(&windows.ctx);
    assert!(matches!(cursor, None | Some(Cursor::System(CursorIcon::Default))), "over AppKit's edge the cursor is left to the OS: {cursor:?}");
}

/// photocraft#16905: a full-window container (RAW Develop, Vanishing Point) in an OS window of its own, in a window the
/// app frames, moves by its title row and resizes by its edges. Its fallback overlay covers the main window and neither
/// moves nor resizes, and the window took that rule: no move band and no grips, so the window could not be moved.
#[trial]
fn a_full_window_container_in_its_own_window_moves_and_resizes_it() {
    const WINDOW: Vec2 = Vec2::new(900.0, 640.0);
    let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
    let integrated =
        craft_window::policy::decide(Os::Linux, Runtime::Native, Backend::X11, &desktop, WindowKind::Main, TitleBarPref::Integrated, None);
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(FakeSystem::new())).frame(integrated));
    let rig = Rig::new(Mount::viewport_with(Overlay::full_window(FocusPolicy::DIALOG), WindowSpec::dialog()));
    let rect = Rect::from_min_size(Pos2::new(100.0, 100.0), WINDOW);
    windows.child_info(ViewportInfo {
        inner_rect: Some(rect),
        outer_rect: Some(rect),
        native_pixels_per_point: Some(1.0),
        ..ViewportInfo::default()
    });
    for _ in 0..4 {
        rig.frame(&windows, vec![], vec![]);
    }
    let title = rig.header.get();
    assert!(title.is_positive(), "the title row showed");
    let sent = drag(pos2(title.center().x + 30.0, title.center().y), vec2(40.0, 20.0), |events| rig.frame(&windows, vec![], events));
    assert!(sent.contains(&ViewportCommand::StartDrag), "a drag of the title row moves the window: {sent:?}");
    let sent = drag(pos2(WINDOW.x - 1.0, WINDOW.y / 2.0), vec2(-20.0, 0.0), |events| rig.frame(&windows, vec![], events));
    assert_eq!(resizes(&sent), [ResizeDirection::East], "a drag of the right edge resizes the window: {sent:?}");
}
