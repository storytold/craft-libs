//! A panel's documents are tabs of its group's strip after the panel tabs; a tab's swatch is a data colour before its
//! name.

use craft_behavior::dock::{Action, Layout, Node, Workspace};
use craft_theme::Srgba;
use craft_ui::contract::{Bind, BodyCx, PlainData, Role, Tracked};
use craft_ui::docking::{DockConfig, DockEvent, DockPanels, DockSource, PanelDocument, PanelSpec, PanelSpecs, SourceKind};
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::pointer::{middle, moved};
use craft_ui::testing::{Fixture, Target, access, trial};
use egui::{Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// A list the trial and the body it hosts both see.
type Shared<T> = Rc<RefCell<Vec<T>>>;

const TIMELINE: u32 = 1;
const OTHER: u32 = 2;

/// The Timeline lists two compositions; the other panel has none.
#[derive(PlainData)]
struct Specs;

impl PanelSpecs<u32> for Specs {
    fn spec(&self, panel: &u32) -> PanelSpec {
        match *panel {
            TIMELINE => PanelSpec::new("Timeline")
                .documents(vec![PanelDocument::new(1, "Comp 1").closable(true).shown(true), PanelDocument::new(2, "Comp 2").closable(true)]),
            _ => PanelSpec::new("Effects"),
        }
    }
}

struct Panels;

impl DockPanels<u32> for Panels {
    fn body(&mut self, _: &u32, _: &mut Ui, _: &mut BodyCx<'_>) {}
}

struct Bed {
    fixture: Fixture,
    workspace: Rc<RefCell<Arc<Workspace<u32>>>>,
    events: Rc<RefCell<Vec<DockEvent<u32>>>>,
    actions: Rc<RefCell<Vec<Action<u32>>>>,
    /// The gesture sources the area reported in its last pass.
    sources: Rc<RefCell<Vec<DockSource<u32>>>>,
}

/// One group of the Timeline and Effects, Effects shown, in a room `width` wide.
fn bed(width: f32) -> Bed {
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(Some(Node::tabs(vec![TIMELINE, OTHER], 1)), Vec::new())))));
    let (events, actions): (Shared<DockEvent<u32>>, Shared<Action<u32>>) = Default::default();
    let (held, seen, taken, specs) = (Rc::clone(&workspace), Rc::clone(&events), Rc::clone(&actions), Tracked::new(Specs));
    let sources: Rc<RefCell<Vec<DockSource<u32>>>> = Rc::default();
    let reported = Rc::clone(&sources);
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = held.borrow().clone();
        let config = DockConfig::default().tab_close_buttons(true);
        let shown = DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut Panels, config).show(ui);
        seen.borrow_mut().extend(shown.events);
        taken.borrow_mut().extend(shown.actions);
        *reported.borrow_mut() = shown.sources;
        *held.borrow_mut() = workspace;
    });
    fixture.room(Vec2::new(width, 300.0)).settle();
    Bed { fixture, workspace, events, actions, sources }
}

/// The tabs the strip shows, by name, in reading order.
fn tabs(fixture: &Fixture) -> Vec<(String, Target)> {
    let mut tabs: Vec<(String, Target)> =
        fixture.by_role(Role::TAB).into_iter().filter_map(|tab| Some((access::name(fixture.tree(), tab.node)?.to_owned(), tab))).collect();
    tabs.sort_by(|a, b| a.1.rect.left().total_cmp(&b.1.rect.left()));
    tabs
}

fn tab(fixture: &Fixture, name: &str) -> Target {
    tabs(fixture).into_iter().find(|(shown, _)| shown == name).map(|(_, tab)| tab).unwrap_or_else(|| panic!("the tab {name} shows"))
}

#[trial]
fn a_panels_documents_are_tabs_after_the_panel_tabs_that_pick_and_close_through_the_host() {
    let mut bed = bed(600.0);
    let names: Vec<String> = tabs(&bed.fixture).into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["Timeline", "Effects", "Comp 1", "Comp 2"], "the panel tabs, then the Timeline's documents");

    let second = tab(&bed.fixture, "Comp 2");
    bed.fixture.click(&second).settle();
    assert!(
        bed.events.borrow().contains(&DockEvent::DocumentPicked { panel: TIMELINE, document: 2 }),
        "the pick names the panel and the document: {:?}",
        bed.events.borrow()
    );
    assert!(bed.actions.borrow().contains(&Action::activate(TIMELINE)), "and shows the Timeline: {:?}", bed.actions.borrow());

    let before = bed.workspace.borrow().layout().clone();
    bed.events.borrow_mut().clear();
    let at = tab(&bed.fixture, "Comp 2").rect.center();
    bed.fixture.frame(vec![moved(at)]).frame(vec![middle(at, true)]).frame(vec![middle(at, false)]).settle();
    assert!(
        bed.events.borrow().contains(&DockEvent::DocumentClosed { panel: TIMELINE, document: 2 }),
        "a middle click asks the host to close the document: {:?}",
        bed.events.borrow()
    );
    assert_eq!(bed.workspace.borrow().layout(), &before, "the layout does not change");
}

/// Each document tab is a gesture source of its own, named by its panel and document, where the tab shows.
#[trial]
fn a_panels_document_tabs_are_sources_named_by_the_panel_and_the_document() {
    let bed = bed(600.0);
    let sources = bed.sources.borrow();
    let at = |kind: SourceKind<u32>| sources.iter().find(|source| source.kind == kind).map(|source| source.rect);
    for (document, name) in [(1, "Comp 1"), (2, "Comp 2")] {
        let center = tab(&bed.fixture, name).rect.center();
        let rect = at(SourceKind::Document(TIMELINE, document));
        assert!(rect.is_some_and(|rect| rect.contains(center)), "the {name} tab is a source where it shows: {rect:?}, {sources:?}");
    }
    assert!(at(SourceKind::Tab(TIMELINE)).is_some(), "the panel's own tab is one too");
}

/// A narrow group keeps one strip: the document that does not fit goes to the strip's overflow menu.
#[trial]
fn a_document_that_does_not_fit_goes_to_the_strips_overflow_menu() {
    let mut bed = bed(190.0);
    assert!(tabs(&bed.fixture).iter().all(|(name, _)| name != "Comp 2"), "the last document does not fit");
    let more = bed.fixture.by_label("More tabs").expect("the strip's overflow menu button shows");
    bed.fixture.click(&more).settle();
    assert!(bed.fixture.by_label("Comp 2").is_some(), "the overflow menu lists the document");
}

/// Swatch specs: two panels named alike, the first with a data colour before its name.
#[derive(PlainData)]
struct Swatches;

impl PanelSpecs<u32> for Swatches {
    fn spec(&self, panel: &u32) -> PanelSpec {
        match *panel {
            TIMELINE => PanelSpec::new("Comp").swatch(Some(Srgba::rgb(0.8, 0.3, 0.3))),
            _ => PanelSpec::new("Comp"),
        }
    }
}

#[trial]
fn a_tabs_swatch_comes_before_its_name_and_is_not_part_of_it() {
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(Node::tabs(vec![TIMELINE, OTHER], 0)), Vec::new())));
    let specs = Tracked::new(Swatches);
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut Panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(600.0, 300.0)).settle();
    let shown = tabs(&fixture);
    assert_eq!(shown.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["Comp", "Comp"], "both tabs have the same name");
    let label_left =
        |tab: &Target| fixture.texts().iter().find(|text| text.text == "Comp" && tab.rect.contains(text.rect.center())).map(|text| text.rect.left());
    let (with, without) = (&shown[0].1, &shown[1].1);
    assert!(with.rect.width() > without.rect.width(), "the tab with the swatch is wider: {with:?} {without:?}");
    let (with_text, without_text) = (label_left(with).expect("the first label shows"), label_left(without).expect("the second label shows"));
    assert!(with_text - with.rect.left() > without_text - without.rect.left(), "its label starts later in the tab");
}
