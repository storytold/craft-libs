//! Stories of the outline bands.

use std::collections::BTreeMap;

use super::{OutlineAxis, OutlineBand, OutlineGroup, bands};
use crate::ui::grid::{GridLayout, LineIndex, SheetGeometry, SheetMetrics};
use craft_ui::contract::Show as _;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

const ROW_LEVELS: u8 = 2;
const COLUMN_LEVELS: u8 = 1;

fn group(start: u32, end: u32, level: u8, collapsed: bool) -> OutlineGroup {
    OutlineGroup::new(start, end, level).collapsed(collapsed)
}

#[story(OutlineBand<'static>, covers = [Default], drive = Drive::Never)]
fn rows_and_columns(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let rows = OutlineAxis::new(ROW_LEVELS, vec![group(2, 7, 1, false), group(3, 4, 2, false), group(5, 6, 2, true)]);
    let columns = OutlineAxis::new(COLUMN_LEVELS, vec![group(1, 3, 1, false)]);
    let room = ui.available_rect_before_wrap();
    let rtl = Env::of(ui).direction.is_rtl();
    let layout_bands = bands((ROW_LEVELS, COLUMN_LEVELS), true, 1.0, 16.0, 4.0);
    let none = BTreeMap::new();
    let layout = GridLayout::new(room, LineIndex::new(&none, 20.0, 1000), LineIndex::new(&none, 64.0, 100)).bands(layout_bands).right_to_left(rtl);
    let geometry = SheetGeometry::new(layout, SheetMetrics::new(20.0, 8.0, 6.0));
    OutlineBand::new("Outline", &rows, &columns, &geometry).show(ui).into()
}
