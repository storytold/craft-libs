//! `LayerCompsPanel`: reordering keeps Last Document State first, and a rename left open does not carry over to another
//! document.

use craft_layers::ui::comps::{LayerCompRow, LayerCompsEvent, LayerCompsPanel};
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The room of the panel: wide enough for the columns, tall enough for the rows and the footer.
const PANEL: Vec2 = Vec2::new(420.0, 400.0);

type Seen = Rc<RefCell<Vec<LayerCompsEvent>>>;

/// The panel over the comps of the document the trial sets, with the events it reports.
struct Rig {
    fixture: Fixture,
    seen: Seen,
    document: Rc<Cell<u32>>,
}

/// Two comps with ids 1 and 2, named by `names`: every document restarts its ids.
fn comps(names: [&str; 2]) -> Vec<LayerCompRow> {
    names.iter().zip(1_u64..).map(|(name, id)| LayerCompRow::new(id, *name)).collect()
}

fn rig() -> Rig {
    let seen: Seen = Rc::default();
    let document = Rc::new(Cell::new(0_u32));
    let selected = Rc::new(RefCell::new(None::<u64>));
    let (sink, shown, choice) = (Rc::clone(&seen), Rc::clone(&document), Rc::clone(&selected));
    let mut fixture = Fixture::new(move |ui| {
        let rows = if shown.get() == 0 { comps(["Alpha", "Beta"]) } else { comps(["Gamma", "Delta"]) };
        let mut chosen = choice.borrow_mut();
        let out = LayerCompsPanel::new("Layer Comps", &rows, Bind::bound(&mut *chosen)).document(shown.get()).show(ui);
        sink.borrow_mut().extend(out.events);
    });
    fixture.room(PANEL);
    fixture.settle();
    Rig { fixture, seen, document }
}

impl Rig {
    /// The reorders reported since the last call, as comp and index.
    fn reorders(&self) -> Vec<(u64, usize)> {
        self.seen
            .borrow_mut()
            .drain(..)
            .filter_map(|event| if let LayerCompsEvent::Reorder { comp, index, .. } = event { Some((comp, index)) } else { None })
            .collect()
    }
}

/// A drag reports the comp and its index among the comps, which leave Last Document State out: before the first comp is
/// index 0 and so is before Last Document State, and a drag moves nothing and applies nothing.
#[trial]
fn dragging_a_comp_row_reorders_without_applying_it() {
    let mut rig = rig();
    let lds = rig.fixture.by_label("Last Document State").expect("the fixed row shows");
    let alpha = rig.fixture.by_label("Alpha").expect("the first comp shows");
    let beta = rig.fixture.by_label("Beta").expect("the second comp shows");
    assert!(lds.rect.top() < alpha.rect.top() && alpha.rect.top() < beta.rect.top(), "Last Document State is the first row");
    let step = beta.rect.top() - alpha.rect.top();

    rig.fixture.drag(&beta, Vec2::new(0.0, alpha.rect.center().y - beta.rect.center().y - step * 0.25));
    rig.fixture.idle(3);
    assert!(!rig.seen.borrow().iter().any(|event| matches!(event, LayerCompsEvent::Apply(_))), "a drag applies nothing");
    assert_eq!(rig.reorders(), [(2, 0)], "Beta above Alpha is the first comp");

    rig.fixture.drag(&alpha, Vec2::new(0.0, beta.rect.center().y - alpha.rect.center().y + step * 0.25));
    rig.fixture.idle(3);
    assert_eq!(rig.reorders(), [(1, 1)], "Alpha below Beta is the second comp");

    rig.fixture.drag(&beta, Vec2::new(0.0, lds.rect.center().y - beta.rect.center().y - step * 0.25));
    rig.fixture.idle(3);
    assert_eq!(rig.reorders(), [(2, 0)], "above Last Document State is index 0");
    let lds = rig.fixture.by_label("Last Document State").expect("the fixed row still shows");
    let alpha = rig.fixture.by_label("Alpha").expect("the comp still shows");
    assert!(lds.rect.top() < alpha.rect.top(), "Last Document State stays the first row");
}

/// Comp ids restart in every document, so the rename editor of comp 1 in one document is not the editor of comp 1 in
/// another.
#[trial]
fn a_rename_left_open_does_not_carry_over_to_another_document() {
    let mut rig = rig();
    let alpha = rig.fixture.by_label("Alpha").expect("the first comp shows");
    rig.fixture.click(&alpha);
    rig.fixture.idle(2);
    rig.fixture.key(Key::F2, Modifiers::NONE);
    rig.fixture.idle(2);
    rig.fixture.type_text("X");
    rig.fixture.key(Key::Enter, Modifiers::NONE);
    rig.fixture.idle(2);
    let renames = |seen: &Seen| -> Vec<(u64, String)> {
        seen.borrow_mut()
            .drain(..)
            .filter_map(|event| if let LayerCompsEvent::Rename { comp, name, .. } = event { Some((comp, name)) } else { None })
            .collect()
    };
    let first = renames(&rig.seen);
    assert!(matches!(first.as_slice(), [(1, name)] if name.contains('X')), "F2 opens the editor of the selected comp: {first:?}");

    rig.fixture.key(Key::F2, Modifiers::NONE);
    rig.fixture.idle(2);
    rig.document.set(1);
    rig.fixture.idle(3);
    assert!(rig.fixture.by_label("Gamma").is_some(), "the other document's comps show");
    rig.fixture.type_text("Y");
    rig.fixture.key(Key::Enter, Modifiers::NONE);
    rig.fixture.idle(2);
    let second = renames(&rig.seen);
    assert!(second.is_empty(), "the editor of the first document does not open on the second's comp 1: {second:?}");

    rig.document.set(0);
    rig.fixture.idle(3);
    rig.fixture.key(Key::Enter, Modifiers::NONE);
    rig.fixture.idle(2);
    let back = renames(&rig.seen);
    assert!(
        back.iter().all(|(_, name)| !name.contains('Y') && !name.starts_with("Gamma") && !name.starts_with("Delta")),
        "a commit back in the first document reports nothing of the second's rows: {back:?}"
    );
}
