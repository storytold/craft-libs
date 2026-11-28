//! An open menu runs a constant number of components per row: 80 rows run 40 rows' worth more than 40 rows, in the room the
//! `menu-80-rows` scenario of ui-bench gives its menu.

use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::testing::{Fixture, last_pass_runs, trial};
use egui::vec2;

/// The room of ui-bench's `MenuScene`.
const ROOM: egui::Vec2 = vec2(800.0, 1400.0);

/// The components run in the last pass, and how many of the `rows` have a node.
fn runs_and_rows_shown(rows: usize) -> (u32, usize) {
    let mut fixture = Fixture::new(move |ui| {
        let mut open = true;
        let _ = Menu::new("Items").open(Bind::bound(&mut open)).id_salt("menu").show(ui, |ui, _| {
            for key in 0..rows {
                let _ = MenuItem::new(format!("Item {key}")).id_salt(key).show(ui);
            }
        });
    });
    fixture.room(ROOM);
    fixture.settle().idle(2);
    let runs = last_pass_runs(fixture.ctx()).iter().map(|(_, runs)| runs).sum();
    let shown = (0..rows).filter(|key| fixture.by_label(&format!("Item {key}")).is_some()).count();
    (runs, shown)
}

#[trial]
fn component_runs_grow_by_a_constant_per_menu_row() {
    let (few, few_shown) = runs_and_rows_shown(40);
    let (many, many_shown) = runs_and_rows_shown(80);
    eprintln!("menu runs: 40 rows {few} runs ({few_shown} shown), 80 rows {many} runs ({many_shown} shown)");
    assert_eq!((few_shown, many_shown), (40, 80), "every row has a node in the bench's room, so runs per row means every row");
    let grown = many.checked_sub(few).expect("more rows run no fewer components");
    assert_eq!(grown % 40, 0, "runs grow by a constant per row: {few} runs at 40 rows, {many} at 80");
    let k = grown / 40;
    assert!(k >= 1, "each row runs at least one component");
    eprintln!("runs per menu row: {k}");
}
