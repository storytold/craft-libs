//! `LayersPanel` filters: Artboard and Group keep each group open or closed, the app answers what only it knows, and
//! Find Layers puts the caret in the Name field.

use craft_behavior::SelectionChange;
use craft_layers::ui::filter::{EffectKind, FilterBy, FilterKind, FilterObject, FilterState, LayerAttribute, LayerFilter, SmartKind, SmartSource};
use craft_layers::ui::mock::FakeLayers;
use craft_layers::ui::panel::{LayersEvent, LayersPanel};
use craft_layers::ui::tree::{LayerInfo, LayerKind, LayerLocks, LayerPart, LayerSelection};
use craft_ui::contract::{Bind, Tracked};
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
    tree: Shared<Tracked<FakeLayers>>,
    filter: Shared<LayerFilter>,
    find: Rc<Cell<bool>>,
}

/// The rig, its filter set by `start`.
fn rig(start: impl FnOnce(&mut LayerFilter)) -> Rig {
    let tree = Rc::new(RefCell::new(Tracked::new(FakeLayers::boards())));
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

/// The layers of the matching table, top first: a type layer, a group, two pixel layers (the lower one hidden and with
/// a stroke, Multiply), a Normal group, then a path with effects, a locked shape, a hidden image, a linked smart object
/// and a hidden guide (their blend key matches no Mode case).
fn matching_layers() -> Vec<LayerInfo> {
    let mut notes = LayerInfo::new("ink notes", LayerKind::Text);
    notes.blend_key = String::from("normal");
    let mut sheets = LayerInfo::new("Sheets", LayerKind::Group);
    sheets.blend_key = String::from("pass_through");
    let mut ink = LayerInfo::new("Ink", LayerKind::Pixel);
    ink.blend_key = String::from("multiply");
    ink.effect_kinds = vec![EffectKind::Stroke];
    let mut paper = LayerInfo::new("Paper", LayerKind::Pixel);
    paper.blend_key = String::from("normal");
    paper.visible = false;
    let mut normal = LayerInfo::new("Normal group", LayerKind::Group);
    normal.blend_key = String::from("normal");
    let other = |name: &str, kind: LayerKind| {
        let mut info = LayerInfo::new(name, kind);
        info.blend_key = String::from("other");
        info
    };
    let mut curve = other("Curve", LayerKind::Path);
    curve.has_effects = true;
    let mut boxed = other("Box", LayerKind::Shape);
    let mut locks = LayerLocks::default();
    locks.all = true;
    boxed.locks = locks;
    let mut photo = other("Photo", LayerKind::Image);
    photo.visible = false;
    let mut placed = other("Placed Logo", LayerKind::SmartObject);
    placed.smart = Some(SmartSource::Linked);
    let mut ruler = other("Ruler", LayerKind::Guide);
    ruler.visible = false;
    vec![notes, sheets, ink, paper, normal, curve, boxed, photo, placed, ruler]
}

/// The names `filter` lists among `layers`.
fn listed<'a>(filter: &LayerFilter, layers: &'a [LayerInfo]) -> Vec<&'a str> {
    layers.iter().filter(|info| filter.shows(info)).map(|info| info.name.as_str()).collect()
}

/// Each filter type lists the layers its rule matches: a group matches no kind, a name matches in any case with the
/// spaces trimmed, a group matches by name, the Mode select offers no Pass Through, and an opposite attribute lists
/// the rest, groups included.
#[trial]
fn each_filter_type_lists_its_matches() {
    let layers = matching_layers();
    let by = |edit: &dyn Fn(&mut LayerFilter)| {
        let mut filter = LayerFilter::default();
        edit(&mut filter);
        filter
    };
    let every: Vec<&str> = layers.iter().map(|info| info.name.as_str()).collect();
    let cases: Vec<(LayerFilter, Vec<&str>, &str)> = vec![
        (by(&|f| (f.by, f.kinds) = (FilterBy::Kind, vec![FilterKind::Pixel])), vec!["Ink", "Paper"], "Kind Pixel: groups match no kind"),
        (by(&|f| (f.by, f.kinds) = (FilterBy::Kind, vec![FilterKind::Type])), vec!["ink notes"], "Kind Type"),
        (by(&|f| (f.by, f.name) = (FilterBy::Name, String::from(" INK "))), vec!["ink notes", "Ink"], "Name: part of it, any case, spaces trimmed"),
        (by(&|f| (f.by, f.name) = (FilterBy::Name, String::from("sheet"))), vec!["Sheets"], "Name: a group matches too"),
        (by(&|f| (f.by, f.name) = (FilterBy::Name, String::from("nothing like it"))), vec![], "Name: no match"),
        (by(&|f| (f.by, f.mode) = (FilterBy::Mode, String::from("multiply"))), vec!["Ink"], "Mode Multiply"),
        (
            by(&|f| (f.by, f.mode) = (FilterBy::Mode, String::from("normal"))),
            vec!["ink notes", "Paper", "Normal group"],
            "Mode Normal lists a Normal group and never a Pass Through one",
        ),
        (by(&|f| (f.by, f.effect) = (FilterBy::Effect, EffectKind::Stroke)), vec!["Ink"], "Effect Stroke"),
        (
            by(&|f| (f.by, f.attribute) = (FilterBy::Attribute, LayerAttribute::NotVisible)),
            vec![],
            "Attribute Not Visible lists what Visible does not",
        ),
        (by(&|f| (f.by, f.objects) = (FilterBy::Objects, vec![FilterObject::Paths])), vec!["Curve"], "Objects: one kind"),
        (
            by(&|f| (f.by, f.objects) = (FilterBy::Objects, vec![FilterObject::Paths, FilterObject::Shapes])),
            vec!["Curve", "Box"],
            "Objects: two kinds are OR-ed",
        ),
        (by(&|f| (f.by, f.states) = (FilterBy::Objects, vec![FilterState::Locked])), vec!["Box"], "Objects: a state alone"),
        (
            by(&|f| (f.by, f.states) = (FilterBy::Objects, vec![FilterState::Hidden])),
            vec!["Paper", "Photo", "Ruler"],
            "Objects: Hidden lists the rows that are not visible",
        ),
        (
            by(&|f| (f.by, f.objects, f.states) = (FilterBy::Objects, vec![FilterObject::Images, FilterObject::Guides], vec![FilterState::Hidden])),
            vec!["Photo", "Ruler"],
            "Objects: kinds AND states",
        ),
        (
            by(&|f| (f.by, f.objects, f.states) = (FilterBy::Objects, vec![FilterObject::Paths], vec![FilterState::Locked])),
            vec![],
            "Objects: no row is both",
        ),
        (
            by(&|f| (f.by, f.objects, f.name) = (FilterBy::Objects, vec![FilterObject::Paths, FilterObject::Shapes], String::from(" BO "))),
            vec!["Box"],
            "Objects: the name AND-ed with the kinds",
        ),
        (by(&|f| f.by = FilterBy::Objects), every.clone(), "Objects: an empty selection lists all"),
        (by(&|f| (f.by, f.objects) = (FilterBy::Objects, vec![FilterObject::Guides])), vec!["Ruler"], "Objects: Guides"),
        (by(&|f| (f.by, f.objects) = (FilterBy::Objects, vec![FilterObject::LinkedFiles])), vec!["Placed Logo"], "Objects: Linked Files"),
        (by(&|f| (f.by, f.objects) = (FilterBy::Objects, vec![FilterObject::Effects])), vec!["Curve"], "Objects: Effects"),
        (by(&|f| (f.by, f.objects) = (FilterBy::Objects, vec![FilterObject::Groups])), vec!["Sheets", "Normal group"], "Objects: Groups"),
    ];
    for (filter, want, case) in cases {
        if filter.attribute == LayerAttribute::NotVisible && filter.by == FilterBy::Attribute {
            let visible = by(&|f| (f.by, f.attribute) = (FilterBy::Attribute, LayerAttribute::Visible));
            let shown = listed(&visible, &layers);
            let rest: Vec<&str> = layers.iter().map(|info| info.name.as_str()).filter(|name| !shown.contains(name)).collect();
            assert_eq!(listed(&filter, &layers), rest, "{case}");
            assert!(rest.contains(&"Paper"), "{case}: the hidden layer is listed");
            continue;
        }
        assert_eq!(listed(&filter, &layers), want, "{case}");
    }
}

/// With Objects on Paths, a closed group holding a path is listed at its own depth above the path, and a group holding
/// none is not; a Kind filter lists no holder.
#[trial]
fn an_object_filter_keeps_the_rows_holding_a_match_at_their_depth() {
    let mut rig = rig(|filter| (filter.by, filter.objects) = (FilterBy::Objects, vec![FilterObject::Paths]));
    let (oak, trees) = (rig.id("Oak"), rig.id("Trees"));
    rig.tree.borrow_mut().layer_mut(oak).info.kind = LayerKind::Path;
    rig.tree.borrow_mut().layer_mut(trees).open = false;
    rig.fixture.idle(3);
    assert_eq!(rig.rows(), ["Cover", "Trees", "Oak"], "the holders and the path; Shapes holds none");
    let level = |rig: &Rig, label: &str| {
        rig.fixture.tree().level_of(rig.fixture.by_label(label).expect("the row is shown").node).expect("a tree row has a level")
    };
    let levels = ["Cover", "Trees", "Oak"].map(|label| level(&rig, label));
    assert!(levels[0] < levels[1] && levels[1] < levels[2], "each holder sits at its own depth: {levels:?}");

    rig.filter_by(|filter| (filter.by, filter.kinds) = (FilterBy::Kind, vec![FilterKind::Pixel]));
    assert_eq!(rig.rows(), ["Pine", "Grass", "Loose", "Background"], "a Kind filter lists no holder");
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
    let tree = Rc::new(RefCell::new(Tracked::new(FakeLayers::boards())));
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

/// #10879: a filter the bar applies deselects the selected layers it hides, keeps the listed ones (the active one last,
/// so it stays active) and clears the selection when none stays listed; the list picks no row in place of a hidden one.
#[trial]
fn a_filter_the_bar_applies_deselects_what_it_hides() {
    let mut tree = FakeLayers::sample();
    tree.selection = LayerSelection::new(vec![3, 2, 4], Some(3), LayerPart::Pixels);
    let tree = Rc::new(RefCell::new(Tracked::new(tree)));
    let filter = Rc::new(RefCell::new(LayerFilter::default()));
    let find = Rc::new(Cell::new(false));
    let seen: Shared<Vec<LayersEvent<u32>>> = Rc::default();
    let (shown, by, asked, sink) = (Rc::clone(&tree), Rc::clone(&filter), Rc::clone(&find), Rc::clone(&seen));
    let mut fixture = Fixture::new(move |ui| {
        let tree = shown.borrow();
        let mut filter = by.borrow_mut();
        let mut panel = LayersPanel::new(&*tree).filter(Bind::bound(&mut *filter));
        if asked.replace(false) {
            panel = panel.focus_filter_name();
        }
        sink.borrow_mut().extend(panel.show(ui).events);
    });
    fixture.room(ONE_PANEL);
    fixture.settle();
    let selects = |seen: &Shared<Vec<LayersEvent<u32>>>| -> Vec<SelectionChange<u32>> {
        seen.borrow_mut().drain(..).filter_map(|event| if let LayersEvent::Select(select) = event { Some(select.change) } else { None }).collect()
    };
    selects(&seen);

    find.set(true);
    fixture.idle(2);
    fixture.type_text("ed");
    fixture.idle(2);
    let changes = selects(&seen);
    assert!(!changes.is_empty(), "the filter that hides Title deselects it");
    assert!(changes.iter().all(|change| *change == SelectionChange::Set(vec![2, 3])), "Clipped and Masked stay, Masked active: {changes:?}");

    tree.borrow_mut().selection = LayerSelection::new(vec![2, 3], Some(3), LayerPart::Pixels);
    find.set(true);
    fixture.idle(2);
    fixture.type_text("zz");
    fixture.idle(2);
    let changes = selects(&seen);
    assert!(!changes.is_empty() && changes.iter().all(|change| *change == SelectionChange::Clear), "none stays listed: {changes:?}");

    tree.borrow_mut().selection = LayerSelection::new(vec![4], Some(4), LayerPart::Pixels);
    fixture.idle(2);
    assert!(selects(&seen).is_empty(), "a layer selected later, while the filter hides it, stays selected");
}

/// Where the Objects filter's Filter button ends under `search`.
fn filter_button_end(search: craft_layers::ui::filter::ObjectsSearch) -> f32 {
    let mut fixture = Fixture::new(move |ui| {
        let mut filter = LayerFilter::default();
        filter.by = FilterBy::Objects;
        let _ = craft_layers::ui::filter::LayerFilterBar::new(Bind::bound(&mut filter)).filter_types([FilterBy::Objects]).show(ui);
    });
    fixture.root_value(search).room(ONE_PANEL).settle();
    fixture.by_label("Filter").expect("the Objects filter shows its Filter button").rect.right()
}

/// An app whose Objects search fills the bar (`ObjectsSearch::Fill`, VectorCraft's Layers panel) gets its Filter
/// button further toward the end edge than the field's own width leaves it.
#[trial]
fn a_filling_objects_search_puts_the_filter_button_at_the_end() {
    use craft_layers::ui::filter::ObjectsSearch;
    let (natural, fill) = (filter_button_end(ObjectsSearch::Natural), filter_button_end(ObjectsSearch::Fill));
    assert!(fill > natural, "the Filter button ends at {fill} under Fill, {natural} under Natural");
}
