//! Stories of the text ruler: Word's ruler with stops, a hanging indent and the tab selector, the Tabs panel's with a
//! picked stop, and a table row's column borders.

use super::items::FIRST;
use super::{RulerValues, TabAlign, TabLeader, TabStop, TextRuler};
use crate::ui::length::DocUnit;
use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::{Message, UnitKind};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::Ui;

/// The column's width, points: a 6.5 in text width.
const COLUMN: f32 = 468.0;
/// The widest left indent the invalid story accepts, points.
const MOST_INDENT: f32 = 216.0;

fn stop(position: f32, align: TabAlign) -> TabStop {
    TabStop { position, align, leader: TabLeader::None }
}

/// The distance between neighbouring sample stops, points.
const STOP_PITCH: f32 = 22.0;
/// The kinds of stop, in the order the sample stops repeat them.
const KINDS: [TabAlign; 5] = [TabAlign::Left, TabAlign::Center, TabAlign::Right, TabAlign::Decimal, TabAlign::Bar];
/// Stops of each kind.
const STOPS_PER_KIND: usize = 4;
/// The stop the Tabs panel story starts with picked.
const PICKED: usize = 3;

/// Four stops of each kind, one pitch apart, the kinds taking turns.
fn sample_stops() -> Vec<TabStop> {
    (0..KINDS.len() * STOPS_PER_KIND).map(|i| stop((i + 1) as f32 * STOP_PITCH, KINDS[i % KINDS.len()])).collect()
}

/// Word's ruler: the sample stops and a hanging indent.
fn word() -> RulerValues {
    RulerValues { stops: sample_stops(), left_indent: 36.0, first_line_indent: -36.0, right_indent: 0.0 }
}

fn inches() -> DocUnit {
    DocUnit::new(UnitKind::Inches, 96.0)
}

#[story(TextRuler<'static>, covers = [Default], drive = Drive::Drag(Grip::new(FIRST)))]
fn with_stops_and_indents(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let kind = *data.value("kind", TabAlign::Left);
    let values = data.value("values", word());
    let out = TextRuler::new("Ruler", Bind::bound(&mut *values), COLUMN).unit(inches()).kind(kind).show(ui);
    if let Some(next) = out.kind {
        *data.value("kind", TabAlign::Left) = next;
    }
    out.into()
}

#[story(TextRuler<'static>, covers = [Mixed], drive = Drive::Never)]
fn paragraphs_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = data.value("values", Mixed::<RulerValues>::Mixed);
    TextRuler::new("Ruler", Bind::bound_mixed(&mut *values), COLUMN).unit(inches()).show(ui).into()
}

#[story(TextRuler<'static>, covers = [Invalid], drive = Drive::Never)]
fn indent_too_wide(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = data.value("values", RulerValues { left_indent: MOST_INDENT + 36.0, ..word() });
    let narrow = |values: &RulerValues| {
        if values.left_indent > MOST_INDENT { Validity::Refuse(Message::new("The indent leaves too little room")) } else { Validity::Ok }
    };
    TextRuler::new("Ruler", Bind::bound(&mut *values).check(narrow), COLUMN).unit(inches()).show(ui).into()
}

#[story(TextRuler<'static>, covers = [], drive = Drive::Never)]
fn tabs_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = data.value("values", RulerValues { stops: sample_stops(), ..RulerValues::default() });
    TextRuler::new("Ruler", Bind::bound(&mut *values), COLUMN).unit(inches()).picked_stop(PICKED).show(ui).into()
}

#[story(TextRuler<'static>, covers = [], drive = Drive::Never)]
fn table_column_borders(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = data.value("values", RulerValues::default());
    TextRuler::new("Ruler", Bind::bound(&mut *values), COLUMN).unit(inches()).columns(&[0.0, 156.0, 312.0, COLUMN]).show(ui).into()
}
