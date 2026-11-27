//! Zoom-to-window sizing of a document window: the canvas size a zoom asks for (`zoom::wrapped_canvas`), the window
//! size between its minimum and the screen (`placement::capped`), and the window taking it when its body files the
//! request (`DocumentWindow::wrap_in`).

use craft_behavior::canvas::CanvasView;
use craft_behavior::canvas::zoom::wrapped_canvas;
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::placement::capped;
use craft_behavior::{Monitor, MonitorKey};
use craft_host::target::{Caps, ShellInfo};
use craft_ui::contract::MAX_EXTENT;
use craft_ui::contract::{Props as _, ShowContainer as _, SizeLimits};
use craft_ui::runtime::InstallSpec;
use craft_ui::shell::{DocumentWindow, Wrapped};
use craft_ui::testing::trial;
use craft_ui::testing::windows::FakeSystem;
use craft_ui_test_kit::windows::Windows;
use egui::{Pos2, Rect, Vec2, ViewportCommand, ViewportId, ViewportInfo, pos2, vec2};
use std::sync::Arc;

/// The window's own minimum, as an app sets it through the fallback layer.
const MIN: Vec2 = Vec2::new(240.0, 160.0);

/// The window's decoration above its inner rect: the OS title bar.
const TITLE: f32 = 30.0;

/// The slack of a size comparison after f32 maths, in points.
const TOLERANCE: f32 = 0.01;

fn near(a: Vec2, b: Vec2) -> bool {
    (a - b).length() < TOLERANCE
}

/// The facts of a normal window whose inner rect is `inner`, under a title bar, at scale 1.
fn normal(inner: Rect) -> ViewportInfo {
    ViewportInfo {
        inner_rect: Some(inner),
        outer_rect: Some(Rect::from_min_max(inner.min - vec2(0.0, TITLE), inner.max)),
        native_pixels_per_point: Some(1.0),
        ..ViewportInfo::default()
    }
}

/// What one frame of the document window did: what it reported, and the last inner size and outer position it sent
/// its OS window.
struct Pass {
    wrapped: Option<Wrapped>,
    size: Option<Vec2>,
    pos: Option<Pos2>,
}

/// One frame of the document window with the window facts `info`, its body calling `wrap_in` with `ask` when set.
fn wrap_frame(windows: &Windows, info: ViewportInfo, ask: Option<(Rect, Vec2)>) -> Pass {
    windows.child_info(info);
    let mut wrapped = None;
    let out = windows.frame(vec![], vec![], |ui| {
        let shown = DocumentWindow::new("Landscape.psd").id_salt("landscape").overlay(|overlay| overlay.size_limits(SizeLimits::at_least(MIN))).show(
            ui,
            |_, body| {
                if let Some((canvas, wanted)) = ask {
                    DocumentWindow::wrap_in(body, canvas, wanted);
                }
            },
        );
        wrapped = shown.output.wrapped;
    });
    let sent = || out.commands.iter().rev().filter(|(id, _)| *id != ViewportId::ROOT).map(|(_, command)| command);
    let size = sent().find_map(|command| match command {
        ViewportCommand::InnerSize(size) => Some(*size),
        _ => None,
    });
    let pos = sent().find_map(|command| match command {
        ViewportCommand::OuterPosition(pos) => Some(*pos),
        _ => None,
    });
    Pass { wrapped, size, pos }
}

/// The production rules of zoom-to-window: the canvas size is the turned document's box plus the rulers and
/// scrollbars; the window size stays between its minimum and the monitor's work area less the frame, the cap winning;
/// the window takes it about its centre and reports `Fits` or `Capped`; a maximised or full-screen window, a window
/// that has not reported its facts, and the fallback layer without OS windows keep their size and report nothing.
#[trial]
fn a_wrapped_document_window_takes_the_document_between_its_minimum_and_the_screen() {
    // The canvas with rulers along the top and the start edge (20 points) and scrollbars along the far edges (15).
    let canvas = Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0));
    let view_rect = Rect::from_min_max(pos2(20.0, 20.0), pos2(485.0, 385.0));
    let document = Size::new(300.0, 200.0);
    let turned_45 = 1000.0 * std::f32::consts::FRAC_1_SQRT_2 + 35.0;
    for (rotation, expected) in [(0.0, vec2(635.0, 435.0)), (90.0, vec2(435.0, 635.0)), (45.0, Vec2::splat(turned_45))] {
        let view = CanvasView::new(view_rect, Point::new(150.0, 100.0), 2.0).rotation(rotation);
        let wanted = wrapped_canvas(&view, document, canvas);
        assert!(near(wanted, expected), "{rotation}°: the turned document at zoom 2 plus the rulers and scrollbars, got {wanted:?}");
    }

    let screen = Monitor::new(MonitorKey::new("DP-1", [1920, 1080]), [0, 0], [1920, 1080], 1.0);
    let retina = Monitor::new(MonitorKey::new("Built-in", [2880, 1800]), [0, 0], [2880, 1800], 2.0).with_work_px([0, 50], [2880, 1700]);
    let frame = vec2(0.0, TITLE);
    let rows = [
        (&screen, vec2(800.0, 600.0), MIN, vec2(800.0, 600.0), "a size inside the work area stays"),
        (&screen, vec2(100.0, 50.0), MIN, MIN, "the minimum raises a small size"),
        (&screen, vec2(3000.0, 2000.0), MIN, vec2(1920.0, 1002.0), "the screen less its taskbar room and the frame caps a large size"),
        (&screen, vec2(100.0, 100.0), vec2(2000.0, 1200.0), vec2(1920.0, 1002.0), "the cap wins over the minimum"),
        (&retina, vec2(2000.0, 2000.0), MIN, vec2(1440.0, 820.0), "a reported work area caps in the monitor's points"),
    ];
    for (monitor, size, min, expected, why) in rows {
        let got = capped(size, monitor, frame, min);
        assert!(near(got, expected), "{why}: {size:?} became {got:?}");
    }

    let system = FakeSystem::new();
    system.set_monitors(vec![screen.clone().with_work_px([0, 0], [1920, 1040])]);
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(system)));
    let inner = Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0));
    // The body's canvas: the whole inner rect below the app's own 30-point title bar.
    let body_canvas = Rect::from_min_size(pos2(0.0, 30.0), vec2(600.0, 370.0));

    let unreported = wrap_frame(&windows, ViewportInfo::default(), Some((body_canvas, vec2(800.0, 500.0))));
    assert_eq!(unreported.wrapped, None, "a window that has not reported its facts is not sized");
    for _ in 0..3 {
        wrap_frame(&windows, normal(inner), None);
    }
    assert_eq!(wrap_frame(&windows, normal(inner), None).wrapped, None, "a pass with no request reports nothing");

    let fits = wrap_frame(&windows, normal(inner), Some((body_canvas, vec2(800.0, 500.0))));
    assert_eq!(fits.wrapped, Some(Wrapped::Fits));
    assert!(fits.size.is_some_and(|size| near(size, vec2(800.0, 530.0))), "the inner size grows by what the canvas asks: {:?}", fits.size);
    // The outer rect (100, 70)–(700, 500) keeps its centre (400, 285) at the new outer size 800 × 560.
    assert!(fits.pos.is_some_and(|pos| near(pos.to_vec2(), vec2(0.0, 5.0))), "the window keeps its centre: {:?}", fits.pos);

    let small = wrap_frame(&windows, normal(inner), Some((body_canvas, vec2(100.0, 40.0))));
    assert_eq!(small.wrapped, Some(Wrapped::Fits), "the minimum still shows the whole document");
    assert!(small.size.is_some_and(|size| near(size, MIN)), "the window's own minimum holds: {:?}", small.size);

    let large = wrap_frame(&windows, normal(inner), Some((body_canvas, vec2(2500.0, 1500.0))));
    assert_eq!(large.wrapped, Some(Wrapped::Capped));
    assert!(large.size.is_some_and(|size| near(size, vec2(1920.0, 1010.0))), "the work area less the frame caps it: {:?}", large.size);

    for (state, info) in [
        ("maximised", ViewportInfo { maximized: Some(true), ..normal(inner) }),
        ("full screen", ViewportInfo { fullscreen: Some(true), ..normal(inner) }),
    ] {
        let kept = wrap_frame(&windows, info, Some((body_canvas, vec2(800.0, 500.0))));
        assert_eq!(kept.wrapped, None, "a {state} window reports nothing");
        assert_eq!(kept.size, None, "a {state} window keeps its size");
    }

    let shell = ShellInfo::default();
    let shell = shell.clone().caps(shell.caps.difference(Caps::VIEWPORTS));
    let layered = Windows::with(InstallSpec::new(shell, Arc::new(FakeSystem::new())));
    for _ in 0..3 {
        wrap_frame(&layered, normal(inner), None);
    }
    let fallback = wrap_frame(&layered, normal(inner), Some((body_canvas, vec2(800.0, 500.0))));
    assert_eq!(fallback.wrapped, None, "the fallback layer has no OS window to size");
    assert_eq!(fallback.size, None, "nothing goes out without OS windows");
}

/// A body that fills its room gets the window's room, not a scroll region's unbounded extent, in the OS window and
/// on the fallback layer.
#[trial]
fn a_document_window_body_fills_the_windows_room_and_does_not_scroll() {
    let inner = Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0));
    let layered_shell = {
        let shell = ShellInfo::default();
        shell.clone().caps(shell.caps.difference(Caps::VIEWPORTS))
    };
    let system = FakeSystem::new();
    system.set_monitors(vec![Monitor::new(MonitorKey::new("DP-1", [1920, 1080]), [0, 0], [1920, 1080], 1.0)]);
    for (mount, windows) in [
        ("OS window", Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(system)))),
        ("fallback layer", Windows::with(InstallSpec::new(layered_shell, Arc::new(FakeSystem::new())))),
    ] {
        let mut height = f32::NAN;
        for _ in 0..4 {
            windows.child_info(normal(inner));
            windows.frame(vec![], vec![], |ui| {
                DocumentWindow::new("Landscape.psd").id_salt("landscape").show(ui, |ui, _| {
                    height = ui.max_rect().height();
                });
            });
        }
        assert!(height < MAX_EXTENT / 2.0, "{mount}: the body's height is the window's room, got {height}");
    }
}

/// A document window at the palette level stands above the app's main window while the app is active (photocraft
/// floating documents, as the references keep them); without a level it is an ordinary window.
#[trial]
fn a_document_window_at_the_palette_level_stands_above_the_main_window() {
    let level = |palette: bool| {
        let windows = Windows::new();
        windows.root_info(ViewportInfo { focused: Some(true), ..ViewportInfo::default() });
        let mut seen = None;
        for _ in 0..3 {
            let out = windows.frame(vec![], vec![], |ui| {
                windows.child_info(ViewportInfo { focused: Some(false), ..ViewportInfo::default() });
                let window = DocumentWindow::new("Landscape.psd");
                let window = if palette { window.level(craft_ui::runtime::windows::WindowLevel::Palette) } else { window };
                window.show(ui, |_, _| {});
            });
            if let Some((_, builder)) = out.builders.iter().find(|(_, builder)| builder.title.as_deref() == Some("Landscape.psd")) {
                seen = builder.window_level.or(seen);
            }
        }
        seen
    };
    assert_eq!(level(true), Some(egui::WindowLevel::AlwaysOnTop), "a palette-level document window stands above the main window");
    assert_ne!(level(false), Some(egui::WindowLevel::AlwaysOnTop), "an ordinary document window does not");
}

/// On the fallback layer, a document window dragged by its title row reports where it went, and a host that places it
/// there shows it there: its access node, which tests and assistive tools read its bounds from, moves with the drag.
#[trial]
fn a_fallback_document_window_dragged_by_its_title_reports_where_it_went() {
    use craft_ui::testing::Fixture;
    use std::cell::Cell;
    use std::rc::Rc;

    let place = Rc::new(Cell::new(Rect::from_min_size(pos2(100.0, 80.0), vec2(400.0, 300.0))));
    let kept = place.clone();
    let mut fixture = Fixture::new(move |ui| {
        let shown = DocumentWindow::new("Landscape.psd").id_salt("landscape").first_placed(kept.get()).show(ui, |_, _| {});
        if let Some(rect) = shown.output.placed {
            kept.set(rect);
        }
    });
    fixture.settle();
    let window = |fixture: &Fixture| fixture.by_label("Landscape.psd").expect("the window's node names it").rect;
    let before = window(&fixture);
    let title = fixture.by_role(craft_ui::contract::Role::HEADING).into_iter().next().expect("the fallback layer shows its title row");
    fixture.drag(&title, vec2(50.0, 30.0)).settle();
    let after = window(&fixture);
    assert!(after.min.x > before.min.x && after.min.y > before.min.y, "the window follows the drag: {before:?} to {after:?}");
    assert!(near(after.size(), before.size()), "a move keeps the window's size: {before:?} to {after:?}");
    assert_eq!(place.get(), after, "the host keeps the window where the drag reported it");
}

/// On the fallback layer, `raise` brings a document window in front of one opened after it in the pass it is set, as a
/// press on its layer does. Guards against a raise that only asked an OS window for focus, so the layer stayed behind.
#[trial]
fn a_raised_fallback_document_window_comes_to_the_front() {
    use craft_ui::runtime::overlay;
    use craft_ui::testing::Fixture;
    use std::cell::Cell;
    use std::rc::Rc;

    let (raise, first, top) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(None)), Rc::new(Cell::new(None)));
    let (asked, seen, front) = (raise.clone(), first.clone(), top.clone());
    let mut fixture = Fixture::new(move |ui| {
        front.set(overlay::top(ui.ctx()));
        for (name, at) in [("First.psd", 40.0), ("Second.psd", 80.0)] {
            let window = DocumentWindow::new(name).id_salt(name).first_placed(Rect::from_min_size(pos2(at, at), vec2(300.0, 200.0)));
            let raised = name == "First.psd" && asked.get();
            let shown = if raised { window.raise() } else { window }.show(ui, |_, _| {});
            if name == "First.psd" {
                seen.set(Some(shown.output.response.id));
            }
        }
    });
    fixture.settle();
    assert!(first.get().is_some() && top.get() != first.get(), "the window opened last is in front");
    raise.set(true);
    fixture.frame(Vec::new());
    raise.set(false);
    fixture.frame(Vec::new());
    assert_eq!(top.get(), first.get(), "the raised window is in front");
}

/// A document window raised in the pass the windows show again after a pass with none shown (Presentation Mode ending)
/// comes to the front where the windows overlap. egui raises every layer that shows again and keeps the old order
/// among them, so a raise asked in that pass alone left the window behind the one opened after it.
#[trial]
fn a_window_raised_as_the_windows_show_again_comes_to_the_front() {
    use craft_ui::testing::Fixture;
    use std::cell::Cell;
    use std::rc::Rc;

    let overlap = pos2(200.0, 200.0);
    let (showing, raise, first, top) = (Rc::new(Cell::new(true)), Rc::new(Cell::new(false)), Rc::new(Cell::new(None)), Rc::new(Cell::new(None)));
    let (shows, asked, seen, front) = (showing.clone(), raise.clone(), first.clone(), top.clone());
    let mut fixture = Fixture::new(move |ui| {
        front.set(ui.ctx().layer_id_at(overlap));
        if !shows.get() {
            return;
        }
        for (name, at) in [("First.psd", 40.0), ("Second.psd", 80.0)] {
            let window = DocumentWindow::new(name).id_salt(name).first_placed(Rect::from_min_size(pos2(at, at), vec2(300.0, 200.0)));
            let raised = name == "First.psd" && asked.get();
            if raised { window.raise() } else { window }.show(ui, |ui, _| {
                if name == "First.psd" {
                    seen.set(Some(ui.layer_id()));
                }
            });
        }
    });
    fixture.settle();
    assert!(first.get().is_some() && top.get() != first.get(), "the window opened last is in front");
    showing.set(false);
    fixture.frame(Vec::new());
    showing.set(true);
    raise.set(true);
    fixture.frame(Vec::new());
    raise.set(false);
    fixture.frame(Vec::new());
    fixture.frame(Vec::new());
    assert_eq!(top.get(), first.get(), "the window raised as they showed again is in front");
}
