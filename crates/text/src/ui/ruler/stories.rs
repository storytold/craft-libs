//! Stories of the text ruler: Word's ruler with stops, a hanging indent and the tab selector, the Tabs panel's with a
//! picked stop, and a table row's column borders.

use super::items::FIRST;
use super::{RulerValues, TabAlign, TabLeader, TabStop, TextRuler};
use crate::ui::length::DocUnit;
use craft_behavior::Mixed;
use craft_ui::canvas::Rulers;
use craft_ui::contract::{Show as _, Validity};
use craft_ui::primitives::RulerFace;
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
    let (values, values_at) = data.value_at("values", word());
    // The selector shows in the corner the host gives, here at the band's start edge in the interface's direction.
    let stage = craft_ui::testing::stage(ui);
    let (strip, _, corner) = Rulers::strips(ui, stage);
    let band = strip.union(corner);
    let corner = if craft_ui::runtime::env::Env::of(ui).direction.is_rtl() {
        egui::Rect::from_min_max(egui::pos2(band.max.x - corner.width(), band.min.y), band.max)
    } else {
        corner
    };
    let out = TextRuler::new("Ruler", values_at.bind(&mut *values), COLUMN).unit(inches()).kind(kind).corner(corner).show_at(ui, band);
    if let Some(next) = out.kind {
        *data.value("kind", TabAlign::Left) = next;
    }
    out.into()
}

#[story(TextRuler<'static>, covers = [Mixed], drive = Drive::Never)]
fn paragraphs_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", Mixed::<RulerValues>::Mixed);
    TextRuler::new("Ruler", values_at.bind_mixed(&mut *values), COLUMN).unit(inches()).show(ui).into()
}

#[story(TextRuler<'static>, covers = [Invalid], drive = Drive::Never)]
fn indent_too_wide(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", RulerValues { left_indent: MOST_INDENT + 36.0, ..word() });
    let narrow = |values: &RulerValues| {
        if values.left_indent > MOST_INDENT { Validity::Refuse(Message::new("The indent leaves too little room")) } else { Validity::Ok }
    };
    TextRuler::new("Ruler", values_at.bind(&mut *values).check(narrow), COLUMN).unit(inches()).show(ui).into()
}

#[story(TextRuler<'static>, covers = [], drive = Drive::Never)]
fn tabs_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", RulerValues { stops: sample_stops(), ..RulerValues::default() });
    TextRuler::new("Ruler", values_at.bind(&mut *values), COLUMN).unit(inches()).picked_stop(PICKED).show(ui).into()
}

/// The page's width, points: 8.5 in.
const PAGE: f32 = 612.0;
/// The page's left margin, points: the ruler's zero.
const PAGE_MARGIN: f32 = 72.0;

/// Two gaps in the text width, between three columns.
const TWO_GAPS: [(f32, f32); 2] = [(144.0, 162.0), (306.0, 324.0)];
/// Three gaps in the text width, between four columns.
const THREE_GAPS: [(f32, f32); 3] = [(108.0, 126.0), (225.0, 243.0), (342.0, 360.0)];

/// Word's ruler over a page, its zero at the left margin, in `face` (unset: the component set's).
fn over_a_page(ui: &mut Ui, data: &mut StoryData, face: Option<RulerFace>) -> StoryOutput {
    let (values, values_at) = data.value_at("values", word());
    let stage = craft_ui::testing::stage(ui);
    let (strip, _, _) = Rulers::strips(ui, stage);
    // The page starts at the strip's start edge: its right edge in a right-to-left interface.
    let (origin, extent) = if craft_ui::runtime::env::Env::of(ui).direction.is_rtl() {
        (strip.max.x - PAGE_MARGIN, egui::Rangef::new(strip.max.x - PAGE, strip.max.x))
    } else {
        (strip.min.x + PAGE_MARGIN, egui::Rangef::new(strip.min.x, strip.min.x + PAGE))
    };
    let mut ruler = TextRuler::new("Ruler", values_at.bind(&mut *values), COLUMN).unit(inches()).origin(origin).extent(extent).scale(1.0);
    if let Some(face) = face {
        ruler = ruler.face(face);
    }
    ruler.show_at(ui, strip).into()
}

#[story(TextRuler<'static>, covers = [], drive = Drive::Never)]
fn page_wide(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    over_a_page(ui, data, None)
}

/// The edge face: ticks rise from the strip's bottom, each number beside its whole-unit tick.
#[story(TextRuler<'static>, covers = [], drive = Drive::Never)]
fn edge_face(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    over_a_page(ui, data, Some(RulerFace::Edge))
}

/// The centred face (the productivity set's): a bar inset in the strip, ticks centred on it, numbers in place of the
/// whole-unit ticks over the column only.
#[story(TextRuler<'static>, covers = [], drive = Drive::Never)]
fn centred_face(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    over_a_page(ui, data, Some(RulerFace::Centred))
}

#[story(TextRuler<'static>, covers = [], drive = Drive::Never)]
fn two_column_gaps(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", RulerValues::default());
    TextRuler::new("Ruler", values_at.bind(&mut *values), COLUMN).unit(inches()).gaps(&TWO_GAPS).show(ui).into()
}

/// Fixed column gaps: shaded as on any ruler, with no grip and no resize cursor, where the document's column settings
/// fix the gaps.
#[story(TextRuler<'static>, covers = [], drive = Drive::Never)]
fn fixed_column_gaps(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", RulerValues::default());
    TextRuler::new("Ruler", values_at.bind(&mut *values), COLUMN).unit(inches()).gaps(&TWO_GAPS).gaps_draggable(false).show(ui).into()
}

#[story(TextRuler<'static>, covers = [], drive = Drive::Never)]
fn three_column_gaps(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", RulerValues::default());
    TextRuler::new("Ruler", values_at.bind(&mut *values), COLUMN).unit(inches()).gaps(&THREE_GAPS).show(ui).into()
}

#[story(TextRuler<'static>, covers = [], drive = Drive::Never)]
fn table_column_borders(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", RulerValues::default());
    TextRuler::new("Ruler", values_at.bind(&mut *values), COLUMN).unit(inches()).columns(&[0.0, 156.0, 312.0, COLUMN]).show(ui).into()
}
