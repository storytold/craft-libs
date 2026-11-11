//! Filter Gallery: the stack's session rules, the workspace's undo, and the columns and preview job shown in a
//! full-window dialog.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use craft_behavior::PixelSize;
use craft_behavior::edit_history::Step;
use craft_host::{AppInfo, Host};
use craft_imaging::ui::filter_gallery::{
    EffectChange, EffectLayer, FilterCatalog, FilterCategory, FilterGalleryEvent, FilterGalleryOpen, FilterGalleryOutput, FilterGallerySession,
    FilterGallerySource, FilterGalleryWorkspace, GalleryFilter, GalleryThumb,
};
use craft_imaging::ui::wide_angle::PreviewDone;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::contract::Job;
use craft_ui::patterns::form_dialogs::{Param, ParamKind};
use craft_ui::primitives::Revision;
use craft_ui::primitives::textures::{FakeThumbSource, ThumbnailOptions, Thumbnails};
use craft_ui::testing::{Fixture, trial};
use craft_ui::value_drag::SliderScale;
use egui::{Color32, ColorImage, Vec2};

const WINDOW: Vec2 = Vec2::new(1100.0, 640.0);
const LAYER: PixelSize = PixelSize::new(400, 300);

struct Catalog;

impl Catalog {
    fn all() -> Vec<GalleryFilter> {
        let filter =
            |key: &str, params: Vec<Param>, uses_colors: bool| GalleryFilter::new(key, key.to_owned(), "artistic", params).uses_colors(uses_colors);
        vec![
            filter("cutout", Param::parse_list(r#"{"levels":2..8=6,"mode":"soft|hard","edges":bool}"#), false),
            filter("sponge", Param::parse_list(r#"{"size":0..40=2}"#), true),
            filter(
                "neon",
                vec![
                    Param::new("glowSize", "Glow Size", ParamKind::range(-24.0, 24.0, 5.0, SliderScale::default())),
                    Param::new("glowColor", "Glow Color", ParamKind::color(Some([1.0, 0.0, 0.5]))),
                    Param::new("note", "Note", ParamKind::text("x")),
                ],
                false,
            ),
        ]
    }
}

impl FilterCatalog for Catalog {
    fn categories(&self) -> Vec<FilterCategory> {
        vec![FilterCategory::new("artistic", "Artistic", Self::all().into_iter().map(|filter| filter.key).collect())]
    }

    fn filter(&self, key: &str) -> Option<GalleryFilter> {
        Self::all().into_iter().find(|filter| filter.key == key)
    }
}

fn filters(session: &FilterGallerySession) -> Vec<&str> {
    session.effects.iter().map(|effect| effect.filter.as_str()).collect()
}

// New copies the selected layer above it and selects it; Delete keeps the last layer; Move and the eye change one
// layer; the top row is applied last.
#[trial]
fn gallery_stack_new_delete_reorder_and_eye() {
    let mut session = FilterGallerySession::new("cutout", &Catalog);
    let mut selected = 0;
    assert!(session.apply(EffectChange::Add, &Catalog, &mut selected));
    assert_eq!(selected, 1);
    assert!(session.apply(EffectChange::set_filter(1, "sponge"), &Catalog, &mut selected));
    assert!(session.apply(EffectChange::Add, &Catalog, &mut selected));
    assert_eq!((filters(&session), selected), (vec!["cutout", "sponge", "sponge"], 2));
    assert!(session.apply(EffectChange::move_to(2, 0), &Catalog, &mut selected));
    assert_eq!((filters(&session), selected), (vec!["sponge", "cutout", "sponge"], 0));
    assert!(!session.apply(EffectChange::move_to(0, 0), &Catalog, &mut selected));
    assert!(session.apply(EffectChange::set_visible(1, false), &Catalog, &mut selected));
    let eyes: Vec<bool> = session.effects.iter().map(|effect| effect.visible).collect();
    assert_eq!(eyes, [true, false, true]);
    selected = 2;
    assert!(session.apply(EffectChange::Delete, &Catalog, &mut selected));
    assert!(session.apply(EffectChange::Delete, &Catalog, &mut selected));
    assert_eq!((filters(&session), selected), (vec!["sponge"], 0));
    assert!(!session.apply(EffectChange::Delete, &Catalog, &mut selected));
    assert_eq!(session.effects.len(), 1);
}

// `GalleryFilter::defaults` takes each Range's default, each Choice's default or first option, each Bool, and the
// colour.

// Undo of a Delete gives the layer back and clamps the selection.

/// A flat picture the size of the layer, whose render finishes only when `done`.
struct Flat {
    done: bool,
}

impl RasterSource for Flat {
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
        out.pixels.fill(Color32::from_gray(120));
    }
}

impl FilterGallerySource for Flat {
    fn preview(&self) -> &dyn RasterSource {
        self
    }

    fn render(&self, _: &[EffectLayer], _: PixelRect, _: f64) -> Job<PreviewDone, String> {
        if self.done {
            Job::new(|_, _| Ok(PreviewDone::new(Duration::from_millis(12))))
        } else {
            Job::new(|_, cancel| {
                while !cancel.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(String::new())
            })
        }
    }
}

/// A workspace in a fixture and what its passes reported.
struct Rig {
    fixture: Fixture,
    workspace: Rc<RefCell<FilterGalleryWorkspace>>,
    outputs: Rc<RefCell<Vec<FilterGalleryOutput>>>,
}

fn rig(session: FilterGallerySession, done: bool) -> Rig {
    let workspace = Rc::new(RefCell::new(FilterGalleryWorkspace::new(FilterGalleryOpen::new(LAYER, "Layer 1", session))));
    let outputs = Rc::new(RefCell::new(Vec::new()));
    let (shown, sink) = (workspace.clone(), outputs.clone());
    let source = Flat { done };
    let mut fixture = Fixture::new(move |ui| {
        let thumbs = Thumbnails::<GalleryThumb>::in_context(ui.ctx(), "gallery-test-thumbs", ThumbnailOptions::default());
        let out = shown.borrow_mut().show(ui, &Catalog, &thumbs, &FakeThumbSource::<GalleryThumb>::new(), &source);
        sink.borrow_mut().push(out);
    });
    fixture.host(Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).build()));
    fixture.room(WINDOW).settle();
    Rig { fixture, workspace, outputs }
}

impl Rig {
    fn events(&self) -> Vec<FilterGalleryEvent> {
        self.outputs.borrow().iter().flat_map(|out| out.events.clone()).collect()
    }
}

// The workspace's `step` clamps the selection to the layers that remain and leaves the `Select` event to the
// pass's comparison with the selection the app knows: it is reported once.
#[trial]
fn gallery_workspace_step_clamps_the_selection_without_queuing_select() {
    let mut rig = rig(FilterGallerySession::new("cutout", &Catalog), true);
    let add = rig.fixture.by_label("New effect layer").expect("the stack offers New");
    rig.fixture.click(&add).settle();
    assert_eq!(rig.workspace.borrow().selected(), 1);
    assert!(rig.events().contains(&FilterGalleryEvent::Select(1)));
    let selects = |rig: &Rig| rig.events().iter().filter(|event| matches!(event, FilterGalleryEvent::Select(0))).count();
    assert_eq!(selects(&rig), 0);

    assert!(rig.workspace.borrow_mut().step(Step::Undo));
    assert_eq!(rig.workspace.borrow().session().effects.len(), 1);
    assert_eq!(rig.workspace.borrow().selected(), 0, "the selection moves onto the layer that remains");
    rig.fixture.settle();
    assert_eq!(selects(&rig), 1, "the next pass reports `Select` from its comparison, once");
    assert!(!rig.workspace.borrow_mut().step(Step::Undo), "nothing left to undo");
}

// The app answers a color request by key and `Srgba`: the selected effect's field takes it as one undo step.

// OK and Cancel sit at the top of the settings column, in the platform's order, over the Filter select and the
// stack.
