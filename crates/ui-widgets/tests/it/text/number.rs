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

/// The window losing focus while a value is typed keeps the draft and commits nothing; back in the window, typing
/// goes on from the draft. The draft is a sum, which applies only on commit (a plain number applies as typed), so
/// the value tells whether the blur committed it.
#[trial]
fn a_window_blur_mid_typing_keeps_the_typed_draft() {
    let mut rig = Rig::new(5.0, None);
    rig.focus();
    rig.key(Key::A, Modifiers::COMMAND);
    rig.frame(vec![Event::Text("1+1".into())]);
    rig.idle(1);
    rig.frame(vec![Event::WindowFocused(false)]);
    rig.idle(2);
    assert_eq!(rig.value(), Some(5.0), "the blur commits nothing");
    assert!(rig.frames.texts().iter().any(|text| text.text == "1+1"), "the field still shows the typed draft");
    rig.frame(vec![Event::WindowFocused(true)]);
    rig.idle(1);
    rig.frame(vec![Event::Text("0".into())]);
    rig.key(Key::Enter, Modifiers::NONE);
    assert_eq!(rig.value(), Some(11.0), "typing went on from the draft");
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
    NumberField::new("Opacity", data.bind("value", Some(50.0))).range(0.0..=100.0).popup_slider(0.0..=100.0).show(ui).into()
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
}

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

/// A value the binding's check calls an error stays: the commit goes through, the field reads invalid and its row shows
/// the message.
#[trial]
fn a_value_the_check_calls_an_error_commits_and_shows_invalid_with_its_message() {
    use craft_ui::contract::{ShowContainer as _, Validity};
    use craft_ui::form::Field;
    use craft_ui::runtime::env::Message;
    const TOO_MANY: &str = "Enter a number from 1 to 10";
    let value = std::rc::Rc::new(RefCell::new(Some(START)));
    let (held, id) = (value.clone(), std::rc::Rc::new(Cell::new(egui::Id::NULL)));
    let shown_id = id.clone();
    let mut fixture = craft_ui::testing::Fixture::new(move |ui| {
        let check = |count: &Option<f64>| match count {
            Some(n) if *n > 10.0 => Validity::Error(Message::new(TOO_MANY)),
            _ => Validity::Ok,
        };
        let shown = *held.borrow();
        let out = Field::new("Number").show(ui, |ui, _| NumberField::new("Number", Bind::given(shown).check(check)).id_salt("number").show(ui)).inner;
        if let Some(out) = out {
            shown_id.set(out.response.id);
            if let Some(typed) = out.value().copied() {
                *held.borrow_mut() = typed;
            }
        }
    });
    fixture.settle();
    assert!(!fixture.tree().invalid(id.get()), "a value in range reads valid");
    let field = fixture.by_label("Number").expect("the field");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("12").key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*value.borrow(), Some(12.0), "the commit went through");
    assert!(fixture.tree().invalid(id.get()), "the field reads invalid");
    assert!(fixture.texts().iter().any(|text| text.text == TOO_MANY), "the row shows the message");
}

/// A field's unit keeps its place at the end of the value area, apart from the number, which centres in the room
/// before it: two fields of one range end their units at the same place whether they show "5" or "100", so a column
/// of fields lines its units up. Guards against the unit following the number's end.
#[trial]
fn the_unit_keeps_its_place_at_the_end_of_the_field() {
    use craft_ui::testing::Fixture;
    let boxes: std::rc::Rc<RefCell<Vec<Rect>>> = std::rc::Rc::default();
    let b = boxes.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut v = b.borrow_mut();
        v.clear();
        for (at, value) in [5.0, 100.0].into_iter().enumerate() {
            let mut shown = Some(value);
            let field = NumberField::new("N", Bind::bound(&mut shown)).range(0.0..=100.0).unit_kind(UnitKind::Points).id_salt(at);
            v.push(field.show(ui).response.rect);
        }
    });
    fixture.settle();
    fixture.idle(3);
    let texts = fixture.texts();
    let ends: Vec<f32> = boxes
        .borrow()
        .iter()
        .zip(["5", "100"])
        .map(|(field, shown)| {
            let number = texts.iter().find(|text| field.contains_rect(text.rect) && text.text == shown).expect("the number is painted");
            let unit = texts.iter().find(|text| field.contains_rect(text.rect) && text.text != shown).expect("the unit is painted");
            assert!(unit.rect.left() > number.rect.right(), "the unit {:?} stands apart from {shown}", unit.text);
            assert!(number.rect.center().x < field.center().x, "{shown} centres in the room before the unit");
            field.right() - unit.rect.right()
        })
        .collect();
    assert!((ends[0] - ends[1]).abs() < 0.5, "the units end at one place from the field's end: {ends:?}");
}

/// A field's digits sit on its centre line at both control sizes when the text row is taller than the part (here
/// egui's own fonts, far larger than the field's; on a Mac, the OS interface face's tall row in a small field): the
/// row is centred on the part, not allocated from the part's top. The side label follows the digits' baseline, so
/// this keeps it level too.
#[trial]
fn the_digits_sit_on_the_field_centre_line() {
    use craft_theme::ControlSize;
    use craft_ui::testing::Fixture;
    let boxes: std::rc::Rc<RefCell<Vec<Rect>>> = std::rc::Rc::default();
    let b = boxes.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut v = b.borrow_mut();
        v.clear();
        for (at, size) in [ControlSize::Sm, ControlSize::Md].into_iter().enumerate() {
            v.push(NumberField::new("N", Bind::bound(&mut Some(100.0))).size(size).id_salt(at).show(ui).response.rect);
        }
    });
    fixture.ctx().all_styles_mut(|style| style.text_styles.values_mut().for_each(|font| font.size *= 3.0));
    fixture.settle();
    fixture.idle(3);
    for field in boxes.borrow().iter() {
        let digits = fixture.texts().iter().find(|text| field.intersects(text.rect) && text.text == "100").expect("the digits are painted");
        let off = digits.rect.center().y - field.center().y;
        assert!(off.abs() <= 0.5, "the digits sit {off} pt off the centre line of {field:?}");
    }
}

/// A typed value past the range commits the range's end and reports it in that pass only, for Enter, Tab and a focus
/// loss to another control's press; arrows, the wheel and a scrub that run into the end never report it.
#[trial]
fn a_typed_value_past_the_range_commits_the_end_and_reports_it_once() {
    use craft_ui::testing::Fixture;
    use craft_ui::testing::pointer::{moved, wheel_lines};
    use craft_ui::text::RangeAlert;
    use std::rc::Rc;
    let value = Rc::new(Cell::new(Some(50.0)));
    let alerts: Rc<RefCell<Vec<RangeAlert>>> = Rc::default();
    let (held, seen) = (value.clone(), alerts.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut now = held.get();
        let out = NumberField::new("Amount", Bind::bound(&mut now))
            .id_salt("amount")
            .range(0.0..=100.0)
            .unit_kind(UnitKind::Percent)
            .commit_decimals(0)
            .show(ui);
        held.set(now);
        seen.borrow_mut().extend(out.out_of_range.clone());
        NumberField::new("Other", Bind::bound(&mut Some(1.0))).id_salt("other").show(ui);
    });
    fixture.settle();
    let amount = fixture.by_label("Amount").expect("the field");
    let other = fixture.by_label("Other").expect("the other field");
    let type_over = |fixture: &mut Fixture, text: &str| {
        fixture.click(&amount).settle();
        fixture.key(Key::A, Modifiers::COMMAND).type_text(text);
    };

    type_over(&mut fixture, "250");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(value.get(), Some(100.0), "a typed value past the end commits the end");
    {
        let alerts = alerts.borrow();
        assert_eq!(alerts.len(), 1, "Enter reports it in one pass: {alerts:?}");
        let alert = &alerts[0];
        assert_eq!((alert.typed, alert.kept, alert.range.clone()), (250.0, 100.0, 0.0..=100.0));
        assert!(alert.whole, "the field commits whole numbers");
        assert_eq!(alert.unit, None, "the field sets no unit label of its own");
        assert_eq!(alert.unit_kind, Some(UnitKind::Percent));
    }

    type_over(&mut fixture, "-5");
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(value.get(), Some(0.0), "Tab commits the start");
    assert_eq!(alerts.borrow().len(), 2, "Tab reports it");
    assert_eq!(alerts.borrow()[1].typed, -5.0);

    type_over(&mut fixture, "300");
    fixture.click(&other).settle();
    assert_eq!(value.get(), Some(100.0), "a press on another control commits the end");
    assert_eq!(alerts.borrow().len(), 3, "the focus loss reports it");

    type_over(&mut fixture, "95");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    fixture.key(Key::PageUp, Modifiers::NONE).key(Key::PageUp, Modifiers::NONE).settle();
    assert_eq!(value.get(), Some(100.0), "Page Up runs into the end");
    fixture.frame(vec![moved(amount.rect.center())]);
    for _ in 0..3 {
        fixture.frame(vec![wheel_lines(1.0)]);
    }
    fixture.settle();
    fixture.click(&other).settle();
    fixture.drag_at(amount.rect.center(), Vec2::new(60.0, 0.0));
    fixture.settle();
    assert_eq!(alerts.borrow().len(), 3, "a value in range, keys, the wheel and a scrub report nothing: {:?}", alerts.borrow());
}

/// A draft that is no number owes no range alert, though "150" was typed over the field before it: text over it,
/// text and the key that leaves in one pass. A number typed again after the refused text owes the alert again. In
/// every case the field holds a value in its range.
#[trial]
fn a_draft_that_is_no_number_owes_no_range_alert() {
    use craft_ui::testing::Fixture;
    use craft_ui::testing::input::text;
    use craft_ui::text::RangeAlert;
    use std::rc::Rc;

    /// A step run after the typing: it gets the fixture and a way to type text.
    type Step<'a> = &'a dyn Fn(&mut Fixture, &dyn Fn(&mut Fixture, &str));

    /// Types "150" over the field without leaving it, then runs `then`; returns the alerts and the value after.
    fn after_150(then: Step<'_>) -> (usize, Option<f64>) {
        let value = Rc::new(Cell::new(Some(50.0)));
        let alerts: Rc<RefCell<Vec<RangeAlert>>> = Rc::default();
        let (held, seen) = (value.clone(), alerts.clone());
        let mut fixture = Fixture::new(move |ui| {
            let mut now = held.get();
            let out = NumberField::new("Amount", Bind::bound(&mut now)).id_salt("amount").range(0.0..=100.0).commit_decimals(0).show(ui);
            held.set(now);
            seen.borrow_mut().extend(out.out_of_range.clone());
            // Where Tab takes the focus, so the field commits.
            NumberField::new("Other", Bind::bound(&mut Some(1.0))).id_salt("other").show(ui);
        });
        fixture.settle();
        let amount = fixture.by_label("Amount").expect("the field");
        let type_over = |fixture: &mut Fixture, typed: &str| {
            fixture.click(&amount).settle();
            fixture.key(Key::A, Modifiers::COMMAND).type_text(typed);
        };
        type_over(&mut fixture, "150");
        then(&mut fixture, &type_over);
        fixture.settle();
        let count = alerts.borrow().len();
        (count, value.get())
    }

    let tab = |fixture: &mut Fixture| {
        fixture.key(Key::Tab, Modifiers::NONE);
    };
    let cases: [(&str, Step<'_>, usize); 4] = [
        (
            "text typed over it, then Tab",
            &|fixture: &mut Fixture, type_over: &dyn Fn(&mut Fixture, &str)| {
                type_over(fixture, "abc");
                tab(fixture);
            },
            0,
        ),
        (
            "text, then the number again, then Tab",
            &|fixture: &mut Fixture, type_over: &dyn Fn(&mut Fixture, &str)| {
                type_over(fixture, "abc");
                type_over(fixture, "150");
                tab(fixture);
            },
            1,
        ),
        (
            "Select All, text and Enter in one pass",
            &|fixture: &mut Fixture, _: &dyn Fn(&mut Fixture, &str)| {
                fixture.frame(vec![key_press(Key::A, Modifiers::COMMAND), text("abc"), key_press(Key::Enter, Modifiers::NONE)]);
            },
            0,
        ),
        (
            "Select All, text and Tab in one pass",
            &|fixture: &mut Fixture, _: &dyn Fn(&mut Fixture, &str)| {
                fixture.frame(vec![key_press(Key::A, Modifiers::COMMAND), text("abc"), key_press(Key::Tab, Modifiers::NONE)]);
            },
            0,
        ),
    ];
    for (case, then, owed) in cases {
        let (alerts, value) = after_150(then);
        assert_eq!(alerts, owed, "{case}: the range alerts reported");
        assert!(value.is_some_and(|value| (0.0..=100.0).contains(&value)), "{case}: the field holds a value in its range: {value:?}");
    }
}

/// The name of an edit event's step.
fn step_name<V>(event: &craft_behavior::EditEvent<V>) -> &'static str {
    use craft_behavior::EditEvent;
    match event {
        EditEvent::Begin { .. } => "begin",
        EditEvent::Update { .. } => "update",
        EditEvent::Commit { .. } => "commit",
        EditEvent::Cancel { .. } => "cancel",
        EditEvent::Set { .. } => "set",
        _ => "other",
    }
}

/// Wheel notches over a field are one edit gesture, one undo step: Begin, then Updates, and one Commit when the
/// wheel gesture ends. A value written from outside while the run is open cancels it, and the end then commits
/// nothing. A notch in the pass whose gap ends a run begins the next run.
#[trial]
fn a_wheel_run_on_a_field_is_one_edit_gesture() {
    use craft_behavior::GESTURE_GAP;
    use craft_ui::testing::Fixture;
    use craft_ui::testing::pointer::{moved, wheel_lines};
    use std::rc::Rc;
    let value = Rc::new(RefCell::new(Some(START)));
    let steps: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let (held, seen) = (value.clone(), steps.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut guard = held.borrow_mut();
        let out = NumberField::new("Size", Bind::bound(&mut *guard)).id_salt("size").range(0.0..=100.0).show(ui);
        for event in &out.edit.events {
            seen.borrow_mut().push(step_name(event));
        }
    });
    fixture.settle();
    let field = fixture.by_label("Size").expect("the field");
    fixture.click(&field).settle();
    fixture.frame(vec![moved(field.rect.center())]);
    steps.borrow_mut().clear();
    for _ in 0..3 {
        fixture.frame(vec![wheel_lines(1.0)]);
    }
    assert_ne!(*value.borrow(), Some(START), "the notches moved the value");
    {
        let steps = steps.borrow();
        assert!(steps.len() >= 2 && steps[0] == "begin", "the first notch begins the run: {steps:?}");
        assert!(steps[1..].iter().all(|step| *step == "update"), "later notches update it, nothing commits yet: {steps:?}");
    }
    fixture.advance(GESTURE_GAP * 2.0).idle(3);
    {
        let steps = steps.borrow();
        assert_eq!(steps.iter().filter(|step| **step == "begin").count(), 1, "one run: {steps:?}");
        assert_eq!(steps.iter().filter(|step| **step == "commit").count(), 1, "the gesture's end commits once: {steps:?}");
        assert_eq!(steps.last(), Some(&"commit"), "{steps:?}");
        assert!(!steps.contains(&"set"), "no notch is a one-shot set: {steps:?}");
    }

    steps.borrow_mut().clear();
    for _ in 0..2 {
        fixture.frame(vec![wheel_lines(1.0)]);
    }
    *value.borrow_mut() = Some(77.0);
    fixture.idle(2);
    assert!(steps.borrow().contains(&"cancel"), "a value written from outside cancels the run: {:?}", steps.borrow());
    fixture.advance(GESTURE_GAP * 2.0).idle(3);
    assert!(!steps.borrow().contains(&"commit"), "the end commits no cancelled run: {:?}", steps.borrow());
    assert_eq!(*value.borrow(), Some(77.0), "the outside value stays");

    // The pass that sees the gap end one run also carries the next notch: the first run commits, the notch begins
    // the second.
    steps.borrow_mut().clear();
    for _ in 0..2 {
        fixture.frame(vec![wheel_lines(1.0)]);
    }
    fixture.advance(GESTURE_GAP * 2.0).frame(vec![wheel_lines(1.0)]);
    fixture.advance(GESTURE_GAP * 2.0).idle(3);
    let gestures: Vec<&str> = steps.borrow().iter().copied().filter(|step| matches!(*step, "begin" | "commit")).collect();
    assert_eq!(gestures, ["begin", "commit", "begin", "commit"], "two edit gestures, the first committed: {:?}", steps.borrow());
}

/// An arrow over a typed sum applies the sum first and steps from it, and the field keeps focus.
#[trial]
fn arrow_keys_step_from_a_typed_sum() {
    use craft_ui::testing::Fixture;
    use std::rc::Rc;
    let value = Rc::new(Cell::new(Some(100.0)));
    let id = Rc::new(Cell::new(egui::Id::NULL));
    let (held, seen) = (value.clone(), id.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut now = held.get();
        let out = NumberField::new("Size", Bind::bound(&mut now)).id_salt("size").range(0.0..=1000.0).show(ui);
        held.set(now);
        seen.set(out.response.id);
    });
    fixture.settle();
    let field = fixture.by_label("Size").expect("the field");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("5+5").settle();
    fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
    assert_eq!(value.get(), Some(11.0), "Arrow Up applies 5+5 and steps from 10");
    fixture.key(Key::ArrowDown, Modifiers::NONE).settle();
    assert_eq!(value.get(), Some(10.0), "Arrow Down steps back");
    assert!(fixture.focused_stop().is_some_and(|(stop, _)| stop == id.get()), "the field keeps focus");
}

/// A drag on the popup slider of a field whose app applies each update, as the Layers panel's Opacity does, runs each
/// drag frame once: an edit of a given value reruns the frame only when something read it earlier in the frame.
#[trial]
fn a_popup_slider_drag_on_a_given_value_runs_one_pass_a_frame() {
    use craft_ui::contract::Role;
    use craft_ui::testing::pointer::moved;
    let model = std::rc::Rc::new(Cell::new(50.0));
    let host = model.clone();
    let mut fixture = craft_ui::testing::Fixture::new(move |ui| {
        let out = NumberField::new("Opacity", Bind::given(Some(host.get()))).range(0.0..=100.0).popup_slider(0.0..=100.0).show(ui);
        if let Some(value) = out.edit.events.value().copied().flatten() {
            host.set(value);
        }
    });
    fixture.settle();
    let chevron = fixture.by_role(Role::BUTTON).first().expect("the chevron").center();
    fixture.drag_hold_at(chevron, Vec2::new(10.0, 0.0));
    fixture.settle();
    let before = model.get();
    let mut passes = Vec::new();
    for step in 1..=6 {
        fixture.frame(vec![moved(chevron + Vec2::new(10.0 + 5.0 * step as f32, 0.0))]);
        passes.push(fixture.passes());
    }
    assert!(model.get() > before, "the drag moved the app's value");
    assert!(passes.iter().all(|&n| n == 1), "slider drag frames ran {passes:?} passes");
}

/// The wheel over a field's open presets list scrolls the list, though the field keeps focus after Enter: the focused
/// field's share of the wheel stops at a popup opened above it.
#[trial]
fn the_wheel_over_an_open_presets_list_scrolls_the_list_while_the_field_keeps_focus() {
    use craft_ui::contract::Role;
    use craft_ui::testing::Fixture;
    use craft_ui::testing::pointer::{moved, wheel_lines};
    let value = std::rc::Rc::new(Cell::new(Some(START)));
    let held = value.clone();
    let presets: Vec<Entry<NumberPick>> = (1..=60).map(|n| Entry::new(NumberPick::Number(f64::from(n)), format!("{n} px"))).collect();
    let mut fixture = Fixture::new(move |ui| {
        let mut now = held.get();
        NumberField::new("Size", Bind::bound(&mut now)).id_salt("size").presets(&presets).show(ui);
        held.set(now);
    });
    fixture.settle();
    let field = fixture.by_label("Size").expect("the field");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("500").key(Key::Enter, Modifiers::NONE).settle();
    let focus = fixture.focused_stop();
    assert!(focus.is_some(), "the field keeps focus after Enter");
    let chevron = fixture.by_role(Role::BUTTON).first().expect("the presets chevron").center();
    fixture.click_at(chevron).settle();
    assert_eq!(fixture.focused_stop(), focus, "opening the list leaves focus on the field");
    let first_row = |fixture: &Fixture| fixture.by_label("1 px").map(|row| row.rect);
    let before = first_row(&fixture).expect("the list is open");
    let committed = value.get();
    fixture.frame(vec![moved(before.center())]);
    for _ in 0..3 {
        fixture.frame(vec![wheel_lines(-1.0)]);
    }
    fixture.settle();
    assert!(first_row(&fixture).is_none_or(|row| row.top() < before.top()), "the list scrolled");
    assert_eq!(value.get(), committed, "the field's value stays");
}

/// Alt+Down opens a field's presets for the keyboard, Enter picks the one the lead is on and the field keeps focus,
/// and Escape closes the list again with the draft and the focus left as they were.
#[trial]
fn alt_down_opens_the_presets_enter_picks_without_the_dialog_and_escape_returns() {
    use craft_ui::testing::Fixture;
    use std::rc::Rc;
    let value = Rc::new(Cell::new(Some(START)));
    let picked: Rc<Cell<Option<NumberPick>>> = Rc::default();
    let id = Rc::new(Cell::new(egui::Id::NULL));
    let (held, pick, seen) = (value.clone(), picked.clone(), id.clone());
    let presets = vec![Entry::new(NumberPick::Number(12.0), "12 px"), Entry::new(NumberPick::Number(24.0), "24 px")];
    let mut fixture = Fixture::new(move |ui| {
        let mut now = held.get();
        let out = NumberField::new("Size", Bind::bound(&mut now)).id_salt("size").presets(&presets).show(ui);
        held.set(now);
        seen.set(out.response.id);
        if out.preset.is_some() {
            pick.set(out.preset);
        }
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    let focus = fixture.focused_stop();
    assert!(focus.is_some(), "Tab puts the field in focus");

    fixture.key(Key::ArrowDown, Modifiers::ALT).settle();
    assert!(fixture.by_label("12 px").is_some(), "Alt+Down opens the presets");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert!(matches!(picked.take(), Some(NumberPick::Number(n)) if n == 12.0), "Enter picks the first preset, the lead of an unchosen list");
    assert!(fixture.by_label("12 px").is_none(), "and closes the list");
    assert_eq!(fixture.focused_stop(), focus, "the field has the focus again");

    fixture.key(Key::A, Modifiers::COMMAND).type_text("5").settle();
    fixture.key(Key::ArrowDown, Modifiers::ALT).settle();
    assert!(fixture.by_label("12 px").is_some(), "Alt+Down opens the presets over a draft");
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(fixture.by_label("12 px").is_none(), "Escape closes the list");
    assert!(picked.take().is_none(), "and picks nothing");
    assert_eq!(fixture.tree().value(id.get()), Some("5"), "the draft stays in the field");
    assert_eq!(fixture.focused_stop(), focus, "and the field keeps the focus");
}

/// A percent field shows one decimal and its sign right after the number ("100.0%"), not the sign apart at the end of
/// the field.
#[trial]
fn a_percent_field_reads_one_decimal_then_the_sign() {
    let mut fixture = craft_ui::testing::Fixture::new(|ui| {
        NumberField::new("Scale", Bind::bound(&mut Some(100.0f64))).unit_kind(UnitKind::Percent).range(1.0..=1000.0).show(ui);
    });
    fixture.settle();
    let painted = |text: &str| fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect);
    let number = painted("100.0")
        .unwrap_or_else(|| panic!("the value shows one decimal: {:?}", fixture.texts().iter().map(|r| r.text.clone()).collect::<Vec<_>>()));
    let sign = painted("%").unwrap_or_else(|| panic!("the field shows the sign"));
    assert!(sign.min.x - number.max.x <= 5.0, "the sign follows the number closely: {number:?} {sign:?}");
}

/// A percent field whose committed values carry no decimals (`commit_decimals(0)`, a paint bar's Opacity) shows none,
/// "10%"; one that sets no decimals of its own keeps the one-decimal look, "10.0%".
#[trial]
fn a_percent_field_shows_the_decimals_its_caller_commits() {
    let shown = |commit: Option<usize>| {
        let mut value = Some(10.0f64);
        let mut fixture = craft_ui::testing::Fixture::new(move |ui| {
            let field = NumberField::new("Opacity", Bind::bound(&mut value)).unit_kind(UnitKind::Percent).range(0.0..=100.0);
            let _ = match commit {
                Some(decimals) => field.commit_decimals(decimals).show(ui),
                None => field.show(ui),
            };
        });
        fixture.settle();
        fixture.texts().iter().map(|run| run.text.clone()).collect::<Vec<_>>()
    };
    let whole = shown(Some(0));
    assert!(whole.iter().any(|text| text == "10") && !whole.iter().any(|text| text == "10.0"), "commit_decimals(0) shows 10%: {whole:?}");
    let default = shown(None);
    assert!(default.iter().any(|text| text == "10.0"), "with no decimals set the field shows 10.0%: {default:?}");
}
