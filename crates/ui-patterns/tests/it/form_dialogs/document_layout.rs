//! A document dialog (`ParamLayout::Document`) lays its rows out as production's dialogs do: captions at the row start
//! with their control right after them, the preview under the rows, the buttons across the foot.

use craft_ui::contract::Bind;
use craft_ui::patterns::form_dialogs::{DialogFields, DialogLayout, Param, ParamDialog, ParamLayout};
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, vec2};

/// A dialog of two number rows, with a preview pane, in `layout`.
fn dialog(layout: ParamLayout) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let mut fields = DialogFields::new();
        let rows = vec![Param::number("pages", "Size", 1.0, 1.0, 0), Param::number("margin", "Corner radius", 2.0, 1.0, 0)];
        ParamDialog::new("document", "Split document", rows).layout(layout).previews(true).values(Bind::bound(&mut fields)).show(ui);
    });
    fixture.room(vec2(900.0, 700.0));
    fixture.settle();
    fixture
}

fn rect_of(fixture: &Fixture, name: &str) -> Rect {
    fixture.by_label(name).unwrap_or_else(|| panic!("{name:?} is shown")).rect
}

fn caption(fixture: &Fixture, text: &str) -> Rect {
    fixture.texts().iter().find(|run| run.text.trim_end_matches(':') == text).map(|run| run.rect).unwrap_or_else(|| panic!("{text:?} is painted"))
}

#[trial]
fn a_document_dialog_sets_controls_after_their_captions_and_the_buttons_across_the_foot() {
    let beside = dialog(ParamLayout::Beside);
    let document = dialog(ParamLayout::Document);
    let (short, long) = (rect_of(&document, "Size"), rect_of(&document, "Corner radius"));
    assert!(short.min.x + 0.5 < long.min.x, "the captions share no column: {short:?} {long:?}");
    let gap = short.min.x - caption(&document, "Size").max.x;
    assert!((0.0..24.0).contains(&gap), "the control follows its caption closely: {gap}");
    let ok = rect_of(&document, "OK");
    assert!(ok.min.y > long.max.y, "the buttons sit under the rows: {ok:?} {long:?}");
    let ok_beside = rect_of(&beside, "OK");
    assert!(ok_beside.min.y < rect_of(&beside, "Corner radius").max.y, "the other layouts keep the buttons beside the rows");
}

/// The rows of a section of a document dialog sit on a card inside the dialog's text column; elsewhere they do not.
#[trial]
fn a_document_dialog_shades_its_option_groups() {
    let start = |layout: ParamLayout| {
        let mut fixture = Fixture::new(move |ui| {
            let mut fields = DialogFields::new();
            let rows = vec![Param::number("size", "Size", 1.0, 1.0, 0), Param::section("Marks"), Param::number("weight", "Weight", 2.0, 1.0, 0)];
            ParamDialog::new("shaded", "Add marks", rows).layout(layout).values(Bind::bound(&mut fields)).show(ui);
        });
        fixture.room(vec2(900.0, 700.0));
        fixture.settle();
        (caption(&fixture, "Size").min.x, caption(&fixture, "Weight").min.x)
    };
    let (outer, inner) = start(ParamLayout::Document);
    assert!(inner > outer + 2.0, "the group's rows are inset on their card: {outer} {inner}");
    let (outer, inner) = start(ParamLayout::Beside);
    assert!((inner - outer).abs() < 24.0 || inner <= outer + 2.0, "a plain dialog keeps its rows: {outer} {inner}");
}

/// An app sets the layout of every dialog once, on the root's values; a dialog that names a layout keeps its own.
#[trial]
fn the_roots_dialog_layout_is_the_default_of_every_dialog() {
    let mut fixture = Fixture::new(|ui| {
        let mut fields = DialogFields::new();
        let rows = vec![Param::number("size", "Size", 1.0, 1.0, 0), Param::number("radius", "Corner radius", 2.0, 1.0, 0)];
        ParamDialog::new("default", "Marks", rows).values(Bind::bound(&mut fields)).show(ui);
    });
    fixture.root_value(DialogLayout(ParamLayout::Document));
    fixture.room(vec2(900.0, 700.0));
    fixture.settle();
    let (short, long) = (rect_of(&fixture, "Size"), rect_of(&fixture, "Corner radius"));
    assert!(short.min.x + 0.5 < long.min.x, "the root's layout applies: {short:?} {long:?}");
}

/// A document dialog writes every decimal place a number row asks for ("0.50"); other dialogs trim the zeros.
#[trial]
fn a_document_dialog_keeps_the_decimals_of_its_number_rows() {
    let shown = |layout: ParamLayout| {
        let mut fixture = Fixture::new(move |ui| {
            let mut fields = DialogFields::new();
            ParamDialog::new("decimals", "Margins", vec![Param::number("top", "Top", 0.5, 0.01, 2)])
                .layout(layout)
                .values(Bind::bound(&mut fields))
                .show(ui);
        });
        fixture.room(vec2(900.0, 700.0));
        fixture.settle();
        fixture.texts().iter().map(|run| run.text.clone()).collect::<Vec<_>>()
    };
    assert!(shown(ParamLayout::Document).iter().any(|text| text == "0.50"), "the document layout writes both places");
    assert!(!shown(ParamLayout::Beside).iter().any(|text| text == "0.50"), "the plain layout trims the zero");
}

/// A dialog that closes when unchanged shows Close in place of OK and Cancel until a value differs from the opening one.
#[trial]
fn an_unchanged_dialog_shows_close_until_a_field_changes() {
    let fields = std::rc::Rc::new(std::cell::RefCell::new(DialogFields::new()));
    let shared = fields.clone();
    let mut fixture = Fixture::new(move |ui| {
        let rows = vec![Param::number("size", "Size", 1.0, 1.0, 0)];
        let mut fields = shared.borrow_mut();
        ParamDialog::new("close", "Properties", rows).close_unchanged(true).values(Bind::bound(&mut *fields)).show(ui);
    });
    fixture.room(vec2(900.0, 700.0));
    fixture.settle();
    assert!(fixture.by_label("OK").is_none() && fixture.by_label("Cancel").is_none(), "unchanged: no OK or Cancel");
    fields.borrow_mut().set("size", 5.0_f64);
    fixture.settle();
    assert!(fixture.by_label("OK").is_some() && fixture.by_label("Cancel").is_some(), "after an edit: OK and Cancel");
}
