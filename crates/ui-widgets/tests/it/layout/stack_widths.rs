//! A content-sized `Stack`'s width roles: `least_width` keeps it at least the role's width with narrow content (a
//! File page's side menu, `LeastWidth::SideMenu`), and `wrap_width` breaks a long paragraph at the role's line length
//! instead of letting it widen the stack (a Print page's settings column, `WrapWidth::Column`), and breaks a wrapping
//! row there too.

use craft_ui::buttons::Button;
use craft_ui::choice::{Radio, RadioGroup};
use craft_ui::contract::{Bind, FieldLayout, Gap, LeastWidth, Pad, Props as _, Scroll, Show as _, ShowContainer as _, SideLabel, Sizing, WrapWidth};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
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

/// In a row beside a page that fills, a full-height side menu keeps its least width: the row places its children by
/// their measure, and that measure held only the menu's content (a File page's rail came out at its widest name).
#[trial]
fn a_side_menu_in_a_row_keeps_its_least_width() {
    let shown = |least: bool| {
        let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
        let out = Rc::clone(&seen);
        let mut fixture = Fixture::new(move |ui| {
            let (menu, page) = (Cell::new(Rect::NOTHING), Cell::new(Rect::NOTHING));
            let _ = Row::new().fill().show(ui, |ui, _| {
                let mut stack = Stack::new().sizing(Sizing::FillHeight).gap(Gap::None).scroll(Scroll::vertical()).footer(|ui, _| {
                    let _ = Button::new("About").sizing(Sizing::FillWidth).show(ui);
                });
                if least {
                    stack = stack.least_width(LeastWidth::SideMenu);
                }
                menu.set(
                    stack
                        .show(ui, |ui, _| {
                            let _ = Button::new("Home").sizing(Sizing::FillWidth).show(ui);
                        })
                        .output
                        .rect,
                );
                page.set(
                    Stack::new()
                        .fill()
                        .show(ui, |ui, _| {
                            let _ = Text::new("Page").show(ui);
                        })
                        .output
                        .rect,
                );
            });
            if !ui.is_sizing_pass() {
                out.set((menu.get(), page.get()));
            }
        });
        fixture.room(Vec2::new(1440.0, 900.0)).settle();
        seen.get()
    };
    let (plain, _) = shown(false);
    let (menu, page) = shown(true);
    assert!(plain != Rect::NOTHING && menu != Rect::NOTHING && page != Rect::NOTHING, "the menu and the page showed");
    assert!(menu.width() > plain.width() + 60.0, "the side menu's least width holds in the row: {menu:?} against {plain:?}");
    assert!(page.left() >= menu.right() - 0.5, "the page starts after the menu: {page:?} after {menu:?}");
    assert!(menu.height() > 800.0, "the menu is as tall as the row: {menu:?}");
}

/// A page's padding stands its content off the edges by two and a half roomy pads: a File page's margin.
#[trial]
fn a_pages_padding_is_two_and_a_half_roomy_pads() {
    let inset = |pad: Pad| {
        let seen = Rc::new(Cell::new(Vec2::ZERO));
        let out = Rc::clone(&seen);
        let mut fixture = Fixture::new(move |ui| {
            let child = Cell::new(Rect::NOTHING);
            let shown = Stack::new().pad(pad).show(ui, |ui, _| child.set(Button::new("Home").show(ui).rect));
            if !ui.is_sizing_pass() {
                out.set(child.get().min - shown.output.rect.min);
            }
        });
        fixture.room(Vec2::new(800.0, 600.0)).settle();
        seen.get()
    };
    let (roomy, page) = (inset(Pad::Roomy), inset(Pad::Page));
    assert!(roomy.x > 0.0, "a roomy stack pads its content: {roomy:?}");
    assert!((page.x - roomy.x * 2.5).abs() < 0.6 && (page.y - roomy.y * 2.5).abs() < 0.6, "a page pads 2.5 roomy pads: {page:?} for {roomy:?}");
}

#[trial]
fn a_stacks_wrap_width_breaks_a_long_paragraph_at_the_column() {
    let (plain, one_line) = shown(Stack::new, true);
    let (column, wrapped) = shown(|| Stack::new().wrap_width(WrapWidth::Column), true);
    assert!(plain.width() > 600.0, "without a wrap width the paragraph widens the stack to the room: {plain:?}");
    assert!(column.width() < plain.width() / 2.0, "the column's wrap width keeps the stack narrow: {column:?} against {plain:?}");
    assert!(wrapped.height() > one_line.height() * 2.0, "the paragraph breaks into lines: {wrapped:?} against {one_line:?}");
}

/// Labels whose row is far wider than a column.
const KINDS: [&str; 6] = ["No grid", "Lines only", "Lines and characters", "Text snaps to characters", "Left-to-right", "Right-to-left"];

/// A wrapping row in a stack with a wrap width breaks at the stack's line length, as its paragraphs do (a dialog's
/// row of radios takes a second line), instead of widening the stack to keep its children on one line.
#[trial]
fn a_wrapping_row_breaks_at_the_stacks_wrap_width() {
    let shown = |wrap: Option<WrapWidth>| {
        let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING, Rect::NOTHING)));
        let out = Rc::clone(&seen);
        let mut fixture = Fixture::new(move |ui| {
            let (first, last) = (Cell::new(Rect::NOTHING), Cell::new(Rect::NOTHING));
            let stack = match wrap {
                Some(wrap) => Stack::new().wrap_width(wrap),
                None => Stack::new(),
            };
            let shown = stack.show(ui, |ui, _| {
                Row::new().wrap(true).show(ui, |ui, _| {
                    for (at, kind) in KINDS.into_iter().enumerate() {
                        let rect = Button::new(kind).id_salt(kind).show(ui).rect;
                        if at == 0 {
                            first.set(rect);
                        }
                        last.set(rect);
                    }
                });
            });
            if !ui.is_sizing_pass() {
                out.set((shown.output.rect, first.get(), last.get()));
            }
        });
        fixture.room(Vec2::new(1600.0, 600.0)).settle();
        seen.get()
    };
    let (plain, plain_first, plain_last) = shown(None);
    let (column, first, last) = shown(Some(WrapWidth::Column));
    assert!((plain_first.center().y - plain_last.center().y).abs() < 1.0, "without a wrap width the row keeps one line: {plain:?}");
    assert!(last.top() >= first.bottom(), "the row breaks onto a second line: {first:?} then {last:?}");
    assert!(column.width() < plain.width() * 0.75, "the wrap width keeps the stack narrow: {column:?} against {plain:?}");
}

/// Labelled radios (a group's `labelled` children, label after the circle) in a wrapping row break onto further lines
/// as buttons do: a radio whose label would not fit what is left of the line starts the next one, at the row's start
/// edge, and keeps its label on one line. The label step places a labelled leaf in the rest of the line before egui
/// allocates its space, so the row used to keep every radio on its first line and squeeze the last label into the
/// sliver left at the end (Page Setup's "Text snaps to characters").
#[trial]
fn labelled_radios_in_a_wrapping_row_break_onto_a_new_line() {
    let mut fixture = Fixture::new(move |ui| {
        let _ = Stack::new().wrap_width(WrapWidth::Column).show(ui, |ui, _| {
            let after = SideLabel::new().layout(FieldLayout::After);
            let _ = RadioGroup::new("Grid", Bind::given(0u8)).show(ui, |ui, cx| {
                Row::new().wrap(true).show(ui, |ui, _| {
                    for (value, kind) in (0u8..).zip(KINDS) {
                        let _ = cx.labelled(ui, kind, kind.into(), after.clone(), Radio::new(value));
                    }
                });
            });
        });
    });
    fixture.room(Vec2::new(1600.0, 600.0)).settle();
    let rect = |kind: &str| fixture.by_label(kind).unwrap_or_else(|| panic!("the {kind} radio")).rect;
    let (first, last) = (rect(KINDS[0]), rect(KINDS[KINDS.len() - 1]));
    assert!(last.top() >= first.bottom(), "the row breaks onto a further line: {first:?} then {last:?}");
    let starts = KINDS.iter().filter(|kind| (rect(kind).left() - first.left()).abs() < 0.5).count();
    assert!(starts >= 2, "a radio that wraps starts its line at the row's start edge: {starts} radios do");
}
