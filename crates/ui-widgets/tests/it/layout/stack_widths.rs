//! A content-sized `Stack`'s width roles: `least_width` keeps it at least the role's width with narrow content (a
//! File page's side menu, `LeastWidth::SideMenu`), and `wrap_width` breaks a long paragraph at the role's line length
//! instead of letting it widen the stack (a Print page's settings column, `WrapWidth::Column`).

use craft_ui::buttons::Button;
use craft_ui::contract::{LeastWidth, Show as _, ShowContainer as _, WrapWidth};
use craft_ui::display::Text;
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

/// A paragraph far longer than any column.
const LONG: &str = "Save a copy in another format. Printing goes through a PDF you can print from any viewer, and the pages you pick \
                    here are the ones that print, at the size and orientation set below.";

/// The rects of the stack and its one child from the last real run, the stack built by `stack`.
fn shown(stack: fn() -> Stack<'static>, long: bool) -> (Rect, Rect) {
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let child = Cell::new(Rect::NOTHING);
        let shown = stack().show(ui, |ui, _| {
            let rect = if long { Text::new(LONG).show(ui).rect } else { Button::new("Home").show(ui).rect };
            child.set(rect);
        });
        if !ui.is_sizing_pass() {
            out.set((shown.output.rect, child.get()));
        }
    });
    fixture.room(Vec2::new(1600.0, 600.0)).settle();
    seen.get()
}

#[trial]
fn a_stacks_least_width_holds_it_wider_than_narrow_content() {
    let (plain, button) = shown(Stack::new, false);
    let (menu, _) = shown(|| Stack::new().least_width(LeastWidth::SideMenu), false);
    assert!(plain != Rect::NOTHING && menu != Rect::NOTHING, "both stacks showed");
    assert!(plain.width() < button.width() + 2.0, "a plain stack is as wide as its button: {plain:?} for {button:?}");
    assert!(menu.width() > plain.width() + 60.0, "the side menu's least width holds it wider: {menu:?} against {plain:?}");
}

#[trial]
fn a_stacks_wrap_width_breaks_a_long_paragraph_at_the_column() {
    let (plain, one_line) = shown(Stack::new, true);
    let (column, wrapped) = shown(|| Stack::new().wrap_width(WrapWidth::Column), true);
    assert!(plain.width() > 600.0, "without a wrap width the paragraph widens the stack to the room: {plain:?}");
    assert!(column.width() < plain.width() / 2.0, "the column's wrap width keeps the stack narrow: {column:?} against {plain:?}");
    assert!(wrapped.height() > one_line.height() * 2.0, "the paragraph breaks into lines: {wrapped:?} against {one_line:?}");
}
