//! Stories of the Adaptive Wide Angle workspace, its constraint lines and its panel.
//!
//! The source is a grey picture and the geometry bows every line outward from the picture's centre, so the curves
//! show; the corrected preview is a lighter picture whose mapping is the identity.

use std::time::Duration;

use craft_behavior::PixelSize;
use craft_behavior::kurbo::Point;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::dialog::{FullWindowCanvas, FullWindowView};
use craft_ui::primitives::Revision;
use craft_ui::runtime::pending::Job;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Color32, ColorImage, Ui};

use super::{
    ConstraintLines, ConstraintOrientation, PreviewDone, WideAngleConstraint, WideAngleGeometry, WideAngleOpen, WideAnglePanel, WideAngleSession,
    WideAngleSource, WideAngleWorkspace,
};

/// The picture's size in pixels.
const IMAGE: PixelSize = PixelSize::new(480, 320);

/// How far a curve bows at its middle, as a share of the line's length.
const BOW: f64 = 0.06;

/// A flat picture.
struct Flat(Color32, u64);

impl RasterSource for Flat {
    fn size(&self) -> PixelSize {
        IMAGE
    }

    fn revision(&self) -> Revision {
        Revision::new(self.1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        Some(PixelRect::default())
    }

    fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
        out.pixels.fill(self.0);
    }
}

/// Lines bowed away from the centre; the corrected preview maps every point to itself.
struct Bowed;

impl WideAngleGeometry for Bowed {
    fn arc(&self, _: &WideAngleSession, a: [f64; 2], b: [f64; 2], segments: usize) -> Vec<Point> {
        let centre = [f64::from(IMAGE.width) / 2.0, f64::from(IMAGE.height) / 2.0];
        let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let middle = [(a[0] + b[0]) / 2.0 - centre[0], (a[1] + b[1]) / 2.0 - centre[1]];
        let norm = (middle[0].powi(2) + middle[1].powi(2)).sqrt().max(1.0);
        (0..=segments)
            .map(|i| {
                let t = i as f64 / segments as f64;
                let bow = 4.0 * t * (1.0 - t) * BOW * length;
                Point::new(a[0] + (b[0] - a[0]) * t + middle[0] / norm * bow, a[1] + (b[1] - a[1]) * t + middle[1] / norm * bow)
            })
            .collect()
    }

    fn corrected(&self, _: &WideAngleSession, at: Point) -> Option<Point> {
        Some(at)
    }
}

/// The two pictures.
struct Pictures {
    source: Flat,
    corrected: Flat,
}

impl WideAngleSource for Pictures {
    fn source(&self) -> &dyn RasterSource {
        &self.source
    }

    fn corrected(&self) -> Option<&dyn RasterSource> {
        Some(&self.corrected)
    }

    fn preview_job(&self, _: &WideAngleSession) -> Job<PreviewDone, String> {
        Job::new(|_, _| Ok(PreviewDone::new(Duration::from_millis(12))))
    }
}

fn pictures() -> Pictures {
    Pictures { source: Flat(Color32::from_gray(0x70), 1), corrected: Flat(Color32::from_gray(0x90), 2) }
}

/// One constraint of each orientation, near the picture's edges where the bow shows.
fn constraints() -> Vec<WideAngleConstraint> {
    let line = WideAngleConstraint::new;
    vec![
        line([60.0, 50.0], [420.0, 50.0], ConstraintOrientation::Horizontal),
        line([50.0, 70.0], [50.0, 280.0], ConstraintOrientation::Vertical),
        line([130.0, 270.0], [400.0, 230.0], ConstraintOrientation::Free),
    ]
}

fn session() -> WideAngleSession {
    WideAngleSession::default().with_constraints(constraints())
}

#[story(WideAngleWorkspace, covers = [Default], drive = Drive::Never)]
fn workspace_source(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut workspace = WideAngleWorkspace::new(WideAngleOpen::new([IMAGE.width, IMAGE.height], "Layer 1").session(session()));
    workspace.preview(false);
    workspace.show(ui, &Bowed, &pictures());
    StoryOutput::default()
}

#[story(WideAngleWorkspace, covers = [], drive = Drive::Never)]
fn workspace_corrected(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut workspace = WideAngleWorkspace::new(WideAngleOpen::new([IMAGE.width, IMAGE.height], "Layer 1").session(session()));
    workspace.show(ui, &Bowed, &pictures());
    StoryOutput::default()
}

#[story(ConstraintLines<'static>, covers = [Default], drive = Drive::Never)]
fn lines(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let session = session();
    let pictures = pictures();
    let view = data.value("view", FullWindowView::new());
    let mut seen = None;
    let canvas = FullWindowCanvas::new(Bind::bound(view), pictures.source()).controls(|ui, rect| {
        seen = Some(
            ConstraintLines::new("Constraint lines", &session, &Bowed)
                .picture([IMAGE.width, IMAGE.height], [IMAGE.width, IMAGE.height])
                .show_at(ui, rect),
        );
    });
    canvas.show(ui);
    seen.map(StoryOutput::from).unwrap_or_default()
}

#[story(WideAnglePanel<'static>, covers = [Default], drive = Drive::Never)]
fn panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let session = session();
    let preview = data.value("preview", true);
    WideAnglePanel::new(&session, Bind::bound(preview)).show(ui).into()
}
