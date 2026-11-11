//! Content-Aware Fill: what its own logic adds to the full-window parts, the tools and the controls.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Color32, ColorImage, Modifiers, Pos2, vec2};

use craft_behavior::PixelSize;
use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_behavior::kurbo::Rect;
use craft_imaging::ui::fill::{
    ContentAwareFillWorkspace, FillAreaEdit, FillEvent, FillLimit, FillLimits, FillOpen, FillOutput, FillOverlay, FillPreviewRequest, FillSession,
    FillSettings, FillSource, FillStrokeEvent, SamplingPlane, SamplingStroke,
};
use craft_imaging::ui::select_mask::EditMode;
use craft_imaging::ui::wide_angle::PreviewDone;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::primitives::Revision;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::pending::Job;
use craft_ui::testing::{Fixture, pointer, trial};

const NONE: Modifiers = Modifiers::NONE;
/// The layer the workspace opens on, in document pixels.
const LAYER: PixelSize = PixelSize::new(400, 300);
/// The room the workspace is shown in.
const ROOM: egui::Vec2 = vec2(1400.0, 900.0);

/// The level of the flat grey picture.
const GREY: u8 = 128;

/// A flat grey picture.
struct Grey;

impl RasterSource for Grey {
    fn size(&self) -> PixelSize {
        LAYER
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
        out.pixels.fill(Color32::from_gray(GREY));
    }
}

/// The app's side: a picture and no planes; its preview job fails.
struct Source;

impl FillSource for Source {
    fn image(&self) -> &dyn RasterSource {
        &Grey
    }

    fn sampling(&self) -> Option<&SamplingPlane> {
        None
    }

    fn fill_area(&self) -> Option<&SamplingPlane> {
        None
    }

    fn sampling_window(&self) -> Option<Rect> {
        None
    }

    fn preview(&self) -> Option<&dyn RasterSource> {
        None
    }

    fn preview_job(&self, _: &FillPreviewRequest) -> Job<PreviewDone, String> {
        Job::new(|_, _| Err("no engine in this test".to_owned()))
    }
}

/// The workspace shown in a fixture, with every pass's output kept.
struct Rig {
    fixture: Fixture,
    outputs: Rc<RefCell<Vec<FillOutput>>>,
}

impl Rig {
    fn new(settings: FillSettings, limits: FillLimits) -> Rig {
        let open = FillOpen::new([LAYER.width, LAYER.height], "Layer 1", limits).settings(settings);
        let work = Rc::new(RefCell::new(ContentAwareFillWorkspace::new(open)));
        let outputs = Rc::new(RefCell::new(Vec::new()));
        let (shown, kept) = (Rc::clone(&work), Rc::clone(&outputs));
        let mut fixture = Fixture::new(move |ui| {
            let output = shown.borrow_mut().show(ui, &Source);
            kept.borrow_mut().push(output);
        });
        fixture.room(ROOM).settle();
        let rig = Rig { fixture, outputs };
        rig.take();
        rig
    }

    /// The middle of the main canvas.
    fn canvas(&self) -> Pos2 {
        self.fixture.by_role(Role::CANVAS).into_iter().next().expect("the workspace shows a canvas").center()
    }

    /// The outputs since the last call.
    fn take(&self) -> Vec<FillOutput> {
        std::mem::take(&mut *self.outputs.borrow_mut())
    }

    /// The events since the last call.
    fn events(&self) -> Vec<FillEvent> {
        self.take().into_iter().flat_map(|output| output.events).collect()
    }

    /// Presses at `from`, moves through `through` and releases at the last point, with `modifiers` held.
    fn stroke(&mut self, from: Pos2, through: &[Pos2], modifiers: Modifiers) {
        self.fixture.hold(modifiers);
        self.fixture.frame(vec![pointer::moved(from)]);
        self.fixture.frame(vec![pointer::press(from)]);
        for at in through {
            self.fixture.frame(vec![pointer::moved(*at)]);
        }
        self.fixture.frame(vec![pointer::release(through.last().copied().unwrap_or(from))]);
        self.fixture.hold(NONE);
        self.fixture.idle(1);
    }

    /// A stroke along a line to the right of the canvas centre in `steps` moves.
    fn line(&mut self, steps: usize) {
        let through: Vec<Pos2> = (1..=steps).map(|i| self.canvas() + vec2(12.0 * i as f32, 0.0)).collect();
        self.stroke(self.canvas(), &through, NONE);
    }
}

/// The stroke a pass committed, if one did.
fn committed(events: &[FillEvent]) -> Option<&SamplingStroke> {
    events.iter().find_map(|event| match event {
        FillEvent::Stroke(FillStrokeEvent::Commit(stroke)) => Some(stroke),
        _ => None,
    })
}

fn stroke_of(points: &[[f64; 2]]) -> SamplingStroke {
    SamplingStroke::new(EditMode::Add, 20.0, points.to_vec())
}

#[trial]
fn fill_limits_refuse_at_the_brush() {
    let limits = FillLimits::new(2, 5);
    let mut session = FillSession::default();
    session.add_stroke(stroke_of(&[[0.0, 0.0]]));
    assert_eq!(session.may_begin_stroke(&limits), Ok(()));
    session.add_stroke(stroke_of(&[[0.0, 0.0], [1.0, 1.0]]));
    assert_eq!(session.may_begin_stroke(&limits), Err(FillLimit::Strokes(2)), "two strokes are the limit");

    // Points are counted over strokes and lasso edits; a total equal to the limit passes an edit.
    let limits = FillLimits::new(10, 5);
    let mut session = FillSession::default();
    session.add_stroke(stroke_of(&[[0.0, 0.0], [1.0, 1.0]]));
    let triangle = || {
        let Some(edit) = FillAreaEdit::lasso(SelectionOperation::Add, vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0]]) else {
            panic!("three points are a lasso")
        };
        edit
    };
    assert_eq!(session.add_fill_edit(triangle(), &limits), Ok(()));
    assert_eq!(session.points_used(None), 5);
    assert_eq!(session.may_begin_stroke(&limits), Err(FillLimit::Vertices(5)));
    assert_eq!(session.add_fill_edit(triangle(), &limits), Err(FillLimit::Vertices(5)));
    assert_eq!(session.fill_edits.len(), 1, "a refused edit records nothing");

    // The brush: no stroke begins at the stroke limit.
    let mut rig = Rig::new(FillSettings::default(), FillLimits::new(1, 100));
    rig.line(2);
    assert!(committed(&rig.events()).is_some(), "the first stroke fits");
    rig.line(2);
    let events = rig.events();
    assert!(events.contains(&FillEvent::Refused(FillLimit::Strokes(1))), "{events:?}");
    assert!(committed(&events).is_none());

    // A stroke at the points limit stops taking points; the message comes once.
    let mut rig = Rig::new(FillSettings::default(), FillLimits::new(10, 3));
    rig.line(6);
    let events = rig.events();
    let refusals = events.iter().filter(|event| matches!(event, FillEvent::Refused(FillLimit::Vertices(3)))).count();
    assert_eq!(refusals, 1, "{events:?}");
    assert_eq!(committed(&events).map(|stroke| stroke.points.len()), Some(3));
}

#[trial]
fn fill_overlay_tints_the_sampling_area_or_the_excluded_area() {
    // A 4 by 1 plane of one-pixel cells: sampled, sampled, not sampled, sampled.
    let sampling = SamplingPlane::new([0, 0], [4, 1], 1, vec![true, true, false, true]);
    // The fill area covers the second cell.
    let fill_area = SamplingPlane::new([0, 0], [4, 1], 1, vec![false, true, false, false]);
    let document = PixelSize::new(6, 1);
    let tinted = |excluded: bool| -> Vec<bool> {
        let overlay = FillOverlay::new(&sampling, Some(&fill_area), document).excluded(excluded);
        let mut out = ColorImage::filled([6, 1], Color32::from_white_alpha(0));
        overlay.read(PixelRect::of_size(document), 1, &mut out);
        out.pixels.iter().map(|pixel| pixel.a() > 0).collect()
    };
    // Sampling Area: the sampled cells the fill area does not cover; nothing outside the plane.
    assert_eq!(tinted(false), [true, false, false, true, false, false]);
    // Excluded Area: the cells inside the plane that are not sampled and not filled.
    assert_eq!(tinted(true), [false, false, true, false, false, false]);
}
