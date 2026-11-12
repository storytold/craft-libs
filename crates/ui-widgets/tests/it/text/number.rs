//! The number field's own logic: stepping, the range on typed values, presets and the label scrub across a pointer warp.

use craft_host::{AppInfo, Host};
use craft_ui::contract::{Bind, Entry, FieldLayout, GlyphLabel, InsideLabel, LabelKind, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft_ui::form::Field;
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::{click, drag};
use craft_ui::testing::{Frames, trial};
use craft_ui::text::{NumberField, NumberPick};
use egui::{Event, Key, Modifiers, Pos2, Rect, Ui, Vec2};
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
    /// The field writes whole numbers (`fixed_decimals(0)`).
    whole: bool,
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
            whole: false,
        };
        rig.idle(3);
        rig
    }

    fn frame(&mut self, events: Vec<Event>) {
        let Rig { frames, value, rect, picked, range, presets, whole } = self;
        frames.frame(events, |ui| show(ui, value, rect, picked, range, presets, *whole));
    }

    fn idle(&mut self, frames: usize) {
        for _ in 0..frames {
            self.frame(Vec::new());
        }
    }

    fn focus(&mut self) {
        let Rig { frames, value, rect, picked, range, presets, whole } = self;
        let at = rect.get().center();
        click(frames, at, &mut |ui| show(ui, value, rect, picked, range, presets, *whole));
        self.idle(2);
    }

    fn drag(&mut self, by: Vec2) {
        let Rig { frames, value, rect, picked, range, presets, whole } = self;
        let from = rect.get().center();
        drag(frames, from, by, &mut |ui| show(ui, value, rect, picked, range, presets, *whole));
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
    whole: bool,
) {
    let mut guard = value.borrow_mut();
    let mut field = NumberField::new("Size", Bind::bound(&mut *guard)).id_salt("size");
    if let Some(range) = range {
        field = field.range(range.clone());
    }
    if !presets.is_empty() {
        field = field.presets(presets);
    }
    if whole {
        field = field.fixed_decimals(0);
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

/// A drag on a field that is not in edit scrubs it; once a click has put it in edit, a drag is the text's.
#[trial]
fn a_drag_scrubs_a_field_not_in_edit_and_is_the_text_s_in_edit() {
    let mut rig = Rig::new(START, None);
    rig.drag(Vec2::new(30.0, 0.0));
    let scrubbed = rig.value().unwrap_or_default();
    assert!(scrubbed > START, "a drag to the right on the field raised the value");
    rig.focus();
    rig.drag(Vec2::new(30.0, 0.0));
    assert_eq!(rig.value(), Some(scrubbed), "a drag in edit leaves the value");
}

/// Alt steps a tenth where the field's decimals show it, and a whole step on a whole-number field.
#[trial]
fn a_fine_step_moves_only_as_far_as_the_decimals_show() {
    let mut rig = Rig::new(5.0, None);
    rig.focus();
    rig.key(Key::ArrowUp, Modifiers::ALT);
    let fine = rig.value().unwrap_or_default() - 5.0;
    let mut whole = Rig::new(5.0, None);
    whole.whole = true;
    whole.focus();
    whole.key(Key::ArrowUp, Modifiers::ALT);
    let plain = whole.value().unwrap_or_default() - 5.0;
    assert!(fine > 0.0 && fine < 1.0, "Alt moves a share of the step on a field with decimals");
    assert_eq!(plain, 1.0, "a whole-number field takes the plain step");
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

/// A field over 0 to 100 with a popup slider.
fn with_slider(ui: &mut Ui, data: &mut craft_ui::testing::StoryData) -> craft_ui::testing::StoryOutput {
    NumberField::new("Opacity", Bind::bound(data.value("value", Some(50.0)))).range(0.0..=100.0).popup_slider(0.0..=100.0).show(ui).into()
}

/// Pressing the popup slider's chevron and dragging moves the value at once, and the release closes the slider.
#[trial]
fn a_drag_from_the_slider_chevron_moves_the_value_and_its_release_closes_the_slider() {
    use craft_ui::contract::Role;
    let mut fixture = craft_ui::testing::Fixture::story(&craft_ui::testing::Story::new("number_slider", with_slider));
    fixture.settle();
    let chevron = fixture.by_role(Role::BUTTON).first().expect("the chevron").center();
    fixture.drag_hold_at(chevron, Vec2::new(30.0, 0.0));
    fixture.settle();
    assert!(!fixture.by_role(Role::SLIDER).is_empty(), "the press opened the slider");
    assert!(fixture.data().value("value", Some(50.0)).is_some_and(|value| value > 50.0), "the drag moved the value at once");
    fixture.release_at(chevron + Vec2::new(30.0, 0.0));
    fixture.settle();
    assert!(fixture.by_role(Role::SLIDER).is_empty(), "the release closed the slider");
}

/// A glyph label inside the field is a scrub handle at the field's start: it lies before the text without
/// overlapping it, and a drag on it scrubs the value, in edit too.
#[trial]
fn an_inside_glyph_label_scrubs_the_field_in_edit_too() {
    let value = std::rc::Rc::new(Cell::new(Some(START)));
    let id = std::rc::Rc::new(Cell::new(None));
    let (shown, seen) = (value.clone(), id.clone());
    let mut fixture = craft_ui::testing::Fixture::new(move |ui| {
        let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text("W"))).layout(FieldLayout::Inside).scrub(true);
        Field::new("Width").side(side).show(ui, |ui, _| {
            let mut now = shown.get();
            let out = NumberField::new("Width", Bind::bound(&mut now)).id_salt("width").scrub_by_label().show(ui);
            shown.set(now);
            seen.set(Some(out.response.id));
        });
    });
    fixture.settle();
    let field = id.get().unwrap_or_else(|| panic!("the field ran"));
    let glyph = fixture.part_rect(field, InsideLabel::PART).unwrap_or_else(|| panic!("the field declares the inside label's slot"));
    let painted = |text: &str| fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect);
    let number = painted("10").unwrap_or_else(|| panic!("the field shows its value"));
    let mark = painted("W").unwrap_or_else(|| panic!("the field shows the glyph"));
    assert!(glyph.contains_rect(mark), "the glyph is painted in its slot: {mark:?} {glyph:?}");
    assert!(glyph.max.x <= number.min.x, "the glyph lies before the value without overlap: {glyph:?} {number:?}");
    let target = fixture.by_label("Width").unwrap_or_else(|| panic!("the label names the field"));
    fixture.click(&target);
    assert!(fixture.focused_stop().is_some_and(|(stop, _)| stop == field), "a click puts the field in edit");
    fixture.drag_at(glyph.center(), Vec2::new(30.0, 0.0));
    fixture.idle(2);
    assert!(value.get().unwrap_or_default() > START, "a drag on the glyph in edit raised the value");

/// The one name of `named_field`'s values, tied to its presets row.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Metrics;

impl craft_ui::text::NumberName for Metrics {
    fn id(self) -> u32 {
        METRICS_ROW
    }

    fn from_id(id: u32) -> Option<Self> {
        (id == METRICS_ROW).then_some(Metrics)
    }
}

const METRICS_ROW: u32 = 7;

/// A field that holds a name shows its presets row's label as its text, and typing the name over a number commits the
/// name (kerning's Metrics).
#[trial]
fn a_named_value_shows_its_row_label_and_the_typed_name_commits_it() {
    use craft_ui::text::NamedOr;
    let value = std::rc::Rc::new(RefCell::new(NamedOr::Named(Metrics)));
    let (held, id) = (value.clone(), std::rc::Rc::new(Cell::new(egui::Id::NULL)));
    let shown_id = id.clone();
    let presets = vec![Entry::new(NumberPick::Choice(METRICS_ROW), "Metrics"), Entry::new(NumberPick::Number(25.0), "25")];
    let mut fixture = craft_ui::testing::Fixture::new(move |ui| {
        let mut guard = held.borrow_mut();
        let field = NumberField::named("Kerning", Bind::bound(&mut *guard)).id_salt("kerning").presets(&presets);
        shown_id.set(field.show(ui).response.id);
    });
    fixture.settle();
    assert_eq!(fixture.tree().value(id.get()), Some("Metrics"), "a named value shows its row's label");
    *value.borrow_mut() = NamedOr::Number(5.0);
    fixture.settle();
    let field = fixture.by_label("Kerning").expect("the field");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("metrics").key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*value.borrow(), NamedOr::Named(Metrics), "the typed name commits the name");
}
