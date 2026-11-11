//! A content-sized host with a room height, inside a scrolling body, gives a child that fills that room in the
//! measuring run and in the real run, and a content child its content height.

use craft_ui::buttons::Button;
use craft_ui::contract::{Scroll, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

const TOLERANCE: f32 = 0.5;

#[trial]
fn a_fill_child_takes_the_hosts_room_height_inside_a_scroll() {
    const ROOM: Vec2 = Vec2::new(400.0, 300.0);
    // The rects of the fill host, its fill child, the content host and its button, from the last real run.
    let seen = Rc::new(Cell::new([Rect::NOTHING; 4]));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_height() / 2.0;
        Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| {
            let filled = Stack::new()
                .fill_width()
                .room_height(room)
                .show(ui, |ui, _| Stack::new().fill().show(ui, |ui, _| Button::new("Inside").show(ui)).output.rect);
            let content = Stack::new().fill_width().room_height(room).show(ui, |ui, _| Button::new("Content").show(ui).response.rect);
            if !ui.is_sizing_pass()
                && let (Some(fill), Some(button)) = (filled.inner, content.inner)
            {
                out.set([filled.output.rect, fill, content.output.rect, button]);
            }
        });
    });
    fixture.room(ROOM).settle();
    let [host, fill, content, button] = seen.get();
    let dump = format!("host {host:?}, fill {fill:?}, content {content:?}, button {button:?}");
    assert!(fill.height().is_finite() && fill.height() > 0.0, "the fill child has a finite room: {dump}");
    assert!((fill.height() - host.height()).abs() <= TOLERANCE, "the host is as tall as its fill child: {dump}");
    assert!(fill.height() < ROOM.y, "the fill child takes the room height, not the scroll's unbounded room: {dump}");
    assert!(content.height() < host.height(), "a content child takes its content height: {dump}");
    assert!(content.contains_rect(button.shrink(TOLERANCE)), "the content host holds its child: {dump}");
}
