//! Stories of `RadioGroup` and `Radio`.

use super::{Radio, RadioGroup};
use crate::contract::{Bind, FieldLayout, Props as _, Show as _, ShowContainer as _, SideLabel, Validity};
use crate::display::{Shade, Text};
use crate::layout_components::{Row, Stack};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{IconId, Mixed};
use egui::{Response, Ui};

/// A label too long for one line of the card: it wraps, the button on its first line.
const LONG_LABEL: &str = "Show the transform controls on the selected layer whenever the move tool is active";

/// The unit names of the groups story.
const UNITS: [&str; 5] = ["Pixels", "Millimetres", "Inches", "Points", "Picas"];

/// Members spread over nested layouts, with the group's body between them: one group, one value.
fn spread(ui: &mut Ui, group: RadioGroup<'_, u8>) -> (StoryOutput, Option<Response>) {
    let mut second = None;
    let shown = group.show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            Radio::new(1u8).label("Small").show(ui);
            Row::new().show(ui, |ui, _| {
                second = Some(Radio::new(2u8).label("Medium").show(ui));
                Radio::new(3u8).label("Large").show(ui);
            });
        });
    });
    (shown.into(), second)
}

/// Three options in one column.
fn column(ui: &mut Ui, group: RadioGroup<'_, u8>) -> StoryOutput {
    group
        .show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                for (value, label) in [(1u8, "Small"), (2, "Medium"), (3, "Large")] {
                    Radio::new(value).label(label).show(ui);
                }
            });
        })
        .into()
}

fn pick_second(fixture: &mut Fixture) {
    if let Some(medium) = fixture.by_label("Medium") {
        fixture.click(&medium);
    }
}

fn pick_millimetres(fixture: &mut Fixture) {
    if let Some(millimetres) = fixture.by_label("Millimetres") {
        fixture.click(&millimetres);
    }
}

/// Groups of three and five units side by side, the fifth group's Points disabled, and what each picked.
#[story(RadioGroup<'static, u8>, covers = [Default], drive = Drive::Custom(pick_millimetres))]
fn radio_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut output = StoryOutput::default();
    Stack::new().show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            for (count, key, first) in [(3usize, "three", 0u8), (5, "five", 2)] {
                let shown = RadioGroup::new(format!("Units ({count})"), Bind::bound(data.value(key, first))).show(ui, |ui, _| {
                    Stack::new().show(ui, |ui, _| {
                        for (at, unit) in UNITS.iter().take(count).enumerate() {
                            Radio::new(at as u8).label(*unit).enabled(at != 3).show(ui);
                        }
                    });
                });
                if count == 3 {
                    output = shown.into();
                }
            }
        });
        let [three, five] = [*data.value("three", 0u8), *data.value("five", 2u8)].map(|at| UNITS[usize::from(at)]);
        Text::new(format!("Selected: {three}, {five}")).shade(Shade::Muted).show(ui);
    });
    output
}

/// Members in nested layouts: the first alone, the others in a row inside the group's body.
#[story(RadioGroup<'static, u8>, covers = [], drive = Drive::Custom(pick_second))]
fn radio_group_nested(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    spread(ui, RadioGroup::new("Size", Bind::bound(data.value("size", 1u8)))).0
}

#[story(RadioGroup<'static, u8>, covers = [Mixed], drive = Drive::Never)]
fn radio_group_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    column(ui, RadioGroup::new("Size", Bind::bound_mixed(data.value("mixed", Mixed::Mixed))))
}

#[story(RadioGroup<'static, u8>, covers = [Invalid], drive = Drive::Never)]
fn radio_group_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_large = |size: &u8| if *size == 3 { Validity::Refuse(Message::new("Too large")) } else { Validity::Ok };
    column(ui, RadioGroup::new("Size", Bind::bound(data.value("large", 3u8)).check(refuse_large)))
}

/// Rows that carry an icon and a count through the body's label step, the label after the button.
#[story(RadioGroup<'static, u8>, covers = [], drive = Drive::Never)]
fn radio_group_with_icons_and_counts(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = [(1u8, "Layers", "list", "12"), (2, "Pages", "grid", "4"), (3, "Masks", "eye", "0")];
    RadioGroup::new("Show", Bind::bound(data.value("show", 1u8)))
        .show(ui, |ui, cx| {
            for (value, label, icon, count) in rows {
                let side = SideLabel::new().layout(FieldLayout::After).icon(IconId::new(icon)).detail(count);
                cx.labelled(ui, value, label.into(), side, Radio::new(value));
            }
        })
        .into()
}

/// Rows with a description under the label: the `SideLabel`'s detail joins the radio's accessible description.
#[story(RadioGroup<'static, u8>, covers = [], drive = Drive::Never)]
fn radio_group_with_descriptions(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows =
        [(1u8, "Fast", "Lower quality, smaller file"), (2, "Balanced", "The default for most photos"), (3, "Best", "Full quality, largest file")];
    RadioGroup::new("Export", Bind::bound(data.value("export", 2u8)))
        .show(ui, |ui, cx| {
            for (value, label, description) in rows {
                let side = SideLabel::new().layout(FieldLayout::After).detail(description);
                cx.labelled(ui, value, label.into(), side, Radio::new(value));
            }
        })
        .into()
}

// The shown member is the selected one, where Tab arrives, so the key drives reach it.
/// A zoom choice: an option with a value beside it, a long label, a disabled option, and a disabled group, picked
/// and not.
#[story(Radio<u8>, covers = [Default])]
fn radio(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut selected = None;
    Stack::new().show(ui, |ui, _| {
        let zoom = Bind::bound(data.value("zoom", 0u8));
        RadioGroup::new("Zoom", zoom).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                selected = Some(Radio::new(0u8).label("Fit to window").show(ui));
                Row::new().show(ui, |ui, _| {
                    Radio::new(1u8).label("Actual size").show(ui);
                    Text::new("100 %").shade(Shade::Muted).show(ui);
                });
                Radio::new(2u8).label(LONG_LABEL).show(ui);
                Radio::new(3u8).label("Print size (no printer)").enabled(false).show(ui);
            });
        });
        RadioGroup::new("Disabled", Bind::bound(data.value("disabled", 0u8))).enabled(false).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                Radio::new(0u8).label("Disabled (selected)").show(ui);
                Radio::new(1u8).label("Disabled").show(ui);
            });
        });
    });
    selected.map(StoryOutput::from).unwrap_or_default()
}
