//! Stories of the page rulers: the vertical ruler with row grips and with the text range shaded, and the rulers around
//! page 2 of a multi-page view, over a table and two columns, and right to left.

use super::{PageRulers, VerticalRuler};
use crate::layout::LayoutOptions;
use crate::setup::Margins;
use crate::ui::fixtures::{DemoPages, PAPER, demo_placed};
use crate::ui::margins::PageEdges;
use crate::ui::view::{PageView, PageViewState};
use craft_text::ui::DocUnit;
use craft_text::ui::ruler::{RulerValues, TabAlign};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::primitives::RulerFace;
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Rangef, Rect, Ui, pos2, vec2};

/// The strip's width the vertical story gives the ruler, points.
const STRIP: f32 = 24.0;
/// Where the page's top edge sits below the stage's top, points.
const PAGE_TOP: f32 = 40.0;
/// The page's distance from the stage's left edge, points.
const PAGE_LEFT: f32 = 40.0;
/// The text range of a Letter page with one inch margins, points from its top.
const TEXT: (f32, f32) = (72.0, 720.0);
/// The table's row bottoms, points from the page's top.
const ROWS: [f32; 3] = [120.0, 168.0, 216.0];
/// The table's column borders, points from the column's start edge.
const BORDERS: [f32; 4] = [0.0, 156.0, 312.0, 468.0];
/// The gap between two text columns, points from the column's start edge.
const GAPS: [(f32, f32); 1] = [(222.0, 246.0)];
/// Pages in the multi-page story's document.
const PAGES: usize = 3;
/// The page of the multi-page story that carries the rulers (the second).
const RULED_PAGE: usize = 1;

fn inches() -> DocUnit {
    DocUnit::new(UnitKind::Inches, 96.0)
}

/// The vertical ruler along a demo page, with `rows`, the text range when `text` is set and `face` when set.
fn vertical(ui: &mut Ui, rows: &[f32], text: Option<Rangef>, face: Option<RulerFace>) -> StoryOutput {
    let full = craft_ui::testing::stage(ui);
    let page = demo_placed(1, pos2(full.min.x + STRIP, full.min.y + PAGE_TOP)).remove(0);
    let strip = Rect::from_min_size(full.min, vec2(STRIP, full.height()));
    let mut ruler = VerticalRuler::new("Vertical ruler", &page).unit(inches()).rows(rows);
    if let Some(range) = text {
        ruler = ruler.text(range);
    }
    if let Some(face) = face {
        ruler = ruler.face(face);
    }
    ruler.show_at(ui, strip).into()
}

#[story(VerticalRuler<'static>, covers = [Default], drive = Drive::Never)]
fn with_row_grips(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    vertical(ui, &ROWS, None, None)
}

#[story(VerticalRuler<'static>, covers = [], drive = Drive::Never)]
fn text_range_shaded(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    vertical(ui, &[], Some(Rangef::new(TEXT.0, TEXT.1)), None)
}

/// The edge face: ticks grow from the strip's inner edge, each number below its whole-unit tick.
#[story(VerticalRuler<'static>, covers = [], drive = Drive::Never)]
fn vertical_edge_face(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    vertical(ui, &ROWS, Some(Rangef::new(TEXT.0, TEXT.1)), Some(RulerFace::Edge))
}

/// The centred face (the productivity set's): a bar inset in the strip, ticks centred on it, numbers in place of the
/// whole-unit ticks over the text range only.
#[story(VerticalRuler<'static>, covers = [], drive = Drive::Never)]
fn vertical_centred_face(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    vertical(ui, &ROWS, Some(Rangef::new(TEXT.0, TEXT.1)), Some(RulerFace::Centred))
}

/// The rulers around one demo page, the margin edges and the paragraph's values kept in the story's data.
fn single_page(ui: &mut Ui, data: &mut StoryData, right_to_left: Option<bool>, face: Option<RulerFace>) -> StoryOutput {
    let full = craft_ui::testing::stage(ui);
    let page = demo_placed(1, pos2(full.min.x + PAGE_LEFT, full.min.y + PAGE_TOP)).remove(0);
    let state = data.value("rulers", (PageEdges::new(Margins::uniform(72.0)), RulerValues::default()));
    let (edges, values) = state;
    let mut rulers = PageRulers::new("Page rulers", &page, Bind::bound(&mut *edges), Bind::bound(&mut *values))
        .unit(inches())
        .kind(Some(TabAlign::Left))
        .columns(&BORDERS)
        .gaps(&GAPS)
        .rows(&ROWS);
    if right_to_left.is_some() {
        rulers = rulers.right_to_left(right_to_left);
    }
    if let Some(face) = face {
        rulers = rulers.face(face);
    }
    rulers.show_at(ui, full).margins.into()
}

#[story(PageRulers<'static>, covers = [Default], drive = Drive::Never)]
fn multi_page_view_with_rulers_on_page_2(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = data.value("view", (PageViewState::default(), PageEdges::new(Margins::uniform(72.0)), RulerValues::default()));
    let (view, edges, values) = state;
    let pages = DemoPages::new(PAGES);
    let out = PageView::new("Pages", &pages, view, PAPER).options(LayoutOptions::ACROBAT).rulers(true).show(ui, |ui, body| {
        if let Some(page) = body.pages.iter().find(|page| page.page == RULED_PAGE) {
            PageRulers::new("Page rulers", page, Bind::bound(&mut *edges), Bind::bound(&mut *values)).unit(inches()).show_at(ui, body.rect);
        }
    });
    out.canvas.response.into()
}

#[story(PageRulers<'static>, covers = [], drive = Drive::Never)]
fn table_and_two_columns(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    single_page(ui, data, None, None)
}

/// Both rulers in the edge face.
#[story(PageRulers<'static>, covers = [], drive = Drive::Never)]
fn page_edge_face(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    single_page(ui, data, None, Some(RulerFace::Edge))
}

/// Both rulers in the centred face (the productivity set's).
#[story(PageRulers<'static>, covers = [], drive = Drive::Never)]
fn page_centred_face(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    single_page(ui, data, None, Some(RulerFace::Centred))
}

#[story(PageRulers<'static>, covers = [], drive = Drive::Never)]
fn right_to_left(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    single_page(ui, data, Some(true), None)
}
