//! `Bar`: a body too wide for the bar shrinks its members before it scrolls.

use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft_ui::pickers::Select;
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

const SLACK: f32 = 0.5;

/// The rects of a select and of the last of `buttons` buttons after it, in an options bar `width` wide.
fn bar(width: f32, buttons: usize) -> (Rect, Rect) {
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(0u8, "Current and Below, No Adjustments"), Entry::new(1, "All Layers")];
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            let select = Select::new("Sample", Bind::given(1u8), &entries).show(ui).response.rect;
            let mut last = Rect::NOTHING;
            for at in 0..buttons {
                last = Button::new(format!("Button {at}")).show(ui).rect;
            }
            out.set((select, last));
        });
    });
    fixture.room(Vec2::new(width, 80.0)).settle();
    seen.get()
}

#[trial]
fn a_bar_too_narrow_for_its_members_elides_a_select_before_it_scrolls() {
    let (wide, _) = bar(900.0, 1);
    let (select, button) = bar(240.0, 1);
    assert!(select.width() < wide.width() - SLACK, "the select kept its width {} in a narrow bar", select.width());
    assert!(button.max.x <= 240.0 + SLACK, "the last member ends at {} past the bar's end", button.max.x);
    // Buttons do not shrink: once the select is at its least, the rest scrolls.
    let (_, button) = bar(240.0, 6);
    assert!(button.max.x > 240.0, "six buttons fit a 240-point bar: {button:?}");
}
