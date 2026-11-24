//! Filter Gallery: the stack's session rules, the workspace's undo, and the columns and preview job shown in a
//! full-window dialog.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use craft_behavior::PixelSize;
use craft_host::{AppInfo, Host};
use craft_imaging::ui::filter_gallery::{
    EffectChange, EffectLayer, EffectParameters, FilterCatalog, FilterCategory, FilterGalleryEvent, FilterGalleryOpen, FilterGallerySession,
    FilterGallerySource, FilterGalleryWorkspace, FilterIndex, FilterTree, FilterTreeLayout, GalleryFilter, GalleryThumb, SessionField,
};
use craft_imaging::ui::wide_angle::PreviewDone;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::contract::{Bind, Job};
use craft_ui::dialog::{FullWindowWorkspace as _, WorkspaceCx};
use craft_ui::patterns::form_dialogs::{Param, ParamKind};
use craft_ui::primitives::Revision;
use craft_ui::primitives::textures::{FakeThumbSource, ThumbSource, ThumbnailOptions, Thumbnails};
use craft_ui::runtime::requests::{PathKind, PathRequest};
use craft_ui::testing::{Fixture, trial};
use craft_ui::value_drag::SliderScale;
use egui::{Color32, ColorImage, Key, Modifiers, Vec2};

const WINDOW: Vec2 = Vec2::new(1100.0, 640.0);
const LAYER: PixelSize = PixelSize::new(400, 300);

struct Catalog;

impl Catalog {
    fn all() -> Vec<GalleryFilter> {
        let filter = |key: &str, category: &str, params: Vec<Param>, uses_colors: bool| {
            GalleryFilter::new(key, key.to_owned(), category, params).uses_colors(uses_colors)
        };
        vec![
            filter("cutout", "artistic", Param::parse_list(r#"{"levels":2..8=6,"mode":"soft|hard","edges":bool}"#), false),
            filter("sponge", "artistic", Param::parse_list(r#"{"size":0..40=2}"#), true),
            filter("chalk", "sketch", Param::parse_list(r#"{"detail":0..20=1}"#), false),
            filter(
                "neon",
                "artistic",
                vec![
                    Param::new("glowSize", "Glow Size", ParamKind::range(-24.0, 24.0, 5.0, SliderScale::default())),
                    Param::new("glowColor", "Glow Color", ParamKind::color(Some([1.0, 0.0, 0.5]))),
                    Param::new("note", "Note", ParamKind::text("x")),
                    Param::new("texturePath", "Texture", ParamKind::text("")).browse(PathKind::Folder),
                ],
                false,
            ),
        ]
    }
}

impl FilterCatalog for Catalog {
    fn categories(&self) -> Vec<FilterCategory> {
        let keys = |keys: &[&str]| keys.iter().map(|key| (*key).to_owned()).collect();
        vec![
            FilterCategory::new("artistic", "Artistic", keys(&["cutout", "sponge", "neon"])),
            FilterCategory::new("sketch", "Sketch", keys(&["chalk"])),
            FilterCategory::new("empty", "Empty", Vec::new()),
        ]
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

/// The app's side: the catalog, the thumbnails, and a flat picture the size of the layer, whose render finishes only
/// when `done`.
struct Flat {
    catalog: Catalog,
    thumbs: FakeThumbSource<GalleryThumb>,
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
    fn catalog(&self) -> &dyn FilterCatalog {
        &self.catalog
    }

    fn thumb_source(&self) -> &dyn ThumbSource<GalleryThumb> {
        &self.thumbs
    }

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

/// A workspace in a fixture and the events its passes reported.
struct Rig {
    fixture: Fixture,
    workspace: Rc<RefCell<FilterGalleryWorkspace>>,
    events: Rc<RefCell<Vec<FilterGalleryEvent>>>,
}

/// Shows the workspace through its trait `show`, as a host does, with the open state kept across passes.
fn rig(session: FilterGallerySession, done: bool) -> Rig {
    let workspace = Rc::new(RefCell::new(FilterGalleryWorkspace::new(FilterGalleryOpen::new(LAYER, "Layer 1", session))));
    let events = Rc::new(RefCell::new(Vec::new()));
    let (shown, sink) = (workspace.clone(), events.clone());
    let source = Flat { catalog: Catalog, thumbs: FakeThumbSource::new(), done };
    let mut open = true;
    let mut fixture = Fixture::new(move |ui| {
        let source: &dyn FilterGallerySource = &source;
        let pass = shown.borrow_mut().show(ui, WorkspaceCx::new(source, Bind::bound(&mut open)));
        sink.borrow_mut().extend(pass.events);
    });
    fixture.host(Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).build()));
    fixture.room(WINDOW).settle();
    Rig { fixture, workspace, events }
}

impl Rig {
    fn events(&self) -> Vec<FilterGalleryEvent> {
        self.events.borrow().clone()
    }
}

// ⌘Z steps the stack's history: the selection moves onto the layers that remain, and the `Select` event comes from
// the pass's comparison with the selection the app knows, once.
#[trial]
fn gallery_undo_by_key_clamps_the_selection_and_reports_select_once() {
    let mut rig = rig(FilterGallerySession::new("cutout", &Catalog), true);
    let add = rig.fixture.by_label("New effect layer").expect("the stack offers New");
    rig.fixture.click(&add).settle();
    assert_eq!(rig.workspace.borrow().selected(), 1);
    assert!(rig.events().contains(&FilterGalleryEvent::Select(1)));
    let selects = |rig: &Rig| rig.events().iter().filter(|event| matches!(event, FilterGalleryEvent::Select(0))).count();
    assert_eq!(selects(&rig), 0);

    rig.fixture.key(Key::Z, Modifiers::COMMAND).settle();
    assert_eq!(rig.workspace.borrow().session().effects.len(), 1);
    assert_eq!(rig.workspace.borrow().selected(), 0, "the selection moves onto the layer that remains");
    assert_eq!(selects(&rig), 1, "the pass reports `Select` from its comparison, once");
    rig.fixture.key(Key::Z, Modifiers::COMMAND).settle();
    assert_eq!(rig.workspace.borrow().session().effects.len(), 1, "nothing left to undo");
}

// The app answers a color request by key and `Srgba`: the selected effect's field takes it as one undo step.

fn host() -> Arc<Host> {
    Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).build())
}

// A Browse press is reported in the pass it happens, once, with the row's key.
#[trial]
fn a_browse_press_in_the_settings_is_reported_once() {
    let layer = FilterGallerySession::new("neon", &Catalog).effects[0].clone();
    let requests: Rc<RefCell<Vec<PathRequest>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = requests.clone();
    let mut fixture = Fixture::new(move |ui| {
        let index = FilterIndex::of(&Catalog);
        sink.borrow_mut().extend(EffectParameters::new(0, &layer, &index).show(ui).path);
    });
    fixture.host(host());
    fixture.room(Vec2::new(400.0, 500.0)).settle();
    assert!(requests.borrow().is_empty());
    let browse = fixture.by_label("Browse…").expect("the Browse button");
    fixture.click(&browse).settle();
    assert_eq!(requests.borrow().iter().map(|request| request.key.as_str()).collect::<Vec<_>>(), ["texturePath"]);
    fixture.idle(3);
    assert_eq!(requests.borrow().len(), 1, "no later pass reports the press again");
}

// Undo restores snapshots with their ids, and the session's counter does not go back with them, so an id is never issued
// twice; moves and edits keep a layer's id, a copy gets its own, and equality ignores ids.
#[trial]
fn layer_ids_survive_moves_and_a_copy_gets_its_own() {
    let mut session = FilterGallerySession::new("cutout", &Catalog);
    let mut selected = 0;
    let first = session.effects[0].id;
    assert!(session.apply(EffectChange::Add, &Catalog, &mut selected));
    let copy = session.effects[1].id;
    assert_ne!(first, copy);
    assert!(session.apply(EffectChange::set_filter(1, "sponge"), &Catalog, &mut selected));
    assert_eq!(session.position(copy), Some(1), "a new filter keeps the layer");
    let neon = Catalog.filter("neon").expect("neon").defaults();
    assert!(session.apply(EffectChange::with_fields(1, neon), &Catalog, &mut selected));
    assert!(session.apply(EffectChange::set_visible(1, false), &Catalog, &mut selected));
    assert_eq!(session.position(copy), Some(1), "fields and the eye keep the layer");
    assert!(session.apply(EffectChange::move_to(1, 0), &Catalog, &mut selected));
    assert_eq!((session.position(copy), session.position(first)), (Some(0), Some(1)));
    assert!(session.apply(EffectChange::Delete, &Catalog, &mut selected));
    assert_eq!((session.position(copy), session.position(first)), (None, Some(0)), "a deleted layer has no place");

    let incoming = FilterGallerySession::new("neon", &Catalog);
    assert!(session.apply(EffectChange::Replace { effects: incoming.effects.clone() }, &Catalog, &mut selected));
    assert_ne!(session.effects[0].id, incoming.effects[0].id, "a replacement gets fresh ids");
    let mut one = FilterGallerySession::new("cutout", &Catalog);
    assert!(one.apply(EffectChange::Add, &Catalog, &mut 0));
    assert_ne!(one.effects[0].id, one.effects[1].id);
    assert_eq!(one.effects[0], one.effects[1], "layer equality is by content");
    let mut two = FilterGallerySession::new("cutout", &Catalog);
    assert!(two.apply(EffectChange::Add, &Catalog, &mut 0) && two.apply(EffectChange::Delete, &Catalog, &mut 1));
    assert!(two.apply(EffectChange::Add, &Catalog, &mut 0));
    assert_ne!(one.effects[1].id, two.effects[1].id);
    assert_eq!(one, two, "session equality is by content");

    let mut rig = rig(FilterGallerySession::new("cutout", &Catalog), true);
    let add = rig.fixture.by_label("New effect layer").expect("the stack offers New");
    rig.fixture.click(&add).settle();
    let undone = rig.workspace.borrow().session().effects[1].id;
    rig.fixture.key(Key::Z, Modifiers::COMMAND).settle();
    assert_eq!(rig.workspace.borrow().session().effects.len(), 1);
    rig.fixture.click(&add).settle();
    assert_ne!(rig.workspace.borrow().session().effects[1].id, undone, "an undone layer's id is not reissued");
}

// The lists layout: a press on a category picks its first filter (the current category's press changes nothing), a
// press or Enter on a filter row picks it, and a category without filters is not listed.
#[trial]
fn the_lists_layout_picks_a_categorys_first_filter() {
    let current = Rc::new(RefCell::new("sponge".to_owned()));
    let picks: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let (now, sink) = (current.clone(), picks.clone());
    let mut fixture = Fixture::new(move |ui| {
        let index = FilterIndex::of(&Catalog);
        let thumbs = Thumbnails::<GalleryThumb>::in_context(ui.ctx(), "trial-tree-thumbs", ThumbnailOptions::default());
        let selected = now.borrow().clone();
        let picked =
            FilterTree::new(&index, &selected, &thumbs, &FakeThumbSource::<GalleryThumb>::new()).layout(FilterTreeLayout::Lists).show(ui).picked;
        if let Some(filter) = picked {
            *now.borrow_mut() = filter.clone();
            sink.borrow_mut().push(filter);
        }
    });
    fixture.host(host());
    fixture.room(Vec2::new(500.0, 400.0)).settle();
    assert!(fixture.by_label("Empty").is_none(), "a category without filters is not listed");

    let artistic = fixture.by_label("Artistic").expect("the first category");
    fixture.click(&artistic).settle();
    assert!(picks.borrow().is_empty(), "the current filter's category is already selected");

    let neon = fixture.by_label("neon").expect("a filter row");
    fixture.click(&neon).settle();
    assert_eq!(*picks.borrow(), ["neon"]);
    picks.borrow_mut().clear();
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*picks.borrow(), ["neon"], "Enter picks the focused filter row");
    picks.borrow_mut().clear();

    let sketch = fixture.by_label("Sketch").expect("the second category");
    fixture.click(&sketch).settle();
    assert_eq!(*picks.borrow(), ["chalk"], "a category press picks its first filter");
    picks.borrow_mut().clear();
    let artistic = fixture.by_label("Artistic").expect("the first category");
    fixture.click(&artistic).settle();
    assert_eq!(*picks.borrow(), ["cutout"]);
}

// The automation vocabulary: each field's change, the selection clamp, `false` as nothing, and the refusals.
#[trial]
fn session_field_change_maps_each_automation_field_and_refuses_the_rest() {
    let mut session = FilterGallerySession::new("cutout", &Catalog);
    let mut selected = 0;
    session.apply(EffectChange::Add, &Catalog, &mut selected);
    session.apply(EffectChange::set_filter(1, "neon"), &Catalog, &mut selected);
    let run = |field: &str, value: serde_json::Value| session.field_change(&Catalog, 1, field, &value);
    let change = |change: EffectChange| Ok(Some(SessionField::Change(change)));

    assert_eq!(run("select", serde_json::json!(9)), Ok(Some(SessionField::Select(1))));
    assert_eq!(run("add", serde_json::json!(true)), change(EffectChange::Add));
    assert_eq!(run("add", serde_json::json!(false)), Ok(None));
    assert_eq!(run("delete", serde_json::json!(true)), change(EffectChange::Delete));
    assert_eq!(run("delete", serde_json::json!(false)), Ok(None));
    assert_eq!(run("toggle", serde_json::json!(1)), change(EffectChange::set_visible(1, false)));
    assert_eq!(run("move", serde_json::json!({"from": 0, "to": 1})), change(EffectChange::move_to(0, 1)));
    assert_eq!(run("filter", serde_json::json!("sponge")), change(EffectChange::set_filter(1, "sponge")));
    assert!(matches!(run("params", serde_json::json!({"glowSize": 3.0})), Ok(Some(SessionField::Change(EffectChange::Fields { index: 1, .. })))));
    let layers = serde_json::to_value(&session.effects).expect("layers serialise");
    assert!(matches!(run("effects", layers.clone()), Ok(Some(SessionField::Change(EffectChange::Replace { .. })))));

    let mut unknown_filter = layers;
    unknown_filter[0]["filter"] = serde_json::json!("nope");
    for (field, value) in [
        ("bogus", serde_json::json!(1)),
        ("toggle", serde_json::json!(7)),
        ("move", serde_json::json!({"from": 0, "to": 5})),
        ("filter", serde_json::json!("nope")),
        ("add", serde_json::json!("yes")),
        ("select", serde_json::json!("last")),
        ("params", serde_json::json!({"glowSize": "wide"})),
        ("effects", serde_json::json!([])),
        ("effects", unknown_filter),
    ] {
        assert!(run(field, value.clone()).is_err(), "{field} = {value} is refused");
    }
}

// OK and Cancel sit at the top of the settings column, in the platform's order, over the Filter select and the
// stack.
