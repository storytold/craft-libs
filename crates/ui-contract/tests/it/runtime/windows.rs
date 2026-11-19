//! `runtime::windows`: the input hook per viewport, window info per viewport, the press release after an OS
//! resize (H4), the raise of a modal's window, the mount of a full-window container (in a window of its own, or in a
//! full-screen or Wayland-maximized main window), the size of a content-sized window, the app id of child windows,
//! injected keys across a focus loss, non-finite injected points, a repeated injected press, the window that has the
//! keys, a wheel with no known pointer, the placement of a minimized window, the root frame decided again, a held
//! close, and `act`.

use craft_behavior::{Cursor, CursorIcon, PointerButton};
use craft_host::target::{Backend, Caps, Os, ShellInfo, WindowChrome, WindowInfo, WindowKind};
use craft_keymap::{Key, Mods};
use craft_ui::contract::{Common, Component, Container, ContainerProps, Cx, Mount, Overlay, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::focus::FocusPolicy;
use craft_ui::runtime::windows::{WindowHost, WindowSpec};
use craft_ui::style::StyleCx;
use craft_ui::testing::pointer::{moved, press, wheel};
use craft_ui::testing::windows::{FakeSystem, NoWindows};
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use craft_ui_test_kit::windows::{Out, Windows};
use craft_window::policy::{Decision, Desktop, DesktopKind, Frame, MacInset, Runtime, TitleBarPref};
use craft_window::restore::Store;
use egui::{Event, Pos2, Rect, Response, UserAttentionType, Vec2, ViewportBuilder, ViewportCommand, ViewportEvent, ViewportId, ViewportInfo};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

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

/// #1518: after the user switches back to a window, the platform sends no pointer move until the mouse moves, so the
/// first wheel turn found no pointer and scrolled nothing. The pass that receives the wheel gets a move to the system
/// pointer, mapped into the window, before it; a rerun pass of that frame gets neither again, and the next wheel, with
/// the pointer known, gets no move.
#[trial]
fn a_wheel_after_the_window_regains_focus_lands_under_the_system_pointer() {
    let system = Arc::new(FakeSystem::new());
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), system.clone()));
    let window_at = Pos2::new(100.0, 50.0);
    windows.root_info(ViewportInfo { inner_rect: Some(Rect::from_min_size(window_at, Vec2::new(800.0, 600.0))), ..ViewportInfo::default() });
    // Each pass: the pointer moves its input carries, and whether it carries the wheel.
    let passes: RefCell<Vec<(Vec<Pos2>, bool)>> = RefCell::default();
    let frame = |events: Vec<Event>, rerun: bool| {
        passes.borrow_mut().clear();
        let first = Cell::new(rerun);
        windows.frame(events, vec![], |ui| {
            if first.take() {
                craft_ui::runtime::frame::discard_pass(ui.ctx(), "test");
            }
            let seen = ui.ctx().input(|input| {
                let moves: Vec<Pos2> = input.events.iter().filter_map(|event| if let Event::PointerMoved(at) = event { Some(*at) } else { None }).collect();
                (moves, input.events.iter().any(|event| matches!(event, Event::MouseWheel { .. })))
            });
            passes.borrow_mut().push(seen);
        });
    };
    for _ in 0..2 {
        frame(vec![], false);
    }
    let under = Pos2::new(200.0, 200.0);
    let desktop = window_at + under.to_vec2();
    system.set_pointer(Some([desktop.x, desktop.y]));

    frame(vec![wheel(-30.0)], true);
    {
        let passes = passes.borrow();
        assert_eq!(passes.len(), 2, "the wheel's frame reruns");
        assert_eq!(passes[0], (vec![under], true), "the wheel's pass moves the pointer to the system pointer first");
        assert_eq!(passes[1], (Vec::new(), false), "the rerun pass carries neither the move nor the wheel again");
    }
    frame(vec![wheel(-30.0)], false);
    assert_eq!(passes.borrow()[0], (Vec::new(), true), "with the pointer known, the next wheel gets no move");
}

/// Production saved the minimized window's size and later restored the window at egui's 64 pt floor: Windows reports
/// a minimized window as 0 × 0 far off screen. The placement store keeps where the window was last shown while it is
/// minimized, and records again once it is shown.
#[trial]
fn a_minimized_window_keeps_its_last_shown_placement() {
    let store = Arc::new(Mutex::new(Store::in_memory()));
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(FakeSystem::new())).placement(store.clone()));
    let shown = |min: Pos2, size: Vec2| ViewportInfo {
        inner_rect: Some(Rect::from_min_size(min, size)),
        outer_rect: Some(Rect::from_min_size(min, size)),
        native_pixels_per_point: Some(1.0),
        ..ViewportInfo::default()
    };
    let saved = || store.lock().unwrap().get(craft_window::restore::MAIN).map(|placement| (placement.normal.size, placement.normal.pos));
    let frames = |count: usize| {
        for _ in 0..count {
            windows.frame(vec![], vec![], |_| {});
        }
    };
    windows.root_info(shown(Pos2::new(120.0, 80.0), Vec2::new(900.0, 700.0)));
    frames(2);
    assert_eq!(saved(), Some(([900.0, 700.0], [120.0, 80.0])), "the shown window's placement is kept");

    windows.root_info(ViewportInfo { minimized: Some(true), ..shown(Pos2::new(-32000.0, -32000.0), Vec2::ZERO) });
    frames(2);
    assert_eq!(saved(), Some(([900.0, 700.0], [120.0, 80.0])), "the minimized window keeps its last shown placement");

    windows.root_info(shown(Pos2::new(200.0, 100.0), Vec2::new(1000.0, 720.0)));
    frames(2);
    assert_eq!(saved(), Some(([1000.0, 720.0], [200.0, 100.0])), "shown again, the window records its placement");
}

/// The user's "Use system title bar" setting changes the open main window at once: `set_root_frame` sends the window
/// the decorations command of the change once, `frame()` follows, an unchanged decision sends nothing, and a change to
/// the inset frame, which no command can apply to an open window, is refused and keeps the frame.
#[trial]
fn the_root_frame_changes_at_once_when_the_app_decides_again() {
    let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
    let decided = |pref| craft_window::policy::decide(Os::Linux, Runtime::Native, Backend::X11, &desktop, WindowKind::Main, pref, None);
    let native = decided(TitleBarPref::System);
    let integrated = decided(TitleBarPref::Integrated);
    assert!(native.frame == Frame::Native && matches!(integrated.frame, Frame::Integrated(_)), "the trial's two decisions");
    let inset = Decision { frame: Frame::Inset(MacInset::default()), ..integrated.clone() };

    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(FakeSystem::new())).frame(native.clone()));
    let host = WindowHost::of(&windows.ctx);
    // One frame in which the app decides `decision` once, as on a change of the setting: whether the host took it, and
    // the decorations commands the main window got.
    let decide = |decision: Decision| {
        let pending = RefCell::new(Some(decision));
        let taken = Cell::new(None);
        let out = windows.frame(vec![], vec![], |ui| {
            if let Some(decision) = pending.borrow_mut().take() {
                taken.set(Some(WindowHost::of(ui.ctx()).set_root_frame(decision)));
            }
        });
        let sent: Vec<ViewportCommand> = out
            .commands
            .into_iter()
            .filter(|(id, command)| *id == ViewportId::ROOT && matches!(command, ViewportCommand::Decorations(_)))
            .map(|(_, command)| command)
            .collect();
        (taken.get().expect("the frame ran"), sent)
    };
    for _ in 0..2 {
        windows.frame(vec![], vec![], |_| {});
    }
    assert_eq!(host.frame(), Frame::Native, "the installed decision");

    assert_eq!(decide(integrated.clone()), (true, vec![ViewportCommand::Decorations(false)]), "the app's bar turns the OS frame off");
    assert_eq!(host.frame(), integrated.frame, "the frame follows at once");
    assert_eq!(decide(integrated.clone()), (true, Vec::new()), "the decision in force sends nothing");
    assert_eq!(decide(inset), (false, Vec::new()), "the inset frame cannot reach an open window");
    assert_eq!(host.frame(), integrated.frame, "a refused decision keeps the frame");
    assert_eq!(decide(native), (true, vec![ViewportCommand::Decorations(true)]), "the system's bar turns the OS frame back on");
    assert_eq!(host.frame(), Frame::Native);
}

/// L-37: the app holds the main window's close to ask about unsaved changes, and the question must reach the user: the
/// held close keeps the window open and brings it forward from behind the user's other windows, restoring it first;
/// on Wayland, where no client may focus a window, it asks for attention. A frame with no close request sends nothing.
#[trial]
fn a_held_close_brings_the_root_window_forward() {
    let attention = ViewportCommand::RequestUserAttention(UserAttentionType::Informational);
    for (backend, raise) in [(Backend::X11, ViewportCommand::Focus), (Backend::Wayland, attention.clone())] {
        let windows = Windows::with(InstallSpec::new(ShellInfo::default().backend(backend), Arc::new(FakeSystem::new())));
        let behind = |events: Vec<ViewportEvent>| ViewportInfo { focused: Some(false), events, ..ViewportInfo::default() };
        let held = Cell::new(0);
        let frame = || {
            windows.frame(vec![], vec![], |ui| {
                if craft_ui::runtime::windows::root_close_requested(ui.ctx()) {
                    craft_ui::runtime::windows::hold_close(ui.ctx());
                    held.set(held.get() + 1);
                }
            })
        };
        windows.root_info(behind(Vec::new()));
        for _ in 0..2 {
            frame();
        }
        windows.root_info(behind(vec![ViewportEvent::Close]));
        let out = frame();
        let sent: Vec<&ViewportCommand> = out.commands.iter().filter(|(id, _)| *id == ViewportId::ROOT).map(|(_, command)| command).collect();
        assert_eq!(held.get(), 1, "{backend:?}: the app held the close once");
        assert!(sent.contains(&&ViewportCommand::CancelClose), "{backend:?}: the window stays open: {sent:?}");
        let at = |wanted: &ViewportCommand| sent.iter().position(|command| *command == wanted);
        let (restored, raised) = (at(&ViewportCommand::Minimized(false)), at(&raise));
        assert!(
            matches!((restored, raised), (Some(restored), Some(raised)) if restored < raised),
            "{backend:?}: the window is restored, then brought forward: {sent:?}"
        );
        let other = if backend == Backend::Wayland { ViewportCommand::Focus } else { attention.clone() };
        assert_eq!(at(&other), None, "{backend:?}: one way of bringing it forward: {sent:?}");

        windows.root_info(behind(Vec::new()));
        let out = frame();
        assert!(
            !out.commands.iter().any(|(id, command)| *id == ViewportId::ROOT && (*command == raise || *command == ViewportCommand::Minimized(false))),
            "{backend:?}: with no close request nothing is raised"
        );
    }
}

/// One frame of `windows` with the `Modal` open in the child viewport.
fn modal_frame(windows: &Windows) -> Out {
    windows.frame(vec![], vec![], |ui| {
        ui.ctx().show_viewport_immediate(child(), ViewportBuilder::default(), |cui, _| {
            let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
            Modal { common: Common::new(), props }.id_salt("modal").show(cui, |_, _| {});
        });
    })
}

/// Whether `out` raised the child's window: restored it from the Dock or taskbar, then focused it (or, on Wayland,
/// asked for attention).
fn raised_child(out: &Out) -> bool {
    out.commands.iter().any(|(id, command)| *id == child() && *command == ViewportCommand::Minimized(false))
}

/// A modal in one window blocks the others: the pointer over them shows the blocked cursor, and a close request on
/// them is refused and raises the modal's window. A minimized window ignores focus, so the raise restores it first;
/// a Wayland compositor lets no client focus a window, so there the raise asks for attention.
#[trial]
fn an_app_modal_in_one_window_blocks_the_cursor_and_the_close_of_the_others() {
    let attention = ViewportCommand::RequestUserAttention(UserAttentionType::Informational);
    for (backend, focus) in [(Backend::X11, ViewportCommand::Focus), (Backend::Wayland, attention.clone())] {
        let windows = Windows::with(InstallSpec::new(ShellInfo::default().backend(backend), Arc::new(FakeSystem::new())));
        for _ in 0..4 {
            modal_frame(&windows);
        }
        let blocked = Some(Cursor::System(CursorIcon::NotAllowed));
        assert_eq!(craft_ui::runtime::cursor::current(&windows.ctx), blocked, "{backend:?}: the root window is blocked");

        windows.root_info(ViewportInfo { events: vec![ViewportEvent::Close], ..ViewportInfo::default() });
        let out = modal_frame(&windows);
        craft_ui::runtime::windows::logic(&windows.ctx);
        let out_after = modal_frame(&windows);
        let sent: Vec<_> = out.commands.iter().chain(&out_after.commands).collect();
        assert!(sent.contains(&&(ViewportId::ROOT, ViewportCommand::CancelClose)), "{backend:?}: the close is refused: {sent:?}");
        let at = |wanted: &ViewportCommand| sent.iter().position(|(id, command)| *id == child() && command == wanted);
        let (restored, raised) = (at(&ViewportCommand::Minimized(false)), at(&focus));
        assert!(raised.is_some(), "{backend:?}: the modal's window is raised: {sent:?}");
        assert!(
            matches!((restored, raised), (Some(restored), Some(raised)) if restored < raised),
            "{backend:?}: the window is restored before it is raised: {sent:?}"
        );
        let other = if backend == Backend::Wayland { ViewportCommand::Focus } else { attention.clone() };
        assert_eq!(at(&other), None, "{backend:?}: one way of raising: {sent:?}");
    }
}

/// A window that a modal in another window blocks takes no wheel, as it takes no press or key: what is under the
/// pointer there does not scroll.
#[trial]
fn a_window_an_app_modal_in_another_window_blocks_takes_no_wheel() {
    use craft_behavior::WheelTaker;
    use craft_ui::testing::wheel::{register, take};
    for open in [false, true] {
        let windows = Windows::new();
        let view = egui::Id::new("view");
        let taken = Cell::new(None);
        let frame = |events: Vec<Event>| {
            windows.frame(events, vec![], |ui| {
                register(ui.ctx(), WheelTaker::new(view, Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0))).vertical(true).scrolls(true));
                if let Some(scroll) = take(ui.ctx(), view) {
                    taken.set(Some(scroll));
                }
                if open {
                    ui.ctx().show_viewport_immediate(child(), ViewportBuilder::default(), |cui, _| {
                        let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
                        Modal { common: Common::new(), props }.id_salt("modal").show(cui, |_, _| {});
                    });
                }
            })
        };
        for _ in 0..4 {
            frame(vec![moved(Pos2::new(200.0, 200.0))]);
        }
        frame(vec![wheel(-30.0)]);
        assert_eq!(taken.get().is_some(), !open, "modal open {open}: the view's share {:?}", taken.get());
    }
}

/// L-17: the user clicking the main window while a modal in another window blocks it brings the modal's window
/// forward with it, in the pass the main window gains focus; a pass without a focus change sends nothing.
#[trial]
fn a_blocking_modal_is_raised_when_the_main_window_gains_focus() {
    let windows = Windows::new();
    let focus = |on| ViewportInfo { focused: Some(on), ..ViewportInfo::default() };
    windows.root_info(focus(false));
    for _ in 0..4 {
        modal_frame(&windows);
    }
    assert!(!raised_child(&modal_frame(&windows)), "the main window stays unfocused: nothing is raised");
    windows.root_info(focus(true));
    assert!(raised_child(&modal_frame(&windows)), "the main window gained focus: the modal's window is raised");
    assert!(!raised_child(&modal_frame(&windows)), "the main window keeps focus: nothing more is raised");

    // A full-window dialog in a window of its own (the Camera Raw form) is raised the same way.
    let windows = Windows::new();
    let frame = || {
        windows.frame(vec![], vec![], |ui| {
            Palette::full_window("Workspace").show(ui, |_, _| {});
        })
    };
    windows.root_info(focus(false));
    let mut out = None;
    for _ in 0..4 {
        out = Some(frame());
    }
    let (dialog, _) = builder_titled(&out.expect("the frames ran"), "Workspace");
    let raised = |out: &Out| out.commands.iter().any(|(id, command)| *id == dialog && *command == ViewportCommand::Minimized(false));
    assert!(!raised(&frame()), "the main window stays unfocused: the dialog's window is not raised");
    windows.root_info(focus(true));
    assert!(raised(&frame()), "the main window gained focus: the dialog's window is raised");
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

    /// A full-window container in a dialog window of its own, as a workspace that opens its own window mounts.
    fn full_window(title: &'static str) -> Self {
        let mount = Mount::viewport_with(Overlay::full_window(FocusPolicy::DIALOG), WindowSpec::dialog());
        Palette { common: Common::new(), props: ContainerProps::new().mount(mount) }.label(title).id_salt(title)
    }

    /// A container that covers the main window, as a workspace without a window of its own mounts.
    fn covering(title: &'static str) -> Self {
        Palette { common: Common::new(), props: ContainerProps::new().mount(Mount::FullWindow) }.label(title).id_salt(title)
    }

    /// A floating panel in a layer of the main window.
    fn floating(title: &'static str) -> Self {
        Palette { common: Common::new(), props: ContainerProps::new().mount(Mount::Layer(Overlay::floating())) }.label(title).id_salt(title)
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

/// A full-window container mounted in its own window opens a resizable OS window titled as the container where
/// viewports are available, and covers the main window's content where they are not.
#[trial]
fn a_full_window_viewport_fills_the_main_window_without_viewports_and_opens_its_own_with_them() {
    let windows = Windows::new();
    let mut out = None;
    for _ in 0..4 {
        out = Some(windows.frame(vec![], vec![], |ui| {
            Palette::full_window("Workspace").show(ui, |_, _| {});
        }));
    }
    let out = out.expect("the frames ran");
    let (_, builder) = builder_titled(&out, "Workspace");
    assert_eq!(builder.resizable, Some(true), "the user sizes the workspace's window");

    let without = ShellInfo::default().caps(Caps::native(Os::Linux).difference(Caps::VIEWPORTS));
    let windows = Windows::with(InstallSpec::new(without, Arc::new(FakeSystem::new())));
    let content = Cell::new(Rect::NOTHING);
    let mut out = None;
    for _ in 0..4 {
        out = Some(windows.frame(vec![], vec![], |ui| {
            content.set(ui.ctx().content_rect());
            Palette::full_window("Workspace").show(ui, |_, _| {});
        }));
    }
    let out = out.expect("the frames ran");
    assert!(out.builders.iter().all(|(_, builder)| builder.title.as_deref() != Some("Workspace")), "no OS window opens");
    let (_, tree) = out.trees.iter().rfind(|(id, _)| *id == ViewportId::ROOT).expect("the root's tree");
    let shown = tree.keys().into_iter().find(|key| tree.role_of(*key) == Some(Role::GROUP) && tree.name_of(*key) == Some("Workspace"));
    assert_eq!(shown.and_then(|key| tree.bounds_of(key)), Some(content.get()), "the workspace covers the main window");
}

/// FW3: a viewport mount opened from a full-screen window, or from a maximized one on Wayland, shows in that window
/// instead of a window of its own: a window opened over it would cover it, the compositor would send it no frames, and
/// the dialog drawn inside them would freeze. A main window that goes full screen under an open dialog takes the dialog
/// in, and the dialog stays open.
#[trial]
fn a_viewport_mount_stays_in_a_full_screen_or_wayland_maximized_window() {
    let state =
        |fullscreen: bool, maximized: bool| ViewportInfo { fullscreen: Some(fullscreen), maximized: Some(maximized), ..ViewportInfo::default() };
    let frame = |windows: &Windows| {
        windows.frame(vec![], vec![], |ui| {
            Palette::full_window("Workspace").show(ui, |_, _| {});
        })
    };
    let own_window = |out: &Out| out.builders.iter().any(|(_, builder)| builder.title.as_deref() == Some("Workspace"));
    let in_root = |out: &Out| {
        out.trees.iter().rfind(|(id, _)| *id == ViewportId::ROOT).is_some_and(|(_, tree)| {
            tree.keys().into_iter().any(|key| tree.role_of(key) == Some(Role::GROUP) && tree.name_of(key) == Some("Workspace"))
        })
    };
    let cases = [
        (Backend::X11, false, false, true),
        (Backend::X11, true, false, false),
        (Backend::X11, false, true, true),
        (Backend::Wayland, false, false, true),
        (Backend::Wayland, true, false, false),
        (Backend::Wayland, false, true, false),
    ];
    for (backend, fullscreen, maximized, own) in cases {
        let windows = Windows::with(InstallSpec::new(ShellInfo::default().backend(backend), Arc::new(FakeSystem::new())));
        windows.root_info(state(fullscreen, maximized));
        let mut out = None;
        for _ in 0..4 {
            out = Some(frame(&windows));
        }
        let out = out.expect("the frames ran");
        let case = format!("{backend:?}, full screen {fullscreen}, maximized {maximized}");
        assert_eq!(own_window(&out), own, "{case}: a window of its own");
        assert_eq!(in_root(&out), !own, "{case}: shown in the main window");
    }

    let windows = Windows::with(InstallSpec::new(ShellInfo::default().backend(Backend::X11), Arc::new(FakeSystem::new())));
    for _ in 0..4 {
        frame(&windows);
    }
    assert!(own_window(&frame(&windows)), "a normal main window: the dialog opens its own window");
    windows.root_info(state(true, false));
    let mut out = None;
    for _ in 0..4 {
        out = Some(frame(&windows));
    }
    let out = out.expect("the frames ran");
    assert!(!own_window(&out) && in_root(&out), "the main window went full screen: the dialog shows in it and is still open");
}

/// A window sized by its content sends no size before it has reported its place, so it never opens at what its body
/// took in room the OS has not given; once it has reported, it takes its content's size.
#[trial]
fn a_content_sized_window_takes_its_content_size_once_it_reported_its_place() {
    const CONTENT: Vec2 = Vec2::new(200.0, 100.0);
    const GIVEN: Vec2 = Vec2::new(400.0, 300.0);
    let windows = Windows::new();
    let frame = || {
        windows.frame(vec![], vec![], |ui| {
            Palette::new("tool", Some(WindowSpec::tool())).show(ui, |ui, _| {
                ui.allocate_exact_size(CONTENT, egui::Sense::hover());
            });
        })
    };
    let sizes = |out: &Out| -> Vec<Vec2> {
        let sent = out.commands.iter().filter(|(id, _)| *id != ViewportId::ROOT);
        sent.filter_map(|(_, command)| match command {
            ViewportCommand::InnerSize(size) => Some(*size),
            _ => None,
        })
        .collect()
    };

    let first = frame();
    let opened = first.builders.iter().filter(|(_, builder)| builder.title.as_deref() == Some("tool"));
    assert!(opened.clone().count() > 0, "the window asked to open");
    assert!(opened.clone().all(|(_, builder)| builder.inner_size.is_none()), "the window opens with no size of its own");
    assert_eq!(sizes(&first), Vec::<Vec2>::new(), "no size goes out before the window reports its place");

    let given = Rect::from_min_size(Pos2::new(100.0, 100.0), GIVEN);
    let reported = ViewportInfo { inner_rect: Some(given), outer_rect: Some(given), native_pixels_per_point: Some(1.0), ..ViewportInfo::default() };
    windows.child_info(reported);
    let sent: Vec<Vec2> = (0..3).flat_map(|_| sizes(&frame())).collect();
    let taken = sent.last().copied().expect("the window takes a size once it reported its place");
    assert!(
        taken.x >= CONTENT.x && taken.y >= CONTENT.y && taken.x < GIVEN.x && taken.y < GIVEN.y,
        "the window takes its content's size, not the room the OS gave it: {taken:?}"
    );
}

/// The OS windows the runtime opens carry the app id the app installed, as its main window does: docks and Wayland
/// compositors find the app's icon and rules by it.
#[trial]
fn a_viewport_opened_through_a_mount_carries_the_installed_app_id() {
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(FakeSystem::new())).app_id("org.example.Craft"));
    let out = windows.frame(vec![], vec![], |ui| {
        Palette::new("tool", Some(WindowSpec::tool())).show(ui, |_, _| {});
    });
    assert_eq!(builder_titled(&out, "tool").1.app_id.as_deref(), Some("org.example.Craft"), "the child window carries the app id");

    let windows = Windows::new();
    let out = windows.frame(vec![], vec![], |ui| {
        Palette::new("tool", Some(WindowSpec::tool())).show(ui, |_, _| {});
    });
    assert_eq!(builder_titled(&out, "tool").1.app_id, None, "without an app id the window carries none");
}

/// fx.F20: a key and a modifier injected as held are let go when the window loses focus, as the platform lets its
/// own go. Otherwise the next injected input, written against the stale state, would not announce its own Shift.
#[trial]
fn a_window_focus_loss_releases_injected_held_keys() {
    let windows = Windows::new();
    let host = WindowHost::of(&windows.ctx);
    let held = Cell::new((false, false));
    let shift_click = Cell::new(false);
    let frame = |events: Vec<Event>| {
        windows.frame(events, vec![], |ui| {
            ui.ctx().input(|input| {
                held.set((input.key_down(egui::Key::A), input.modifiers.shift));
                if input.pointer.primary_pressed() && input.modifiers.shift {
                    shift_click.set(true);
                }
            });
        });
    };
    frame(vec![]);
    host.inject_key(ViewportId::ROOT, Key::A, Mods::SHIFT, true);
    for _ in 0..4 {
        frame(vec![]);
    }
    assert_eq!(held.get(), (true, true), "the injected key and Shift are held");

    frame(vec![Event::WindowFocused(false)]);
    frame(vec![Event::WindowFocused(true)]);
    host.inject_click(ViewportId::ROOT, Pos2::new(40.0, 40.0), PointerButton::Primary, 1, Mods::SHIFT, false);
    for _ in 0..8 {
        frame(vec![]);
    }
    assert!(shift_click.get(), "the click after the focus loss holds its own Shift");
    assert_eq!(held.get(), (false, false), "nothing stays held after the click");
}

/// A withdrawal after a step whose pass ran keeps the rest of that step's request (a key press's own release, when the
/// press opened a dialog that changed the document) and drops the requests queued behind it.
#[trial]
fn a_withdrawal_keeps_the_rest_of_the_request_whose_step_ran() {
    let windows = Windows::new();
    let host = WindowHost::of(&windows.ctx);
    let (a_down, b_pressed) = (Cell::new(false), Cell::new(false));
    let frame = || {
        windows.frame(vec![], vec![], |ui| {
            ui.ctx().input(|input| {
                a_down.set(input.key_down(egui::Key::A));
                b_pressed.set(b_pressed.get() || input.key_pressed(egui::Key::B));
            });
        });
    };
    frame();
    host.inject_key(ViewportId::ROOT, Key::A, Mods::NONE, false);
    host.inject_key(ViewportId::ROOT, Key::B, Mods::NONE, false);
    frame();
    assert!(a_down.get(), "the press of A ran");
    host.withdraw_injected(ViewportId::ROOT);
    for _ in 0..6 {
        frame();
    }
    assert!(!a_down.get(), "A's own release still came");
    assert!(!b_pressed.get(), "the request behind it was dropped");
    assert!(host.injected_idle());
}

/// Injected input with a NaN or infinite point queues nothing, as a window never reports one, and a release at such a
/// point still lets the held button go.
#[trial]
fn non_finite_injected_points_queue_nothing_and_a_release_there_lets_go() {
    use craft_ui::runtime::windows::InjectedPointer;

    let windows = Windows::new();
    let host = WindowHost::of(&windows.ctx);
    let presses = Cell::new(0);
    let down = Cell::new(false);
    let frames = |count| {
        for _ in 0..count {
            windows.frame(vec![], vec![], |ui| {
                ui.ctx().input(|input| {
                    presses.set(presses.get() + u32::from(input.pointer.primary_pressed()));
                    down.set(input.pointer.primary_down());
                });
            });
        }
    };
    frames(1);
    let nan = Pos2::new(f32::NAN, 10.0);
    host.inject_click(ViewportId::ROOT, nan, PointerButton::Primary, 1, Mods::NONE, false);
    host.inject_move(ViewportId::ROOT, Pos2::new(f32::INFINITY, 0.0), Mods::NONE, 0.0);
    host.inject_wheel(ViewportId::ROOT, Pos2::new(20.0, 20.0), Vec2::new(0.0, f32::NAN), Mods::NONE);
    host.inject_pointer(ViewportId::ROOT, InjectedPointer::down(PointerButton::Primary, nan, Mods::NONE, None));
    frames(6);
    assert_eq!(presses.get(), 0, "no non-finite press reaches egui");
    assert!(!down.get(), "nothing is held");

    host.inject_click(ViewportId::ROOT, Pos2::new(40.0, 40.0), PointerButton::Primary, 1, Mods::NONE, true);
    frames(4);
    assert!(down.get(), "the finite press is held");
    host.inject_pointer(ViewportId::ROOT, InjectedPointer::up(nan, Mods::NONE));
    frames(4);
    assert!(!down.get(), "a release at a non-finite point lets the button go");
    assert_eq!(presses.get(), 1);
}

/// An injected press of a button injected input holds already queues nothing, as a window never sends one: the
/// pointer stays where the first press went down, and one release lets the button go. A modal that opens during a
/// drag on a canvas sees no second press, and the drag keeps its moves and its release.
#[trial]
fn a_press_of_a_held_injected_button_queues_nothing() {
    use craft_ui::runtime::windows::InjectedPointer;

    let windows = Windows::new();
    let host = WindowHost::of(&windows.ctx);
    let presses = Cell::new(0);
    let down = Cell::new(false);
    let latest = Cell::new(None);
    let frames = |count| {
        for _ in 0..count {
            windows.frame(vec![], vec![], |ui| {
                ui.ctx().input(|input| {
                    presses.set(presses.get() + u32::from(input.pointer.primary_pressed()));
                    down.set(input.pointer.primary_down());
                    latest.set(input.pointer.latest_pos());
                });
            });
        }
    };
    frames(1);
    let (first, second) = (Pos2::new(40.0, 40.0), Pos2::new(90.0, 60.0));
    host.inject_pointer(ViewportId::ROOT, InjectedPointer::down(PointerButton::Primary, first, Mods::NONE, None));
    frames(4);
    assert!(down.get(), "the first press is held");
    host.inject_pointer(ViewportId::ROOT, InjectedPointer::down(PointerButton::Primary, second, Mods::NONE, None));
    frames(4);
    assert_eq!(presses.get(), 1, "the repeated press reaches nothing");
    assert_eq!(latest.get(), Some(first), "nor moves the pointer");
    host.inject_pointer(ViewportId::ROOT, InjectedPointer::up(second, Mods::NONE));
    frames(4);
    assert!(!down.get(), "one release lets the button go");
    assert_eq!(presses.get(), 1);
}

/// The window the platform reports focused has the keys: `typing` reads its text field, and a key equivalent queued
/// for it reaches that field, not the main window. With no window reported focused, the main window has them.
#[trial]
fn the_focused_window_has_the_keys_and_its_text_field_types() {
    use craft_behavior::{EditCommand, Input, Intent};
    use craft_ui::runtime::focus::{FocusTarget, KeyUse};
    use craft_ui::runtime::keys::intents_of;
    use craft_ui::runtime::windows::KeyEquivalent;
    use craft_ui::testing::focus::{Stop, named_id, register_stop, widget};

    let windows = Windows::new();
    let host = WindowHost::of(&windows.ctx);
    let field = named_id("child_field");
    let focus_field = Cell::new(false);
    let copied = Cell::new((false, false));
    let frame = || {
        copied.set((false, false));
        windows.frame(vec![], vec![], |ui| {
            let copy = Intent::Edit(EditCommand::Copy);
            let root_copy = intents_of(ui.ctx(), None, false).take(copy.clone());
            ui.ctx().show_viewport_immediate(child(), ViewportBuilder::default(), |cui, _| {
                let ctx = cui.ctx().clone();
                widget(cui, field);
                register_stop(&ctx, Stop::new(field, KeyUse::of(Input::Text)));
                if focus_field.take() {
                    craft_ui::runtime::focus::request(&ctx, FocusTarget::Id(field));
                }
                copied.set((root_copy, intents_of(&ctx, Some(field), false).take(copy.clone())));
            });
        });
    };
    let focused = |on| ViewportInfo { focused: Some(on), ..ViewportInfo::default() };
    windows.root_info(focused(false));
    windows.child_info(focused(true));
    for _ in 0..2 {
        frame();
    }
    assert_eq!(host.key_window(), child(), "the focused child window has the keys");
    assert!(!host.typing(), "nothing is focused in it yet");
    focus_field.set(true);
    for _ in 0..3 {
        frame();
    }
    assert!(host.typing(), "the child's text field has the keys");

    host.key_equivalent(host.key_window(), KeyEquivalent::Copy);
    frame();
    assert_eq!(copied.get(), (false, true), "the key equivalent reaches the child's field only");

    windows.root_info(focused(true));
    windows.child_info(focused(false));
    frame();
    assert_eq!(host.key_window(), ViewportId::ROOT, "focus back on the main window gives it the keys");
    assert!(!host.typing(), "the main window has no text field");

    windows.root_info(ViewportInfo::default());
    windows.child_info(ViewportInfo::default());
    frame();
    assert_eq!(host.key_window(), ViewportId::ROOT, "with no window reported focused, the main window has the keys");
}

/// A floating panel opened from the body of a full-window container lies above the container, in its family's tier,
/// and takes a click.
#[trial]
fn a_floating_panel_opened_from_a_full_window_container_takes_input() {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let shown = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        Palette::covering("Workspace").show(ui, |ui, _| {
            Palette::floating("Scope").show(ui, |ui, _| {
                Probe::new(&shown).show(ui);
            });
        });
    });
    fixture.settle();
    let probe = fixture.by_label("Probe").expect("the panel's probe is in the access tree");
    fixture.click(&probe).idle(2);
    assert_eq!(seen.borrow().activations, 1, "the click reaches the panel above the full-window container");
}
