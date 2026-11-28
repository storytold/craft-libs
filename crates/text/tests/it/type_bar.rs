//! #12081: the type bar's colour well of a host whose target has no colour to change. The host names a glyph for the
//! well, which then shows that glyph and reports no colour; a host that names nothing keeps the colour it always
//! showed. Either well still files its colour request when it is clicked: what the host makes of that answer is its own.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::IconId;
use craft_text::ui::character::{CharacterValues, ParagraphValues};
use craft_text::ui::type_bar::TypeBar;
use craft_theme::Rgba;
use craft_ui::contract::Role;
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::testing::{Fixture, trial};

/// The colour the values hold, the same in every trial so only the well's state differs.
const INK: Rgba = Rgba([51, 102, 153, 255]);

/// The bar over `color`, naming `glyph` for its well when the host has no colour to show, collecting the colour
/// requests that well files.
fn bar(color: Option<Rgba>, glyph: Option<IconId>) -> (Fixture, Rc<RefCell<Vec<ColorRequest>>>) {
    let asked = Rc::new(RefCell::new(Vec::new()));
    let sink = asked.clone();
    let fixture = Fixture::new(move |ui| {
        let character = CharacterValues { color, ..CharacterValues::default() };
        let (paragraph, families, styles) = (ParagraphValues::default(), Vec::<String>::new(), Vec::<String>::new());
        let bar = TypeBar::new(&character, &paragraph, &families, &styles);
        let bar = match glyph.clone() {
            Some(glyph) => bar.color_placeholder(glyph),
            None => bar,
        };
        if let Some(request) = bar.show(ui).requests.take_color() {
            sink.borrow_mut().push(request);
        }
    });
    (fixture, asked)
}

/// The bar's colour well, named as the bar names it. The bar is a long row, so the trial gives it a wide room: the
/// well sits after the alignment buttons and the direction buttons.
fn well(fixture: &mut Fixture) -> craft_ui::testing::Target {
    fixture.room(egui::vec2(1600.0, 200.0)).idle(4);
    fixture.by_label("Text color").expect("the bar shows the colour well")
}

/// A host that names no glyph keeps today's well: a colour well whose value is the colour it shows, and its click
/// opens at that colour.
#[trial]
fn the_well_of_a_host_that_names_no_glyph_shows_the_colour() {
    let (mut fixture, asked) = bar(Some(INK), None);
    let target = well(&mut fixture);
    assert_eq!(fixture.tree().role_of(target.node), Some(Role::COLOR_WELL), "the well is a colour well");
    let shown = fixture.tree().value_of(target.node).unwrap_or_default().to_string();
    assert_eq!(shown.len(), 7, "the well names its colour: {shown}");
    fixture.click(&target).idle(4);
    let asked = asked.borrow();
    assert_eq!(asked.len(), 1, "its click files one request");
    assert_eq!(asked[0].color.map(|color| color.to_hex()).as_deref(), Some(shown.as_str()), "opened at the colour the well shows");
}

/// A host that names a glyph for a target with no colour: the well shows that glyph, so it reports no colour value.
/// The glyph here is a stand-in borrowed from the catalogue (`text_cursor`); what this trial pins is the well's state,
/// not the shape of any drawing.
#[trial]
fn a_well_named_a_glyph_shows_no_colour() {
    let (mut fixture, _) = bar(Some(INK), None);
    let plain = well(&mut fixture);
    assert!(fixture.tree().value_of(plain.node).is_some(), "a well that names no glyph has a colour to read");
    let (mut fixture, _) = bar(Some(INK), Some(IconId::new("text_cursor")));
    let target = well(&mut fixture);
    assert_eq!(fixture.tree().value_of(target.node), None, "a placeholder well has no colour value");
    assert_eq!(fixture.tree().role_of(target.node), Some(Role::LIST_BOX_OPTION), "the framework's glyph cell, named as before");
    assert_eq!(fixture.tree().name_of(target.node), Some("Text color"), "the placeholder well keeps the well's name");
}

/// The placeholder well is still the bar's own well: its click files its request, which carries no colour, and the
/// host decides what a pick does. Again the glyph is the borrowed stand-in, not a claim about the mark's shape.
#[trial]
fn a_well_named_a_glyph_still_files_its_request() {
    let (mut fixture, asked) = bar(Some(INK), Some(IconId::new("text_cursor")));
    let target = well(&mut fixture);
    fixture.click(&target).idle(4);
    let asked = asked.borrow();
    assert_eq!(asked.len(), 1, "the click of a placeholder well files one request");
    assert_eq!(asked[0].color, None, "and it carries no colour, because the well shows none");
}

/// The suite's own placeholder mark is in the catalogue - an id outside it paints nothing and only logs - and a well
/// named with it reaches the same state: this pins that a catalogued name resolves and reaches the well, not what the
/// drawing looks like.
#[trial]
fn the_suites_own_placeholder_mark_reaches_the_well() {
    assert!(craft_ui::primitives::has_icon(&IconId::new("question_mark")), "the catalogue holds `question_mark`");
    let (mut fixture, asked) = bar(Some(INK), Some(IconId::new("question_mark")));
    let target = well(&mut fixture);
    assert_eq!(fixture.tree().value_of(target.node), None, "the well named with the suite's mark shows no colour value");
    fixture.click(&target).idle(4);
    assert_eq!(asked.borrow().len(), 1, "and its click still files its request");
}

/// A mixed selection keeps its mixed well and its request when the host names no glyph, as before.
#[trial]
fn a_mixed_selection_without_a_named_glyph_keeps_the_mixed_well() {
    let (mut fixture, asked) = bar(None, None);
    let target = well(&mut fixture);
    assert_eq!(fixture.tree().value_of(target.node), Some("Mixed"), "the mixed well reads Mixed as it did");
    fixture.click(&target).idle(4);
    assert_eq!(asked.borrow().len(), 1, "and its click still files its request");
}
