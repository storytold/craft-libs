//! Tests of the Select and Mask workspace: its tools, session, panel and keys.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use craft_behavior::PixelSize;
use craft_host::{AppInfo, Host, InlineJobs};
use craft_imaging::trace::MaskSource;
use craft_imaging::ui::select_mask::{
    SelectMaskEvent, SelectMaskLimits, SelectMaskOpen, SelectMaskPreviewRequest, SelectMaskSource, SelectMaskWorkspace,
};
use craft_imaging::ui::wide_angle::PreviewDone;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::primitives::Revision;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::pending::Job;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Color32, ColorImage, Pos2, Vec2};

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

/// What the app keeps of the workspace and what it saw.
struct App {
    workspace: SelectMaskWorkspace,
    source: Source,
    events: Vec<SelectMaskEvent>,
}

/// The workspace over a solid picture, settled.
fn open() -> (Fixture, Rc<RefCell<App>>, Arc<AtomicUsize>) {
    let jobs = Arc::new(AtomicUsize::new(0));
    let size = PixelSize::new(DOCUMENT[0], DOCUMENT[1]);
    let app = Rc::new(RefCell::new(App {
        workspace: SelectMaskWorkspace::new(SelectMaskOpen::new(DOCUMENT, "Photo", LIMITS)),
        source: Source { image: Solid { size }, jobs: Arc::clone(&jobs) },
        events: Vec::new(),
    }));
    let shown = Rc::clone(&app);
    let mut fixture = Fixture::new(move |ui| {
        let mut app = shown.borrow_mut();
        let App { workspace, source, .. } = &mut *app;
        let out = workspace.show(ui, source);
        app.events.extend(out.events);
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
    let (mut fixture, _, jobs) = open();
    let first = jobs.load(Ordering::SeqCst);
    assert_eq!(first, 1, "one preview for the opening state");
    fixture.frame(vec![pointer::moved(canvas_point(&fixture, Vec2::ZERO))]);
    fixture.settle();
    assert_eq!(jobs.load(Ordering::SeqCst), first, "hovering changes nothing");
    press_label(&mut fixture, "Invert");
    assert_eq!(jobs.load(Ordering::SeqCst), first + 1, "an edit asks for a new preview");
}
