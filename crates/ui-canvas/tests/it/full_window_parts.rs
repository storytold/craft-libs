//! The full-window parts' own logic: the view's fit and zoom rules, the wheel zoom and the mapped overlay.

use craft_behavior::PixelSize;
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
    FullWindowView::new().settled(Rect::from_min_size(Pos2::ZERO, CANVAS), IMAGE, SCALE)
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
    let kept = view.settled(rect, IMAGE, SCALE);
    assert!((kept.device_zoom() - view.device_zoom()).abs() < SAME, "a view that does not fit keeps its zoom when settled");
    assert!(view.apply(ViewCommands::FitOnScreen));
    assert!((view.settled(rect, IMAGE, SCALE).device_zoom() - fit).abs() < SAME, "settling a fitted view fits again");
}

/// The canvas's navigation: the wheel, the middle button and the Hand tool it falls back to.
mod canvas_navigation {
    use craft_behavior::PixelSize;
    use craft_behavior::kurbo::Point;
    use craft_ui::canvas::{NavigationOptions, PixelRect, RasterSource};
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
        navigation.zoom_with_wheel = true;
        navigation.animated_zoom = false;
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

/// An overlay drawn through a frame.
mod mapped_overlay {
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
