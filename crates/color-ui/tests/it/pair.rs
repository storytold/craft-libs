//! `SwatchPair` wells take the theme's well shape (`shape.wells`): a round well is round to the pointer too.

use std::cell::Cell;
use std::rc::Rc;

use craft_color_ui::ui::{PairWell, SwatchPair, Well};
use craft_theme::{Srgba, ThemeId};
use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Vec2, vec2};

/// Where a click lands on the primary well in theme `theme`: at the well's middle and just inside the corner of its
/// square. Each is the well the click reached, if any.
fn clicks(theme: &str) -> [Option<Well>; 2] {
    let clicked = Rc::new(Cell::new(None));
    let seen = clicked.clone();
    let mut fixture = Fixture::new(move |ui| {
        let wells = [PairWell::new("Foreground colour", Srgba::rgb(0.1, 0.3, 0.8)), PairWell::new("Background colour", Srgba::rgb(1.0, 1.0, 1.0))];
        let shown = SwatchPair::new("Colours", wells, Bind::given(Well::Primary)).popover(false).show(ui);
        if shown.clicked.is_some() {
            seen.set(shown.clicked);
        }
    });
    fixture.theme_id(ThemeId::parse(theme).expect("a theme id")).room(vec2(200.0, 200.0));
    fixture.settle();
    let well = fixture.by_role(Role::RADIO_BUTTON).first().expect("the primary well").rect;
    [well.center(), well.min + Vec2::splat(1.5)].map(|at| {
        clicked.set(None);
        fixture.click_at(at);
        fixture.settle();
        clicked.get()
    })
}

/// Adwaita's wells are round: a click in the corner of a well's square misses it. Pro's are square: it hits.
#[trial]
fn the_well_corner_follows_the_theme_well_shape() {
    assert_eq!(clicks("craft.adwaita"), [Some(Well::Primary), None], "round wells");
    assert_eq!(clicks("craft.pro"), [Some(Well::Primary), Some(Well::Primary)], "square wells");
}

/// Each well is named by its own label, not the pair's: a screen reader, and a test, tells the two apart.
#[trial]
fn each_well_is_named_by_its_own_label() {
    let mut fixture = Fixture::new(|ui| {
        let wells = [PairWell::new("Fill", Srgba::rgb(0.1, 0.3, 0.8)), PairWell::new("Stroke", Srgba::rgb(1.0, 1.0, 1.0))];
        SwatchPair::new("Fill and Stroke", wells, Bind::given(Well::Primary)).popover(false).show(ui);
    });
    fixture.room(vec2(200.0, 200.0)).settle();
    let wells = fixture.by_role(Role::RADIO_BUTTON);
    let names: Vec<Option<&str>> = wells.iter().map(|well| fixture.tree().name_of(well.node)).collect();
    assert_eq!(names, vec![Some("Fill"), Some("Stroke")], "the two wells are radio buttons named by their own labels, the primary first");
    let fill = fixture.by_label("Fill").expect("a node is named Fill");
    let stroke = fixture.by_label("Stroke").expect("a node is named Stroke");
    assert_eq!(fill.node, wells[0].node, "Fill names the primary well's radio button");
    assert_eq!(stroke.node, wells[1].node, "Stroke names the secondary well's radio button");
}
