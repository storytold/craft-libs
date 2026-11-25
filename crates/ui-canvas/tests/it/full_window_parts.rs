//! The full-window parts' own logic: the view's fit and zoom rules, the wheel zoom, Escape on a focused canvas, the
//! close of the dialog's own window and the mapped overlay.

use craft_behavior::{PixelSize, ZoomScheme};
use craft_ui::canvas::ViewCommands;
use craft_ui::dialog::FullWindowView;
use craft_ui::testing::trial;
use egui::{Pos2, Rect, Vec2};

/// The canvas region's size in points.
const CANVAS: Vec2 = Vec2::new(800.0, 600.0);

/// An image larger than the canvas, so a fitted view shows it reduced.
const IMAGE: PixelSize = PixelSize::new(4000, 3000);

/// Pixels per point of the tests' screen.
const SCALE: f32 = 1.0;

/// Zooms closer than this are the same zoom.
const SAME: f64 = 1e-6;

fn fitted() -> FullWindowView {
    FullWindowView::new().settled(Rect::from_min_size(Pos2::ZERO, CANVAS), IMAGE, SCALE, &ZoomScheme::PHOTOSHOP)
}

#[trial]
fn a_zoom_command_ends_the_fit_and_fit_returns_to_it() {
    let rect = Rect::from_min_size(Pos2::ZERO, CANVAS);
    let mut view = fitted();
    let fit = view.device_zoom();
    assert!(view.is_fit() && fit < 1.0, "a new view fits an image larger than the canvas, reduced");
    assert!(!view.apply(ViewCommands::FitOnScreen), "fitting a fitted view changes nothing");
    assert!(view.apply(ViewCommands::ZoomIn));
    assert!(!view.is_fit() && view.device_zoom() > fit);
    let kept = view.settled(rect, IMAGE, SCALE, &ZoomScheme::PHOTOSHOP);
    assert!((kept.device_zoom() - view.device_zoom()).abs() < SAME, "a view that does not fit keeps its zoom when settled");
    assert!(view.apply(ViewCommands::FitOnScreen));
    assert!((view.settled(rect, IMAGE, SCALE, &ZoomScheme::PHOTOSHOP).device_zoom() - fit).abs() < SAME, "settling a fitted view fits again");
}

/// The canvas's navigation: the wheel, the middle button and the Hand tool it falls back to.
pub(crate) mod canvas_navigation {
    use craft_behavior::PixelSize;
    use craft_behavior::kurbo::Point;
    use craft_ui::canvas::{NavigationOptions, PixelRect, RasterSource, WheelScheme};
    use craft_ui::contract::Bind;
    use craft_ui::dialog::{FullWindowCanvas, FullWindowView};
    use craft_ui::primitives::Revision;
    use craft_ui::testing::{Fixture, pointer, trial};
    use egui::{Color32, ColorImage, Pos2};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    /// Where the pointer rests: inside the canvas, off its centre.
    const OVER: Pos2 = Pos2::new(260.0, 170.0);

    /// How far a document point may move under the pointer in a wheel zoom, in image pixels.
    const STILL: f64 = 0.5;

    /// An image larger than the canvas.
    struct Grey;

    impl RasterSource for Grey {
        fn size(&self) -> PixelSize {
            PixelSize::new(1600, 1200)
        }

        fn revision(&self) -> Revision {
            Revision::new(1)
        }

        fn damage(&self, _: Revision) -> Option<PixelRect> {
            Some(PixelRect::default())
        }

        fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
            out.pixels.fill(Color32::from_rgb(0x80, 0x80, 0x80));
        }
    }

    /// A canvas with no tool, the view the app owns and the image point under the pointer.
    struct Rig {
        fixture: Fixture,
        view: Rc<RefCell<FullWindowView>>,
        hover: Rc<Cell<Option<Point>>>,
    }

    fn rig(navigation: NavigationOptions) -> Rig {
        let view = Rc::new(RefCell::new(FullWindowView::new()));
        let hover = Rc::new(Cell::new(None));
        let (shown, seen) = (view.clone(), hover.clone());
        let mut fixture = Fixture::new(move |ui| {
            let mut view = shown.borrow_mut();
            seen.set(FullWindowCanvas::new(Bind::bound(&mut *view), &Grey).navigation(navigation).show(ui).hover);
        });
        fixture.settle();
        Rig { fixture, view, hover }
    }

    impl Rig {
        fn zoom(&self) -> f64 {
            self.view.borrow().device_zoom()
        }
    }

    #[trial]
    fn the_wheel_zooms_about_the_pointer_by_the_navigation_options() {
        let mut plain = rig(NavigationOptions::default());
        let fitted = plain.zoom();
        plain.fixture.frame(vec![pointer::moved(OVER)]);
        plain.fixture.frame(vec![pointer::wheel_lines(1.0)]);
        assert_eq!(plain.zoom(), fitted, "the plain wheel does not zoom by default");

        let mut navigation = NavigationOptions::default();
        navigation.wheel = WheelScheme::Zoom;
        navigation.animated_zoom = false;
        // Overscroll on: the fit leaves room around the image, and with Overscroll off an image still smaller than the
        // canvas after one notch is centred again, whatever the pointer.
        navigation.overscroll = true;
        let mut zooming = rig(navigation);
        let fitted = zooming.zoom();
        zooming.fixture.frame(vec![pointer::moved(OVER)]);
        let under = zooming.hover.get().expect("the pointer is over the image");
        zooming.fixture.frame(vec![pointer::wheel_lines(1.0)]);
        assert!(zooming.zoom() > fitted, "the wheel zooms in with Zoom with Scroll Wheel");
        assert!(!zooming.view.borrow().is_fit(), "a wheel zoom ends the fit");
        let still = zooming.hover.get().expect("the pointer is still over the image");
        assert!((still.x - under.x).abs() < STILL && (still.y - under.y).abs() < STILL, "the point under the pointer stays: {under:?} {still:?}");
    }
}

/// The momentary tools every full-window canvas runs.
pub(crate) mod momentary {
    use craft_behavior::{CancelReason, PixelSize};
    use craft_ui::canvas::{CanvasTool, PixelRect, RasterSource, ToolAction, ToolContext, ToolPath, ToolPointer};
    use craft_ui::contract::{Bind, ShowContainer as _};
    use craft_ui::dialog::{FullWindowCanvas, FullWindowDialog, FullWindowView};
    use craft_ui::primitives::{Draw, Revision};
    use craft_ui::runtime::focus::Initial;
    use craft_ui::testing::input::key_press;
    use craft_ui::testing::{Fixture, pointer, trial};
    use egui::{Color32, ColorImage, Key, Modifiers, Vec2};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    /// An image larger than the canvas.
    struct Grey;

    impl RasterSource for Grey {
        fn size(&self) -> PixelSize {
            PixelSize::new(1600, 1200)
        }

        fn revision(&self) -> Revision {
            Revision::new(1)
        }

        fn damage(&self, _: Revision) -> Option<PixelRect> {
            Some(PixelRect::default())
        }

        fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
            out.pixels.fill(Color32::from_rgb(0x80, 0x80, 0x80));
        }
    }

    /// A tool that counts the presses it takes.
    struct Counter(Rc<Cell<usize>>);

    impl CanvasTool for Counter {
        fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
            self.0.set(self.0.get() + 1);
            ToolAction::Claimed
        }

        fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

        fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

        fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

        fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

        fn command_form(&self) -> ToolPath {
            ToolPath::Command("the Edit command")
        }
    }

    // The app gives the canvas no momentary set: Space held turns a drag into a pan, and the tool sees no press.
    #[trial]
    fn space_held_pans_a_full_window_canvas_instead_of_pressing_the_tool() {
        let view = Rc::new(RefCell::new(FullWindowView::new()));
        let presses = Rc::new(Cell::new(0));
        let (shown, counted) = (view.clone(), presses.clone());
        let mut fixture = Fixture::new(move |ui| {
            let mut open = true;
            let mut view = shown.borrow_mut();
            let mut tool = Counter(counted.clone());
            FullWindowDialog::new("Edit").opened(Bind::bound(&mut open)).initial_focus(Initial::First).show(ui, |ui, _| {
                FullWindowCanvas::new(Bind::bound(&mut *view), &Grey).tool(&mut tool).show(ui);
            });
        });
        fixture.settle();
        view.borrow_mut().set_device_zoom(2.0);
        fixture.settle();
        let before = view.borrow().view().center;
        let at = view.borrow().view().rect.center();

        fixture.frame(vec![pointer::moved(at), key_press(Key::Space, Modifiers::NONE)]);
        fixture.frame(vec![pointer::press(at)]);
        for step in 1..=4 {
            fixture.frame(vec![pointer::moved(at + Vec2::new(15.0, 10.0) * step as f32)]);
        }
        fixture.frame(vec![pointer::release(at + Vec2::new(60.0, 40.0))]);
        fixture.settle();

        assert_eq!(presses.get(), 0, "the tool sees no press while Space is held");
        let after = view.borrow().view().center;
        assert!(after.x < before.x && after.y < before.y, "the drag pans the view: {before:?} {after:?}");
    }
}

/// Escape on a focused canvas inside a full-window dialog.
pub(crate) mod escape {
    use craft_behavior::{CancelReason, Intent, PixelSize};
    use craft_ui::canvas::{CanvasTool, PixelRect, RasterSource, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
    use craft_ui::contract::{Bind, ShowContainer as _};
    use craft_ui::dialog::{FullWindowCanvas, FullWindowDialog, FullWindowView};
    use craft_ui::primitives::{Draw, Revision};
    use craft_ui::runtime::access::Role;
    use craft_ui::runtime::focus::Initial;
    use craft_ui::runtime::keys;
    use craft_ui::testing::input::key_press;
    use craft_ui::testing::{Fixture, trial};
    use egui::{Color32, ColorImage, Key, Modifiers};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    struct Grey;

    impl RasterSource for Grey {
        fn size(&self) -> PixelSize {
            PixelSize::new(800, 600)
        }

        fn revision(&self) -> Revision {
            Revision::new(1)
        }

        fn damage(&self, _: Revision) -> Option<PixelRect> {
            Some(PixelRect::default())
        }

        fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
            out.pixels.fill(Color32::from_rgb(0x80, 0x80, 0x80));
        }
    }

    /// A tool that takes its clicks and, when `takes_escape`, the Escape.
    struct Picker {
        takes_escape: bool,
    }

    impl CanvasTool for Picker {
        fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
            ToolAction::Claimed
        }

        fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

        fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

        fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

        fn key(&mut self, key: &ToolKey, _: &mut ToolContext<'_>) -> bool {
            self.takes_escape && *key == ToolKey::Cancel
        }

        fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

        fn command_form(&self) -> ToolPath {
            ToolPath::Command("the Edit command")
        }
    }

    // The RAW Develop form: the canvas a click focused leaves the Escape its tool refuses to the dialog, which closes;
    // the window, reading its intents last, sees none. An Escape the tool takes closes nothing.
    #[trial]
    fn escape_on_a_focused_canvas_in_a_full_window_dialog_closes_the_dialog_and_not_the_window() {
        for takes_escape in [false, true] {
            let open = Rc::new(Cell::new(true));
            let window_cancels = Rc::new(Cell::new(0));
            let view = Rc::new(RefCell::new(FullWindowView::new()));
            let (shown, seen, viewed) = (open.clone(), window_cancels.clone(), view.clone());
            let mut fixture = Fixture::new(move |ui| {
                let mut is_open = shown.get();
                let mut tool = Picker { takes_escape };
                let mut view = viewed.borrow_mut();
                FullWindowDialog::new("Workspace").opened(Bind::bound(&mut is_open)).initial_focus(Initial::Keep).show(ui, |ui, _| {
                    FullWindowCanvas::new(Bind::bound(&mut *view), &Grey).tool(&mut tool).show(ui);
                });
                shown.set(is_open);
                keys::with_window_intents(ui.ctx(), false, |window| {
                    let took = window.take(Intent::Cancel);
                    seen.set(seen.get() + u32::from(took));
                    took
                });
            });
            fixture.settle();
            let canvas = fixture.by_role(Role::CANVAS).remove(0);
            fixture.click(&canvas).settle();
            assert!(open.get(), "a click on the canvas leaves the dialog open");
            window_cancels.set(0);
            fixture.frame(vec![key_press(Key::Escape, Modifiers::NONE)]).settle();
            assert_eq!(open.get(), takes_escape, "Escape closes the dialog only when the tool leaves it (tool takes it: {takes_escape})");
            assert_eq!(window_cancels.get(), 0, "the window sees no Escape (tool takes it: {takes_escape})");
        }
    }
}

/// A full-window dialog in a window of its own.
pub(crate) mod own_window {
    use craft_ui::contract::{Bind, ShowContainer as _};
    use craft_ui::dialog::{DialogResult, FullWindowDialog};
    use craft_ui::runtime::overlay::CloseReason;
    use craft_ui::runtime::windows::WindowSpec;
    use craft_ui::testing::trial;
    use craft_ui_test_kit::windows::Windows;
    use egui::{ViewportEvent, ViewportInfo};
    use std::cell::Cell;

    // The OS closing the dialog's window (its close button, a window manager's close) is a cancel: the dialog reports
    // `Requested`, which every host but OK reads as Cancel, so an outer gesture over its edits is taken back.
    #[trial]
    fn closing_a_full_window_dialogs_own_window_cancels_it() {
        let windows = Windows::new();
        let open = Cell::new(true);
        let closed = Cell::new(None);
        let frame = || {
            windows.frame(vec![], vec![], |ui| {
                let mut is_open = open.get();
                let output = FullWindowDialog::new("Workspace")
                    .own_window(WindowSpec::dialog())
                    .outer_gesture(true)
                    .opened(Bind::bound(&mut is_open))
                    .show(ui, |_, _| {})
                    .output;
                open.set(is_open);
                if let Some(reason) = output.closed {
                    closed.set(Some((reason, matches!(output.result(|| ()), Some(DialogResult::Cancel)))));
                }
            })
        };
        for _ in 0..4 {
            frame();
        }
        assert!(open.get(), "the dialog opened in its own window");
        windows.child_info(ViewportInfo { events: vec![ViewportEvent::Close], ..ViewportInfo::default() });
        for _ in 0..2 {
            frame();
        }
        assert!(!open.get(), "closing the dialog's window closes the dialog");
        assert_eq!(closed.get(), Some((CloseReason::Requested, true)), "the close is a cancel");
    }

    /// photocraft#16905: in a window the app frames, a full-window dialog's own window moves by its title row. Its
    /// fallback covers the main window and does not move, and the window took that rule, so RAW Develop's window could
    /// not be moved.
    #[trial]
    fn a_full_window_dialogs_own_window_moves_by_its_title_row() {
        use craft_host::target::{Backend, Os, ShellInfo, WindowKind};
        use craft_ui::runtime::InstallSpec;
        use craft_ui::testing::pointer::{moved, press, release};
        use craft_ui::testing::windows::FakeSystem;
        use craft_window::policy::{Desktop, DesktopKind, Runtime, TitleBarPref};
        use egui::{Pos2, Rect, Vec2, ViewportCommand, pos2, vec2};
        use std::sync::Arc;

        let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
        let integrated =
            craft_window::policy::decide(Os::Linux, Runtime::Native, Backend::X11, &desktop, WindowKind::Main, TitleBarPref::Integrated, None);
        let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(FakeSystem::new())).frame(integrated));
        let open = Cell::new(true);
        let frame = |events: Vec<egui::Event>| {
            windows.frame(vec![], events, |ui| {
                let mut is_open = open.get();
                FullWindowDialog::new("Workspace").own_window(WindowSpec::dialog()).opened(Bind::bound(&mut is_open)).show(ui, |_, _| {});
                open.set(is_open);
            })
        };
        let rect = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(900.0, 640.0));
        windows.child_info(ViewportInfo {
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            native_pixels_per_point: Some(1.0),
            ..ViewportInfo::default()
        });
        for _ in 0..4 {
            frame(vec![]);
        }
        // Clear of the top edge's grip and of the title row's own parts (the title at the start, the close action at
        // the end).
        let at = pos2(450.0, 14.0);
        let mut sent = Vec::new();
        for events in [
            vec![moved(at)],
            vec![press(at)],
            vec![moved(at + vec2(20.0, 10.0))],
            vec![moved(at + vec2(40.0, 20.0))],
            vec![release(at + vec2(40.0, 20.0))],
        ] {
            sent.extend(frame(events).commands.into_iter().map(|(_, command)| command));
        }
        assert!(open.get(), "the dialog stays open");
        assert!(sent.contains(&ViewportCommand::StartDrag), "a drag of the title row moves the window: {sent:?}");
    }
}

/// An overlay drawn through a frame.
pub(crate) mod mapped_overlay {
    use craft_behavior::PixelSize;
    use craft_behavior::geometry::quad::Homography;
    use craft_behavior::kurbo::{Point, Rect};
    use craft_ui::canvas::{PixelRect, RasterSource};
    use craft_ui::contract::Bind;
    use craft_ui::dialog::{FullWindowCanvas, FullWindowView, RasterOverlay};
    use craft_ui::primitives::Revision;
    use craft_ui::testing::{Frames, trial};
    use egui::epaint::Mesh;
    use egui::{Color32, ColorImage, Pos2, Shape};

    /// The overlay's and the document's size.
    const IMAGE: PixelSize = PixelSize::new(40, 40);

    /// The mesh's cells a side.
    const CELLS: u32 = 4;

    /// How far a mesh corner may lie from the corner it should reach, in points.
    const NEAR: f32 = 0.01;

    /// An image of one colour.
    struct Flat;

    impl RasterSource for Flat {
        fn size(&self) -> PixelSize {
            IMAGE
        }

        fn revision(&self) -> Revision {
            Revision::new(1)
        }

        fn damage(&self, _: Revision) -> Option<PixelRect> {
            None
        }

        fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
            out.pixels.fill(Color32::from_rgb(0xff, 0xff, 0xff));
        }
    }

    /// The document's corners, clockwise from the top left.
    fn document() -> [Point; 4] {
        let (w, h) = (f64::from(IMAGE.width), f64::from(IMAGE.height));
        [Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)]
    }

    /// The meshes a settled pass paints with the overlay `make` gives over `Flat`.
    fn meshes(make: impl Fn() -> RasterOverlay<'static>) -> Vec<Mesh> {
        let mut view = FullWindowView::new();
        let mut frames = Frames::new();
        let mut shapes = Vec::new();
        for _ in 0..2 {
            shapes = frames.frame_shapes(|ui| {
                FullWindowCanvas::new(Bind::bound(&mut view), &Flat).overlay(make()).show(ui);
            });
        }
        shapes
            .into_iter()
            .filter_map(|shape| match shape {
                Shape::Mesh(mesh) => Some(std::sync::Arc::unwrap_or_clone(mesh)),
                _ => None,
            })
            .collect()
    }

    /// The corners of the flat overlay's quad: its four vertices, clockwise from the top left.
    fn quad_corners(mesh: &Mesh) -> [Pos2; 4] {
        [0, 1, 2, 3].map(|i| mesh.vertices[i].pos)
    }

    /// The corners of a mapped mesh of `cells` squared cells (vertices in rows), clockwise from the top left.
    fn corners(mesh: &Mesh, cells: u32) -> [Pos2; 4] {
        let row = (cells + 1) as usize;
        let v = |i: usize| mesh.vertices[i].pos;
        [v(0), v(cells as usize), v(row * row - 1), v(row * cells as usize)]
    }

    #[trial]
    fn a_mapped_overlay_lands_its_corners_on_the_frames_corners_as_the_flat_one_does() {
        let frame = Homography::unit_to_quad(document()).expect("a box");
        let flat = meshes(|| RasterOverlay::new(&Flat));
        let mapped = meshes(|| RasterOverlay::new(&Flat).mapped(frame, CELLS));
        let flat = flat.last().expect("the flat overlay's quad");
        let mapped = mapped.iter().find(|mesh| mesh.vertices.len() == ((CELLS + 1) * (CELLS + 1)) as usize).expect("the mapped mesh");
        for (a, b) in quad_corners(flat).iter().zip(corners(mapped, CELLS)) {
            assert!(a.distance(b) < NEAR, "the mesh corner {b:?} is the flat quad's corner {a:?}");
        }
        let half = meshes(|| RasterOverlay::new(&Flat).mapped(frame, CELLS).placed(Rect::new(0.0, 0.0, 0.5, 0.5)));
        let half = half.iter().find(|mesh| mesh.vertices.len() == ((CELLS + 1) * (CELLS + 1)) as usize).expect("the placed mesh");
        let (whole, part) = (corners(mapped, CELLS), corners(half, CELLS));
        assert!(part[0].distance(whole[0]) < NEAR && (part[2] - whole[0]).length() < (whole[2] - whole[0]).length() * 0.51);
    }
}

/// The title row under the macOS traffic lights.
pub(crate) mod title {
    use craft_host::target::WindowChrome;
    use craft_ui::contract::ShowContainer as _;
    use craft_ui::dialog::FullWindowDialog;
    use craft_ui::testing::{Fixture, trial};

    const TITLE: &str = "Liquify (Photo, 66%)";

    /// Where the title's text starts, with the window drawn as `chrome`.
    fn title_left(chrome: WindowChrome) -> f32 {
        let mut fixture = Fixture::new(|ui| {
            let _ = FullWindowDialog::new(TITLE).show(ui, |_, _| {});
        });
        fixture.chrome(chrome).settle();
        fixture.texts().iter().find(|text| text.text == TITLE).expect("the title is painted").rect.left()
    }

    // The dialog covers the window's top, where an inset frame (macOS) has the traffic lights over the content.
    #[trial]
    fn a_full_window_dialog_title_starts_after_the_traffic_lights() {
        let native = title_left(WindowChrome::Native);
        let inset = title_left(WindowChrome::Inset);
        let lights = craft_window::policy::MacInset::default().leading;
        assert!(native < lights, "with a native frame the title starts at the dialog's own inset");
        assert!(inset >= lights, "with the traffic lights over the content the title starts after them ({inset} < {lights})");
    }
}
