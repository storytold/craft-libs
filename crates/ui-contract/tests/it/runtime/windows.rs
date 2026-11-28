//! `runtime::windows`: the input hook per viewport, window info per viewport, the press release after an OS resize
//! (H4), the raise of a modal's window and the keyboard it takes from a window it blocks, the mount of a full-window
//! container (in a window of its own, or in a full-screen or Wayland-maximized main window), the size of a
//! content-sized window, the app id of child windows, a palette's level under a modal and its visibility while the main
//! window is minimized, injected keys across a focus loss, non-finite injected points, a repeated injected press, the
//! window that has the keys, a wheel with no known pointer, what a layer in a popup window keeps while it is open, the
//! placement of a minimized window, the root frame decided again, a held close, `act`, and a hosted window's
//! Control-click and window-input policy.

use craft_behavior::{Cursor, CursorIcon, PointerButton, Sense};
use craft_host::target::{Backend, Caps, Os, ShellInfo, WindowChrome, WindowInfo, WindowKind};
use craft_keymap::{Key, Mods};
use craft_ui::contract::{Common, Component, Container, ContainerProps, Cx, Mount, Overlay, Props as _, Role, Scroll, Show as _, ShowContainer as _};
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::focus::FocusPolicy;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::runtime::windows::{WindowHost, WindowLevel, WindowSpec};
use craft_ui::style::StyleCx;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::{moved, press, release, wheel};
use craft_ui::testing::windows::{FakeSystem, NoWindows};
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use craft_ui_test_kit::windows::{FilesOver, Out, Windows};
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
                let moves: Vec<Pos2> =
                    input.events.iter().filter_map(|event| if let Event::PointerMoved(at) = event { Some(*at) } else { None }).collect();
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

/// A window that moves reports its inner size again, in points from its physical pixels: at a fractional scale the
/// same size can come back a float step off (290.4 as 290.40002). That is no OS resize, so the press stays down: a
/// release there ended every move of a floating panel on Windows a few points into the drag.
#[trial]
fn a_press_survives_a_size_that_differs_only_by_float_rounding() {
    let shell = ShellInfo::default()
        .os(Os::Windows)
        .caps(Caps::native(Os::Windows))
        .window(WindowInfo::new(WindowKind::Main).chrome(WindowChrome::Integrated));
    let windows = Windows::with(InstallSpec::new(shell, Arc::new(NoWindows)));
    let down = Cell::new(false);
    let sized = |width: f32| ViewportInfo { inner_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 600.0))), ..ViewportInfo::default() };
    let frame = |events: Vec<Event>| {
        windows.frame(events, vec![], |ui| down.set(ui.ctx().input(|input| input.pointer.any_down())));
    };
    windows.root_info(sized(800.0));
    frame(vec![]);
    frame(vec![moved(Pos2::new(20.0, 20.0))]);
    frame(vec![press(Pos2::new(20.0, 20.0))]);
    assert!(down.get(), "the press is down");
    windows.root_info(sized(800.0 + 800.0 * f32::EPSILON));
    frame(vec![]);
    frame(vec![]);
    assert!(down.get(), "a float step of size is no resize: the press stays down");
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

/// Window facts with the focus as given, at a place the window system tells.
fn focus(on: bool) -> ViewportInfo {
    ViewportInfo { focused: Some(on), inner_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(300.0))), ..ViewportInfo::default() }
}

fn on_x11() -> Windows {
    Windows::with(InstallSpec::new(ShellInfo::default().backend(Backend::X11), Arc::new(FakeSystem::new())))
}

/// How many focus requests `out` sent to `viewport`.
fn focus_requests(out: &Out, viewport: ViewportId) -> usize {
    out.commands.iter().filter(|(id, command)| *id == viewport && *command == ViewportCommand::Focus).count()
}

/// Frames that last longer than a modal's window waits before it takes the keyboard.
const PAST_THE_WAIT: usize = 40;

/// The app's own modal that opens while a window it blocks has the keyboard (a command typed in a floated panel opens
/// the save prompt in the main window) gets the keys it needs: its window takes the OS focus from the blocked one,
/// once, and the wait before it wakes an idle app. Nothing is sent when no blocked window has the focus by then (an OS
/// dialog that opened with the modal took it, or the modal's window has it) or the modal closed meanwhile.
#[trial]
fn an_app_modal_takes_the_keyboard_from_a_window_it_blocks() {
    // The frames after the modal opens in which the main window, which shows it, got a focus request, with the main
    // window and a child window focused as given and `before` run before each frame.
    let requests = |root: bool, float: bool, before: &dyn Fn(&Windows, usize)| -> Vec<usize> {
        let windows = on_x11();
        windows.root_info(focus(root));
        windows.child_info(focus(float));
        let frame = || {
            windows.frame(vec![], vec![], |ui| {
                Palette::new("float", Some(WindowSpec::tool())).show(ui, |_, _| {});
            })
        };
        for _ in 0..4 {
            frame();
        }
        windows.app_modal(Some(ViewportId::ROOT));
        (0..PAST_THE_WAIT)
            .flat_map(|at| {
                before(&windows, at);
                vec![at; focus_requests(&frame(), ViewportId::ROOT)]
            })
            .collect()
    };
    let none: [usize; 0] = [];
    let sent = requests(false, true, &|_, _| {});
    let &[handed] = sent.as_slice() else { panic!("the main window takes the keyboard from the float once: {sent:?}") };
    assert!(handed > 1, "not before an OS dialog that opens with the modal could take it: frame {handed}");
    let closes = |windows: &Windows, at| {
        if at == handed {
            windows.app_modal(None);
        }
    };
    assert_eq!(requests(false, true, &closes), none, "the modal closed meanwhile");
    let os_dialog = |windows: &Windows, at| {
        if at == 1 {
            windows.child_info(focus(false));
        }
    };
    assert_eq!(requests(false, true, &os_dialog), none, "an OS dialog took the focus");
    assert_eq!(requests(false, false, &|_, _| {}), none, "no window of the app has the focus");
    assert_eq!(requests(true, false, &|_, _| {}), none, "the modal's window has the focus");

    let woken = Arc::new(Mutex::new(false));
    let settled = |windows: &Windows, at| {
        if at == 2 {
            let woken = woken.clone();
            windows.ctx.set_request_repaint_callback(move |info| *woken.lock().unwrap() |= !info.delay.is_zero());
        }
    };
    requests(false, true, &settled);
    assert!(*woken.lock().unwrap(), "an idle app wakes for the hand-over");
}

/// A framework modal in a window of its own takes the keyboard from the main window it blocks.
#[trial]
fn a_framework_modal_takes_the_keyboard_from_the_window_it_blocks() {
    let windows = on_x11();
    windows.root_info(focus(true));
    windows.child_info(focus(false));
    let sent: usize = (0..PAST_THE_WAIT).map(|_| focus_requests(&modal_frame(&windows), child())).sum();
    assert_eq!(sent, 1, "once");
}

/// A menu's layer hung from a place near its window's corner: opened in a palette it shows in a popup window.
fn menu_layer() -> ContainerProps {
    use craft_behavior::overlay::{Align, Side};
    use craft_ui::contract::overlay::{Anchor, Place};
    let anchor = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::splat(10.0));
    ContainerProps::new().mount(Mount::Layer(Overlay::menu().place(Place::anchored(Anchor::Rect(anchor), Side::Below, Align::Start))))
}

/// The popup window `out` shows beside the main window and `palette`.
fn popup_window(out: &Out, palette: ViewportId) -> Option<ViewportId> {
    out.builders.iter().map(|(id, _)| *id).find(|id| ![ViewportId::ROOT, palette].contains(id))
}

/// A popup window opened from the window that shows the app's modal keeps the keyboard.
#[trial]
fn a_popup_window_of_the_modals_window_keeps_the_keyboard() {
    let windows = on_x11();
    windows.root_info(focus(false));
    windows.child_info(focus(true));
    let open = Cell::new(false);
    let frame = || {
        windows.frame(vec![], vec![], |ui| {
            Palette::new("tool", Some(WindowSpec::tool())).show(ui, |ui, _| {
                if open.get() {
                    Modal { common: Common::new(), props: menu_layer() }.id_salt("popup").show(ui, |_, _| {});
                }
            });
        })
    };
    let palette = builder_titled(&frame(), "tool").0;
    windows.app_modal(Some(palette));
    open.set(true);
    frame();
    windows.child_info(focus(false));
    let outs: Vec<Out> = (0..PAST_THE_WAIT).map(|_| frame()).collect();
    let popup = outs.last().and_then(|out| popup_window(out, palette)).expect("the layer opened in the palette shows in a popup window");
    assert_eq!(windows.facts(popup).focused, Some(true), "the popup window has the keyboard");
    assert_eq!(outs.iter().map(|out| focus_requests(out, palette)).sum::<usize>(), 0, "the modal's window leaves it there");
}

/// A popup window opened in a palette's first frames stays open while the palette's window has the focus: the
/// palette counts among the app's windows from the frame it opened in, before the main window's input lists it, so
/// the app is not away.
#[trial]
fn a_popup_window_opened_with_its_palette_stays_open() {
    let windows = on_x11();
    windows.root_info(focus(false));
    windows.child_info(focus(true));
    let frame = || {
        windows.frame(vec![], vec![], |ui| {
            Palette::new("tool", Some(WindowSpec::tool())).show(ui, |ui, _| {
                Modal { common: Common::new(), props: menu_layer() }.id_salt("popup").show(ui, |_, _| {});
            });
        })
    };
    let outs: Vec<Out> = (0..4).map(|_| frame()).collect();
    let palette = builder_titled(&outs[0], "tool").0;
    assert!(popup_window(&outs[3], palette).is_some(), "the layer opened with its palette still shows in its popup window");
}

/// A layer in a popup window keeps the scroll that a focus move below its view gave it while it is open, and starts at
/// the top when it opens again.
#[trial]
fn a_layer_in_a_popup_window_keeps_its_scroll_until_it_closes() {
    use craft_ui::runtime::focus::{FocusTarget, request};
    let windows = on_x11();
    windows.root_info(focus(true));
    windows.child_info(focus(true));
    let (open, reveal, top) = (Cell::new(false), Cell::new(false), Cell::new(0.0));
    let (tall, last) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let frame = || {
        windows.frame(vec![], vec![], |ui| {
            Palette::new("tool", Some(WindowSpec::tool())).show(ui, |ui, _| {
                if open.get() {
                    let props = menu_layer().scroll(Scroll::vertical());
                    Modal { common: Common::new(), props }.id_salt("popup").show(ui, |ui, _| {
                        if !craft_ui::testing::is_measuring(ui.ctx()) {
                            top.set(ui.next_widget_position().y);
                        }
                        Probe::new(&tall).id_salt("tall").size(Vec2::new(100.0, 2000.0)).show(ui);
                        Probe::new(&last).id_salt("last").show(ui);
                        if let Some(id) = last.borrow().id.filter(|_| reveal.take()) {
                            request(ui.ctx(), FocusTarget::Id(id));
                        }
                    });
                }
            });
        })
    };
    let settled = || {
        let mut out = frame();
        for _ in 1..PAST_THE_WAIT {
            out = frame();
        }
        out
    };
    let palette = builder_titled(&frame(), "tool").0;
    open.set(true);
    assert!(popup_window(&settled(), palette).is_some(), "the layer opened in the palette shows in a popup window");
    let start = top.get();
    reveal.set(true);
    settled();
    let scrolled = top.get();
    assert!(scrolled < start - 1.0, "the layer stays scrolled: its content starts at {scrolled}, {start} unscrolled");
    open.set(false);
    assert_eq!(popup_window(&settled(), palette), None, "the layer closed");
    open.set(true);
    settled();
    assert!((top.get() - start).abs() < 0.5, "opened again, it starts at the top: {} after {scrolled}, {start} unscrolled", top.get());
}

/// A popup that a press in another window of the app opened stays open, also when its opener runs a second pass of the
/// frame it opens in, and a later press there closes it.
#[trial]
fn a_popup_window_stays_after_the_press_in_another_window_that_opened_it() {
    let windows = on_x11();
    windows.root_info(focus(true));
    windows.child_info(focus(true));
    let (open, again, closed) = (Cell::new(false), Cell::new(false), RefCell::new(Vec::new()));
    let frame = |root: Vec<Event>| {
        windows.frame(root, vec![], |ui| {
            Palette::new("tool", Some(WindowSpec::tool())).show(ui, |ui, _| {
                if open.get() {
                    Capture { common: Common::new(), props: menu_layer(), closed: &closed }.id_salt("popup").show(ui, |_, _| {});
                    if again.take() {
                        craft_ui::runtime::frame::discard_pass(ui.ctx(), "test");
                    }
                }
            });
        })
    };
    let at = Pos2::new(250.0, 250.0);
    frame(vec![]);
    let out = frame(vec![moved(at)]);
    let palette = builder_titled(&out, "tool").0;
    frame(vec![press(at), release(at)]);
    open.set(true);
    again.set(true);
    let mut out = frame(vec![]);
    for _ in 0..4 {
        out = frame(vec![]);
    }
    assert!(!again.get() && popup_window(&out, palette).is_some(), "the popup opened, its opener's pass repeated");
    assert_eq!(*closed.borrow(), [], "the press that opened it leaves it open");
    frame(vec![press(at), release(at)]);
    frame(vec![]);
    assert_eq!(closed.borrow().first(), Some(&CloseReason::Outside), "a later press in the main window closes it");
}

/// A window that a modal in another window blocks takes no wheel, as it takes no press or key: what is under the
/// pointer there does not scroll.
#[trial]
fn a_window_an_app_modal_in_another_window_blocks_takes_no_wheel() {
    use craft_behavior::WheelTaker;
    use craft_ui::testing::wheel::{register, take};
    for open in [false, true] {
        let windows = Windows::new();
        let view = egui::Id::unique("view");
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

#[trial]
fn wheel_over_a_modal_never_reaches_a_view_under_it_during_a_drag() {
    use craft_behavior::WheelTaker;
    use craft_ui::testing::wheel::{register, take};
    let windows = Windows::new();
    let view = egui::Id::unique("dragging view");
    let modal_view = egui::Id::unique("modal view");
    let taken = Cell::new(None);
    let modal_taken = Cell::new(None);
    let open = Cell::new(false);
    let scrolls = Cell::new(false);
    let modal_rect = Cell::new(Rect::NOTHING);
    let seen = RefCell::new(Seen::default());
    let frame = |events: Vec<Event>| {
        windows.frame(events, vec![], |ui| {
            Probe::new(&seen).size(Vec2::splat(800.0)).sense(Sense::DRAG).show(ui);
            register(ui.ctx(), WheelTaker::new(view, Rect::from_min_size(Pos2::ZERO, Vec2::splat(800.0))).vertical(true).scrolls(true));
            if let Some(scroll) = take(ui.ctx(), view) {
                taken.set(Some(scroll));
            }
            if open.get() {
                let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
                let modal = Modal { common: Common::new(), props }.id_salt("modal").show(ui, |ui, _| {
                    if scrolls.get() {
                        register(ui.ctx(), WheelTaker::new(modal_view, modal_rect.get()).vertical(true).scrolls(true));
                        if let Some(scroll) = take(ui.ctx(), modal_view) {
                            modal_taken.set(Some(scroll));
                        }
                    }
                });
                modal_rect.set(modal.output.rect);
            }
        })
    };
    let at = Pos2::new(30.0, 30.0);
    for _ in 0..4 {
        frame(vec![moved(at)]);
    }
    frame(vec![press(at)]);
    frame(vec![moved(Pos2::new(100.0, 30.0))]);
    assert!(windows.ctx.dragged_id().is_some(), "the drag is held before the modal opens");
    open.set(true);
    for _ in 0..4 {
        frame(vec![]);
    }
    let over_modal = modal_rect.get().center();
    frame(vec![moved(over_modal)]);
    frame(vec![wheel(-30.0)]);
    assert!(taken.get().is_none(), "wheel over the modal reached the view underneath: {:?}", taken.get());
    scrolls.set(true);
    for _ in 0..3 {
        frame(vec![]);
    }
    frame(vec![wheel(-30.0)]);
    assert!(modal_taken.get().is_some(), "the modal's own scroll region takes the wheel");
    assert!(taken.get().is_none(), "the background stays still when the modal scrolls");
}

/// A window that closes leaves the next frame's input, as an integration drops it, so the modal it showed in its last
/// pass blocks no other window: the root window's close request then reaches the app in that frame.
#[trial]
fn a_closed_windows_modal_no_longer_blocks_the_root_close() {
    let windows = Windows::new();
    modal_frame(&windows);
    windows.frame(vec![], vec![], |_| {});
    windows.root_info(ViewportInfo { events: vec![ViewportEvent::Close], ..ViewportInfo::default() });
    let (listed, asked) = (Cell::new(true), Cell::new(false));
    windows.frame(vec![], vec![], |ui| {
        listed.set(ui.ctx().input(|input| input.raw.viewports.contains_key(&child())));
        asked.set(craft_ui::runtime::windows::root_close_requested(ui.ctx()));
    });
    assert!(!listed.get(), "the closed window is gone from the input");
    assert!(asked.get(), "the closed window's modal refuses no close");
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

/// A modal window that opens over a modal in another window comes first in the app's modal stack: the screen picker's
/// capture windows, one per monitor and all opened in one frame, take the click and the Escape that end the pick and
/// block none of each other, while the window of the Color Picker under them stays blocked until they close.
#[trial]
fn modal_windows_opened_over_a_modal_in_another_window_take_input_and_block_it() {
    let windows = Windows::new();
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0));
    windows.child_info(ViewportInfo { inner_rect: Some(screen), focused: Some(true), ..ViewportInfo::default() });
    let titles = ["monitor 1", "monitor 2"];
    let seen = [RefCell::new(Seen::default()), RefCell::new(Seen::default())];
    let closed = RefCell::new(Vec::new());
    // The capture windows shown, in show order. The trial's renderer hands the child events to the first child pass of
    // the frame, so the order picks the window a click goes to.
    let frame = |shown: &[usize], root: Vec<Event>, child: Vec<Event>| {
        windows.frame(root, child, |ui| {
            let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
            Modal { common: Common::new(), props }.id_salt("color picker").show(ui, |_, _| {});
            for &at in shown {
                Capture::new(titles[at], screen, &closed).show(ui, |ui, _| {
                    Probe::new(&seen[at]).size(Vec2::splat(120.0)).sense(Sense::CLICK).show(ui);
                });
            }
        })
    };
    let (both, flipped) = ([0, 1], [1, 0]);
    let blocked = Some(Cursor::System(CursorIcon::NotAllowed));
    for _ in 0..3 {
        frame(&[], vec![], vec![]);
    }
    let mut out = frame(&both, vec![], vec![]);
    for _ in 0..3 {
        out = frame(&both, vec![], vec![]);
    }
    let captures = titles.map(|title| builder_titled(&out, title).0);

    let at = Pos2::new(50.0, 50.0);
    for order in [both, flipped] {
        frame(&order, vec![], vec![moved(at)]);
        frame(&order, vec![], vec![press(at)]);
        frame(&order, vec![], vec![release(at)]);
        frame(&order, vec![], vec![]);
    }
    let clicks: Vec<u32> = seen.iter().map(|seen| seen.borrow().activations).collect();
    assert_eq!(clicks, [1, 1], "a click reaches each capture window");

    assert_eq!(craft_ui::runtime::cursor::current(&windows.ctx), blocked, "the Color Picker's window is blocked");
    let mut sent = frame(&both, vec![moved(at), press(at)], vec![]).commands;
    sent.extend(frame(&both, vec![release(at)], vec![]).commands);
    assert!(
        sent.iter().any(|(id, command)| captures.contains(id) && *command == ViewportCommand::Focus),
        "a press in the Color Picker's window raises a capture window: {sent:?}"
    );

    frame(&both, vec![], vec![key_press(egui::Key::Escape, egui::Modifiers::NONE)]);
    frame(&both, vec![], vec![]);
    assert_eq!(*closed.borrow(), [CloseReason::Escape], "Escape closes the capture window, which ends the pick");
    for _ in 0..3 {
        frame(&[], vec![], vec![]);
    }
    assert_ne!(craft_ui::runtime::cursor::current(&windows.ctx), blocked, "the pick ended: the Color Picker's window takes input again");
}

/// A modal container in an OS window of its own over `over`, as the screen picker's capture windows mount, that adds
/// how it closed to `closed`.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Capture<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    closed: &'a RefCell<Vec<CloseReason>>,
}

impl<'a> Capture<'a> {
    fn new(title: &'static str, over: Rect, closed: &'a RefCell<Vec<CloseReason>>) -> Self {
        let mount = Mount::viewport_with(Overlay::modal().placed(over), WindowSpec::dialog().level(WindowLevel::AlwaysOnTop).taskbar(false));
        Capture { common: Common::new(), props: ContainerProps::new().mount(mount), closed }.label(title).id_salt(title)
    }
}

impl Component for Capture<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: CaptureStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(120.0, 60.0))
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Capture<'_> {
    fn closed(&mut self, why: CloseReason) {
        self.closed.borrow_mut().push(why);
    }
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

    /// A tool window that may not be given less than `least`.
    fn at_least(title: &'static str, least: Vec2) -> Self {
        let mount = Mount::viewport_with(Overlay::floating().min_size(least), WindowSpec::tool());
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
    // Focus leaves the palette's window, which the main window's input still lists as focused. Windows puts an
    // inactive app's palette at the bottom, since `Normal` there stands in front of other apps' windows.
    let inactive = Some(if cfg!(windows) { egui::WindowLevel::AlwaysOnBottom } else { egui::WindowLevel::Normal });
    frame(false, false);
    assert_eq!(frame(false, false), inactive, "the frame after focus left every window");
    frame(true, false);
    assert_eq!(frame(true, false), on_top, "the frame after the main window took focus again");
}

/// A palette stands as an ordinary window while an app-modal layer in the main window blocks it, so the modal is never
/// behind a palette: the frame after the modal opens builds the palette's window at the ordinary level, a later frame
/// raises the main window above it once, by its level (egui ignores `Focus` for a window that has focus), and the
/// palette stands above the app's windows again once the modal closes.
#[trial]
fn a_palette_stands_behind_the_window_that_holds_a_modal() {
    let windows = Windows::new();
    windows.root_info(ViewportInfo { focused: Some(true), ..ViewportInfo::default() });
    let frame = |modal: bool| {
        let out = windows.frame(vec![], vec![], |ui| {
            windows.child_info(ViewportInfo { focused: Some(false), ..ViewportInfo::default() });
            Palette::new("tool", Some(WindowSpec::tool())).show(ui, |_, _| {});
            if modal {
                let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
                Modal { common: Common::new(), props }.id_salt("save prompt").show(ui, |_, _| {});
            }
        });
        let raise: Vec<ViewportCommand> = out
            .commands
            .iter()
            .filter(|(id, command)| *id == ViewportId::ROOT && matches!(command, ViewportCommand::WindowLevel(_) | ViewportCommand::Focus))
            .map(|(_, command)| command.clone())
            .collect();
        (builder_titled(&out, "tool").1.window_level, raise)
    };
    let (on_top, normal) = (Some(egui::WindowLevel::AlwaysOnTop), Some(egui::WindowLevel::Normal));
    assert_eq!(frame(false), (on_top, vec![]), "no modal: the palette stands above the main window");
    let frames: Vec<_> = (0..4).map(|_| frame(true)).collect();
    assert_eq!(frames.last().map(|(level, _)| *level), Some(normal), "the modal open: the palette is an ordinary window");
    let raises: Vec<usize> = frames.iter().enumerate().filter(|(_, (_, raise))| !raise.is_empty()).map(|(at, _)| at).collect();
    let lowered = frames.iter().position(|(level, _)| *level == normal);
    assert_eq!(raises.len(), 1, "the main window is raised once: {frames:?}");
    assert!(lowered.is_some_and(|lowered| raises[0] > lowered), "the raise follows the frame that lowered the palette: {frames:?}");
    let level = |level| ViewportCommand::WindowLevel(level);
    assert_eq!(
        frames[raises[0]].1,
        vec![level(egui::WindowLevel::AlwaysOnTop), level(egui::WindowLevel::Normal)],
        "the raise stands the main window above every window and back, without focus"
    );
    frame(false);
    assert_eq!(frame(false), (on_top, vec![]), "the modal closed: the palette stands above again");
}

/// A palette's window is hidden while the main window is minimized and shown again at its place when the main window
/// is restored, on Windows and Linux; a document window stays. On macOS a minimized main window leaves the app active,
/// and its palettes stay.
#[trial]
fn palettes_hide_while_the_main_window_is_minimized() {
    for os in [Os::Windows, Os::Linux, Os::Mac] {
        let windows = Windows::with(InstallSpec::new(ShellInfo::default().os(os).caps(Caps::native(os)), Arc::new(FakeSystem::new())));
        let frame = |minimized: bool| {
            windows.root_info(ViewportInfo { minimized: Some(minimized), ..ViewportInfo::default() });
            windows.frame(vec![], vec![], |ui| {
                Palette::new("tool", Some(WindowSpec::tool())).show(ui, |_, _| {});
                Palette::new("doc", Some(WindowSpec::document())).show(ui, |_, _| {});
            })
        };
        let first = frame(false);
        let (tool, doc) = (builder_titled(&first, "tool").0, builder_titled(&first, "doc").0);
        frame(false);
        let commands = |minimized: bool| -> Vec<(ViewportId, ViewportCommand)> { (0..3).flat_map(|_| frame(minimized).commands).collect() };
        let sent = |commands: &[(ViewportId, ViewportCommand)], to: ViewportId| -> Vec<ViewportCommand> {
            commands.iter().filter(|(id, _)| *id == to).map(|(_, command)| command.clone()).collect()
        };
        let (minimized, restored) = (commands(true), commands(false));
        let hides = os != Os::Mac;
        let visible = |on: bool| if hides { vec![ViewportCommand::Visible(on)] } else { vec![] };
        let (tool_minimized, doc_minimized) = (sent(&minimized, tool), sent(&minimized, doc));
        let (tool_restored, doc_restored) = (sent(&restored, tool), sent(&restored, doc));
        assert_eq!(tool_minimized, visible(false), "{os:?}: the main window minimized: the palette is hidden, and nothing else is sent");
        assert_eq!(tool_restored, visible(true), "{os:?}: the main window restored: the palette is shown where it was");
        assert_eq!((doc_minimized, doc_restored), (vec![], vec![]), "{os:?}: a document window is left as it is");
    }
}

/// A tool window asks for the Windows tool-window style, and a document window, a tool window with a taskbar entry
/// and a tool window whose spec turns into a document window's do not.
#[trial]
fn tool_windows_ask_for_the_windows_tool_window_style() {
    let windows = Windows::with(InstallSpec::new(ShellInfo::default().os(Os::Windows).caps(Caps::native(Os::Windows)), Arc::new(FakeSystem::new())));
    let frame = |document: bool| {
        windows.frame(vec![], vec![], |ui| {
            let spec = if document { WindowSpec::document() } else { WindowSpec::tool() };
            Palette::new("tool", Some(spec)).show(ui, |_, _| {});
            Palette::new("listed", Some(WindowSpec::tool().taskbar(true))).show(ui, |_, _| {});
            Palette::new("doc", Some(WindowSpec::document())).show(ui, |_, _| {});
        })
    };
    let first = frame(false);
    assert_eq!(builder_titled(&first, "tool").1.tool_window, Some(true), "a tool window has the style");
    assert_eq!(builder_titled(&first, "listed").1.tool_window, Some(false), "a tool window with a taskbar entry does not");
    assert_eq!(builder_titled(&first, "doc").1.tool_window, Some(false), "a document window does not");
    frame(false);
    let turned = frame(true);
    assert_eq!(builder_titled(&turned, "tool").1.tool_window, Some(false), "the window turned into a document window drops the style");
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

/// photocraft#16595: a full-window container in a window of its own keeps the size the window has. Its body takes
/// less than the window (side panels, header and footer are no part of it), and the window was sent that size each
/// frame, so it shrank until it was a pixel wide.
#[trial]
fn a_full_window_viewport_keeps_its_window_size() {
    const GIVEN: Vec2 = Vec2::new(1000.0, 700.0);
    let windows = Windows::new();
    let frame = || {
        windows.frame(vec![], vec![], |ui| {
            Palette::full_window("Workspace").show(ui, |ui, _| {
                ui.allocate_exact_size(Vec2::new(300.0, 200.0), egui::Sense::hover());
            });
        })
    };
    let (dialog, _) = builder_titled(&frame(), "Workspace");
    let report = |size: Vec2| {
        let rect = Rect::from_min_size(Pos2::new(100.0, 100.0), size);
        windows.child_info(ViewportInfo {
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            native_pixels_per_point: Some(1.0),
            ..ViewportInfo::default()
        });
    };
    let sizes_sent = || -> Vec<Vec2> {
        (0..6)
            .flat_map(|_| frame().commands)
            .filter_map(|(id, command)| match command {
                ViewportCommand::InnerSize(size) if id == dialog => Some(size),
                _ => None,
            })
            .collect()
    };
    report(GIVEN);
    assert_eq!(sizes_sent(), Vec::<Vec2>::new(), "the workspace's window keeps the size it has");
    report(Vec2::new(1200.0, 820.0));
    assert_eq!(sizes_sent(), Vec::<Vec2>::new(), "a size the user gives the window is kept too");
}

/// Placing a minimized window below its smallest inner size restores it and lowers that size to the placed one, so the
/// window takes the place it is given.
#[trial]
fn placing_a_minimized_window_below_its_minimum_restores_it_at_the_placed_size() {
    const PLACED: Rect = Rect { min: Pos2::new(10.0, 20.0), max: Pos2::new(50.0, 50.0) };
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(FakeSystem::new())));
    let frame = || {
        windows.frame(vec![], vec![], |ui| {
            Palette::at_least("tool", Vec2::new(200.0, 150.0)).show(ui, |_, _| {});
        })
    };
    let first = frame();
    let (id, _) = builder_titled(&first, "tool");
    let given = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(400.0, 300.0));
    windows.child_info(ViewportInfo {
        inner_rect: Some(given),
        outer_rect: Some(given),
        native_pixels_per_point: Some(1.0),
        minimized: Some(true),
        ..ViewportInfo::default()
    });
    frame();
    frame();
    assert!(WindowHost::of(&windows.ctx).set_outer(id, PLACED), "the commands went out");
    let sent: Vec<ViewportCommand> = frame().commands.into_iter().filter(|(window, _)| *window == id).map(|(_, command)| command).collect();
    assert!(sent.contains(&ViewportCommand::Minimized(false)), "the window is restored: {sent:?}");
    assert!(sent.contains(&ViewportCommand::InnerSize(PLACED.size())), "the window takes the placed size: {sent:?}");
    let lowered = sent.iter().find_map(|command| match command {
        ViewportCommand::MinInnerSize(size) => Some(*size),
        _ => None,
    });
    let lowered = lowered.expect("the minimum went below the placed size");
    assert!(lowered.x <= PLACED.width() && lowered.y <= PLACED.height(), "the minimum fits the placed size: {lowered:?}");
}

/// On a desktop of two monitors at different scales, a window whose corner lies on the second reports the second as
/// its monitor, and, with no scale of its own from the platform, takes that monitor's scale: one rule finds both.
#[trial]
fn a_window_on_the_second_of_two_monitors_reports_that_monitor_and_its_scale() {
    use craft_behavior::{Monitor, MonitorKey};
    let system = Arc::new(FakeSystem::new());
    let first = Monitor::new(MonitorKey::new("first", [1920, 1080]), [0, 0], [1920, 1080], 1.0);
    let second = Monitor::new(MonitorKey::new("second", [2880, 1800]), [1920, 0], [2880, 1800], 2.0);
    system.set_monitors(vec![first, second.clone()]);
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), system));
    let frame = || {
        windows.frame(vec![], vec![], |ui| {
            Palette::new("tool", Some(WindowSpec::tool())).show(ui, |_, _| {});
        })
    };
    let (id, _) = builder_titled(&frame(), "tool");
    // 2000 points is past the first monitor at its scale of 1 and, at 4000 pixels, inside the second at its scale of 2.
    let placed = Rect::from_min_size(Pos2::new(2000.0, 100.0), Vec2::new(400.0, 300.0));
    windows.child_info(ViewportInfo { inner_rect: Some(placed), outer_rect: Some(placed), ..ViewportInfo::default() });
    frame();
    frame();
    let host = WindowHost::of(&windows.ctx);
    assert_eq!(host.monitor_of(id).map(|monitor| monitor.key), Some(second.key), "the window is on the second monitor");
    assert_eq!(host.facts(id).and_then(|facts| facts.scale), Some(second.scale), "the window takes the second monitor's scale");
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
    host.inject_wheel(ViewportId::ROOT, Pos2::new(20.0, 20.0), Vec2::new(0.0, f32::NAN), egui::MouseWheelUnit::Point, Mods::NONE);
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

/// A button the platform reports up while egui holds it (the window never got the release) is let go once, at the
/// pointer's last position so no click results. Nothing is released while the platform reports the button down, while
/// injected input holds it, or while the platform tells nothing.
#[trial]
fn a_button_the_platform_reports_up_is_released_once() {
    use craft_ui::runtime::windows::InjectedPointer;
    use craft_ui::testing::windows::HeldButtons;

    let system = Arc::new(FakeSystem::new());
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), system.clone()));
    let host = WindowHost::of(&windows.ctx);
    let secondary = egui::PointerButton::Secondary;
    let down = Cell::new(false);
    let clicked = Cell::new(false);
    let releases = RefCell::new(Vec::new());
    let frame = |events: Vec<Event>| {
        windows.frame(events, vec![], |ui| {
            ui.ctx().input(|input| {
                down.set(input.pointer.button_down(secondary));
                clicked.set(clicked.get() || input.pointer.button_clicked(secondary));
                releases.borrow_mut().extend(input.raw.events.iter().filter_map(|event| match event {
                    Event::PointerButton { pos, pressed: false, .. } => Some(*pos),
                    _ => None,
                }));
            });
        });
    };
    let at = Pos2::new(20.0, 20.0);
    frame(vec![]);
    frame(vec![moved(at)]);
    system.set_buttons_held(Some(HeldButtons::SECONDARY));
    frame(vec![Event::PointerButton { pos: at, button: secondary, pressed: true, modifiers: egui::Modifiers::NONE }]);
    assert!(down.get(), "the press is down");

    frame(vec![moved(Pos2::new(60.0, 20.0))]);
    assert!(down.get() && releases.borrow().is_empty(), "the system reports the button down: nothing is released");
    system.set_buttons_held(None);
    frame(vec![moved(Pos2::new(80.0, 20.0))]);
    assert!(down.get() && releases.borrow().is_empty(), "the platform tells nothing: nothing is released");

    system.set_buttons_held(Some(HeldButtons::NONE));
    let away = Pos2::new(120.0, 20.0);
    frame(vec![moved(away)]);
    frame(vec![moved(Pos2::new(140.0, 20.0))]);
    assert!(!down.get(), "the system reports the button up: egui lets it go");
    assert_eq!(*releases.borrow(), [away], "one release, at the pointer's position in the frame");
    assert!(!clicked.get(), "a release away from the press is no click");

    host.inject_pointer(ViewportId::ROOT, InjectedPointer::down(PointerButton::Secondary, at, Mods::NONE, None));
    for _ in 0..4 {
        frame(vec![]);
    }
    assert!(down.get(), "the injected press is held");
    for _ in 0..3 {
        frame(vec![moved(Pos2::new(160.0, 20.0))]);
    }
    assert!(down.get(), "an injected hold between steps is no lost button");
}

/// A palette window hides while the root window shows an app-modal layer, returns in the frames after it closes, and
/// one that opens meanwhile opens hidden.
#[trial]
fn a_palette_hides_while_the_root_shows_a_modal_or_popup_and_returns_when_it_closes() {
    let windows = Windows::new();
    let frame = |modal: bool, second: bool| {
        windows.frame(vec![], vec![], |ui| {
            Palette::new("tool", Some(WindowSpec::tool())).show(ui, |_, _| {});
            if second {
                Palette::new("second", Some(WindowSpec::tool())).show(ui, |_, _| {});
            }
            if modal {
                let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
                Modal { common: Common::new(), props }.id_salt("modal").show(ui, |_, _| {});
            }
        })
    };
    let visibility = |out: &Out| {
        out.commands.iter().rev().find_map(|(id, command)| match command {
            ViewportCommand::Visible(visible) if *id != ViewportId::ROOT => Some(*visible),
            _ => None,
        })
    };
    for _ in 0..4 {
        frame(false, false);
    }
    assert_eq!((0..2).find_map(|_| visibility(&frame(false, false))), None, "no layer: the palette stays as it is");
    let hidden = (0..3).filter_map(|_| visibility(&frame(true, false))).reduce(|_, latest| latest);
    assert_eq!(hidden, Some(false), "the palette hides under the modal");
    let out = frame(true, true);
    assert_eq!(builder_titled(&out, "second").1.visible, Some(false), "a palette opened meanwhile opens hidden");
    let back = (0..3).filter_map(|_| visibility(&frame(false, false))).reduce(|_, latest| latest);
    assert_eq!(back, Some(true), "the palette returns once the modal closed");
}

/// A palette that opens while the palettes still yield to a modal or popup of the root window that has just closed
/// opens hidden and appears in the frames after.
#[trial]
fn a_palette_opened_as_the_roots_modal_or_popup_closes_appears() {
    let windows = Windows::new();
    let frame = |modal: bool, palette: bool| {
        windows.frame(vec![], vec![], |ui| {
            if modal {
                let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
                Modal { common: Common::new(), props }.id_salt("modal").show(ui, |_, _| {});
            }
            if palette {
                Palette::new("tool", Some(WindowSpec::tool())).show(ui, |_, _| {});
            }
        })
    };
    for _ in 0..3 {
        frame(true, false);
    }
    frame(false, false);
    let out = frame(false, true);
    let (tool, builder) = builder_titled(&out, "tool");
    let mut shown = builder.visible != Some(false);
    assert!(!shown, "it opens hidden while the palettes yield");
    for out in [out].into_iter().chain((0..3).map(|_| frame(false, true))) {
        for (id, command) in out.commands {
            if let (true, ViewportCommand::Visible(visible)) = (id == tool, command) {
                shown = visible;
            }
        }
    }
    assert!(shown, "it appears once the modal is gone");
}

/// A key automation holds is in `keys::held` while it is held and not after its release.
#[trial]
fn a_key_held_by_automation_reads_as_held() {
    let windows = Windows::new();
    let host = WindowHost::of(&windows.ctx);
    let seen = RefCell::new(Vec::new());
    let frame = || {
        windows.frame(vec![], vec![], |ui| *seen.borrow_mut() = craft_ui::runtime::keys::held(ui.ctx()));
    };
    frame();
    host.inject_hold(ViewportId::ROOT, Key::A);
    for _ in 0..3 {
        frame();
    }
    assert_eq!(*seen.borrow(), [Key::A], "the held key reads as down");
    host.inject_let_go(ViewportId::ROOT, Key::A);
    for _ in 0..3 {
        frame();
    }
    assert!(seen.borrow().is_empty(), "the released key reads as up");
}

/// SG-30: X11 rejects a window title that holds a NUL, so `act` drops every NUL before it compares the title with the
/// one it sent last: the window gets the title without it, a title sent again (or differing only by NULs) sends no
/// second command, and a changed title sends one.
#[trial]
fn a_title_with_nul_reaches_the_window_without_it() {
    use craft_ui::runtime::windows::{WindowAction, act};
    let shell = ShellInfo::default().os(Os::Linux).backend(Backend::X11).caps(Caps::native(Os::Linux));
    let windows = Windows::with(InstallSpec::new(shell, Arc::new(NoWindows)));
    windows.frame(vec![], vec![], |_| {});
    // The titles one frame that asks for `title` sends to the main window, over every pass of the frame.
    let send = |title: &str| -> Vec<String> {
        let out = windows.frame(vec![], vec![], |ui| {
            act(ui.ctx(), ViewportId::ROOT, WindowAction::SetTitle(title.to_owned()));
        });
        out.commands
            .iter()
            .filter_map(|(id, command)| match command {
                ViewportCommand::Title(sent) if *id == ViewportId::ROOT => Some(sent.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(send("Report\0 draft.docx"), ["Report draft.docx"], "the window gets the title without its NUL");
    assert!(send("Report\0 draft.docx").is_empty(), "the same title again sends nothing");
    assert!(send("Report draft.docx").is_empty(), "a title that differs only by NULs is the one already sent");
    assert_eq!(send("Notes.docx"), ["Notes.docx"], "a changed title is sent");
    assert_eq!(send("\0Report\0\0 draft.docx\0"), ["Report draft.docx"], "a title changed back is sent again, every NUL dropped");
}

/// A double click on the move band of an app-drawn frame follows the desktop's setting (GNOME's
/// `action-double-click-titlebar` gives an integrated frame minimize or nothing), as a macOS inset frame follows
/// its own; a native frame leaves it to the OS.
#[trial]
fn a_double_click_on_an_app_drawn_frame_follows_the_setting() {
    use craft_host::target::DoubleClick;
    use craft_ui::runtime::windows::{WindowAction, double_click_action};
    use craft_window::policy::{CaptionFamily, Captions};
    let integrated = Frame::Integrated(Captions { family: CaptionFamily::Adwaita, layout: craft_window::Layout::default() });
    let inset = Frame::Inset(MacInset::default());
    for frame in [&integrated, &inset] {
        assert_eq!(double_click_action(frame, DoubleClick::Zoom, false), Some(WindowAction::SetMaximized(true)), "{frame:?}");
        assert_eq!(double_click_action(frame, DoubleClick::Zoom, true), Some(WindowAction::SetMaximized(false)), "{frame:?}");
        assert_eq!(double_click_action(frame, DoubleClick::Minimize, false), Some(WindowAction::Minimize), "{frame:?}");
        assert_eq!(double_click_action(frame, DoubleClick::Nothing, false), None, "{frame:?}");
    }
    assert_eq!(double_click_action(&Frame::Native, DoubleClick::Zoom, false), None, "the OS acts on its own frame");
}

/// What the hosted child window's passes saw: its last pass's pointer, events and OS files, and the clicks since the
/// last reset.
#[derive(Clone, Copy, Debug, Default)]
struct ChildSeen {
    pointer: bool,
    events: usize,
    files: usize,
    primary: bool,
    secondary: bool,
}

/// One frame of a root window that hosts the child window as an OS window with a 200-point click area at its origin:
/// `root` events go to the root, `events` and `dropped` files to the child. Returns whether the root knows the pointer.
fn hosted_frame(windows: &Windows, root: Vec<Event>, events: Vec<Event>, dropped: Vec<String>, seen: &Cell<ChildSeen>) -> bool {
    let root_pointer = Cell::new(false);
    windows.frame_with_files(root, events, FilesOver::Child, Vec::new(), dropped, |ui| {
        root_pointer.set(ui.ctx().input(|input| input.pointer.latest_pos().is_some()));
        ui.ctx().show_viewport_immediate(child(), ViewportBuilder::default(), |cui, _| {
            let area = cui.allocate_rect(Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0)), egui::Sense::click());
            let (pointer, events, files) = cui.ctx().input(|input| {
                (input.pointer.latest_pos().is_some(), input.events.len(), input.raw.hovered_files.len() + input.raw.dropped_files.len())
            });
            let was = seen.get();
            seen.set(ChildSeen {
                pointer,
                events,
                files,
                primary: was.primary || area.clicked(),
                secondary: was.secondary || area.secondary_clicked(),
            });
        });
    });
    root_pointer.get()
}

/// A panel floated into its own OS window gets a macOS Control+Click as a secondary click, as the main window does:
/// the Control-click mapping runs on every hosted window's input, which eframe's `raw_input_hook` never sees.
#[trial]
fn a_control_click_in_a_hosted_window_is_a_secondary_click_on_macos() {
    let shell = ShellInfo::default().os(Os::Mac).caps(Caps::native(Os::Mac));
    let windows = Windows::with(InstallSpec::new(shell, Arc::new(FakeSystem::new())));
    let seen = Cell::new(ChildSeen::default());
    let at = Pos2::new(50.0, 50.0);
    let control = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::CTRL };
    hosted_frame(&windows, vec![], vec![], vec![], &seen);
    hosted_frame(&windows, vec![], vec![moved(at)], vec![], &seen);
    hosted_frame(&windows, vec![], vec![control(true)], vec![], &seen);
    hosted_frame(&windows, vec![], vec![control(false)], vec![], &seen);
    hosted_frame(&windows, vec![], vec![], vec![], &seen);
    assert!(seen.get().secondary && !seen.get().primary, "the Control+Click is a secondary click there: {:?}", seen.get());
}

/// `WindowInput::Dropped` (an agent-driven session): a hosted window's own events and OS files reach none of its
/// passes, while the root's input stays the host's and injected input still reaches the hosted window. Under `Live`
/// the same input reaches it.
#[trial]
fn dropped_window_input_leaves_a_hosted_window_only_injected_input() {
    use craft_ui::runtime::frame::WindowInput;
    let at = Pos2::new(50.0, 50.0);
    for policy in [WindowInput::Live, WindowInput::Dropped] {
        let windows = Windows::new();
        windows.window_input(policy);
        let seen = Cell::new(ChildSeen::default());
        hosted_frame(&windows, vec![], vec![], vec![], &seen);
        let root_pointer = hosted_frame(&windows, vec![moved(at)], vec![moved(at)], vec!["clip.mov".to_owned()], &seen);
        assert!(root_pointer, "{policy:?}: the root's input is the host's");
        let after_move = seen.get();
        hosted_frame(&windows, vec![], vec![press(at)], vec![], &seen);
        hosted_frame(&windows, vec![], vec![release(at)], vec![], &seen);
        hosted_frame(&windows, vec![], vec![], vec![], &seen);
        let live = policy == WindowInput::Live;
        assert_eq!(after_move.pointer, live, "{policy:?}: the window's pointer move {after_move:?}");
        assert_eq!(after_move.events > 0, live, "{policy:?}: the window's events {after_move:?}");
        assert_eq!(after_move.files > 0, live, "{policy:?}: the files dropped on the window {after_move:?}");
        assert_eq!(seen.get().primary, live, "{policy:?}: the window's own click {:?}", seen.get());

        seen.set(ChildSeen::default());
        let host = WindowHost::of(&windows.ctx);
        host.inject_click(child(), Pos2::new(60.0, 60.0), PointerButton::Primary, 1, Mods::NONE, false);
        for _ in 0..8 {
            hosted_frame(&windows, vec![], vec![], vec![], &seen);
        }
        assert!(seen.get().primary, "{policy:?}: the injected click reaches the window: {:?}", seen.get());
    }
}
