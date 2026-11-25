//! `SliderField`: Tab visits its slider and its number field in the order a reader meets them on the row.

use craft_ui::contract::{Bind, Props as _, Role, Show as _, ShowContainer as _, TooltipSpec};
use craft_ui::form::{Field, SliderField, SliderFieldLayout};
use craft_ui::layout_components::Row;
use craft_ui::runtime::env::Direction;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, Target, trial};
use craft_ui::text::NumberField;
use egui::{Key, Modifiers};

/// A focused control: its role and its name.
type Stop = (Option<Role>, Option<String>);

/// The control keyboard focus is on.
fn focused(fixture: &Fixture) -> Stop {
    let tree = fixture.tree();
    let key = tree.focused_key();
    (tree.role_of(key), tree.name_of(key).map(str::to_owned))
}

/// A row that holds a number field, a slider field laid out as `layout` and another number field, under `direction`.
/// A disabled slider field carries a reason, so its first part stays a Tab stop.
fn row(layout: SliderFieldLayout, direction: Direction, enabled: bool) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        Row::new().show(ui, |ui, _| {
            let mut before = Some(1.0);
            let mut size = 20.0;
            let mut after = Some(2.0);
            NumberField::new("Before", Bind::bound(&mut before)).range(0.0..=10.0).show(ui);
            SliderField::new("Size", Bind::bound(&mut size), 0.0..=100.0)
                .layout(layout)
                .enabled(enabled)
                .tooltip(TooltipSpec::new("The brush size").disabled_text("Pick a brush first"))
                .show(ui);
            NumberField::new("After", Bind::bound(&mut after)).range(0.0..=10.0).show(ui);
        });
    });
    fixture.direction(direction).settle();
    fixture
}

/// The controls `count` presses of Tab reach from the start of `fixture`, then those `count - 1` presses of Shift-Tab
/// reach back from the last one.
fn tabs(fixture: &mut Fixture, count: usize) -> (Vec<Stop>, Vec<Stop>) {
    let mut forward = Vec::new();
    for _ in 0..count {
        fixture.key(Key::Tab, Modifiers::NONE).settle();
        forward.push(focused(fixture));
    }
    let mut back = vec![focused(fixture)];
    for _ in 1..count {
        fixture.key(Key::Tab, Modifiers::SHIFT).settle();
        back.push(focused(fixture));
    }
    (forward, back)
}

/// A part of the slider field.
fn size(role: Role) -> Stop {
    (Some(role), Some("Size".to_owned()))
}

/// The number field `name` beside the slider field.
fn other(name: &str) -> Stop {
    (Some(Role::SPIN_BUTTON), Some(name.to_owned()))
}

/// In both directions, Tab goes from the field before the slider field to its slider, then its number field, then the
/// field after it; Shift-Tab goes back the same way. In the inline layout the slider comes first on the line; in the
/// stacked one the number field is on the caption line above the slider.
#[trial]
fn tab_visits_a_slider_fields_parts_in_reading_order() {
    for (layout, [first, second]) in [
        (SliderFieldLayout::Inline, [size(Role::SLIDER), size(Role::SPIN_BUTTON)]),
        (SliderFieldLayout::Stacked, [size(Role::SPIN_BUTTON), size(Role::SLIDER)]),
    ] {
        let order = vec![other("Before"), first, second, other("After")];
        let reversed: Vec<Stop> = order.iter().rev().cloned().collect();
        for direction in [Direction::Ltr, Direction::Rtl] {
            let (forward, back) = tabs(&mut row(layout, direction, true), order.len());
            assert_eq!(forward, order, "Tab order of a {layout:?} slider field under {direction:?}");
            assert_eq!(back, reversed, "Shift-Tab order of a {layout:?} slider field under {direction:?}");
        }
    }
}

/// A slider field laid out as `layout` under `direction`: inline in a `Field` row that labels it, stacked under its own
/// caption.
fn labelled(layout: SliderFieldLayout, direction: Direction) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let mut size = 20.0;
        if matches!(layout, SliderFieldLayout::Inline) {
            Field::new("Size").show(ui, |ui, _| SliderField::new("Size", Bind::bound(&mut size), 0.0..=100.0).layout(layout).show(ui));
        } else {
            SliderField::new("Size", Bind::bound(&mut size), 0.0..=100.0).layout(layout).show(ui);
        }
    });
    fixture.direction(direction).settle();
    fixture
}

/// The label that names the slider field: the "Size" label node.
fn label(fixture: &Fixture) -> Target {
    let tree = fixture.tree();
    let key = tree.keys().into_iter().find(|key| tree.name_of(*key) == Some("Size") && tree.role_of(*key) == Some(Role::LABEL));
    key.and_then(|key| fixture.target(key)).unwrap_or_else(|| panic!("no label names the slider field"))
}

/// A click on the label of a slider field focuses its number field, the part that takes typing, whichever part Tab
/// reaches first: inline, where the slider is the first stop, and stacked, under its own caption, in both directions.
/// A pointer click and an accessibility click both go there.
#[trial]
fn a_click_on_a_slider_fields_label_focuses_its_number_field() {
    for layout in [SliderFieldLayout::Inline, SliderFieldLayout::Stacked] {
        for direction in [Direction::Ltr, Direction::Rtl] {
            let mut fixture = labelled(layout, direction);
            let caption = label(&fixture);
            fixture.click(&caption).settle();
            assert_eq!(focused(&fixture), size(Role::SPIN_BUTTON), "a click on a {layout:?} slider field's label under {direction:?}");

            let mut fixture = labelled(layout, direction);
            let caption = label(&fixture);
            fixture.access(AccessActionKind::Click, &caption).settle();
            assert_eq!(focused(&fixture), size(Role::SPIN_BUTTON), "an access click on a {layout:?} slider field's label under {direction:?}");
        }
    }
}

/// An enabled inline slider field's tooltip describes both its parts: the slider Tab reaches first and the number field
/// a click on the row's label focuses.
#[trial]
fn an_inline_slider_fields_tooltip_describes_both_its_parts() {
    let fixture = row(SliderFieldLayout::Inline, Direction::Ltr, true);
    let tree = fixture.tree();
    for role in [Role::SLIDER, Role::SPIN_BUTTON] {
        let part = tree.with_role(role).into_iter().find(|key| tree.name_of(*key) == Some("Size"));
        let description = part.and_then(|key| tree.description_of(key));
        assert_eq!(description, Some("The brush size"), "the {role:?} of an inline slider field");
    }
}

/// A disabled slider field's reason is carried by the part read first, which stays the one Tab stop of the field: the
/// slider on the inline line, the number field when stacked.
#[trial]
fn a_disabled_slider_fields_reason_is_on_the_part_read_first() {
    for (layout, first) in [(SliderFieldLayout::Inline, size(Role::SLIDER)), (SliderFieldLayout::Stacked, size(Role::SPIN_BUTTON))] {
        let order = vec![other("Before"), first, other("After")];
        let (forward, _) = tabs(&mut row(layout, Direction::Ltr, false), order.len());
        assert_eq!(forward, order, "Tab order of a disabled {layout:?} slider field");
    }
}
