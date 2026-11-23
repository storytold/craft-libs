//! A docked panel group.

use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::docking::{DockGroup, DragUnit, GroupGrip, PanelKey, PanelTitle, Titles};
use craft_ui::testing::{Fixture, Frames, pointer, trial};
use egui::{Pos2, Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

// A minimized group shows its header alone: its body does not run, so it leaves no node and nothing a press could
// reach under the groups below it. Expanded, the same body shows.
#[trial]
fn a_minimized_group_runs_no_body() {
    let body = |minimized: bool| {
        let runs = Rc::new(Cell::new(0));
        let counted = Rc::clone(&runs);
        let panels = vec![PanelTitle::new(PanelKey::new(1), "Color")];
        let mut fixture = Fixture::new(move |ui| {
            let group = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[0].key);
            let group = if minimized { group.minimized() } else { group };
            let counted = Rc::clone(&counted);
            group.show_at(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(240.0, 200.0)), move |ui, _| {
                counted.set(counted.get() + 1);
                Button::new("Body button").show(ui);
            });
        });
        fixture.idle(4);
        let found = fixture.by_label("Body button").is_some();
        (runs.get(), found)
    };
    let (runs, found) = body(false);
    assert!(runs > 0 && found, "an expanded group shows its body: {runs} runs, node {found}");
    assert_eq!(body(true), (0, false), "a minimized group runs no body and leaves no node");
}

// A toolbar's marked grip is a handle (photocraft#16467): the grab cursor over it and grabbing while it is held, as
// the grips of a collapsed column show. An unmarked band and a grip that does not move keep the plain arrow.
#[trial]
fn a_marked_grip_shows_the_grab_cursor() {
    let strip = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(60.0, 16.0));
    for (ticks, movable) in [(true, true), (false, true), (true, false)] {
        let mut frames = Frames::new();
        let mut show = |ui: &mut egui::Ui| {
            GroupGrip::new(DragUnit::Group(1usize), PanelKey::new(1)).ticks(ticks).movable(movable).show_at(ui, strip);
        };
        frames.frame(vec![], &mut show);
        frames.frame(vec![pointer::moved(strip.center())], &mut show);
        frames.frame(vec![], &mut show);
        let (over, held) = if ticks && movable {
            (egui::CursorIcon::Grab, egui::CursorIcon::Grabbing)
        } else {
            (egui::CursorIcon::Default, egui::CursorIcon::Default)
        };
        assert_eq!(frames.cursor_icon(), over, "over the grip (ticks {ticks}, movable {movable})");
        frames.frame(vec![pointer::press(strip.center())], &mut show);
        frames.frame(vec![], &mut show);
        assert_eq!(frames.cursor_icon(), held, "the grip held (ticks {ticks}, movable {movable})");
    }
}
