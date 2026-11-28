//! The options bar's place: a drag of its grip docks or floats it on the release; Escape, a blur and a release outside
//! the body leave it where it was.

use craft_ui::contract::{Bind, Show as _};
use craft_ui::display::Text;
use craft_ui::runtime::env::WindowKey;
use craft_ui::shell::{AppShell, BarGrip, OptionsBarPlace};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Event, Key, Modifiers, Pos2, Rect, pos2, vec2};

/// What one frame of the shell showed.
struct Seen {
    grip: Rect,
    body: Rect,
    options: Option<Rect>,
}

/// A window shell whose options bar holds a grip, with the place the bar writes.
struct Window {
    frames: Frames,
    place: OptionsBarPlace,
}

impl Window {
    fn new() -> Self {
        Window { frames: Frames::new(), place: OptionsBarPlace::default() }
    }

    fn pass(&mut self, events: Vec<Event>) -> Seen {
        let Window { frames, place } = self;
        let mut seen = Seen { grip: Rect::NOTHING, body: Rect::NOTHING, options: None };
        frames.frame(events, |ui| {
            let mut grip = Rect::NOTHING;
            let out = AppShell::new(WindowKey::main())
                .options_place(Bind::bound(&mut *place))
                .options_bar(
                    true,
                    |_| None,
                    |ui, _, _: &mut ()| {
                        grip = BarGrip::new().show(ui).rect;
                        Text::new("Options").show(ui);
                    },
                )
                .body(|_, _, _| {})
                .show(ui, &mut ());
            let options = out.regions.slots.iter().find(|(key, _)| *key == "options").map(|(_, rect)| *rect);
            seen = Seen { grip, body: out.regions.body, options };
        });
        seen
    }

    fn settle(&mut self) -> Seen {
        (0..3).map(|_| self.pass(Vec::new())).last().expect("three passes")
    }

    /// Presses the grip, drags it past the slop and on to `to`, and stops there with the button held.
    fn hold(&mut self, to: Pos2) {
        let from = self.settle().grip.center();
        self.pass(vec![moved(from)]);
        self.pass(vec![press(from)]);
        self.pass(vec![moved(from + vec2(0.0, 24.0))]);
        self.pass(vec![moved(to)]);
    }

    /// Releases at `at` and lets the shell settle.
    fn drop_at(&mut self, at: Pos2) -> Seen {
        self.pass(vec![release(at)]);
        self.settle()
    }
}

fn escape() -> Event {
    Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

#[trial]
fn dragging_the_bar_grip_docks_or_floats_the_options_bar_and_escape_leaves_it() {
    let mut window = Window::new();
    let docked = window.settle();
    let bar = docked.options.expect("the bar shows in the top slot");
    let body = docked.body;
    assert!(bar.bottom() <= body.top() + 0.5, "docked at the top, the bar is above the body");
    let grab = docked.grip.center() - bar.min;

    let from = docked.grip.center();
    window.pass(vec![moved(from)]);
    window.pass(vec![press(from)]);
    window.pass(vec![release(from)]);
    window.settle();
    assert_eq!(window.place, OptionsBarPlace::default(), "a press with no drag changes nothing");

    let middle = body.center();
    window.hold(middle);
    assert_eq!(window.place, OptionsBarPlace::default(), "the bar stays where it is until the release");
    window.drop_at(middle);
    let at = window.place.at.expect("a released floating bar has a place");
    assert!(window.place.floating && !window.place.bottom, "a release between the bands floats the bar");
    assert!(at[0].abs() < 0.5, "the bar is as wide as the body, so it stays inside at its left edge");
    assert!((at[1] - (middle.y - grab.y - body.top())).abs() < 0.5, "the bar keeps the grab: pointer minus grab, relative to the body");
    let floating = window.settle();
    assert!(floating.options.is_none(), "a floating bar claims no slot");
    assert!(floating.body.height() > body.height(), "a floating bar takes no room from the body");

    let to_bottom = pos2(floating.body.center().x, floating.body.bottom() - 2.0);
    window.hold(to_bottom);
    let bottom = window.drop_at(to_bottom);
    assert_eq!(window.place, OptionsBarPlace::new(true, false, Some(at)), "the bottom band docks the bar and docking keeps the corner");
    assert!(
        bottom.options.expect("the bar shows in the bottom slot").top() >= bottom.body.bottom() - 0.5,
        "docked at the bottom, the bar is below the body"
    );

    let to_top = pos2(bottom.body.center().x, bottom.body.top() + 2.0);
    window.hold(to_top);
    window.drop_at(to_top);
    assert_eq!(window.place, OptionsBarPlace::new(false, false, Some(at)), "the top band docks the bar at the top");

    let rest = window.place;
    window.hold(middle);
    window.pass(vec![escape()]);
    window.drop_at(middle);
    assert_eq!(window.place, rest, "Escape leaves the drag");

    let outside = pos2(-40.0, -40.0);
    window.hold(middle);
    window.pass(vec![moved(outside)]);
    window.drop_at(outside);
    assert_eq!(window.place, rest, "a release outside the body leaves the bar");

    window.hold(middle);
    window.pass(vec![Event::WindowFocused(false)]);
    window.drop_at(middle);
    assert_eq!(window.place, rest, "losing the window's focus leaves the drag");
}
