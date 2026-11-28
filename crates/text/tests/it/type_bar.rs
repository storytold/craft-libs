//! #12061: the type bar's orientation toggle. The host says whether it has a text layer to convert, and a greyed
//! toggle takes no click; a host that says nothing keeps the toggle it always had.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_text::ui::character::{CharacterValues, ParagraphValues};
use craft_text::ui::type_bar::TypeBar;
use craft_text::ui::type_bar::TypeBarChange;
use craft_text::ui::type_options::TextOrientation;
use craft_ui::testing::{Fixture, trial};

/// The changes the edits carry, in order.
fn changes(edits: &RefCell<Vec<EditEvent<TypeBarChange>>>) -> Vec<TypeBarChange> {
    edits.borrow().iter().filter_map(|edit| edit.value().cloned()).collect()
}

/// The orientation changes of this pass.
fn orientations(edits: &RefCell<Vec<EditEvent<TypeBarChange>>>) -> Vec<TypeBarChange> {
    changes(edits).into_iter().filter(|change| matches!(change, TypeBarChange::Orientation(_))).collect()
}

/// A bar over empty values with a `target` orientation, saying whether it has a text target, collecting its edits.
fn bar(target: TextOrientation, has_text_target: bool) -> (Fixture, Rc<RefCell<Vec<EditEvent<TypeBarChange>>>>) {
    let edits = Rc::new(RefCell::new(Vec::new()));
    let sink = edits.clone();
    let fixture = Fixture::new(move |ui| {
        let (character, paragraph, families, styles) =
            (CharacterValues::default(), ParagraphValues::default(), Vec::<String>::new(), Vec::<String>::new());
        sink.borrow_mut().extend(bar_of(&character, &paragraph, &families, &styles, target, has_text_target).show(ui).edits);
    });
    (fixture, edits)
}

fn bar_of<'a>(
    character: &'a CharacterValues,
    paragraph: &'a ParagraphValues,
    families: &'a [String],
    styles: &'a [String],
    target: TextOrientation,
    has_text_target: bool,
) -> TypeBar<'a> {
    let bar = TypeBar::new(character, paragraph, families, styles).orientation(target);
    if has_text_target { bar } else { bar.can_orient(false) }
}

/// The toggle's access node: the bar names it, so the greyed node is the greyed state a reader hears.
fn toggle(fixture: &mut Fixture) -> craft_ui::testing::Target {
    fixture.idle(4);
    fixture.by_label("Toggle text orientation").expect("the bar shows the orientation toggle")
}

/// A host without a text layer greys the toggle, and its click changes nothing.
#[trial]
fn a_bar_without_a_text_target_greys_the_toggle_and_its_click_changes_nothing() {
    let (mut fixture, edits) = bar(TextOrientation::Horizontal, false);
    let target = toggle(&mut fixture);
    assert!(fixture.tree().is_disabled_key(target.node), "the toggle is greyed when the host has no text target");
    fixture.click(&target).idle(4);
    assert!(orientations(&edits).is_empty(), "a click on the greyed toggle asks for no orientation: {:?}", changes(&edits));
}

/// A host that says nothing keeps the clickable toggle it always had, and its click still converts.
#[trial]
fn the_bar_of_a_host_that_says_nothing_keeps_the_toggle_clickable() {
    let (mut fixture, edits) = bar(TextOrientation::Horizontal, true);
    let target = toggle(&mut fixture);
    assert!(!fixture.tree().is_disabled_key(target.node), "a host that says nothing keeps the toggle live");
    fixture.click(&target).idle(4);
    assert_eq!(orientations(&edits), vec![TypeBarChange::Orientation(TextOrientation::Vertical)], "its click asks for the other orientation");
}

/// The toggle of a vertical target is greyed by the same word and takes no click.
#[trial]
fn a_vertical_target_is_greyed_by_the_same_word() {
    let (mut fixture, edits) = bar(TextOrientation::Vertical, false);
    let target = toggle(&mut fixture);
    assert!(fixture.tree().is_disabled_key(target.node), "the vertical bar is greyed by the same word");
    fixture.click(&target).idle(4);
    assert!(orientations(&edits).is_empty(), "the greyed vertical toggle asks for nothing: {:?}", changes(&edits));
}
