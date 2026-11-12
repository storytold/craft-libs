//! `LayersPanel` reveals: the layer the app names scrolls into view with its closed groups opened, and each document
//! keeps its own scroll position.

use craft_layers::ui::mock::FakeLayers;
use craft_layers::ui::panel::{LayersEvent, LayersPanel};
use craft_layers::ui::tree::{LayerInfo, LayerKind, LayerPart, LayerSelection};
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// One panel's room: shorter than the tall tree, inside the 800 by 600 screen of a fixture.
const ONE_PANEL: Vec2 = Vec2::new(400.0, 480.0);

type Shared<T> = Rc<RefCell<T>>;

/// The layers of the tall tree the trials name.
struct Tall {
    top: u32,
    group: u32,
    inner: u32,
}

/// 30 layers above a closed group holding "Inner", then the Background: Inner is far below the view.
fn tall() -> (FakeLayers, Tall) {
    let mut tree = FakeLayers::default();
    let mut top = 0;
    for n in (0..30).rev() {
        let id = tree.add(None, LayerInfo::new(format!("Stack {n:02}"), LayerKind::Pixel), false);
        if n == 29 {
            top = id;
        }
    }
    let group = tree.add(None, LayerInfo::new("Group", LayerKind::Group), false);
    let inner = tree.add(Some(group), LayerInfo::new("Inner", LayerKind::Pixel), false);
    tree.add(None, LayerInfo::new("Background", LayerKind::Pixel), false);
    tree.selection = LayerSelection::new(vec![top], Some(top), LayerPart::Pixels);
    (tree, Tall { top, group, inner })
}

/// A short document: two layers.
fn short() -> FakeLayers {
    let mut tree = FakeLayers::default();
    let top = tree.add(None, LayerInfo::new("Short Top", LayerKind::Pixel), false);
    tree.add(None, LayerInfo::new("Short Background", LayerKind::Pixel), false);
    tree.selection = LayerSelection::new(vec![top], Some(top), LayerPart::Pixels);
    tree
}

struct Rig {
    fixture: Fixture,
    docs: Vec<Shared<FakeLayers>>,
    /// The document shown, by its position in `docs`.
    shown: Rc<Cell<usize>>,
    reveal: Rc<Cell<Option<u32>>>,
    seen: Shared<Vec<LayersEvent<u32>>>,
}

/// The panel over the documents `docs`, each keyed by its position when there are several. The host applies the
/// panel's opens in the pass that reports them, as an app does.
fn rig(docs: Vec<FakeLayers>) -> Rig {
    let docs: Vec<Shared<FakeLayers>> = docs.into_iter().map(|tree| Rc::new(RefCell::new(tree))).collect();
    let keyed = docs.len() > 1;
    let shown: Rc<Cell<usize>> = Rc::default();
    let reveal: Rc<Cell<Option<u32>>> = Rc::default();
    let seen: Shared<Vec<LayersEvent<u32>>> = Rc::default();
    let (trees, which, asked, sink) = (docs.clone(), Rc::clone(&shown), Rc::clone(&reveal), Rc::clone(&seen));
    let mut fixture = Fixture::new(move |ui| {
        let tree = &trees[which.get()];
        let events = {
            let held = tree.borrow();
            let mut panel = LayersPanel::new(&*held);
            if keyed {
                panel = panel.document(which.get());
            }
            if let Some(id) = asked.take() {
                panel = panel.reveal(id);
            }
            panel.show(ui).events
        };
        for event in &events {
            if let LayersEvent::Open(change) = event {
                tree.borrow_mut().layer_mut(change.key).open = change.open;
            }
        }
        sink.borrow_mut().extend(events);
    });
    fixture.room(ONE_PANEL);
    fixture.settle();
    Rig { fixture, docs, shown, reveal, seen }
}

impl Rig {
    /// The rows' view: the list's node.
    fn view(&self) -> Rect {
        self.fixture.by_label("Layers").expect("the rows list is shown").rect
    }

    /// Whether the row `name` is drawn wholly inside the rows' view.
    fn in_view(&self, name: &str) -> bool {
        self.fixture.by_label(name).is_some_and(|row| self.view().contains_rect(row.rect))
    }

    /// Where the row `name` is drawn.
    fn row(&self, name: &str) -> Rect {
        self.fixture.by_label(name).expect("the row is drawn").rect
    }

    /// One frame that reveals `id`, with what it reported; asserts that the frame ran once and measured no row twice.
    fn reveal(&mut self, id: u32) -> Vec<LayersEvent<u32>> {
        self.seen.borrow_mut().clear();
        self.reveal.set(Some(id));
        self.fixture.idle(1);
        assert_eq!(self.fixture.passes(), 1, "a reveal runs its frame once");
        let measures = craft_ui::runtime::measure::row_measures(self.fixture.ctx());
        assert!(measures.iter().all(|(_, count)| *count == 1), "a reveal measures no row twice: {measures:?}");
        self.seen.borrow_mut().drain(..).collect()
    }

    fn select(&self, id: u32) {
        self.docs[self.shown.get()].borrow_mut().selection = LayerSelection::new(vec![id], Some(id), LayerPart::Pixels);
    }
}

/// #152: the layer the app reveals opens its closed groups and scrolls wholly into view in the frame that reveals it;
/// a row already in view doesn't move the list, and a pass with no reveal reports nothing.
#[trial]
fn a_revealed_layer_opens_its_groups_and_scrolls_into_view() {
    let (tree, Tall { top, group, inner }) = tall();
    let mut rig = rig(vec![tree]);
    assert!(rig.in_view("Stack 29"), "the panel starts at the top");
    assert!(rig.fixture.by_label("Inner").is_none(), "Inner is hidden in its closed group");

    rig.select(inner);
    let events = rig.reveal(inner);
    assert!(matches!(events.as_slice(), [LayersEvent::Open(change)] if change.key == group && change.open), "the closed group opens: {events:?}");
    assert!(rig.in_view("Inner"), "Inner is scrolled wholly into view");
    assert!(!rig.in_view("Stack 29"), "the top scrolled away");
    rig.fixture.idle(2);
    assert!(rig.in_view("Inner") && rig.seen.borrow().is_empty(), "the next passes keep the place and report nothing");

    rig.select(top);
    rig.reveal(top);
    assert!(rig.in_view("Stack 29"), "scrolled back to the top");
    let before = rig.row("Stack 29");
    let events = rig.reveal(top + 1);
    assert!(events.is_empty(), "a reveal of a layer in view reports nothing: {events:?}");
    assert_eq!(rig.row("Stack 29"), before, "no scroll for a row in view");
}

/// Each document keeps its own scroll position: a short document shown between doesn't move the tall one's.
#[trial]
fn each_document_keeps_its_scroll_position() {
    let (tree, Tall { inner, .. }) = tall();
    let mut rig = rig(vec![tree, short()]);
    rig.select(inner);
    rig.reveal(inner);
    rig.fixture.settle();
    let at = rig.row("Inner");
    assert!(rig.in_view("Inner"), "the tall document is scrolled to Inner");
    for _ in 0..3 {
        rig.shown.set(1);
        rig.fixture.settle();
        assert!(rig.in_view("Short Top"), "the short document shows from its top");
        rig.shown.set(0);
        rig.fixture.settle();
        assert_eq!(rig.row("Inner"), at, "the tall document shows again where it was left");
    }
}
