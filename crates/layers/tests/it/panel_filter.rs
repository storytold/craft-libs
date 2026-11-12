//! `LayersPanel` filters: Artboard and Group keep each group open or closed, the app answers what only it knows, and
//! Find Layers puts the caret in the Name field.

use craft_layers::ui::filter::{FilterBy, LayerAttribute, LayerFilter, SmartKind};
use craft_layers::ui::mock::FakeLayers;
use craft_layers::ui::panel::LayersPanel;
use craft_layers::ui::tree::LayerSelection;
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// One panel's room: tall enough for the artboard tree and the footer, and inside the 800 by 600 screen of a fixture.
const ONE_PANEL: Vec2 = Vec2::new(400.0, 560.0);

/// Every layer of [`FakeLayers::boards`], top first.
const BOARDS: [&str; 12] = ["Cover", "Title", "Trees", "Oak", "Pine", "Back", "Grass", "Shapes", "Circle", "Linked Logo", "Loose", "Background"];

type Shared<T> = Rc<RefCell<T>>;

/// A panel over the artboard tree and a filter the trial sets; Find Layers runs in the pass after `find` is set.
struct Rig {
    fixture: Fixture,
    tree: Shared<FakeLayers>,
    filter: Shared<LayerFilter>,
    find: Rc<Cell<bool>>,
}

/// The rig, its filter set by `start`.
fn rig(start: impl FnOnce(&mut LayerFilter)) -> Rig {
    let tree = Rc::new(RefCell::new(FakeLayers::boards()));
    let mut filter = LayerFilter::default();
    start(&mut filter);
    let filter = Rc::new(RefCell::new(filter));
    let find = Rc::new(Cell::new(false));
    let (shown, by, asked) = (Rc::clone(&tree), Rc::clone(&filter), Rc::clone(&find));
    let mut fixture = Fixture::new(move |ui| {
        let tree = shown.borrow();
        let mut filter = by.borrow_mut();
        let mut panel = LayersPanel::new(&*tree).filter(Bind::bound(&mut *filter));
        if asked.replace(false) {
            panel = panel.focus_filter_name();
        }
        let _ = panel.show(ui);
    });
    fixture.room(ONE_PANEL);
    fixture.settle();
    Rig { fixture, tree, filter, find }
}

impl Rig {
    /// The rows on screen, top to bottom.
    fn rows(&self) -> Vec<&'static str> {
        let mut shown: Vec<(f32, &str)> = BOARDS.iter().filter_map(|label| self.fixture.by_label(label).map(|t| (t.rect.top(), *label))).collect();
        shown.sort_by(|a, b| a.0.total_cmp(&b.0));
        shown.into_iter().map(|(_, label)| label).collect()
    }

    fn id(&self, name: &str) -> u32 {
        self.tree.borrow().layers.iter().find(|layer| layer.info.name == name).map(|layer| layer.id).expect("the layer is in the tree")
    }

    fn filter_by(&mut self, edit: impl FnOnce(&mut LayerFilter)) {
        edit(&mut self.filter.borrow_mut());
        self.fixture.idle(3);
    }
}

/// Group lists every group with its contents and Artboard the artboard of the selection; both keep each group open or
/// closed as it is, where the other types list a closed group's matches.
#[trial]
fn artboard_and_group_filters_keep_each_group_open_or_closed() {
    let mut rig = rig(|filter| filter.by = FilterBy::Group);
    assert_eq!(rig.rows(), ["Trees", "Oak", "Pine", "Shapes", "Circle"], "an artboard is no group, the group inside it is");
    let trees = rig.id("Trees");
    rig.tree.borrow_mut().layer_mut(trees).open = false;
    rig.fixture.idle(3);
    assert_eq!(rig.rows(), ["Trees", "Shapes", "Circle"], "a closed group stays closed");

    rig.filter_by(|filter| filter.by = FilterBy::Artboard);
    assert_eq!(rig.rows(), ["Cover", "Title", "Trees"], "the artboard that holds the selected Oak");
    rig.tree.borrow_mut().selection = LayerSelection::default();
    rig.fixture.idle(3);
    let every = ["Cover", "Title", "Trees", "Back", "Grass", "Shapes", "Circle", "Linked Logo", "Loose", "Background"];
    assert_eq!(rig.rows(), every, "nothing selected: every row");

    rig.filter_by(|filter| {
        filter.by = FilterBy::Name;
        filter.name = String::from("oak");
    });
    assert_eq!(rig.rows(), ["Oak"], "Name lists a match inside a closed group");
}

/// Empty, Not Empty and Missing list what the app's `listed` says.
#[trial]
fn the_app_decides_what_only_it_knows() {
    let mut rig = rig(|_| {});
    let loose = rig.id("Loose");
    rig.tree.borrow_mut().empty = vec![loose];
    rig.filter_by(|filter| {
        filter.by = FilterBy::Attribute;
        filter.attribute = LayerAttribute::Empty;
    });
    assert_eq!(rig.rows(), ["Loose"]);
    rig.filter_by(|filter| filter.attribute = LayerAttribute::NotEmpty);
    assert!(!rig.rows().contains(&"Loose") && rig.rows().contains(&"Oak"));
    rig.filter_by(|filter| {
        filter.by = FilterBy::SmartObject;
        filter.smart = vec![SmartKind::Missing];
    });
    assert_eq!(rig.rows(), ["Linked Logo"]);
}

/// Find Layers switches to Name and selects the field's text, so typing replaces it.
#[trial]
fn find_layers_puts_the_caret_in_the_name_field_with_its_text_selected() {
    let mut rig = rig(|filter| filter.name = String::from("old"));
    rig.find.set(true);
    rig.fixture.idle(3);
    assert_eq!(rig.filter.borrow().by, FilterBy::Name);
    rig.fixture.type_text("pine");
    rig.fixture.idle(2);
    assert_eq!(rig.filter.borrow().name, "pine", "the typed text replaces the old");
    assert_eq!(rig.rows(), ["Pine"]);
}

/// A panel offering Kind and Name lists only those two in the type select, and a filter set to Artboard falls back to
/// Kind, reported as a filter event so the app stores it.
#[trial]
fn the_type_select_lists_only_the_offered_types() {
    let tree = Rc::new(RefCell::new(FakeLayers::boards()));
    let mut start = LayerFilter::default();
    start.by = FilterBy::Artboard;
    let filter = Rc::new(RefCell::new(start));
    let (shown, by) = (Rc::clone(&tree), Rc::clone(&filter));
    let mut fixture = Fixture::new(move |ui| {
        let tree = shown.borrow();
        let mut filter = by.borrow_mut();
        let _ = LayersPanel::new(&*tree).filter(Bind::bound(&mut *filter)).filter_types([FilterBy::Name, FilterBy::Kind]).show(ui);
    });
    fixture.room(ONE_PANEL);
    fixture.settle();
    assert_eq!(filter.borrow().by, FilterBy::Kind, "the stored type falls back to the first offered");
    let select = fixture.by_label("Filter type").expect("the type select is shown");
    fixture.click(&select);
    fixture.idle(3);
    assert!(fixture.by_label("Name").is_some(), "Name is offered");
    for hidden in ["Mode", "Effect", "Artboard", "Group"] {
        assert!(fixture.by_label(hidden).is_none(), "{hidden} is not offered");
    }
    let rows = BOARDS.iter().filter(|label| fixture.by_label(label).is_some()).count();
    assert_eq!(rows, BOARDS.len(), "the fallback Kind filter is empty, so every layer is listed");
}
