//! The body step moves and resizes an OS window (16.3, 21.2): a move band acts only where the app draws the frame and
//! the window is not full screen (a maximized window's move is the platform's, which restores it), and grips only where
//! it is neither maximized nor full screen; a `Window` mount puts its grips on the window's edges; a double click on the
//! move band is reported once.

use craft_host::target::{ShellInfo, WindowChrome, WindowInfo, WindowKind};
use craft_ui::contract::overlay::Resize;
use craft_ui::contract::{
    BodyCx, Common, Component, Container, ContainerProps, Cx, DragBand, HeaderRegion, Mount, Props as _, Role, Show as _, ShowContainer as _, Sizing,
};
use craft_ui::runtime::InstallSpec;
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::allocate;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::trial;
use craft_ui::testing::windows::FakeSystem;
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use craft_ui_test_kit::windows::Windows;
use egui::viewport::ResizeDirection;
use egui::{Event, Pos2, Rect, Response, Ui, Vec2, ViewportCommand, pos2, vec2};
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
