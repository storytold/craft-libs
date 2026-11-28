//! Tests of the Select and Mask workspace: its tools, session, panel and keys.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use craft_behavior::PixelSize;
use craft_host::{AppInfo, Host, InlineJobs};
use craft_imaging::trace::MaskSource;
use craft_imaging::ui::mask_view::MaskViewMode;
use craft_imaging::ui::select_mask::{
    EditMode, LimitRefused, RefineStroke, SelectMaskEdit, SelectMaskEvent, SelectMaskLimits, SelectMaskOp, SelectMaskOpen, SelectMaskPreviewRequest,
    SelectMaskSession, SelectMaskSource, SelectMaskWorkspace,
};
use craft_imaging::ui::wide_angle::PreviewDone;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::dialog::{DialogHost, HostedWorkspace};
use craft_ui::primitives::Revision;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::pending::Job;
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Color32, ColorImage, Key, Modifiers, Pos2, Vec2};

/// The document's size in pixels.
const DOCUMENT: [u32; 2] = [800, 600];

/// The edit limits the workspace opens with.
const LIMITS: SelectMaskLimits = SelectMaskLimits::new(3, 10, 2, 4);

// ---- The workspace, on a fixture ----

/// A solid picture.
struct Solid {
    size: PixelSize,
}

impl RasterSource for Solid {
    fn size(&self) -> PixelSize {
        self.size
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
        out.pixels.fill(Color32::from_rgb(40, 120, 200));
    }
}

/// The app's side: a picture and a count of the preview jobs asked for.
struct Source {
    image: Solid,
    jobs: Arc<AtomicUsize>,
}

impl SelectMaskSource for Source {
    fn view(&self) -> &dyn RasterSource {
        &self.image
    }

    fn image(&self) -> &dyn RasterSource {
        &self.image
    }

    fn ants(&self) -> Option<(&dyn MaskSource, u32)> {
        None
    }

    fn preview_job(&self, _: &SelectMaskPreviewRequest) -> Job<PreviewDone, String> {
        self.jobs.fetch_add(1, Ordering::SeqCst);
        Job::new(|_, _| Ok(PreviewDone::new(Duration::ZERO)))
    }
}

/// What the app keeps: the dialog host that owns the workspace, and the events it took from it.
struct App {
    host: DialogHost,
    events: Vec<SelectMaskEvent>,
}

impl App {
    /// The hosted workspace.
    fn hosted(&self) -> &HostedWorkspace<SelectMaskWorkspace> {
        self.host.get::<HostedWorkspace<SelectMaskWorkspace>>().expect("the workspace is open")
    }
}

/// The workspace over a solid picture, settled.
// A pass's `WorkspaceCx` is built by the host only, so this runs the workspace on a `DialogHost`.
fn open() -> (Fixture, Rc<RefCell<App>>, Arc<AtomicUsize>) {
    let jobs = Arc::new(AtomicUsize::new(0));
    let size = PixelSize::new(DOCUMENT[0], DOCUMENT[1]);
    let source: Arc<dyn SelectMaskSource> = Arc::new(Source { image: Solid { size }, jobs: Arc::clone(&jobs) });
    let mut host = DialogHost::new();
    host.open(HostedWorkspace::new(SelectMaskWorkspace::new(SelectMaskOpen::new(DOCUMENT, "Photo", LIMITS)), source));
    let app = Rc::new(RefCell::new(App { host, events: Vec::new() }));
    let shown = Rc::clone(&app);
    let mut fixture = Fixture::new(move |ui| {
        let mut app = shown.borrow_mut();
        app.host.run(ui);
        let taken = app.host.get_mut::<HostedWorkspace<SelectMaskWorkspace>>().map(HostedWorkspace::take_events).unwrap_or_default();
        app.events.extend(taken);
    });
    fixture.host(Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(Arc::new(InlineJobs)).build()));
    fixture.room(Vec2::new(1200.0, 900.0));
    fixture.settle();
    (fixture, app, jobs)
}

fn canvas_point(fixture: &Fixture, offset: Vec2) -> Pos2 {
    let canvas = fixture.by_role(Role::CANVAS).into_iter().next().expect("the workspace shows a canvas");
    canvas.center() + offset
}

fn press_label(fixture: &mut Fixture, label: &str) {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("a control named {label}"));
    fixture.click(&target);
    fixture.settle();
}

#[trial]
fn select_mask_preview_is_asked_for_only_when_it_changes() {
    let (mut fixture, app, jobs) = open();
    let first = jobs.load(Ordering::SeqCst);
    assert_eq!(first, 1, "one preview for the opening state");
    fixture.frame(vec![pointer::moved(canvas_point(&fixture, Vec2::ZERO))]);
    fixture.settle();
    assert_eq!(jobs.load(Ordering::SeqCst), first, "hovering changes nothing");
    press_label(&mut fixture, "Invert");
    assert_eq!(jobs.load(Ordering::SeqCst), first + 1, "an edit asks for a new preview");
    let size = PixelSize::new(DOCUMENT[0], DOCUMENT[1]);
    let edited: Arc<dyn SelectMaskSource> = Arc::new(Source { image: Solid { size }, jobs: Arc::clone(&jobs) });
    app.borrow_mut().host.get_mut::<HostedWorkspace<SelectMaskWorkspace>>().expect("the workspace is open").set_source(edited);
    fixture.settle();
    assert_eq!(jobs.load(Ordering::SeqCst), first + 2, "a document edit asks for a new preview of the same request");
}

// The view keys work from inside the workspace with no help from the app: K picks Black & White, X held hides the
// view until it is released.
#[trial]
fn select_mask_view_keys_pick_the_view_and_x_held_hides_it() {
    let (mut fixture, app, _) = open();
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    fixture.key(Key::K, Modifiers::NONE).settle();
    assert_eq!(app.borrow().hosted().workspace().recipe().mode, MaskViewMode::BlackAndWhite);

    fixture.frame(vec![key_press(Key::X, Modifiers::NONE)]);
    fixture.settle();
    assert!(app.borrow().hosted().workspace().recipe().hidden, "X held hides the view");
    fixture.frame(vec![key_release(Key::X, Modifiers::NONE)]);
    fixture.settle();
    assert!(!app.borrow().hosted().workspace().recipe().hidden, "the release shows it again");
}

// The limits count edits and points over the whole session: a total equal to a limit passes, an edit past it is
// refused with the limit's number and records nothing, and `perform` gives the same refusal.
#[trial]
fn select_mask_limits_refuse_at_the_brush() {
    let lasso = |points: usize| SelectMaskEdit::lasso(vec![[1.0, 2.0]; points], EditMode::Add);
    let refine = |points: usize| RefineStroke::new(vec![[1.0, 2.0]; points], 20.0, EditMode::Add);

    let mut session = SelectMaskSession::default();
    for _ in 0..3 {
        assert_eq!(session.add_edit(lasso(1), &LIMITS), Ok(()));
    }
    assert_eq!(session.add_edit(lasso(1), &LIMITS), Err(LimitRefused::Edits(3)), "three edits are the limit");
    assert_eq!(session.edits.len(), 3, "a refused edit records nothing");

    let mut session = SelectMaskSession::default();
    assert_eq!(session.add_edit(lasso(6), &LIMITS), Ok(()));
    assert_eq!(session.add_edit(lasso(5), &LIMITS), Err(LimitRefused::EditPoints(10)), "6 and 5 points pass the 10-point limit");
    assert_eq!(session.edits.len(), 1);
    assert_eq!(session.add_edit(lasso(4), &LIMITS), Ok(()), "a total equal to the limit passes");

    let mut session = SelectMaskSession::default();
    assert_eq!(session.add_stroke(refine(3), &LIMITS), Ok(()));
    assert_eq!(session.add_stroke(refine(2), &LIMITS), Err(LimitRefused::StrokePoints(4)));
    assert_eq!(session.strokes.len(), 1, "a refused stroke records nothing");
    assert_eq!(session.add_stroke(refine(1), &LIMITS), Ok(()));
    assert_eq!(session.add_stroke(refine(0), &LIMITS), Err(LimitRefused::Strokes(2)), "two strokes are the limit");

    let mut workspace = SelectMaskWorkspace::new(SelectMaskOpen::new(DOCUMENT, "Photo", LIMITS));
    for _ in 0..3 {
        assert_eq!(workspace.perform(SelectMaskOp::Edit(lasso(1))), Ok(()));
    }
    assert_eq!(workspace.perform(SelectMaskOp::Edit(lasso(1))), Err(LimitRefused::Edits(3)));
    assert_eq!(workspace.session().edits.len(), 3, "a refused request records nothing");
}
