//! Tests of the Vanishing Point workspace: the plane editor's tools and parts, the history and the dialog, over a fake
//! scene and a solid layer.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::Point;
use craft_behavior::{EditEvent, PixelSize};
use craft_imaging::ui::vanishing_point::{
    PasteImage, PlaneCheck, PlaneEdge, StampLimits, TornPlane, VanishingPlane, VanishingPointCommand, VanishingPointEvent, VanishingPointOpen,
    VanishingPointSession, VanishingPointSource, VanishingPointWorkspace, VanishingPreviewRequest, VanishingScene, VanishingTool, slide_edge,
};
use craft_imaging::ui::wide_angle::PreviewDone;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::contract::Job;
use craft_ui::dialog::{DialogHost, DialogResult, FullWindowWorkspace, HostedWorkspace};
use craft_ui::primitives::Revision;
use craft_ui::runtime::access::Role;
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

/// A solid layer with the scene its planes live in; with `clipboard` it is also the clipboard image ⌘V pastes.
struct Layer {
    scene: Flat,
    clipboard: bool,
}

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
    fn scene(&self) -> &dyn VanishingScene {
        &self.scene
    }

    fn image(&self) -> &dyn RasterSource {
        self
    }

    fn stamp_preview(&self) -> Option<&dyn RasterSource> {
        None
    }

    fn paste(&self) -> Option<&dyn RasterSource> {
        self.clipboard.then_some(self as &dyn RasterSource)
    }

    fn preview_job(&self, _: &VanishingPreviewRequest) -> Job<PreviewDone, String> {
        Job::new(|_, _| Ok(PreviewDone::new(Duration::ZERO)))
    }
}

type Hosted = HostedWorkspace<VanishingPointWorkspace>;

/// What the app keeps of the workspace and what it saw.
struct App {
    host: DialogHost,
    events: Vec<VanishingPointEvent>,
    closed: Option<DialogResult<VanishingPointCommand>>,
    /// Every frame runs its pass twice.
    rerun: bool,
}

/// Runs `f` on the workspace the host holds.
fn with_workspace<T>(app: &Rc<RefCell<App>>, f: impl FnOnce(&VanishingPointWorkspace) -> T) -> T {
    let app = app.borrow();
    f(app.host.get::<Hosted>().expect("the workspace is open").workspace())
}

/// The workspace over `session` with `tool` picked and `verdicts` for the planes, settled.
fn open_with(tool: VanishingTool, session: VanishingPointSession, verdicts: Vec<PlaneCheck>, limits: StampLimits) -> (Fixture, Rc<RefCell<App>>) {
    open_on(tool, session, Layer { scene: Flat { verdicts }, clipboard: false }, limits)
}

/// The workspace over `session` on `layer` with `tool` picked, settled.
fn open_on(tool: VanishingTool, session: VanishingPointSession, layer: Layer, limits: StampLimits) -> (Fixture, Rc<RefCell<App>>) {
    let mut workspace = VanishingPointWorkspace::new(VanishingPointOpen::new(LAYER, "Photo", limits).session(session));
    workspace.tool(tool);
    let mut host = DialogHost::new();
    host.open(HostedWorkspace::new(workspace, Arc::new(layer)));
    let app = Rc::new(RefCell::new(App { host, events: Vec::new(), closed: None, rerun: false }));
    let shown = Rc::clone(&app);
    let mut fixture = Fixture::new(move |ui| {
        let mut app = shown.borrow_mut();
        if app.rerun && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the trial runs a second pass");
        }
        app.host.run(ui);
        let taken = app.host.get_mut::<Hosted>().map(HostedWorkspace::take_events).unwrap_or_default();
        app.events.extend(taken);
        if let Some(result) = app.host.take::<Hosted>() {
            app.closed = Some(result);
        }
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
    with_workspace(app, |workspace| workspace.session().clone())
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
        _ => "other",
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
    let (selected, tool) = with_workspace(&app, |workspace| (workspace.editor().selected(), workspace.editor().tool()));
    assert_eq!((selected, tool), (Some(1), VanishingTool::EditPlane));
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
        VanishingPlane::new([[300.0, 250.0], [500.0, 250.0], [500.0, 350.0], [300.0, 350.0]]),
        VanishingPlane::new([[0.0; 2]; 4]),
        VanishingPlane::new([[0.0; 2]; 4]),
    ];
    let torn = vec![None, Some(TornPlane::new(0, PlaneEdge::Bottom, 90.0, 1.0)), Some(TornPlane::new(1, PlaneEdge::Bottom, 90.0, 1.0))];
    let planes = Flat { verdicts: Vec::new() }.resolve(&drawn_planes, &torn);
    let mut kept = VanishingPointSession::default();
    kept.planes.clone_from(&planes);
    kept.torn.clone_from(&torn);
    let (mut fixture, app) = open_with(VanishingTool::EditPlane, kept, Vec::new(), LIMITS);
    // Select the first plane by clicking inside it: it holds the layer's middle, which the fitted view shows at the
    // canvas's middle whatever zoom fits the canvas.
    let inside_first_plane = at(&fixture, Vec2::ZERO);
    click(&mut fixture, inside_first_plane);
    assert_eq!(with_workspace(&app, |workspace| workspace.editor().selected()), Some(0));
    fixture.key(Key::Delete, Modifiers::NONE).settle();
    let after = session(&app);
    assert_eq!(after.planes, planes[1..], "the children keep their place");
    assert_eq!(
        after.torn,
        [None, torn[2].map(|child| TornPlane::new(0, child.edge, child.angle, child.depth))],
        "the child is drawn, its child follows it"
    );
    assert_eq!(with_workspace(&app, |workspace| workspace.editor().selected()), None);

    // Escape: the half plane first, then the dialog's Cancel.
    let (mut fixture, app) = open(VanishingTool::CreatePlane);
    for offset in &corners()[..2] {
        let to = at(&fixture, *offset);
        click(&mut fixture, to);
    }
    assert_eq!(with_workspace(&app, |workspace| workspace.editor().pending().len()), 2);
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(
        with_workspace(&app, |workspace| workspace.editor().pending().is_empty()) && app.borrow().closed.is_none(),
        "Escape drops the corners and keeps the dialog"
    );
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert_eq!(app.borrow().closed, Some(DialogResult::Cancel), "with nothing pending Escape cancels");
}

/// The first four corners of the planes the command trial writes: a drawn plane, and a plane torn off its bottom edge.
fn drawn_and_torn() -> serde_json::Value {
    serde_json::json!([
        {"corners": [[100.0, 50.0], [300.0, 50.0], [300.0, 100.0], [100.0, 100.0]]},
        {"from": 0, "edge": "bottom", "angle": 90.0, "depth": 1.0},
    ])
}

#[trial]
fn the_command_writes_and_reads_the_engines_planes_and_strokes() {
    let (mut fixture, app) = open(VanishingTool::EditPlane);
    let layer = Layer { scene: Flat { verdicts: Vec::new() }, clipboard: false };
    let ctx = fixture.ctx().clone();
    let set = |field: &str, value: serde_json::Value| {
        let mut app = app.borrow_mut();
        let workspace = app.host.get_mut::<Hosted>().expect("the workspace is open").workspace_mut();
        workspace.set_json_field(&ctx, &layer, field, value)
    };

    assert_eq!(set("planes", drawn_and_torn()), Ok(()));
    let stroke =
        serde_json::json!([{"source": [10.0, 10.0], "points": [[20.0, 20.0], [30.0, 30.0]], "size": 50.0, "hardness": 25.0, "opacity": 75.0}]);
    assert_eq!(set("clone", stroke), Ok(()));
    let pastes = serde_json::json!([{"plane": 1, "at": [0.1, 0.2], "width": 0.3, "angle": 30.0, "flipH": true}]);
    assert_eq!(set("pastes", pastes), Ok(()));
    assert!(set("pastes", serde_json::json!([{"plane": 2}])).is_err(), "a paste needs an existing plane");

    // Reading `planes` refuses a torn plane whose `from` is not below its own index, and a depth outside (0, 1000].
    let refused = [
        serde_json::json!([{"from": 0}]),
        serde_json::json!([{"corners": [[0, 0], [1, 0], [1, 1], [0, 1]]}, {"from": 1}]),
        serde_json::json!([{"corners": [[0, 0], [1, 0], [1, 1], [0, 1]]}, {"from": 0, "depth": 0.0}]),
        serde_json::json!([{"corners": [[0, 0], [1, 0], [1, 1], [0, 1]]}, {"from": 0, "depth": 1000.5}]),
    ];
    for planes in refused {
        assert!(set("planes", planes.clone()).is_err(), "{planes} is refused");
    }
    assert_eq!(with_workspace(&app, |workspace| workspace.session().planes.len()), 2, "a refused write changes nothing");

    let written = with_workspace(&app, |workspace| (workspace.command(false), workspace.command(true)));
    let without_clipboard = serde_json::to_value(&written.0).expect("the command serialises");
    assert_eq!(without_clipboard["planes"][0], serde_json::json!({"corners": [[100.0, 50.0], [300.0, 50.0], [300.0, 100.0], [100.0, 100.0]]}));
    assert_eq!(without_clipboard["planes"][1], serde_json::json!({"from": 0, "edge": "bottom", "angle": 90.0, "depth": 1.0}));
    assert_eq!(without_clipboard["clone"][0]["hardness"], 25.0, "hardness is in percent");
    assert_eq!(without_clipboard["clone"][0]["opacity"], 75.0, "opacity is in percent");
    assert!(without_clipboard.get("paste").is_none(), "without a clipboard image nothing is pasted: {without_clipboard}");
    assert!(without_clipboard.get("newLayer").is_none(), "New Layer is written only when on");
    let with_clipboard = serde_json::to_value(&written.1).expect("the command serialises");
    assert_eq!(
        with_clipboard["paste"][0],
        serde_json::json!({"plane": 1, "at": [0.1, 0.2], "width": 0.3, "angle": 30.0, "flipH": true}),
        "every paste with its turn; an unmirrored axis is left out"
    );

    // The setting reaches the app once, though the frame runs its pass twice.
    fixture.frame(Vec::new());
    assert_eq!(set("newLayer", serde_json::json!(true)), Ok(()));
    {
        let mut app = app.borrow_mut();
        app.events.clear();
        app.rerun = true;
    }
    fixture.frame(Vec::new());
    assert_eq!(fixture.passes(), 2, "the frame ran a second pass");
    let settings = app.borrow().events.iter().filter(|event| matches!(event, VanishingPointEvent::Settings(_))).count();
    assert_eq!(settings, 1, "the first pass reports the queued setting, the second pass has none");
    assert!(with_workspace(&app, |workspace| workspace.settings().new_layer));
}

#[trial]
fn command_v_pastes_again_and_again_and_each_paste_moves_and_deletes_on_its_own() {
    let (mut fixture, app) = open_on(
        VanishingTool::CreatePlane,
        VanishingPointSession::default(),
        Layer { scene: Flat { verdicts: Vec::new() }, clipboard: true },
        LIMITS,
    );
    draw_plane(&mut fixture);
    fixture.key(Key::V, Modifiers::COMMAND).settle();
    fixture.key(Key::V, Modifiers::COMMAND).settle();
    let pasted = session(&app).pastes;
    assert_eq!(pasted.len(), 2, "each ⌘V pastes once more");
    assert!(pasted.iter().all(|p| p.plane == 0 && p.angle == 0.0), "{pasted:?}");
    assert_eq!(
        with_workspace(&app, |workspace| (workspace.editor().tool(), workspace.editor().selected_paste())),
        (VanishingTool::Transform, Some(1))
    );

    // The top paste lies over the other: a drag on the canvas's middle moves it alone.
    let from = at(&fixture, Vec2::ZERO);
    drag(&mut fixture, from, from + Vec2::new(40.0, 20.0));
    let moved = session(&app).pastes;
    assert_eq!(moved[0], pasted[0], "the paste below stays");
    assert!(moved[1].at[0] > pasted[1].at[0] && moved[1].at[1] > pasted[1].at[1], "{:?} → {:?}", pasted[1], moved[1]);
    assert_eq!(moved[1].width, pasted[1].width);

    // Delete removes the selected paste, not the plane; undo brings it back.
    fixture.key(Key::Delete, Modifiers::NONE).settle();
    let after = session(&app);
    assert_eq!((after.pastes.len(), after.planes.len()), (1, 1));
    fixture.key(Key::Z, Modifiers::COMMAND).settle();
    assert_eq!(session(&app).pastes, moved, "undo restores the deleted paste");
    let command = with_workspace(&app, |workspace| workspace.command(true));
    assert_eq!(command.paste.len(), 2, "OK pastes every image");
}

/// Edit Plane moves a plane whole: a drag inside it, or an arrow while it is selected; each is one undo step.
#[trial]
fn a_drag_inside_a_plane_or_an_arrow_moves_it_whole() {
    let (mut fixture, app) = open(VanishingTool::CreatePlane);
    draw_plane(&mut fixture);
    let drawn = session(&app).planes[0].corners;
    let shift = |a: [[f64; 2]; 4], b: [[f64; 2]; 4]| -> Option<[f64; 2]> {
        let d = [b[0][0] - a[0][0], b[0][1] - a[0][1]];
        a.iter().zip(&b).all(|(p, q)| near([p[0] + d[0], p[1] + d[1]], *q)).then_some(d)
    };

    let middle = at(&fixture, Vec2::ZERO);
    drag(&mut fixture, middle, middle + Vec2::new(30.0, 20.0));
    let dragged = session(&app).planes[0].corners;
    let by = shift(drawn, dragged).expect("every corner moves the same way");
    assert!(by[0] > 0.0 && by[1] > 0.0, "the plane follows the pointer: {by:?}");

    fixture.key(Key::ArrowLeft, Modifiers::NONE).settle();
    let nudged = session(&app).planes[0].corners;
    let by = shift(dragged, nudged).expect("an arrow moves the plane whole");
    assert!(by[0] < 0.0 && by[1].abs() < NEAR, "{by:?}");

    fixture.key(Key::Z, Modifiers::COMMAND).settle();
    assert!(shift(dragged, session(&app).planes[0].corners) == Some([0.0, 0.0]), "undo takes back the arrow alone");
}

/// The arrows move the selected paste without a pointer: one step each, Shift a far step, each one undo step.
#[trial]
fn the_arrows_move_the_selected_paste() {
    let layer = Layer { scene: Flat { verdicts: Vec::new() }, clipboard: true };
    let (mut fixture, app) = open_on(VanishingTool::CreatePlane, VanishingPointSession::default(), layer, LIMITS);
    draw_plane(&mut fixture);
    fixture.key(Key::V, Modifiers::COMMAND).settle();
    let pasted = session(&app).pastes[0];

    fixture.key(Key::ArrowRight, Modifiers::NONE).settle();
    let near = session(&app).pastes[0];
    assert!(near.at[0] > pasted.at[0] && (near.at[1] - pasted.at[1]).abs() < 1e-9, "{pasted:?} → {near:?}");

    fixture.key(Key::ArrowDown, Modifiers::SHIFT).settle();
    let far = session(&app).pastes[0];
    assert!(far.at[1] - near.at[1] > 5.0 * (near.at[0] - pasted.at[0]), "Shift moves further: {near:?} → {far:?}");
    assert_eq!((far.width, far.plane), (pasted.width, pasted.plane), "only the place changes");

    fixture.key(Key::Z, Modifiers::COMMAND).settle();
    assert_eq!(session(&app).pastes[0], near, "each arrow is one undo step");
}

#[trial]
fn command_v_without_a_clipboard_image_is_refused() {
    let (mut fixture, app) = open(VanishingTool::CreatePlane);
    draw_plane(&mut fixture);
    fixture.key(Key::V, Modifiers::COMMAND).settle();
    assert!(session(&app).pastes.is_empty());
    assert!(app.borrow().events.iter().any(|event| matches!(event, VanishingPointEvent::Refused(_))), "the status bar says why");
}

/// The desktop shells hand ⌘V to egui as a paste event, not a key press: it pastes all the same.
#[trial]
fn the_paste_event_of_command_v_pastes() {
    let (mut fixture, app) = open_on(
        VanishingTool::CreatePlane,
        VanishingPointSession::default(),
        Layer { scene: Flat { verdicts: Vec::new() }, clipboard: true },
        LIMITS,
    );
    draw_plane(&mut fixture);
    fixture.frame(vec![egui::Event::Paste(String::new())]).settle();
    assert_eq!(session(&app).pastes.len(), 1);
}

/// A paste remembers the strokes made before it: OK lays it over those and under the later ones, so the edits replay
/// in the order they were made. A paste before any stroke writes no order and lands first, as older commands did.
#[trial]
fn a_paste_lands_over_the_strokes_made_before_it() {
    let layer = Layer { scene: Flat { verdicts: Vec::new() }, clipboard: true };
    let (mut fixture, app) = open_on(VanishingTool::CreatePlane, VanishingPointSession::default(), layer, LIMITS);
    draw_plane(&mut fixture);
    fixture.key(Key::V, Modifiers::COMMAND).settle();
    let ctx = fixture.ctx().clone();
    let source = Layer { scene: Flat { verdicts: Vec::new() }, clipboard: true };
    let stroke = serde_json::json!([{"source": [1.0, 1.0], "points": [[2.0, 2.0]]}]);
    let set =
        app.borrow_mut().host.get_mut::<Hosted>().expect("the workspace is open").workspace_mut().set_json_field(&ctx, &source, "clone", stroke);
    assert_eq!(set, Ok(()));
    fixture.key(Key::V, Modifiers::COMMAND).settle();

    let command = serde_json::to_value(with_workspace(&app, |workspace| workspace.command(true))).expect("the command serialises");
    assert!(command["paste"][0].get("after").is_none(), "the first paste lands before every stroke: {command}");
    assert_eq!(command["paste"][1]["after"], 1, "the second lands over the stroke made before it: {command}");
}

fn set_field(fixture: &Fixture, app: &Rc<RefCell<App>>, field: &str, value: serde_json::Value) {
    let source = Layer { scene: Flat { verdicts: Vec::new() }, clipboard: true };
    let mut app = app.borrow_mut();
    let workspace = app.host.get_mut::<Hosted>().expect("the workspace is open").workspace_mut();
    workspace.set_json_field(fixture.ctx(), &source, field, value).expect("the field is valid");
}

/// A clipped part of a paste cannot take a click, while its visible interior remains selectable.
#[trial]
fn a_clipped_paste_does_not_take_clicks_outside_its_plane() {
    let layer = Layer { scene: Flat { verdicts: Vec::new() }, clipboard: true };
    let (mut fixture, app) = open_on(VanishingTool::CreatePlane, VanishingPointSession::default(), layer, LIMITS);
    draw_plane(&mut fixture);
    set_field(&fixture, &app, "pastes", serde_json::json!([{"plane": 0, "at": [0.8, 0.25], "width": 0.5}]));
    set_field(&fixture, &app, "tool", serde_json::json!("transform"));
    set_field(&fixture, &app, "selectPaste", serde_json::Value::Null);
    fixture.settle();
    let outside = at(&fixture, Vec2::new(140.0, 0.0));
    click(&mut fixture, outside);
    assert_eq!(with_workspace(&app, |w| w.editor().selected_paste()), None, "the clipped image cannot take the click");
    let inside = at(&fixture, Vec2::new(80.0, 0.0));
    click(&mut fixture, inside);
    assert_eq!(with_workspace(&app, |w| w.editor().selected_paste()), Some(0), "the visible image still takes the click");
}

/// The keyboard paste command cannot add a history step inside a pointer gesture's older snapshot.
#[trial]
fn command_v_during_a_paste_drag_is_refused_without_changing_history() {
    let layer = Layer { scene: Flat { verdicts: Vec::new() }, clipboard: true };
    let (mut fixture, app) = open_on(VanishingTool::CreatePlane, VanishingPointSession::default(), layer, LIMITS);
    draw_plane(&mut fixture);
    fixture.key(Key::V, Modifiers::COMMAND).settle();
    let before = session(&app).pastes;
    let from = at(&fixture, Vec2::ZERO);
    let to = from + Vec2::new(30.0, 15.0);
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::press(from)]);
    fixture.frame(vec![pointer::moved(to)]);
    assert!(with_workspace(&app, |w| w.editor().busy()), "the paste is being dragged");
    let during = session(&app).pastes;
    app.borrow_mut().events.clear();
    fixture.key(Key::V, Modifiers::COMMAND).settle();
    assert_eq!(session(&app).pastes, during, "refusing the paste preserves the active drag");
    assert!(app.borrow().events.iter().any(|e| matches!(e, VanishingPointEvent::Refused(_))), "a status explains the refusal");
    fixture.frame(vec![pointer::release(to)]).settle();
    fixture.key(Key::Z, Modifiers::COMMAND).settle();
    assert_eq!(session(&app).pastes, before, "one undo takes back only the drag");
    fixture.key(Key::Z, Modifiers::COMMAND).settle();
    assert!(session(&app).pastes.is_empty(), "the next undo takes back the original paste");
}

/// A fitted image can be narrower than the ordinary scale minimum; clicking its corner must preserve it.
#[trial]
fn a_stationary_scale_gesture_preserves_a_fitted_paste_width() {
    let layer = Layer { scene: Flat { verdicts: Vec::new() }, clipboard: true };
    let mut kept = VanishingPointSession::default();
    kept.planes.push(VanishingPlane::new([[300.0, 200.0], [500.0, 200.0], [500.0, 400.0], [300.0, 400.0]]));
    kept.pastes.push(PasteImage::new([0.5, 0.5], 0.004));
    let (mut fixture, app) = open_on(VanishingTool::Transform, kept, layer, LIMITS);
    set_field(&fixture, &app, "selectPaste", serde_json::json!(0));
    fixture.settle();
    let before = session(&app).pastes[0];
    let corner = at(&fixture, Vec2::ZERO);
    click(&mut fixture, corner);
    let after = session(&app).pastes[0];
    assert!((after.width - before.width).abs() < 1e-9, "a click alone must not resize the paste: {before:?} → {after:?}");
    assert!(near(after.at, before.at), "a click alone must not move the paste");
}

/// Replacing planes removes only orphaned pastes, and undo restores the planes and pastes together.
#[trial]
fn replacing_planes_removes_orphaned_pastes_in_the_same_undo_step() {
    let (mut fixture, app) = open(VanishingTool::EditPlane);
    let planes = drawn_and_torn();
    set_field(&fixture, &app, "planes", planes.clone());
    set_field(&fixture, &app, "pastes", serde_json::json!([{"plane": 0}, {"plane": 1}]));
    fixture.settle();
    let before = session(&app);
    set_field(&fixture, &app, "planes", serde_json::json!([planes[0].clone()]));
    fixture.settle();
    assert_eq!(session(&app).pastes, before.pastes[..1], "the surviving plane keeps its paste");
    fixture.key(Key::Z, Modifiers::COMMAND).settle();
    let restored = session(&app);
    assert_eq!(restored.planes, before.planes);
    assert_eq!(restored.pastes, before.pastes, "one undo restores both planes and their pastes");
    set_field(&fixture, &app, "planes", serde_json::json!([]));
    fixture.settle();
    assert!(session(&app).pastes.is_empty(), "clearing all planes removes every paste");
    set_field(&fixture, &app, "planes", planes);
    fixture.settle();
    assert!(with_workspace(&app, |w| w.command(true).paste.is_empty()), "new planes cannot resurrect old pastes");
}
