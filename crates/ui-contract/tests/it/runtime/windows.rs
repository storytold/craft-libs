//! `runtime::windows`: the input hook per viewport, window info per viewport, the press release after an OS
//! resize (H4), the raise of a modal's window, and `act`.

use craft_behavior::{Cursor, CursorIcon};
use craft_host::target::{Caps, Os, ShellInfo, WindowChrome, WindowInfo, WindowKind};
use craft_ui::contract::{Common, Component, Container, ContainerProps, Cx, Mount, Overlay, Props as _, Role, ShowContainer as _};
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::windows::WindowSpec;
use craft_ui::style::StyleCx;
use craft_ui::testing::pointer::{moved, press};
use craft_ui::testing::trial;
use craft_ui::testing::windows::NoWindows;
use craft_ui_test_kit::contract::PlainStyle;
use craft_ui_test_kit::windows::Windows;
use egui::{Event, Pos2, Rect, Response, Vec2, ViewportBuilder, ViewportCommand, ViewportEvent, ViewportId, ViewportInfo};
use std::cell::Cell;
use std::sync::Arc;

fn child() -> ViewportId {
    ViewportId::from_hash_of("child")
}

/// A modal layer with a fixed body.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Modal {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Component for Modal {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ModalStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(120.0, 60.0))
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Modal {}

/// H4: where the OS resizes undecorated windows, such a window never gets the button release, so the input hook
/// releases the press when the window's size changes under it. A decorated window keeps its press through the same
/// size change, and so does an undecorated window on a platform that delivers the release itself.
#[trial]
fn a_press_is_released_after_an_os_resize_of_an_undecorated_window() {
    let cases =
        [(Os::Windows, WindowChrome::Integrated, true), (Os::Windows, WindowChrome::Native, false), (Os::Linux, WindowChrome::Integrated, false)];
    for (os, chrome, released) in cases {
        let shell = ShellInfo::default().os(os).caps(Caps::native(os)).window(WindowInfo::new(WindowKind::Main).chrome(chrome));
        let windows = Windows::with(InstallSpec::new(shell, Arc::new(NoWindows)));
        let down = Cell::new(false);
        let sized =
            |width: f32| ViewportInfo { inner_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 600.0))), ..ViewportInfo::default() };
        let frame = |events: Vec<Event>| {
            windows.frame(events, vec![], |ui| down.set(ui.ctx().input(|input| input.pointer.any_down())));
        };
        windows.root_info(sized(800.0));
        frame(vec![]);
        frame(vec![moved(Pos2::new(20.0, 20.0))]);
        frame(vec![press(Pos2::new(20.0, 20.0))]);
        assert!(down.get(), "the press is down");
        windows.root_info(sized(812.0));
        frame(vec![]);
        frame(vec![]);
        assert_eq!(!down.get(), released, "{os:?} {chrome:?}: the press after the OS resize");
    }
}

/// A modal in one window blocks the others: the pointer over them shows the blocked cursor, and a close request on
/// them is refused and raises the modal's window.
#[trial]
fn an_app_modal_in_one_window_blocks_the_cursor_and_the_close_of_the_others() {
    let windows = Windows::new();
    let frame = || {
        windows.frame(vec![], vec![], |ui| {
            ui.ctx().show_viewport_immediate(child(), ViewportBuilder::default(), |cui, _| {
                let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
                Modal { common: Common::new(), props }.id_salt("modal").show(cui, |_, _| {});
            });
        })
    };
    for _ in 0..4 {
        frame();
    }
    assert_eq!(craft_ui::runtime::cursor::current(&windows.ctx), Some(Cursor::System(CursorIcon::NotAllowed)), "the root window is blocked");

    windows.root_info(ViewportInfo { events: vec![ViewportEvent::Close], ..ViewportInfo::default() });
    let out = frame();
    craft_ui::runtime::windows::logic(&windows.ctx);
    let out_after = frame();
    let sent: Vec<_> = out.commands.iter().chain(&out_after.commands).collect();
    assert!(sent.contains(&&(ViewportId::ROOT, ViewportCommand::CancelClose)), "the close is refused: {sent:?}");
    assert!(sent.contains(&&(child(), ViewportCommand::Focus)), "the modal's window is raised: {sent:?}");
}

/// A container mounted in an OS viewport that opens as `spec`, with a body the trial gives it.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Palette {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Palette {
    fn new(title: &'static str, spec: Option<WindowSpec>) -> Self {
        let overlay = Overlay::floating();
        let mount = match spec {
            Some(spec) => Mount::viewport_with(overlay, spec),
            None => Mount::viewport(overlay),
        };
        Palette { common: Common::new(), props: ContainerProps::new().mount(mount) }.label(title).id_salt(title)
    }
}

impl Component for Palette {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PaletteStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(120.0, 60.0))
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Palette {}

fn builder_titled<'a>(out: &'a craft_ui_test_kit::windows::Out, title: &str) -> (ViewportId, &'a ViewportBuilder) {
    let (id, builder) = out.builders.iter().find(|(_, builder)| builder.title.as_deref() == Some(title)).expect("the window asked to open");
    (*id, builder)
}

/// A palette stays above the app's windows while one of them has focus. Each frame's activity is decided once
/// every window has reported its focus in its own pass, and the palette's window is built with it in the next frame.
/// Focus moving from the main window to the palette's never drops the palette, though the main window's input still
/// lists the palette's window as unfocused in that frame; focus leaving every window drops it in the next frame, and
/// focus coming back raises it in the next frame again.
#[trial]
fn a_palette_drops_its_level_while_the_app_is_inactive() {
    let windows = Windows::new();
    let focus = |on| ViewportInfo { focused: Some(on), ..ViewportInfo::default() };
    // One frame in which the main window reports `main` and the palette's window, in its own pass, `palette`. Like an
    // integration, the kit lists the palette's window in the main window's input as it reported in its last pass.
    let frame = |main: bool, palette: bool| {
        windows.root_info(focus(main));
        let out = windows.frame(vec![], vec![], |ui| {
            windows.child_info(focus(palette));
            Palette::new("tool", Some(WindowSpec::tool())).show(ui, |_, _| {});
        });
        builder_titled(&out, "tool").1.window_level
    };
    let on_top = Some(egui::WindowLevel::AlwaysOnTop);
    assert_eq!(frame(true, false), on_top, "the main window has focus");
    for _ in 0..2 {
        assert_eq!(frame(false, true), on_top, "focus moved to the palette's window: the app stays active in every frame");
    }
    // Focus leaves the palette's window, which the main window's input still lists as focused.
    frame(false, false);
    assert_eq!(frame(false, false), Some(egui::WindowLevel::Normal), "the frame after focus left every window");
    frame(true, false);
    assert_eq!(frame(true, false), on_top, "the frame after the main window took focus again");
}
