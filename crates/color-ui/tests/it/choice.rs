//! A named/custom color choice reports only a committed row or custom picker request.

use std::cell::RefCell;
use std::rc::Rc;

use craft_color_ui::ui::{ChosenColor, ColorChoice, NamedColor};
use craft_theme::Srgba;
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, vec2};

const RED: Srgba = Srgba::rgb(0.8, 0.1, 0.2);
const BLUE: Srgba = Srgba::rgb(0.1, 0.2, 0.8);
const CUSTOM: Srgba = Srgba::rgb(0.2, 0.6, 0.4);

#[derive(Default)]
struct Reports {
    chosen: Vec<ChosenColor>,
    requests: Vec<ColorRequest>,
}

struct Rig {
    fixture: Fixture,
    value: Rc<RefCell<ChosenColor>>,
    reports: Rc<RefCell<Reports>>,
}

impl Rig {
    fn new(start: ChosenColor, custom: Option<Srgba>) -> Self {
        let value = Rc::new(RefCell::new(start));
        let reports = Rc::new(RefCell::new(Reports::default()));
        let (held, reported) = (value.clone(), reports.clone());
        let mut fixture = Fixture::new(move |ui| {
            let value = held.borrow().clone();
            let shown = ColorChoice::new("Color", &value)
                .named([NamedColor::new("red", "Red", Some(RED)), NamedColor::new("blue", "Blue", Some(BLUE))])
                .custom(custom)
                .show(ui);
            if let Some(chosen) = shown.chosen {
                *held.borrow_mut() = chosen.clone();
                reported.borrow_mut().chosen.push(chosen);
            }
            reported.borrow_mut().requests.extend(shown.pick_color);
        });
        fixture.room(vec2(400.0, 240.0)).settle();
        Rig { fixture, value, reports }
    }

    fn open(&mut self) {
        let field = self.fixture.by_label("Color").expect("the color choice");
        self.fixture.click(&field);
    }
}

/// A provisional named row never replaces the host's named or custom choice; Enter reports it once.
#[trial]
fn canceled_color_navigation_keeps_the_hosts_original_choice() {
    for original in [ChosenColor::Named("red".into()), ChosenColor::Custom] {
        let key = if original == ChosenColor::Custom { Key::ArrowUp } else { Key::ArrowDown };
        let mut rig = Rig::new(original.clone(), Some(CUSTOM));
        rig.open();
        rig.fixture.key(key, Modifiers::NONE).idle(2);
        assert_eq!(*rig.value.borrow(), original, "navigation alone does not replace the original color");
        assert!(rig.reports.borrow().chosen.is_empty());
        rig.fixture.key(Key::Escape, Modifiers::NONE).settle();
        assert_eq!(*rig.value.borrow(), original);
        assert!(rig.reports.borrow().chosen.is_empty());
        assert!(rig.reports.borrow().requests.is_empty());

        rig.open();
        rig.fixture.key(key, Modifiers::NONE).key(Key::Enter, Modifiers::NONE).settle().idle(2);
        assert_eq!(*rig.value.borrow(), ChosenColor::Named("blue".into()));
        assert_eq!(rig.reports.borrow().chosen, [ChosenColor::Named("blue".into())]);
        assert!(rig.reports.borrow().requests.is_empty());
    }
}

/// Proposing Select Custom Color neither requests the picker nor changes the color; Enter requests it once.
#[trial]
fn custom_color_picker_waits_for_a_committed_row() {
    for custom in [Some(CUSTOM), None] {
        let original = ChosenColor::Named("blue".into());
        let mut rig = Rig::new(original.clone(), custom);
        rig.open();
        rig.fixture.key(Key::End, Modifiers::NONE).idle(2);
        assert!(rig.reports.borrow().requests.is_empty(), "a provisional custom row cannot open a picker");
        rig.fixture.key(Key::Escape, Modifiers::NONE).settle();
        assert_eq!(*rig.value.borrow(), original);
        assert!(rig.reports.borrow().requests.is_empty());
        assert!(rig.reports.borrow().chosen.is_empty());

        rig.open();
        rig.fixture.key(Key::End, Modifiers::NONE).key(Key::Enter, Modifiers::NONE).settle().idle(2);
        assert_eq!(*rig.value.borrow(), original, "the host changes its color only after the requested picker answers");
        let reports = rig.reports.borrow();
        assert!(reports.chosen.is_empty());
        assert_eq!(reports.requests.len(), 1);
        assert_eq!(reports.requests[0].key, ColorChoice::CUSTOM_KEY);
        assert_eq!(reports.requests[0].color, Some(custom.unwrap_or(BLUE)));
    }
}
