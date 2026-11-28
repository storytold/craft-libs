//! `Sizing::FillHeight` in a row given its height: the child takes its content's width and the row's height, so a side
//! menu beside a page is as tall as the page with its footer at the bottom, and the page beside it takes the rest of
//! the width (the File page's menu, as tall as the window, About at its foot).

use craft_ui::buttons::Button;
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

const SNAP: f32 = 1.0;

#[trial]
fn a_fill_height_menu_takes_its_content_width_and_the_rows_height() {
    // The row, the menu, the menu's footer button, its widest item and the page, from the last real run.
    let seen = Rc::new(Cell::new([Rect::NOTHING; 5]));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let out = Rc::clone(&out);
        let parts = Cell::new([Rect::NOTHING; 4]);
        let row = Row::new().fill().show(ui, |ui, _| {
            let footer = Cell::new(Rect::NOTHING);
            let widest = Cell::new(Rect::NOTHING);
            let menu = Stack::new().sizing(Sizing::FillHeight).footer(|ui, _| footer.set(Button::new("About").show(ui).rect)).show(ui, |ui, _| {
                Button::new("Home").show(ui);
                widest.set(Button::new("Save As a copy").show(ui).rect);
            });
            let page = Stack::new().fill().show(ui, |ui, _| {
                Button::new("Browse").show(ui);
            });
            parts.set([menu.output.rect, footer.get(), widest.get(), page.output.rect]);
        });
        if !ui.is_sizing_pass() {
            let [menu, footer, widest, page] = parts.get();
            out.set([row.output.rect, menu, footer, widest, page]);
        }
    });
    fixture.room(Vec2::new(600.0, 400.0)).settle();
    let [row, menu, footer, widest, page] = seen.get();
    assert!(row != Rect::NOTHING, "the row showed");
    assert!(row.height() > 300.0, "the row fills the room: {row:?}");
    assert!((menu.top() - row.top()).abs() <= SNAP && (menu.bottom() - row.bottom()).abs() <= SNAP, "the menu {menu:?} spans the row {row:?}");
    assert!(menu.width() <= widest.width() + 40.0, "the menu {menu:?} is about as wide as its widest item {widest:?}");
    assert!(menu.width() < row.width() / 2.0, "the menu takes no share of the width: {menu:?} in {row:?}");
    assert!(row.bottom() - footer.bottom() < 40.0, "the footer {footer:?} sits at the menu's foot {menu:?}");
    assert!(page.left() >= menu.right() - SNAP && (page.right() - row.right()).abs() <= SNAP, "the page {page:?} takes the rest of {row:?}");
}
