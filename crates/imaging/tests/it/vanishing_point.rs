//! Tests of the Vanishing Point workspace: the plane editor's tools and parts, the history and the dialog, over a fake
//! scene and a solid layer.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::Point;
use craft_behavior::{EditEvent, PixelSize};
use craft_imaging::ui::vanishing_point::{
    PlaneCheck, PlaneEdge, StampLimits, TornPlane, VanishingPlane, VanishingPointEvent, VanishingPointOpen, VanishingPointSession,
    VanishingPointSource, VanishingPointWorkspace, VanishingPreviewRequest, VanishingScene, VanishingTool, slide_edge,
};
use craft_imaging::ui::wide_angle::PreviewDone;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::contract::Job;
use craft_ui::primitives::Revision;
use craft_ui::runtime::access::Role;
use craft_ui::sections::DialogRole;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Color32, ColorImage, Key, Modifiers, Pos2, Vec2};

/// The layer's size in document pixels.
const LAYER: [u32; 2] = [800, 600];

/// The engine's clone limits, as the app gives them.
const LIMITS: StampLimits = StampLimits::new(4096, 200_000);

/// Half the width and height of the plane the tests draw on screen, in points.
const HALF: Vec2 = Vec2::new(100.0, 50.0);

/// How close two document points count as the same.
const NEAR: f64 = 1e-6;

/// How far an angle may be off, in degrees.
const DEGREES: f64 = 1.5;

/// A scene whose perspective is flat: torn planes are rectangles on the parent's edge, turned by the angle. It has the
/// contract of the app's scene, not its maths. `verdicts` gives each plane's check; planes past them are valid.
struct Flat {
    verdicts: Vec<PlaneCheck>,
}

fn turns(q: &[[f64; 2]; 4]) -> f64 {
    let e = |i: usize| [q[(i + 1) % 4][0] - q[i][0], q[(i + 1) % 4][1] - q[i][1]];
    (0..4).map(|i| e(i)[0] * e((i + 1) % 4)[1] - e(i)[1] * e((i + 1) % 4)[0]).sum()
}

impl VanishingScene for Flat {
    fn normalized(&self, plane: VanishingPlane) -> VanishingPlane {
        let mut q = plane.corners;
        if turns(&q) < 0.0 {
            q = [q[0], q[3], q[2], q[1]];
        }
        let first = (0..4).min_by(|&a, &b| (q[a][0] + q[a][1]).total_cmp(&(q[b][0] + q[b][1]))).unwrap_or(0);
        VanishingPlane::new([q[first], q[(first + 1) % 4], q[(first + 2) % 4], q[(first + 3) % 4]])
    }

    fn resolve(&self, planes: &[VanishingPlane], torn: &[Option<TornPlane>]) -> Vec<VanishingPlane> {
        let mut out: Vec<VanishingPlane> = Vec::new();
        for (i, plane) in planes.iter().enumerate() {
            let Some(t) = torn.get(i).copied().flatten().filter(|t| t.from < i) else {
                out.push(*plane);
                continue;
            };
            let c = out[t.from].corners;
            let e = PlaneEdge::ALL.iter().position(|x| *x == t.edge).unwrap_or(0);
            let (a, b, inward) = (c[e], c[(e + 1) % 4], c[[3, 0, 1, 2][e]]);
            let v = [inward[0] - a[0], inward[1] - a[1]];
            let len = v[0].hypot(v[1]);
            let turn = (t.angle - 90.0).to_radians();
            let o = [-v[0] / len, -v[1] / len];
            let o = [o[0] * turn.cos() - o[1] * turn.sin(), o[0] * turn.sin() + o[1] * turn.cos()];
            let far = |p: [f64; 2]| [p[0] + o[0] * len * t.depth, p[1] + o[1] * len * t.depth];
            let mut corners = [[0.0; 2]; 4];
            corners[e] = far(a);
            corners[(e + 1) % 4] = far(b);
            corners[(e + 2) % 4] = b;
            corners[(e + 3) % 4] = a;
            out.push(VanishingPlane::new(corners));
        }
        out
    }

    fn check(&self, _: &[VanishingPlane], index: usize) -> PlaneCheck {
        self.verdicts.get(index).copied().unwrap_or(PlaneCheck::Valid)
    }

    fn plane_at(&self, planes: &[VanishingPlane], at: [f64; 2]) -> Option<usize> {
        let inside = |q: &[[f64; 2]; 4]| {
            let side = |i: usize| {
                let (a, b) = (q[i], q[(i + 1) % 4]);
                (b[0] - a[0]) * (at[1] - a[1]) - (b[1] - a[1]) * (at[0] - a[0])
            };
            (0..4).all(|i| side(i) >= 0.0) || (0..4).all(|i| side(i) <= 0.0)
        };
        planes.iter().rposition(|p| inside(&p.corners))
    }

    fn frame(&self, planes: &[VanishingPlane], index: usize) -> Option<(Homography, f64, f64)> {
        let p = planes.get(index)?;
        let len = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
        Some((p.homography()?, len(p.corners[0], p.corners[1]), len(p.corners[0], p.corners[3])))
    }

    fn stamp_ring(&self, _: &[VanishingPlane], _: [f64; 2], _: f64) -> Option<Vec<Point>> {
        None
    }

    fn clone_source(&self, planes: &[VanishingPlane], source: [f64; 2], anchor: [f64; 2], at: [f64; 2]) -> Option<[f64; 2]> {
        self.plane_at(planes, source)?;
        Some([source[0] + at[0] - anchor[0], source[1] + at[1] - anchor[1]])
    }
}

/// A solid layer.
struct Layer;

impl RasterSource for Layer {
    fn size(&self) -> PixelSize {
        PixelSize::new(LAYER[0], LAYER[1])
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
        out.pixels.fill(Color32::from_gray(0x80));
    }
}

impl VanishingPointSource for Layer {
    fn image(&self) -> &dyn RasterSource {
        self
    }

    fn stamp_preview(&self) -> Option<&dyn RasterSource> {
        None
    }

    fn paste(&self) -> Option<&dyn RasterSource> {
        None
    }

    fn preview_job(&self, _: &VanishingPreviewRequest) -> Job<PreviewDone, String> {
        Job::new(|_, _| Ok(PreviewDone::new(Duration::ZERO)))
    }
}

/// What the app keeps of the workspace and what it saw.
struct App {
    workspace: VanishingPointWorkspace,
    scene: Flat,
    events: Vec<VanishingPointEvent>,
    closed: Option<DialogRole>,
}

/// The workspace over `session` with `tool` picked and `verdicts` for the planes, settled.
fn open_with(tool: VanishingTool, session: VanishingPointSession, verdicts: Vec<PlaneCheck>, limits: StampLimits) -> (Fixture, Rc<RefCell<App>>) {
    let mut workspace = VanishingPointWorkspace::new(VanishingPointOpen::new(LAYER, "Photo", limits).session(session));
    workspace.tool(tool);
    let app = Rc::new(RefCell::new(App { workspace, scene: Flat { verdicts }, events: Vec::new(), closed: None }));
    let shown = Rc::clone(&app);
    let mut fixture = Fixture::new(move |ui| {
        let mut app = shown.borrow_mut();
        let App { workspace, scene, .. } = &mut *app;
        let out = workspace.show(ui, scene, &Layer);
        app.events.extend(out.events);
        app.closed = app.closed.or(out.closed);
    });
    fixture.room(Vec2::new(1000.0, 900.0));
    fixture.settle();
    (fixture, app)
}

fn open(tool: VanishingTool) -> (Fixture, Rc<RefCell<App>>) {
    open_with(tool, VanishingPointSession::default(), Vec::new(), LIMITS)
}

/// The middle of the canvas, moved by `offset`: the tests lay their planes out in screen points, whatever the zoom.
fn at(fixture: &Fixture, offset: Vec2) -> Pos2 {
    let canvas = fixture.by_role(Role::CANVAS).into_iter().next().expect("the workspace shows a canvas");
    canvas.center() + offset
}

fn click(fixture: &mut Fixture, to: Pos2) {
    fixture.frame(vec![pointer::moved(to)]);
    fixture.frame(vec![pointer::press(to)]);
    fixture.frame(vec![pointer::release(to)]);
    fixture.frame(Vec::new());
}

/// Press at `from`, move to `to` in two steps, release.
fn drag(fixture: &mut Fixture, from: Pos2, to: Pos2) {
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::press(from)]);
    fixture.frame(vec![pointer::moved(from + (to - from) * 0.5)]);
    fixture.frame(vec![pointer::moved(to)]);
    fixture.frame(vec![pointer::release(to)]);
    fixture.frame(Vec::new());
}

/// The plane's corners as screen offsets from the middle of the canvas, clockwise from the top left.
fn corners() -> [Vec2; 4] {
    [Vec2::new(-HALF.x, -HALF.y), Vec2::new(HALF.x, -HALF.y), Vec2::new(HALF.x, HALF.y), Vec2::new(-HALF.x, HALF.y)]
}

/// Click the four corners of the tests' plane.
fn draw_plane(fixture: &mut Fixture) {
    for offset in corners() {
        let to = at(fixture, offset);
        click(fixture, to);
    }
}

fn session(app: &Rc<RefCell<App>>) -> VanishingPointSession {
    app.borrow().workspace.session().clone()
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) < NEAR
}

/// Which kind of change an event reports.
fn phase(event: &VanishingPointEvent) -> &'static str {
    fn of<T>(kind: &'static str, event: &EditEvent<T>) -> &'static str {
        match (kind, event) {
            ("planes", EditEvent::Begin { .. }) => "planes begin",
            ("planes", EditEvent::Update { .. }) => "planes update",
            ("planes", EditEvent::Commit { .. }) => "planes commit",
            ("planes", EditEvent::Cancel { .. }) => "planes cancel",
            ("planes", EditEvent::Set { .. }) => "planes set",
            ("stamp", EditEvent::Begin { .. }) => "stamp begin",
            ("stamp", EditEvent::Commit { .. }) => "stamp commit",
            _ => "other",
        }
    }
    match event {
        VanishingPointEvent::Planes(e) => of("planes", e),
        VanishingPointEvent::Paste(e) => of("paste", e),
        VanishingPointEvent::Stamp(e) => of("stamp", e),
    }
}

fn phases(app: &Rc<RefCell<App>>) -> Vec<&'static str> {
    app.borrow().events.iter().map(phase).collect()
}

#[trial]
fn an_edge_node_slides_along_the_planes_perspective() {
    let (mut fixture, app) = open(VanishingTool::CreatePlane);
    draw_plane(&mut fixture);
    let before = session(&app).planes[0];
    let node = at(&fixture, Vec2::new(0.0, -HALF.y));
    drag(&mut fixture, node, node - Vec2::new(0.0, HALF.y * 0.8));
    let c = session(&app).planes[0].corners;
    assert!(near(c[2], before.corners[2]) && near(c[3], before.corners[3]), "the bottom edge stays: {c:?}");
    let off = |p: [f64; 2], a: [f64; 2], b: [f64; 2]| ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])).abs();
    assert!(off(c[0], before.corners[0], before.corners[3]) < 1e-6 && off(c[1], before.corners[1], before.corners[2]) < 1e-6, "{c:?}");
    assert!(c[0][1] < before.corners[0][1], "the edge moved up: {c:?}");
    let seen = phases(&app);
    assert_eq!((seen.get(1).copied(), seen.last().copied()), (Some("planes begin"), Some("planes commit")), "{seen:?}");
    let at_press = before.homography().expect("a valid plane");
    let node = before.edge_nodes()[0];
    let same = slide_edge(&Flat { verdicts: Vec::new() }, &at_press, 0, node).expect("the node's own place");
    assert!(same.corners.iter().zip(before.corners).all(|(a, b)| near(*a, b)), "{same:?}");
}

#[trial]
fn a_command_drag_tears_off_a_plane_and_the_far_edge_sets_depth_or_angle() {
    let (mut fixture, app) = open(VanishingTool::CreatePlane);
    draw_plane(&mut fixture);
    let bottom = at(&fixture, Vec2::new(0.0, HALF.y));
    fixture.hold(Modifiers::COMMAND);
    drag(&mut fixture, bottom, bottom + Vec2::new(0.0, HALF.y * 2.0));
    fixture.hold(Modifiers::NONE);
    let kept = session(&app);
    let Some(t) = kept.torn.get(1).copied().flatten() else { panic!("plane 1 was torn off: {:?}", kept.torn) };
    assert_eq!((t.from, t.edge, t.angle), (0, PlaneEdge::Bottom, 90.0));
    assert!((t.depth - 1.0).abs() < 0.05, "depth {}", t.depth);
    let tool = app.borrow().workspace.editor().tool();
    assert_eq!((app.borrow().workspace.editor().selected(), tool), (Some(1), VanishingTool::EditPlane));
    let seen = phases(&app);
    assert_eq!((seen.get(1).copied(), seen.last().copied()), (Some("planes begin"), Some("planes commit")), "{seen:?}");

    // Alt on the far edge node turns the plane about the edge instead.
    let far = bottom + Vec2::new(0.0, HALF.y * 2.0);
    let turned = bottom + Vec2::new(-HALF.y * 2.0 * std::f32::consts::FRAC_1_SQRT_2, HALF.y * 2.0 * std::f32::consts::FRAC_1_SQRT_2);
    fixture.hold(Modifiers::ALT);
    drag(&mut fixture, far, turned);
    fixture.hold(Modifiers::NONE);
    let Some(t) = session(&app).torn[1] else { panic!("still torn") };
    assert!((t.angle - 135.0).abs() <= DEGREES, "angle {}", t.angle);
}

#[trial]
fn delete_removes_the_plane_and_escape_drops_a_half_plane_first() {
    let drawn_planes = [
        VanishingPlane::new([[100.0, 50.0], [300.0, 50.0], [300.0, 100.0], [100.0, 100.0]]),
        VanishingPlane::new([[0.0; 2]; 4]),
        VanishingPlane::new([[0.0; 2]; 4]),
    ];
    let torn = vec![None, Some(TornPlane::new(0, PlaneEdge::Bottom, 90.0, 1.0)), Some(TornPlane::new(1, PlaneEdge::Bottom, 90.0, 1.0))];
    let planes = Flat { verdicts: Vec::new() }.resolve(&drawn_planes, &torn);
    let mut kept = VanishingPointSession::default();
    kept.planes.clone_from(&planes);
    kept.torn.clone_from(&torn);
    let (mut fixture, app) = open_with(VanishingTool::EditPlane, kept, Vec::new(), LIMITS);
    // Select the first plane by clicking inside it: the layer shows at zoom 1, centred on the canvas.
    let layer_middle = Vec2::new(LAYER[0] as f32, LAYER[1] as f32) / 2.0;
    let inside_first_plane = at(&fixture, Vec2::new(200.0, 75.0) - layer_middle);
    click(&mut fixture, inside_first_plane);
    assert_eq!(app.borrow().workspace.editor().selected(), Some(0));
    fixture.key(Key::Delete, Modifiers::NONE).settle();
    let after = session(&app);
    assert_eq!(after.planes, planes[1..], "the children keep their place");
    assert_eq!(
        after.torn,
        [None, torn[2].map(|child| TornPlane::new(0, child.edge, child.angle, child.depth))],
        "the child is drawn, its child follows it"
    );
    assert_eq!(app.borrow().workspace.editor().selected(), None);

    // Escape: the half plane first, then the dialog's Cancel.
    let (mut fixture, app) = open(VanishingTool::CreatePlane);
    for offset in &corners()[..2] {
        let to = at(&fixture, *offset);
        click(&mut fixture, to);
    }
    assert_eq!(app.borrow().workspace.editor().pending().len(), 2);
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(app.borrow().workspace.editor().pending().is_empty() && app.borrow().closed.is_none(), "Escape drops the corners and keeps the dialog");
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert_eq!(app.borrow().closed, Some(DialogRole::Cancel), "with nothing pending Escape cancels");
}
