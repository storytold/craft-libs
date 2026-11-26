//! Content-Aware Fill: what its own logic adds to the full-window parts, the tools and the controls.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use egui::{Color32, ColorImage, Modifiers, Pos2, vec2};

use craft_behavior::PixelSize;
use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_behavior::kurbo::Rect;
use craft_imaging::ui::fill::{
    ContentAwareFillWorkspace, FillAreaEdit, FillEvent, FillLimit, FillLimits, FillOpen, FillOverlay, FillPreviewRequest, FillSession, FillSettings,
    FillSource, FillStrokeEvent, SamplingPlane, SamplingStroke,
};
use craft_imaging::ui::select_mask::EditMode;
use craft_imaging::ui::wide_angle::PreviewDone;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::dialog::{DialogHost, DialogResult, FullWindowWorkspace as _, HostedWorkspace};
use craft_ui::primitives::Revision;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::memory::{SaveKey, saved_value};
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

/// The hosted workspace.
type Hosted = HostedWorkspace<ContentAwareFillWorkspace>;

/// The workspace on a dialog host shown in a fixture, with every pass's events kept.
// A pass's `WorkspaceCx` is built by the host only, so this runs the workspace on a `DialogHost`.
struct Rig {
    fixture: Fixture,
    host: Rc<RefCell<DialogHost>>,
    events: Rc<RefCell<Vec<FillEvent>>>,
}

impl Rig {
    fn new(settings: FillSettings, limits: FillLimits) -> Rig {
        let open = FillOpen::new([LAYER.width, LAYER.height], "Layer 1", limits).settings(settings);
        Rig::over(ContentAwareFillWorkspace::new(open))
    }

    /// `workspace` on the host, settled.
    fn over(workspace: ContentAwareFillWorkspace) -> Rig {
        let source: Arc<dyn FillSource> = Arc::new(Source);
        let mut host = DialogHost::new();
        host.open(HostedWorkspace::new(workspace, source));
        let host = Rc::new(RefCell::new(host));
        let events = Rc::new(RefCell::new(Vec::new()));
        let (shown, kept) = (Rc::clone(&host), Rc::clone(&events));
        let mut fixture = Fixture::new(move |ui| {
            let mut host = shown.borrow_mut();
            host.run(ui);
            let taken = host.get_mut::<Hosted>().map(Hosted::take_events).unwrap_or_default();
            kept.borrow_mut().extend(taken);
        });
        fixture.room(ROOM).settle();
        let rig = Rig { fixture, host, events };
        rig.events();
        rig
    }

    /// The middle of the main canvas.
    fn canvas(&self) -> Pos2 {
        self.fixture.by_role(Role::CANVAS).into_iter().next().expect("the workspace shows a canvas").center()
    }

    /// The events since the last call.
    fn events(&self) -> Vec<FillEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    /// Runs `answer` on the workspace, as the app does between passes.
    fn with<R>(&self, answer: impl FnOnce(&mut ContentAwareFillWorkspace) -> R) -> R {
        let mut host = self.host.borrow_mut();
        answer(host.get_mut::<Hosted>().expect("the workspace is open").workspace_mut())
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

// The Apply button reports one `Apply` and the workspace stays open; the app's answer shows its message or clears the
// history; OK is not held by it. A change made between passes is reported once, however many passes the frames run.
#[trial]
fn answer_apply_answers_the_apply_button() {
    let mut rig = Rig::new(FillSettings::default(), FillLimits::new(10, 1000));
    rig.line(2);
    rig.events();

    let apply = rig.fixture.by_label("Apply").expect("the footer has Apply");
    rig.fixture.click(&apply);
    let events = rig.events();
    let applied: Vec<_> = events.iter().filter_map(|event| if let FillEvent::Apply(command) = event { Some(command) } else { None }).collect();
    assert_eq!(applied.len(), 1, "{events:?}");
    assert_eq!(applied[0].strokes.len(), 1, "the command carries the stroke");
    assert!(rig.host.borrow().is_open::<Hosted>(), "Apply keeps the workspace open");

    rig.with(|work| work.answer_apply(Err("the fill failed".to_owned())));
    rig.fixture.idle(2);
    assert!(rig.fixture.by_label("the fill failed").is_some(), "the message shows under the panel");

    rig.with(|work| {
        work.answer_apply(Ok(()));
        work.undo();
        assert_eq!(work.session().strokes.len(), 1, "an applied fill is one document step: undo cannot take the stroke back");
    });
    rig.fixture.idle(2);
    assert!(rig.fixture.by_label("the fill failed").is_none(), "the answer clears the message");

    rig.with(|work| work.settings_mut().opacity = 10.0);
    rig.fixture.idle(3);
    let changed = rig.events().into_iter().filter(|event| matches!(event, FillEvent::SettingsChanged(settings) if settings.opacity == 10.0)).count();
    assert_eq!(changed, 1, "a change between passes is reported once");

    let ok = rig.fixture.by_label("OK").expect("the footer has OK");
    rig.fixture.click(&ok);
    let result = rig.host.borrow_mut().take::<Hosted>();
    assert!(matches!(result, Some(DialogResult::Ok(command)) if command.strokes.len() == 1), "OK answers the command");
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

const REMEMBERED: SaveKey = SaveKey::new("trial.fill");

/// With `remember`, every close stores the settings and the next Content-Aware Fill opens with them: the footer's
/// Cancel, and a cancel from the host (the app's control route) with no pass after the change.
#[trial]
fn fill_remembers_its_settings_at_every_close() {
    let remembered =
        || ContentAwareFillWorkspace::new(FillOpen::new([LAYER.width, LAYER.height], "Layer 1", FillLimits::new(10, 1000))).remember(REMEMBERED);
    let mut rig = Rig::over(remembered());
    rig.with(|work| work.settings_mut().opacity = 10.0);
    rig.fixture.idle(2);
    assert_eq!(saved_value::<Option<FillSettings>>(rig.fixture.ctx(), &REMEMBERED), None, "a change alone stores nothing");

    let cancel = rig.fixture.by_label("Cancel").expect("the footer has Cancel");
    rig.fixture.click(&cancel);
    let stored = saved_value::<Option<FillSettings>>(rig.fixture.ctx(), &REMEMBERED);
    assert!(stored.is_some_and(|settings| settings.opacity == 10.0), "Cancel stores the settings");

    let _ = rig.host.borrow_mut().take::<Hosted>();
    let source: Arc<dyn FillSource> = Arc::new(Source);
    rig.host.borrow_mut().open(HostedWorkspace::new(remembered(), source));
    rig.fixture.idle(3);
    assert_eq!(rig.with(|work| work.settings().opacity), 10.0, "the next opening takes the stored settings");

    rig.with(|work| work.settings_mut().opacity = 40.0);
    let ctx = rig.fixture.ctx().clone();
    assert!(rig.host.borrow_mut().cancel::<Hosted>(&ctx), "the host cancels the workspace");
    let stored = saved_value::<Option<FillSettings>>(&ctx, &REMEMBERED);
    assert!(stored.is_some_and(|settings| settings.opacity == 40.0), "a cancel from the host stores the settings");
}

/// The automation keys `expand` and `contract` edit the fill area by that many pixels, like the bar's buttons; a
/// value that is not a number is refused.
#[trial]
fn expand_and_contract_keys_edit_the_fill_area() {
    let rig = Rig::new(FillSettings::default(), FillLimits::new(10, 100));
    let ctx = rig.fixture.ctx().clone();
    let source: Arc<dyn FillSource> = Arc::new(Source);
    let set = |field: &str, value: serde_json::Value| rig.with(|workspace| workspace.set_json_field(&ctx, &*source, field, value));
    assert_eq!(set("expand", serde_json::json!(3)), Ok(()));
    assert_eq!(set("contract", serde_json::json!(2)), Ok(()));
    assert!(set("expand", serde_json::json!("wide")).is_err(), "text is not an amount");
    let edits = rig.with(|workspace| workspace.session().fill_edits.clone());
    assert_eq!(edits, vec![FillAreaEdit::grow(true, 3.0), FillAreaEdit::grow(false, 2.0)]);
}
