//! Stories of the Vanishing Point workspace, its planes control and its options bar.
//!
//! The workspace stories run it over `MockImage` with a scene of flat planes: the planes (a valid, a warning and an
//! invalid plane, one selected, a torn wall), Create Plane with pending corners, Stamp with its source cross over a
//! stroke, and a paste on the selected plane.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use egui::Ui;

use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::Point;
use craft_ui::canvas::RasterSource;
use craft_ui::contract::{Job, Show as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};

use super::{
    PlaneCheck, PlaneEdge, StampLimits, StampStroke, TornPlane, VanishingPlane, VanishingPlanes, VanishingPointBar, VanishingPointEditor,
    VanishingPointOpen, VanishingPointSession, VanishingPointSettings, VanishingPointSource, VanishingPointWorkspace, VanishingPreviewRequest,
    VanishingScene, VanishingTool,
};
use crate::ui::mock_image::{MockImage, StoryView, story_canvas};
use crate::ui::wide_angle::PreviewDone;

/// The clone limits the stories use.
const LIMITS: StampLimits = StampLimits::new(4096, 200_000);

/// The ring of the stamp brush on a plane: the circle of the diameter, squashed as a floor would squash it.
const RING_SQUASH: f64 = 0.45;

/// The number of points of the ring.
const RING_POINTS: usize = 48;

/// The scene's verdict on each story plane, in order.
const VERDICTS: [PlaneCheck; 4] = [PlaneCheck::Valid, PlaneCheck::Questionable, PlaneCheck::Invalid, PlaneCheck::Valid];

/// A scene of flat planes: a torn plane is a rectangle on its parent's edge, turned by its angle.
struct StoryScene;

impl VanishingScene for StoryScene {
    fn normalized(&self, plane: VanishingPlane) -> VanishingPlane {
        plane
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
        VERDICTS.get(index).copied().unwrap_or(PlaneCheck::Valid)
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

    fn stamp_ring(&self, _: &[VanishingPlane], at: [f64; 2], diameter: f64) -> Option<Vec<Point>> {
        let r = diameter / 2.0;
        Some(
            (0..RING_POINTS)
                .map(|k| {
                    let angle = std::f64::consts::TAU * k as f64 / RING_POINTS as f64;
                    Point::new(at[0] + r * angle.cos(), at[1] + r * RING_SQUASH * angle.sin())
                })
                .collect(),
        )
    }

    fn clone_source(&self, planes: &[VanishingPlane], source: [f64; 2], anchor: [f64; 2], at: [f64; 2]) -> Option<[f64; 2]> {
        self.plane_at(planes, source)?;
        Some([source[0] + at[0] - anchor[0], source[1] + at[1] - anchor[1]])
    }
}

/// The app's side over `MockImage`: the picture, and a smaller picture as the clipboard.
struct StorySource {
    image: MockImage,
    clipboard: Option<MockImage>,
}

impl VanishingPointSource for StorySource {
    fn image(&self) -> &dyn RasterSource {
        &self.image
    }

    fn stamp_preview(&self) -> Option<&dyn RasterSource> {
        None
    }

    fn paste(&self) -> Option<&dyn RasterSource> {
        self.clipboard.as_ref().map(|image| image as &dyn RasterSource)
    }

    fn preview_job(&self, _: &VanishingPreviewRequest) -> Job<PreviewDone, String> {
        Job::new(|_, _| Ok(PreviewDone::new(Duration::ZERO)))
    }
}

/// The story session: a wall, a floor, a plane the scene calls invalid, and a plane torn off the wall's right edge.
fn session() -> VanishingPointSession {
    let mut session = VanishingPointSession::default();
    let drawn = [
        VanishingPlane::new([[300.0, 300.0], [800.0, 300.0], [800.0, 700.0], [300.0, 700.0]]),
        VanishingPlane::new([[200.0, 760.0], [900.0, 760.0], [1000.0, 1000.0], [100.0, 1000.0]]),
        VanishingPlane::new([[1000.0, 200.0], [1100.0, 200.0], [1100.0, 230.0], [1000.0, 230.0]]),
        VanishingPlane::new([[0.0; 2]; 4]),
    ];
    session.torn = vec![None, None, None, Some(TornPlane::new(0, PlaneEdge::Right, 90.0, 0.5))];
    session.planes = StoryScene.resolve(&drawn, &session.torn);
    session
}

/// The workspace a story keeps between passes, compared by identity.
#[derive(Clone)]
struct Kept(Arc<Mutex<VanishingPointWorkspace>>);

impl PartialEq for Kept {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Shows the workspace over the mock picture; `setup` puts it in the state of the story.
fn show_workspace(ui: &mut Ui, data: &mut StoryData, clipboard: bool, setup: impl FnOnce(&mut VanishingPointWorkspace)) -> StoryOutput {
    let image = MockImage::default();
    let size = image.size();
    let kept = data
        .value("workspace", {
            let mut workspace = VanishingPointWorkspace::new(VanishingPointOpen::new([size.width, size.height], "House", LIMITS).session(session()));
            setup(&mut workspace);
            Kept(Arc::new(Mutex::new(workspace)))
        })
        .clone();
    let mut workspace = kept.0.lock().unwrap_or_else(PoisonError::into_inner);
    let clipboard = clipboard.then(|| MockImage::new([size.width / 4, size.height / 4]));
    workspace.show(ui, &StoryScene, &StorySource { image, clipboard }).into()
}

#[story(VanishingPointWorkspace, covers = [Default], drive = Drive::Never)]
fn planes(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, false, |workspace| {
        workspace.tool(VanishingTool::EditPlane).selected_plane(Some(0));
    })
}

#[story(VanishingPointWorkspace, covers = [], drive = Drive::Never)]
fn create_plane_with_pending_corners(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, false, |workspace| {
        workspace.tool(VanishingTool::CreatePlane);
        workspace.editor.pending = vec![[1200.0, 300.0], [1450.0, 320.0], [1440.0, 600.0]];
        workspace.editor.pointer = Some([1250.0, 640.0]);
    })
}

#[story(VanishingPointWorkspace, covers = [], drive = Drive::Never)]
fn stamp_with_its_source(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, false, |workspace| {
        workspace.tool(VanishingTool::Stamp);
        workspace.session.source = Some([450.0, 450.0]);
        workspace.session.strokes.push(StampStroke::new([450.0, 450.0], vec![[600.0, 500.0], [640.0, 520.0]], 40.0, 0.5, 1.0));
        workspace.editor.pointer = Some([600.0, 850.0]);
    })
}

#[story(VanishingPointWorkspace, covers = [], drive = Drive::Never)]
fn paste_on_the_selected_plane(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, true, |workspace| {
        workspace.tool(VanishingTool::EditPlane).selected_plane(Some(0));
        workspace.settings.paste = true;
    })
}

/// What the planes story keeps: the editor, the session and the settings.
#[derive(Clone)]
struct Demo(Arc<Mutex<(VanishingPointEditor, VanishingPointSession, VanishingPointSettings)>>);

impl PartialEq for Demo {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[story(VanishingPlanes<'static>, covers = [Default], drive = Drive::Never)]
fn planes_alone_on_a_canvas(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let demo = data
        .value("demo", {
            let mut editor = VanishingPointEditor::new(LIMITS);
            let session = session();
            editor.set_tool(VanishingTool::EditPlane);
            editor.select(Some(0), &session);
            Demo(Arc::new(Mutex::new((editor, session, VanishingPointSettings::default()))))
        })
        .clone();
    let mut guard = demo.0.lock().unwrap_or_else(PoisonError::into_inner);
    let (editor, session, settings) = &mut *guard;
    let image = MockImage::default();
    let source = StorySource { image, clipboard: None };
    let ctx = ui.ctx().clone();
    let pass = editor.pass(&ctx, session, settings, &StoryScene, &source);
    story_canvas(ui, Some(&image), StoryView::Fit, |ui, rect| pass.planes().show_at(ui, rect).into())
}

fn bar(ui: &mut Ui, tool: VanishingTool, clipboard: bool) -> StoryOutput {
    let settings = VanishingPointSettings::default();
    VanishingPointBar::new(tool, &settings, VanishingPointSession::default().paste, clipboard).has_strokes(clipboard).show(ui).into()
}

#[story(VanishingPointBar<'static>, covers = [Default], drive = Drive::Never)]
fn bar_for_edit_plane(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    bar(ui, VanishingTool::EditPlane, false)
}

#[story(VanishingPointBar<'static>, covers = [], drive = Drive::Never)]
fn bar_for_stamp(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    bar(ui, VanishingTool::Stamp, false)
}

#[story(VanishingPointBar<'static>, covers = [], drive = Drive::Never)]
fn bar_with_a_clipboard_image(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    bar(ui, VanishingTool::CreatePlane, true)
}
