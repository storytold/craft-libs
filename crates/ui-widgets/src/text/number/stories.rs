//! Stories of `NumberField`.

use super::{NamedOr, NumberField, NumberLook, NumberName, NumberPick, NumberPrecision, StepArrows, UnitPlacement};
use crate::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Validity};
use crate::display::Text;
use crate::form::{Field, FieldGroup};
use crate::layout_components::Row;
use crate::runtime::env::{Env, LengthUnits, Message, UnitKind, with_env};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, pointer, story};
use craft_behavior::{Mixed, NumberCodec, Reader};
use craft_theme::ControlSize;
use egui::{Key, Modifiers, Ui};

const SIZE: f64 = 12.5;
const HUGE: f64 = 123_456_789_012_345.0;
/// The number the Auto word reads as.
const AUTO: f64 = 0.0;
const RANGE: std::ops::RangeInclusive<f64> = 0.0..=100.0;
/// The range of the Invalid rows: the value 7 lies inside it.
const COLUMNS: std::ops::RangeInclusive<f64> = 1.0..=12.0;

/// A row of the states story, in the group's form rows.
fn row(label: &'static str) -> Field {
    Field::new(label).id_salt(label)
}

/// The zoom stops of the old PhotoCraft zoom field.
const ZOOM_STOPS: [f64; 11] = [12.5, 25.0, 50.0, 100.0, 200.0, 400.0, 800.0, 1600.0, 3200.0, 6400.0, 12_800.0];

/// The next zoom stop above `value` for a positive `direction`, the previous one below it otherwise; the stops do not
/// scale with a modifier.
fn zoom_step(value: f64, direction: i32, _scale: f64) -> f64 {
    let mut stops = ZOOM_STOPS.into_iter();
    match direction > 0 {
        true => stops.find(|stop| *stop > value).unwrap_or(value),
        false => stops.rfind(|stop| *stop < value).unwrap_or(value),
    }
}

/// A check no value passes: the field reads as invalid whatever it holds.
fn refuse_all(_: &Option<f64>) -> Validity<Option<f64>> {
    Validity::Refuse(Message::new("Not accepted"))
}

#[story(NumberField<'static>, covers = [Default], drive = Drive::Type("42"))]
fn number_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("control_size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let units = Env::of(ui).view().units();
    let out = FieldGroup::new("Number field states").show(ui, |ui, _| {
        let first = row("Plain").show(ui, |ui, _| NumberField::new("Plain", data.bind("plain", Some(42.0))).size(size).show(ui).edit);
        row("With unit").show(ui, |ui, _| {
            NumberField::new("Gap", data.bind("gap", Some(SIZE)))
                .range(RANGE)
                .step(0.5)
                .unit(units.symbol(UnitKind::Millimetres))
                .size(size)
                .show(ui);
        });
        Row::new().show(ui, |ui, _| {
            row("Unit after").show(ui, |ui, _| {
                NumberField::new("Angle", data.bind("angle", Some(120.0)))
                    .range(0.0..=360.0)
                    .unit_kind(UnitKind::Degrees)
                    .unit_placement(UnitPlacement::After)
                    .size(size)
                    .show(ui);
            });
            Text::new(units.symbol(UnitKind::Degrees)).show(ui);
        });
        row("Mixed").show(ui, |ui, _| {
            NumberField::new("Mixed size", data.bind_mixed("mixed", Mixed::<Option<f64>>::Mixed))
                .unit(units.symbol(UnitKind::Points))
                .size(size)
                .show(ui);
        });
        row("Derived").show(ui, |ui, _| {
            NumberField::new("Leading", data.bind("derived", Some(SIZE))).unit_kind(UnitKind::Points).derived(true).size(size).show(ui);
        });
        row("Invalid").show(ui, |ui, _| {
            NumberField::new("Columns", data.bind("invalid", Some(7.0)).check(refuse_all)).range(COLUMNS).size(size).show(ui);
        });
        row("Pop-up slider").show(ui, |ui, _| {
            NumberField::new("Opacity", data.bind("opacity", Some(100.0))).range(RANGE).unit("%").popup_slider(RANGE).size(size).show(ui);
        });
        row("Zoom stops").show(ui, |ui, _| {
            NumberField::new("Zoom", data.bind("zoom", Some(100.0))).range(0.1..=12_800.0).unit("%").stepper(zoom_step).size(size).show(ui);
        });
        row("Disabled").show(ui, |ui, _| {
            NumberField::new("Disabled", Bind::bound(&mut Some(64.0)))
                .range(RANGE)
                .step(0.5)
                .unit(units.symbol(UnitKind::Pixels))
                .enabled(false)
                .size(size)
                .show(ui);
        });
        row("Not a number").show(ui, |ui, _| {
            // Held outside the story data: NaN is never equal to itself, so the suite would read it as changed.
            NumberField::new("Not a number", Bind::bound(&mut Some(f64::NAN))).size(size).show(ui);
        });
        first.inner
    });
    out.inner.flatten().map(StoryOutput::from).unwrap_or_default()
}

#[story(NumberField<'static>, covers = [LongText], drive = Drive::Never)]
fn number_field_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    NumberField::new("Size", data.bind("long", Some(HUGE))).show(ui).edit.into()
}

#[story(NumberField<'static>, covers = [Mixed], drive = Drive::Never)]
fn number_field_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    NumberField::new("Size", data.bind_mixed("mixed", Mixed::<Option<f64>>::Mixed)).show(ui).edit.into()
}

/// A value that follows from other settings (an automatic leading) reads in parentheses, with its unit, until it is edited.
#[story(NumberField<'static>, covers = [], drive = Drive::Never)]
fn number_field_derived(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    NumberField::new("Leading", data.bind("derived", Some(SIZE))).unit_kind(UnitKind::Points).derived(true).show(ui).edit.into()
}

#[story(NumberField<'static>, covers = [Invalid], drive = Drive::Never)]
fn number_field_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    NumberField::new("Size", data.bind("invalid", Some(7.0)).check(refuse_all)).range(COLUMNS).show(ui).edit.into()
}

/// Focuses the field and opens its list or slider with Alt+Down.
fn open_popup(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.idle(4);
}

#[story(NumberField<'static>, covers = [Open], drive = Drive::Custom(open_popup))]
fn number_field_presets(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let presets = [
        Entry::new(NumberPick::Choice(0), "Auto"),
        Entry::separator(),
        Entry::new(NumberPick::Number(8.0), "Small"),
        Entry::new(NumberPick::Number(24.0), "Large"),
    ];
    NumberField::new("Size", data.bind("presets", Some(SIZE))).presets(&presets).show(ui).edit.into()
}

#[story(NumberField<'static>, covers = [Open], drive = Drive::Custom(open_popup))]
fn number_field_popup_slider(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    NumberField::new("Size", data.bind("slider", Some(SIZE))).range(RANGE).popup_slider(RANGE).show(ui).edit.into()
}

#[story(NumberField<'static>, covers = [], drive = Drive::Never)]
fn number_field_step_arrows(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    NumberField::new("Size", data.bind("arrows", Some(SIZE))).step_arrows(StepArrows::Leading).show(ui).edit.into()
}

/// Presses the up arrow and keeps it down, short of the repeat delay: one step, then the held arrow. The repeat is
/// the step-arrow trial's: every repeated step is an edit of its own, and a drive ends its edit once.
fn hold_up(fixture: &mut Fixture) {
    fixture.settle();
    let at = fixture.root_id().and_then(|root| fixture.part_rect(root, NumberField::STEP_UP)).map(|rect| rect.center());
    if let Some(at) = at {
        fixture.frame(vec![pointer::moved(at)]);
        fixture.frame(vec![pointer::press(at)]);
        fixture.idle(2);
    }
}

#[story(NumberField<'static>, covers = [], drive = Drive::Custom(hold_up))]
fn number_field_step_arrows_held(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    NumberField::new("Size", data.bind("held", Some(SIZE))).step_arrows(StepArrows::Leading).show(ui).edit.into()
}

#[story(NumberField<'static>, covers = [], drive = Drive::Never)]
fn number_field_step_arrows_disabled(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    NumberField::new("Size", Bind::bound(&mut Some(SIZE))).step_arrows(StepArrows::Leading).enabled(false).show(ui).edit.into()
}

#[story(NumberField<'static>, covers = [], drive = Drive::Type("Auto"))]
fn number_field_auto(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let codec = NumberCodec::new(Reader::new(), 1).or_parse(|text| text.trim().eq_ignore_ascii_case("auto").then_some(AUTO));
    NumberField::new("Size", data.bind("auto", Some(SIZE))).codec(codec).range(RANGE).show(ui).edit.into()
}

/// The one name of the named-length story's presets.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LengthName {
    Auto,
}

impl NumberName for LengthName {
    fn id(self) -> u32 {
        0
    }

    fn from_id(id: u32) -> Option<Self> {
        (id == 0).then_some(LengthName::Auto)
    }
}

/// A named-or-length field: "Auto" is a name, and the lengths are stored in points and shown in inches. It is a story
/// of the registered `NumberField` card: the named instantiation shares its parts, style and value cases, and a
/// registration of its own would need a full set of state stories for one builder.
#[story(NumberField<'static>, covers = [], drive = Drive::Never)]
fn number_field_named_length(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let presets = [Entry::new(NumberPick::Choice(0), "Auto")];
    let lengths = LengthUnits::new(UnitKind::Points, UnitKind::Inches, 72.0);
    NumberField::named("Leading", data.bind("named_length", NamedOr::<LengthName>::Number(36.0)))
        .lengths(lengths)
        .presets(&presets)
        .show(ui)
        .edit
        .into()
}

#[story(NumberField<'static>, covers = [], drive = Drive::Type("7"))]
fn number_field_snapped(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    NumberField::new("Size", data.bind("snapped", Some(SIZE))).range(RANGE).step(5.0).snap_typed(true).show(ui).edit.into()
}

/// Fields in the inline look ([`NumberLook::Inline`]): the number and its unit one run at the field's start, at rest,
/// mixed and disabled. The drive types into the first, so its caret and selection run through the unit too.
fn inline_fields(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let lengths = LengthUnits::new(UnitKind::Points, UnitKind::Points, 72.0);
    let out = with_env(
        ui,
        |overrides| {
            overrides.values.set(NumberLook::Inline);
        },
        |ui| {
            FieldGroup::new("Inline number fields")
                .show(ui, |ui, _| {
                    let first = row("Length").show(ui, |ui, _| NumberField::length("Width", data.bind("width", Some(720.0)), lengths).show(ui).edit);
                    row("Angle").show(ui, |ui, _| {
                        NumberField::new("Angle", data.bind("angle", Some(0.0))).range(0.0..=360.0).unit_kind(UnitKind::Degrees).show(ui);
                    });
                    row("Percent").show(ui, |ui, _| {
                        NumberField::new("Opacity", data.bind("opacity", Some(100.0))).range(RANGE).unit("%").popup_slider(RANGE).show(ui);
                    });
                    row("Mixed").show(ui, |ui, _| {
                        NumberField::length("Height", data.bind_mixed("mixed", Mixed::<Option<f64>>::Mixed), lengths).show(ui);
                    });
                    row("Disabled").show(ui, |ui, _| {
                        NumberField::length("Depth", Bind::bound(&mut Some(64.0)), lengths).enabled(false).show(ui);
                    });
                    first.inner
                })
                .inner
        },
    );
    out.flatten().map(StoryOutput::from).unwrap_or_default()
}

#[story(NumberField<'static>, covers = [], drive = Drive::Type("42"), title = "Inline unit")]
fn number_field_inline(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    inline_fields(ui, data)
}

/// Focuses the first field with Tab: its whole run selected.
fn focus_first(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.idle(2);
}

#[story(NumberField<'static>, covers = [], drive = Drive::Custom(focus_first), title = "Inline unit, focused")]
fn number_field_inline_focused(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    inline_fields(ui, data)
}

/// Fields at the measured precision ([`NumberPrecision::Measured`]): a percentage reads whole, a length to a thousandth
/// of a point.
#[story(NumberField<'static>, covers = [], drive = Drive::Never, title = "Measured precision")]
fn number_field_measured(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let out = with_env(
        ui,
        |overrides| {
            overrides.values.set(NumberPrecision::Measured);
        },
        |ui| {
            FieldGroup::new("Measured precision")
                .show(ui, |ui, _| {
                    let first = row("Percent").show(ui, |ui, _| {
                        NumberField::new("Opacity", data.bind("opacity", Some(60.0))).range(RANGE).unit_kind(UnitKind::Percent).show(ui).edit
                    });
                    row("Length").show(ui, |ui, _| {
                        let lengths = LengthUnits::new(UnitKind::Points, UnitKind::Points, 72.0);
                        NumberField::length("X", data.bind("x", Some(357.1138)), lengths).show(ui);
                    });
                    first.inner
                })
                .inner
        },
    );
    out.flatten().map(StoryOutput::from).unwrap_or_default()
}
