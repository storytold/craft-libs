//! Behaviour of the Adaptive Wide Angle workspace: the Constraint tool, the constraint lines, the context menu, the
//! Add Constraint form and the preview job.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use craft_behavior::PixelSize;
use craft_behavior::kurbo::Point;
use craft_host::{AppInfo, Host, ManualJobs};
use craft_imaging::ui::wide_angle::{
    ConstraintOrientation, PreviewDone, WideAngleConstraint, WideAngleGeometry, WideAngleOpen, WideAngleSession, WideAngleSource, WideAngleWorkspace,
};
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::primitives::Revision;
use craft_ui::runtime::pending::Job;
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, ColorImage, Event, Modifiers, PointerButton, Pos2, Vec2};

const IMAGE: [u32; 2] = [400, 200];

/// The test picture's colour: data, not a theme colour.
const PICTURE: Color32 = Color32::from_gray(0x80);

/// A distance along one axis, in points.
fn along(x: f32, y: f32) -> Vec2 {
    Vec2::X * x + Vec2::Y * y
}

/// A grey picture.
struct Grey(u64);

impl RasterSource for Grey {
    fn size(&self) -> PixelSize {
        PixelSize::new(IMAGE[0], IMAGE[1])
    }

    fn revision(&self) -> Revision {
        Revision::new(self.0)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        Some(PixelRect::default())
    }

    fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
        out.pixels.fill(PICTURE);
    }
}

/// Straight lines; the corrected preview moves every point down by `SHIFT`.
struct Lines;

const SHIFT: f64 = 10.0;

impl WideAngleGeometry for Lines {
    fn arc(&self, _: &WideAngleSession, a: [f64; 2], b: [f64; 2], segments: usize) -> Vec<Point> {
        (0..=segments)
            .map(|i| {
                let t = i as f64 / segments as f64;
                Point::new(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
            })
            .collect()
    }

    fn corrected(&self, _: &WideAngleSession, at: Point) -> Option<Point> {
        Some(Point::new(at.x, at.y + SHIFT))
    }
}

struct Sources {
    source: Grey,
    corrected: Grey,
}

impl WideAngleSource for Sources {
    fn source(&self) -> &dyn RasterSource {
        &self.source
    }

    fn corrected(&self) -> Option<&dyn RasterSource> {
        Some(&self.corrected)
    }

    fn preview_job(&self, _: &WideAngleSession) -> Job<PreviewDone, String> {
        Job::new(|_, _| Ok(PreviewDone::new(Duration::from_millis(42))))
    }
}

struct App {
    workspace: WideAngleWorkspace,
    sources: Sources,
    /// The preview job waits here until a trial runs it.
    jobs: Arc<ManualJobs>,
}

fn fixture(constraints: Vec<WideAngleConstraint>) -> (Fixture, Rc<RefCell<App>>) {
    let session = WideAngleSession::default().with_constraints(constraints);
    let app = Rc::new(RefCell::new(App {
        workspace: WideAngleWorkspace::new(WideAngleOpen::new(IMAGE, "Layer 1").session(session)),
        sources: Sources { source: Grey(1), corrected: Grey(2) },
        jobs: Arc::new(ManualJobs::new()),
    }));
    let shared = app.clone();
    let jobs = app.borrow().jobs.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut app = shared.borrow_mut();
        let App { workspace, sources, .. } = &mut *app;
        workspace.show(ui, &Lines, sources);
    });
    fixture.host(Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs).build()));
    fixture.room(along(900.0, 600.0));
    fixture.settle();
    (fixture, app)
}

/// Where the image pixel `at` is on screen: the fitted image is centred in the canvas.
fn screen(fixture: &Fixture, app: &Rc<RefCell<App>>, at: [f64; 2]) -> Pos2 {
    let canvas = fixture.by_label("Image").map(|target| target.center()).expect("the Image canvas");
    let zoom = (app.borrow().workspace.zoom_percent() / 100.0) as f32;
    canvas + Vec2::new((at[0] - f64::from(IMAGE[0]) / 2.0) as f32 * zoom, (at[1] - f64::from(IMAGE[1]) / 2.0) as f32 * zoom)
}

fn drag(fixture: &mut Fixture, from: Pos2, by: Vec2, modifiers: Modifiers) {
    fixture.hold(modifiers);
    fixture.frame(vec![Event::PointerMoved(from)]);
    fixture.frame(vec![Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers }]);
    fixture.frame(vec![Event::PointerMoved(from + by * 0.5)]);
    fixture.frame(vec![Event::PointerMoved(from + by)]);
    fixture.frame(vec![Event::PointerButton { pos: from + by, button: PointerButton::Primary, pressed: false, modifiers }]);
    fixture.hold(Modifiers::NONE);
    fixture.idle(2);
}

fn constraints(app: &Rc<RefCell<App>>) -> Vec<WideAngleConstraint> {
    app.borrow().workspace.session().constraints.clone()
}

#[trial]
fn a_drag_over_six_points_adds_a_constraint_and_shift_aligns_it() {
    let tie = WideAngleConstraint::from_drag(Point::new(0.0, 0.0), Point::new(5.0, 5.0), true);
    assert_eq!(tie.orientation, ConstraintOrientation::Horizontal, "a tie is Horizontal");
    assert_eq!(WideAngleConstraint::from_drag(Point::new(0.0, 0.0), Point::new(-3.0, 9.0), true).orientation, ConstraintOrientation::Vertical);

    let (mut fixture, app) = fixture(Vec::new());
    let from = screen(&fixture, &app, [200.0, 100.0]);
    drag(&mut fixture, from, along(3.0, 0.0), Modifiers::NONE);
    assert!(constraints(&app).is_empty(), "3 points is a click");

    drag(&mut fixture, from, along(60.0, 4.0), Modifiers::NONE);
    drag(&mut fixture, from, along(40.0, 10.0), Modifiers::SHIFT);
    drag(&mut fixture, from, along(10.0, 40.0), Modifiers::SHIFT);
    let orientations: Vec<_> = constraints(&app).iter().map(|c| c.orientation).collect();
    assert_eq!(orientations, [ConstraintOrientation::Free, ConstraintOrientation::Horizontal, ConstraintOrientation::Vertical]);
}
