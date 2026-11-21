//! Trials of `ToolRail`: its columns for a width, its measure, a short room, its glyph role, its captions and its grid
//! in a scope of its own.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use super::stories::tools;
use super::{LastUsed, ToolRail, ToolSlot};
use crate::buttons::Button;
use crate::contract::Show as _;
use crate::contract::{Props as _, ThemeScope};
use crate::testing::pointer::{moved, wheel};
use crate::testing::relations::{above, grows_with};
use crate::testing::{Fixture, trial};
use craft_theme::Size;
use egui::{Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

/// The slots of the trials: one tool each, labelled Move, Brush, Pencil, Eraser and Clone Stamp.
const LABELS: [&str; 5] = ["Move", "Brush", "Pencil", "Eraser", "Clone Stamp"];

fn slots() -> Vec<ToolSlot<u8>> {
    (0..5_u8).map(|id| ToolSlot::new(u32::from(id), tools(&[id]))).collect()
}

/// The bounds of the node labelled `label`.
fn rect(fixture: &Fixture, label: &str) -> Rect {
    fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown")).rect
}

/// How many columns the buttons of the rail stand in: the distinct left edges.
fn columns_of(fixture: &Fixture) -> usize {
    let mut lefts: Vec<f32> = Vec::new();
    for label in LABELS {
        let left = rect(fixture, label).left();
        if !lefts.iter().any(|seen| (seen - left).abs() < 1.0) {
            lefts.push(left);
        }
    }
    lefts.len()
}

/// A rail in a room `width` wide and `height` high with a one-button footer, and the size `measure_rail` gives it at
/// that width in the same pass.
fn rail_in(width: f32, height: f32) -> (Fixture, Rc<Cell<Vec2>>) {
    let measured = Rc::new(Cell::new(Vec2::ZERO));
    let report = Rc::clone(&measured);
    let mut fixture = Fixture::new(move |ui| {
        let slots = slots();
        let last = LastUsed::default();
        let size = ToolRail::new("Tools", &slots, &0, &last).measure_rail(ui, width, |ui, _| {
            Button::new("Foot").show(ui);
        });
        report.set(size);
        ToolRail::new("Tools", &slots, &0, &last).show_rail(ui, |ui, _| {
            Button::new("Foot").show(ui);
        });
    });
    fixture.room(Vec2::new(width, height)).settle();
    (fixture, measured)
}

/// The rail lays out the most whole columns its width holds: a width of n steps shows n columns, a width a pixel short
/// of them one fewer, and a wider one no more columns than the rail has cells.
#[trial]
fn a_tool_rail_lays_out_the_most_whole_columns_its_width_holds() {
    let grid = ToolRail::<u8>::dock_grid(Fixture::new(|_| {}).ctx(), &ThemeScope::new(), LABELS.len());
    for steps in 1..=3_u16 {
        let (fixture, _) = rail_in(grid.width(steps), 800.0);
        assert_eq!(columns_of(&fixture), usize::from(steps), "a width of {steps} steps shows {steps} columns");
        let rail = rect(&fixture, "Tools");
        assert!(near(rail.width(), grid.width(steps)), "{steps} steps: the rail is as wide as its columns: {rail:?}");
        if steps > 1 {
            let (short, _) = rail_in(grid.width(steps) - 1.0, 800.0);
            assert_eq!(columns_of(&short), usize::from(steps) - 1, "a width short of {steps} steps shows one column fewer");
        }
    }
    let (wide, _) = rail_in(grid.width(grid.most) * 2.0, 800.0);
    assert_eq!(columns_of(&wide), LABELS.len(), "a wide room fills one row");
}

/// The measure at a width is the size the rail shows at it: exactly its rows plus the footer between equal paddings,
/// and fewer columns need more rows.
#[trial]
fn a_tool_rail_measures_its_rows_plus_its_footer_at_a_width() {
    let grid = ToolRail::<u8>::dock_grid(Fixture::new(|_| {}).ctx(), &ThemeScope::new(), LABELS.len());
    let mut heights = Vec::new();
    for steps in [1_u16, 2] {
        let (fixture, measured) = rail_in(grid.width(steps), 800.0);
        let (rail, foot, first) = (rect(&fixture, "Tools"), rect(&fixture, "Foot"), rect(&fixture, "Move"));
        let size = measured.get();
        assert!(near(size.y, rail.height()), "{steps} steps: the measure ({size:?}) is the shown rail ({rail:?})");
        assert!(near(size.x, rail.width()), "{steps} steps: the measure is as wide as the shown rail");
        let (top, bottom) = (first.top() - rail.top(), rail.bottom() - foot.bottom());
        assert!(near(top, bottom), "{steps} steps: the rows and the footer fill the rail between equal paddings ({top} above, {bottom} below)");
        heights.push(size.y);
    }
    assert!(grows_with(heights[1], heights[0]), "one column is taller than two: {heights:?}");
}

/// When the room is too short for every row, the footer stays in view at the bottom and the rows scroll in the room
/// above it, down to a room smaller than one scroll region's usual least height.
#[trial]
fn in_a_short_room_the_rows_scroll_above_the_footer_that_stays_in_view() {
    let grid = ToolRail::<u8>::dock_grid(Fixture::new(|_| {}).ctx(), &ThemeScope::new(), LABELS.len());
    for height in [160.0, 100.0] {
        let (mut fixture, measured) = rail_in(grid.width(1), height);
        let foot = rect(&fixture, "Foot");
        assert!(foot.bottom() <= height, "{height}: the footer is in the room: {foot:?}");
        assert!(measured.get().y > height, "{height}: the rows do not fit the room");
        let first = rect(&fixture, "Move");
        assert!(first.top() < foot.top(), "{height}: the first button is above the footer: {first:?}");
        assert!(rect(&fixture, "Clone Stamp").bottom() > foot.top(), "{height}: the last button starts out of view");
        // The bar lies in the end padding, so the buttons keep the middle of the rail.
        let rail = rect(&fixture, "Tools");
        assert!(near(first.center().x, rail.center().x), "{height}: the buttons stay centred in a rail that scrolls: {first:?} in {rail:?}");

        fixture.frame(vec![moved(first.center())]);
        for _ in 0..10 {
            fixture.frame(vec![wheel(-120.0)]);
        }
        fixture.settle();
        let last = rect(&fixture, "Clone Stamp");
        assert!(grows_with(first.top(), last.top()), "{height}: the last button is in view after scrolling: {last:?}");
        assert!(above(last, foot), "{height}: the last button is above the footer after scrolling: {last:?}");
        assert_eq!(rect(&fixture, "Foot"), foot, "{height}: the footer stays put");
    }
}

/// The tool rail's buttons are larger than a lone icon button's and rest in a quieter ink than they show under the
/// pointer.
#[trial]
fn the_tool_rail_shows_its_buttons_icons_in_its_glyph_role() {
    use crate::buttons::IconButton;
    use craft_behavior::IconId;
    let glyphs = |rail: bool| {
        let mut fixture = Fixture::new(move |ui| {
            if rail {
                let slots = vec![ToolSlot::new(0, tools(&[0, 1]))];
                ToolRail::new("Tools", &slots, &0, &LastUsed::default()).show_rail(ui, |_, _| {});
            } else {
                IconButton::new(IconId::new("move")).label("Alone").show(ui);
            }
        });
        fixture.settle();
        let seen = fixture.styles_of(std::any::type_name::<IconButton<'static>>());
        let glyph = seen.first().and_then(|seen| seen.looks.icons.first().cloned());
        glyph.unwrap_or_else(|| panic!("an icon button showed (rail {rail})"))
    };
    let (alone, tool) = (glyphs(false), glyphs(true));
    assert!(tool.side > alone.side, "a tool rail icon ({}) is larger than a lone one ({})", tool.side, alone.side);
    assert!(tool.color.off.rest != tool.color.off.hover, "a tool rail icon rests quieter than under the pointer");
}

/// A caption longer than any rail is wide.
const CAPTION: &str = "Painting, Retouching and Every Other Tool That Puts Pixels Down";

/// A rail with a caption on its first slot measures taller with captions shown than without, and as tall as a rail with
/// no caption while they are off; the shown caption is cut at the rail's width.
#[trial]
fn captions_take_room_only_while_the_rail_shows_them() {
    let width = ToolRail::<u8>::dock_grid(Fixture::new(|_| {}).ctx(), &ThemeScope::new(), LABELS.len()).width(1);
    let measured = Rc::new(Cell::new([Vec2::ZERO; 3]));
    let report = Rc::clone(&measured);
    let mut fixture = Fixture::new(move |ui| {
        let plain = slots();
        let captioned: Vec<ToolSlot<u8>> =
            slots().into_iter().enumerate().map(|(at, slot)| if at == 0 { slot.captioned(CAPTION) } else { slot }).collect();
        let last = LastUsed::default();
        let none = ToolRail::new("Tools", &plain, &0, &last).measure_rail(ui, width, |_, _| {});
        let off = ToolRail::new("Tools", &captioned, &0, &last).measure_rail(ui, width, |_, _| {});
        let on = ToolRail::new("Tools", &captioned, &0, &last).captions().measure_rail(ui, width, |_, _| {});
        report.set([none, off, on]);
        ToolRail::new("Tools", &captioned, &0, &last).captions().show_rail(ui, |_, _| {});
    });
    fixture.room(Vec2::new(width, 800.0)).settle();
    let [none, off, on] = measured.get();
    assert!(near(off.y, none.y), "a caption takes no room while captions are off: {off:?} against {none:?}");
    assert!(on.y >= off.y + 1.0, "a shown caption adds its row: {on:?} against {off:?}");
    let (rail, caption) = (rect(&fixture, "Tools"), rect(&fixture, CAPTION));
    assert!(rail.contains_rect(caption), "the caption {caption:?} is cut at the rail's width {rail:?}");
    assert!(above(caption, rect(&fixture, "Move")), "the caption heads its group");
}

/// A rail at `width` in a room 800 high, its tokens resolved in `scope`: its columns, its rect and its Move button.
fn rail_in_scope(width: f32, scope: ThemeScope) -> (usize, Rect, Rect) {
    let mut fixture = Fixture::new(move |ui| {
        let slots = slots();
        ToolRail::new("Tools", &slots, &0, &LastUsed::default()).theme(scope.clone()).show_rail(ui, |_, _| {});
    });
    fixture.room(Vec2::new(width, 800.0)).settle();
    (columns_of(&fixture), rect(&fixture, "Tools"), rect(&fixture, "Move"))
}

/// A rail in a compact scope fills the grid `dock_grid` gives for that scope: its first step shows one column, its
/// second two, each of buttons smaller than a rail at the root's density shows; the compact grid is narrower than the
/// root one.
#[trial]
fn a_rail_in_a_compact_scope_fills_the_grid_dock_grid_gives_for_that_scope() {
    let probe = Fixture::new(|_| {});
    let compact = ThemeScope::new().density(Size::Compact);
    let (root, scoped) = (
        ToolRail::<u8>::dock_grid(probe.ctx(), &ThemeScope::new(), LABELS.len()),
        ToolRail::<u8>::dock_grid(probe.ctx(), &compact, LABELS.len()),
    );
    assert!(scoped.first < root.first && scoped.step < root.step, "the compact grid {scoped:?} is narrower than the root one {root:?}");
    let (_, _, root_button) = rail_in_scope(root.width(1), ThemeScope::new());
    for steps in [1_u16, 2] {
        let (columns, rail, button) = rail_in_scope(scoped.width(steps), compact.clone());
        assert_eq!(columns, usize::from(steps), "the compact rail fills {steps} steps of its own grid");
        assert!(near(rail.width(), scoped.width(steps)), "{steps} steps: the rail is as wide as its compact columns: {rail:?}");
        assert!(button.width() < root_button.width(), "{steps} steps: a compact button {button:?} is smaller than a root one {root_button:?}");
    }
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1.0
}
