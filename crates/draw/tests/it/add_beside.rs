//! `AddBesideHandles`: which buttons show, the keyboard path and an Alt click, on a canvas.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_draw::ui::{AddBesideHandles, AddSide};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Rect, Vec2, vec2};
use std::cell::Cell;
use std::rc::Rc;

const ROOM: Vec2 = vec2(400.0, 300.0);

/// The object the buttons surround.
fn board() -> DocRect {
    DocRect::new(150.0, 100.0, 250.0, 200.0)
}

/// A fixture showing the buttons, and the last add it reported.
struct Rig {
    fixture: Fixture,
    added: Rc<Cell<Option<(AddSide, bool)>>>,
}

/// What a test varies: the object, whether the buttons hide, the neighbour's rect in document units and the view's
/// turn.
struct Setup {
    object: DocRect,
    hidden: bool,
    keep_clear: Option<DocRect>,
    rotation: f64,
}

impl Default for Setup {
    fn default() -> Self {
        Setup { object: board(), hidden: false, keep_clear: None, rotation: 0.0 }
    }
}

fn rig(setup: Setup) -> Rig {
    let shown = Rc::new(Cell::new(None));
    let seen = Rc::clone(&shown);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0).rotation(setup.rotation);
        let clear: Vec<Rect> = setup
            .keep_clear
            .iter()
            .map(|rect| Rect::from_two_pos(view.to_screen(rect.origin()), view.to_screen(Point::new(rect.x1, rect.y1))))
            .collect();
        let shown_here = Canvas::new("Document").view(view).show(ui, |ui, body| {
            let mut handles = AddBesideHandles::new("Add beside", setup.object).clear_of(&clear);
            if setup.hidden {
                handles = handles.hidden();
            }
            handles.show_at(ui, body.rect)
        });
        if let Some(output) = shown_here.inner
            && let Some(side) = output.added
        {
            seen.set(Some((side, output.copy)));
        }
    });
    fixture.room(ROOM);
    fixture.settle();
    Rig { fixture, added: shown }
}

const NAMES: [(AddSide, &str); 4] =
    [(AddSide::Top, "Add above"), (AddSide::Right, "Add right"), (AddSide::Bottom, "Add below"), (AddSide::Left, "Add left")];

impl Rig {
    /// Tabs until the focus is on one of the buttons.
    fn focus_buttons(&mut self) {
        for _ in 0..4 {
            self.fixture.key(Key::Tab, Modifiers::NONE);
            if self.fixture.focused_stop().is_some_and(|(_, part)| part.is_some()) {
                return;
            }
        }
        panic!("Tab reaches a button");
    }
}

/// On a view turned a quarter, the up arrow reaches the button that sits highest on screen, not the document's top
/// side.
#[trial]
fn arrows_follow_the_screen_direction_on_a_turned_view() {
    let mut rig = rig(Setup { rotation: 90.0, ..Setup::default() });
    let highest = NAMES
        .iter()
        .filter_map(|(side, name)| rig.fixture.by_label(name).map(|target| (*side, target.center().y)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(side, _)| side)
        .expect("four buttons");
    assert_ne!(highest, AddSide::Top, "the quarter turn moved the top side");
    rig.focus_buttons();
    rig.fixture.key(Key::ArrowUp, Modifiers::NONE);
    rig.fixture.key(Key::Enter, Modifiers::NONE);
    assert_eq!(rig.added.take().map(|(side, _)| side), Some(highest));
}
