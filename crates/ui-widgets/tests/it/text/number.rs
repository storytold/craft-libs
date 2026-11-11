//! The number field's own logic: stepping, the range on typed values, presets and the label scrub across a pointer warp.

use craft_host::{AppInfo, Host};
use craft_ui::contract::{Bind, Entry, Props as _, Show as _};
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::click;
use craft_ui::testing::{Frames, trial};
use craft_ui::text::{NumberField, NumberPick};
use egui::{Event, Key, Modifiers, Pos2, Rect, Ui};
use std::cell::{Cell, RefCell};
use std::ops::RangeInclusive;
use std::sync::Arc;

const START: f64 = 10.0;

/// A number field of a fixed id, driven frame by frame.
struct Rig {
    frames: Frames,
    value: RefCell<Option<f64>>,
    rect: Cell<Rect>,
    picked: Cell<Option<NumberPick>>,
    range: Option<RangeInclusive<f64>>,
    presets: Vec<Entry<NumberPick>>,
}

impl Rig {
    fn new(start: f64, range: Option<RangeInclusive<f64>>) -> Rig {
        let mut rig = Rig {
            frames: Frames::new(),
            value: RefCell::new(Some(start)),
            rect: Cell::new(Rect::NOTHING),
            picked: Cell::new(None),
            range,
            presets: Vec::new(),
        };
        rig.idle(3);
        rig
    }

    fn frame(&mut self, events: Vec<Event>) {
        let Rig { frames, value, rect, picked, range, presets } = self;
        frames.frame(events, |ui| show(ui, value, rect, picked, range, presets));
    }

    fn idle(&mut self, frames: usize) {
        for _ in 0..frames {
            self.frame(Vec::new());
        }
    }

    fn focus(&mut self) {
        let Rig { frames, value, rect, picked, range, presets } = self;
        let at = rect.get().center();
        click(frames, at, &mut |ui| show(ui, value, rect, picked, range, presets));
        self.idle(2);
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.frame(vec![key_press(key, modifiers)]);
        self.idle(3);
    }

    fn value(&self) -> Option<f64> {
        *self.value.borrow()
    }
}

fn show(
    ui: &mut Ui,
    value: &RefCell<Option<f64>>,
    rect: &Cell<Rect>,
    picked: &Cell<Option<NumberPick>>,
    range: &Option<RangeInclusive<f64>>,
    presets: &[Entry<NumberPick>],
) {
    let mut guard = value.borrow_mut();
    let mut field = NumberField::new("Size", Bind::bound(&mut *guard)).id_salt("size");
    if let Some(range) = range {
        field = field.range(range.clone());
    }
    if !presets.is_empty() {
        field = field.presets(presets);
    }
    let out = field.show(ui);
    rect.set(out.response.rect);
    if out.preset.is_some() {
        picked.set(out.preset);
    }
}

/// An arrow goes to the next multiple of the step in its direction, Shift moves ten steps, and the range holds.
#[trial]
fn arrows_step_to_the_next_multiple_and_stay_in_range() {
    let mut rig = Rig::new(5.1, Some(0.0..=7.0));
    rig.focus();
    rig.key(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(rig.value(), Some(5.0), "down from 5.1 goes to 5");
    rig.key(Key::ArrowUp, Modifiers::NONE);
    rig.key(Key::ArrowUp, Modifiers::NONE);
    assert_eq!(rig.value(), Some(7.0), "up reaches the end of the range");
    rig.key(Key::ArrowUp, Modifiers::NONE);
    assert_eq!(rig.value(), Some(7.0), "up at the end stays there");
    rig.key(Key::ArrowDown, Modifiers::SHIFT);
    assert_eq!(rig.value(), Some(0.0), "Shift moves ten steps and stops at the start of the range");
}

/// A field of `kind` (plain without one) under `host`'s locale, with `text` typed over its value and committed.
fn typed(host: Option<Arc<Host>>, kind: Option<UnitKind>, text: &str) -> Option<f64> {
    typed_with(host, text, |field| match kind {
        Some(kind) => field.unit_kind(kind),
        None => field,
    })
}

/// A field set up by `configure` under `host`'s locale, with `text` typed over its value and committed.
fn typed_with(host: Option<Arc<Host>>, text: &str, configure: impl for<'a> Fn(NumberField<'a>) -> NumberField<'a>) -> Option<f64> {
    let mut frames = Frames::new();
    if let Some(host) = host {
        frames.host(host);
    }
    let value = RefCell::new(Some(START));
    let at = Cell::new(Pos2::ZERO);
    let mut show = |ui: &mut Ui| {
        let mut guard = value.borrow_mut();
        let field = configure(NumberField::new("Size", Bind::bound(&mut *guard)).id_salt("size"));
        at.set(field.show(ui).response.rect.center());
    };
    frames.idle(3, &mut show);
    click(&mut frames, at.get(), &mut show);
    frames.frame(vec![key_press(Key::A, Modifiers::COMMAND)], &mut show);
    frames.frame(vec![Event::Text(text.into())], &mut show);
    frames.idle(1, &mut show);
    frames.frame(vec![key_press(Key::Enter, Modifiers::NONE)], &mut show);
    frames.idle(3, &mut show);
    value.into_inner()
}

/// Under a decimal-comma locale the comma is the decimal separator.
#[trial]
fn a_localised_decimal_types_in() {
    let (i18n, problems) = craft_i18n::I18n::from_owned([("de".to_owned(), String::new(), "@id\ttrial.save\tSpeichern\n".to_owned())], &["de"]);
    assert!(problems.is_empty(), "{problems:?}");
    i18n.set_format(craft_i18n::Format::from_tag("de"));
    let host = Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).i18n(Arc::new(i18n)).build());
    assert_eq!(typed(Some(host), None, "12,5"), Some(12.5));
}
