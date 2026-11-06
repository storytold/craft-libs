use egui::{Color32, Context};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use egui_kittest::Harness;

use super::*;

const SECTIONS: Sections<'static> = Sections { recent: "Recent", all: "All" };
const STYLE: RowStyle = RowStyle { fill: Color32::GRAY, accent: Color32::BLUE };
const SWATCH: SwatchStyle = SwatchStyle { accent: Color32::BLUE, border: Color32::BLACK };

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn a_noted_hover_is_read_once_in_its_pass() {
    let mut h = Harness::builder().build_ui_state(
        |ui, seen: &mut Vec<Option<String>>| {
            seen.clear();
            note_hover(ui.ctx(), "a".into());
            note_hover(ui.ctx(), "b".into());
            seen.push(take_hover(ui.ctx()));
            seen.push(take_hover(ui.ctx()));
        },
        Vec::new(),
    );
    h.run();
    assert_eq!(h.state(), &vec![Some("b".to_string()), None]);
}

#[test]
fn a_hover_noted_in_an_earlier_pass_is_dropped() {
    let mut h = Harness::builder().build_ui_state(
        |ui, s: &mut (bool, Option<String>)| {
            if s.0 {
                s.1 = take_hover(ui.ctx());
            } else {
                note_hover(ui.ctx(), "old".into());
            }
        },
        (false, None),
    );
    h.step();
    h.state_mut().0 = true;
    h.step();
    assert_eq!(h.state().1, None);
}

#[test]
fn the_current_pick_is_set_and_cleared() {
    let ctx = Context::default();
    assert_eq!(current_pick(&ctx), None);
    set_current_pick(&ctx, Some("FF0000".into()));
    assert_eq!(current_pick(&ctx).as_deref(), Some("FF0000"));
    set_current_pick(&ctx, None);
    assert_eq!(current_pick(&ctx), None);
}

#[test]
fn hex_is_uppercase_rrggbb() {
    assert_eq!(hex(Color32::from_rgb(0, 176, 240)), "00B0F0");
}

#[test]
fn section_rows_put_headings_around_the_recent_entries() {
    let items = names(&["A", "B", "C"]);
    let recent = names(&["B"]);
    let item = |k: usize| items.get(k);
    assert_eq!(section_rows(0, 3), 3);
    assert_eq!(section_rows(1, 3), 6);
    let rows: Vec<_> = (0..6).map(|k| section_row(k, &recent, SECTIONS, &item)).collect();
    assert_eq!(
        rows,
        vec![
            Some(Row::Heading("Recent")),
            Some(Row::Item(&recent[0])),
            Some(Row::Heading("All")),
            Some(Row::Item(&items[0])),
            Some(Row::Item(&items[1])),
            Some(Row::Item(&items[2])),
        ]
    );
    assert_eq!(section_row(1, &[], SECTIONS, &item), Some(Row::Item(&items[1])));
}

#[test]
fn highlight_row_skips_the_headings() {
    assert_eq!(highlight_row(3, 0), 3);
    assert_eq!(highlight_row(0, 2), 1);
    assert_eq!(highlight_row(1, 2), 2);
    assert_eq!(highlight_row(2, 2), 4);
}

struct Rows {
    items: Vec<String>,
    recent: Vec<String>,
    current: String,
    picked: Option<String>,
    hovered: Option<String>,
}

fn rows_harness(items: &[&str], recent: &[&str], current: &str) -> Harness<'static, Rows> {
    Harness::builder().with_size(egui::vec2(300.0, 400.0)).build_ui_state(
        |ui, s: &mut Rows| {
            let items = s.items.clone();
            let recent = s.recent.clone();
            let item = |k: usize| items.get(k);
            let row = |k: usize| section_row(k, &recent, SECTIONS, &item);
            let opts = PickRows {
                width: 120.0,
                current: &s.current,
                range: 0..section_rows(recent.len(), items.len()),
                row: &row,
                highlight: None,
                preview: None,
                row_height: 20.0,
                recent_rows: recent.len(),
                style: STYLE,
            };
            let _ = take_hover(ui.ctx());
            if let Some(p) = pick_rows(ui, opts) {
                s.picked = Some(p);
            }
            s.hovered = take_hover(ui.ctx());
        },
        Rows { items: names(items), recent: names(recent), current: current.into(), picked: None, hovered: None },
    )
}

#[test]
fn a_click_returns_the_entry_and_a_hover_notes_it() {
    let mut h = rows_harness(&["Arial", "Calibri"], &[], "Arial");
    h.run();
    h.get_by_label("Calibri").hover();
    h.run();
    assert_eq!(h.state().hovered.as_deref(), Some("Calibri"));
    h.get_by_label("Calibri").click();
    h.run();
    assert_eq!(h.state().picked.as_deref(), Some("Calibri"));
}

#[test]
fn headings_are_drawn_and_do_not_pick() {
    let mut h = rows_harness(&["Arial", "Calibri"], &["Calibri"], "Arial");
    h.run();
    h.get_by_label("Recent").click();
    h.get_by_label("All").click();
    h.run();
    assert_eq!(h.state().picked, None);
}

#[test]
fn the_current_entry_is_selected_in_a_plain_list() {
    let mut h = rows_harness(&["Arial", "Calibri"], &[], "Calibri");
    h.run();
    assert_eq!(h.get_by_label("Calibri").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    assert_ne!(h.get_by_label("Arial").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
}

#[test]
fn a_current_entry_in_recent_is_not_selected_again_in_all() {
    let mut h = rows_harness(&["Arial", "Calibri"], &["Calibri"], "Calibri");
    h.run();
    let marked = h.query_all_by_label("Calibri").filter(|node| node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)).count();
    assert_eq!(h.query_all_by_label("Calibri").count(), 2);
    assert_eq!(marked, 1, "the current entry is marked only in Recent");
}

#[test]
fn frozen_recent_holds_its_order_after_a_pick_until_the_menu_closes() {
    let ctx = Context::default();
    let frozen = FrozenRecent::new(Id::unique("frozen"));
    let before = names(&["A", "B"]);
    assert_eq!(frozen.order(&ctx, &before), before);
    frozen.settle(&ctx, true, &before);
    let after = names(&["B", "A"]);
    assert_eq!(frozen.order(&ctx, &after), before, "a pick keeps the drawn order");
    frozen.settle(&ctx, false, &before);
    assert_eq!(frozen.order(&ctx, &after), after, "a frame without a pick thaws it");
}

#[test]
fn a_swatch_returns_a_click_and_notes_its_colour() {
    let mut h = Harness::builder().with_size(egui::vec2(100.0, 60.0)).build_ui_state(
        |ui, s: &mut (bool, Option<String>)| {
            let _ = take_hover(ui.ctx());
            s.0 |= swatch(ui, Color32::from_rgb(255, 192, 0), SWATCH);
            s.1 = take_hover(ui.ctx());
        },
        (false, None),
    );
    h.run();
    h.hover_at(egui::pos2(14.0, 14.0));
    h.run();
    assert_eq!(h.state().1.as_deref(), Some("FFC000"));
    click(&mut h, egui::pos2(14.0, 14.0));
    h.run();
    assert!(h.state().0);
}

fn click(h: &mut Harness<'_, (bool, Option<String>)>, pos: egui::Pos2) {
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.run();
    }
}

struct MenuState {
    picked: Option<String>,
    open: bool,
}

#[test]
fn a_pick_in_a_menu_closes_it() {
    let mut h = Harness::builder().with_size(egui::vec2(300.0, 400.0)).build_ui_state(
        |ui, s: &mut MenuState| {
            let field = ui.button("Font");
            let items = names(&["Arial", "Calibri"]);
            let item = |k: usize| items.get(k);
            let row = |k: usize| section_row(k, &[], SECTIONS, &item);
            let picked = crate::combo::Menu::new(ui, &field).show_ui(|ui| {
                let opts = PickRows {
                    width: 80.0,
                    current: "Arial",
                    range: 0..2,
                    row: &row,
                    highlight: None,
                    preview: None,
                    row_height: 20.0,
                    recent_rows: 0,
                    style: STYLE,
                };
                pick_rows_in_menu(ui, opts)
            });
            s.picked = picked.flatten().or(s.picked.take());
            s.open = egui::Popup::is_any_open(ui.ctx());
        },
        MenuState { picked: None, open: false },
    );
    h.run();
    h.get_by_label("Font").click();
    h.run();
    assert!(h.state().open);
    h.get_by_label("Calibri").click();
    h.run();
    h.run();
    assert_eq!(h.state().picked.as_deref(), Some("Calibri"));
    assert!(!h.state().open, "the menu closes on a pick");
}
