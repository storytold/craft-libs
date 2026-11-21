//! Trials of `ToolRail`: its columns for a width, its measure, a short room and its glyph role.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use super::super::FlyoutButton;
use super::stories::tools;
use super::{LastUsed, ToolRail, ToolSlot};
use crate::buttons::Button;
use crate::contract::Props as _;
use crate::contract::Show as _;
use crate::contract::ShowContainer as _;
use crate::contract::Tracked;
use crate::docking::{DockGrid, DockSizes};
use crate::layout_components::Stack;
use crate::runtime::env::ThemeScope;
use crate::testing::pointer::secondary;
use crate::testing::pointer::{moved, wheel};
use crate::testing::relations::{above, grows_with};
use crate::testing::{Fixture, MemoRun, last_frame_derived_builds, last_frame_memo_runs, last_pass_memo_key_bytes, trial};
use egui::{Rect, Vec2};
use std::cell::{Cell, RefCell};
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
    let (slots, last) = (Tracked::new(slots()), Tracked::new(LastUsed::default()));
    let mut fixture = Fixture::new(move |ui| {
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
    let grid = DockGrid::rail(LABELS.len()).resolved(&DockSizes::of(Fixture::new(|_| {}).ctx()));
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
    let grid = DockGrid::rail(LABELS.len()).resolved(&DockSizes::of(Fixture::new(|_| {}).ctx()));
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
    let grid = DockGrid::rail(LABELS.len()).resolved(&DockSizes::of(Fixture::new(|_| {}).ctx()));
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
        let (slots, last) = (Tracked::new(vec![ToolSlot::new(0, tools(&[0, 1]))]), Tracked::new(LastUsed::default()));
        let mut fixture = Fixture::new(move |ui| {
            if rail {
                ToolRail::new("Tools", &slots, &0, &last).show_rail(ui, |_, _| {});
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

/// What a rail scene saw: the footer's runs, the rail's reports and the Edit Toolbar response, per frame.
#[derive(Default)]
struct Seen {
    current: u8,
    footer_runs: usize,
    selected: Vec<u8>,
    edit_toolbar: Option<Rect>,
}

/// A rail with the Edit Toolbar button, a footer and a tool held in `Seen::current`.
fn memo_rail() -> (Fixture, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let shared = Rc::clone(&seen);
    let (slots, last) = (Tracked::new(slots()), Tracked::new(LastUsed::default()));
    let mut fixture = Fixture::new(move |ui| {
        let current = shared.borrow().current;
        let footer = &shared;
        let out = ToolRail::new("Tools", &slots, &current, &last).edit_toolbar("Edit Toolbar").show_rail(ui, |ui, _| {
            footer.borrow_mut().footer_runs += 1;
            Button::new("Foot").show(ui);
        });
        let mut seen = shared.borrow_mut();
        seen.selected.extend(out.select);
        seen.edit_toolbar = out.edit_toolbar.map(|response| response.rect);
    });
    fixture.room(Vec2::new(200.0, 600.0)).settle();
    (fixture, seen)
}

/// How the slot grid took part in the last frame: (ran, replayed).
fn grid_runs(fixture: &Fixture) -> (usize, usize) {
    let runs = last_frame_memo_runs(fixture.ctx());
    let of = |kind: MemoRun| runs.iter().filter(|(name, _, run)| name.contains("GridProps") && *run == kind).count();
    (of(MemoRun::Ran), of(MemoRun::Replayed))
}

/// An idle rail replays its slot grid and still runs its footer; a tool change runs the grid once.
#[trial]
fn an_idle_tool_rail_replays_its_slots_and_a_tool_change_runs_them_once() {
    let (mut fixture, seen) = memo_rail();
    let before = seen.borrow().footer_runs;
    fixture.frame(vec![]);
    assert_eq!(grid_runs(&fixture), (0, 1), "an idle frame replays the grid");
    assert!(seen.borrow().footer_runs > before, "the footer runs every pass, outside the view");
    seen.borrow_mut().current = 1;
    fixture.frame(vec![]);
    assert_eq!(grid_runs(&fixture).0, 1, "a tool change runs the grid once: {:?}", last_frame_memo_runs(fixture.ctx()));
    fixture.settle();
    for _ in 0..3 {
        fixture.frame(vec![]);
    }
    assert_eq!(grid_runs(&fixture), (0, 1), "then it replays");
}

/// The Edit Toolbar button's response is read back by id, so a replayed grid still reports it.
#[trial]
fn a_replayed_slot_grid_still_reports_the_edit_toolbar_response() {
    let (mut fixture, seen) = memo_rail();
    let shown = seen.borrow().edit_toolbar.expect("the button's response is reported after a run");
    fixture.frame(vec![]);
    assert_eq!(grid_runs(&fixture), (0, 1), "the grid replays");
    assert_eq!(seen.borrow().edit_toolbar, Some(shown), "a replayed grid answers the same response");
}

/// A click on a slot's button selects its tool once, from the run the pointer engaged.
#[trial]
fn a_click_on_a_slot_selects_its_tool_once() {
    let (mut fixture, seen) = memo_rail();
    let brush = rect(&fixture, "Brush");
    fixture.click_at(brush.center());
    fixture.settle();
    assert_eq!(seen.borrow().selected, [1], "the click selected the brush once");
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1.0
}

/// A rail in a scope whose density differs from the root's counts its columns in that scope's style, the style its dock
/// column is stepped in: under a comfortable root, a compact scope two rail steps wide shows two columns, where the
/// root's rail would fit one.
#[trial]
fn a_rail_in_another_density_counts_its_columns_in_that_density() {
    let scoped: Rc<Cell<Option<DockGrid>>> = Rc::default();
    let seen = Rc::clone(&scoped);
    let compact = || ThemeScope::new().density(craft_theme::Size::Compact);
    let mut probe = Fixture::new(move |ui| {
        Stack::new().theme(compact()).show(ui, |ui, _| {
            seen.set(Some(DockGrid::rail(LABELS.len()).resolved(&DockSizes::of_scope(ui))));
        });
    });
    probe.density(craft_theme::Size::Comfortable).settle();
    let grid = scoped.get().expect("the scope ran");
    let root = DockGrid::rail(LABELS.len()).resolved(&DockSizes::of(probe.ctx()));
    let width = grid.width(2);
    assert_eq!(root.steps_within(width), 1, "the root's rail fits one column in two compact steps: {grid:?}, root {root:?}");

    let (slots, last) = (Tracked::new(slots()), Tracked::new(LastUsed::default()));
    let mut fixture = Fixture::new(move |ui| {
        Stack::new().theme(compact()).show(ui, |ui, _| {
            ToolRail::new("Tools", &slots, &0, &last).show_rail(ui, |ui, _| {
                Button::new("Foot").show(ui);
            });
        });
    });
    let tall = 800.0;
    fixture.density(craft_theme::Size::Comfortable).room(Vec2::new(width, tall)).settle();
    assert_eq!(columns_of(&fixture), 2, "two rail steps of the scope's style show two columns");
}

/// A flyout resolves its rows through the command board, so an open flyout builds no derived value in any frame.
#[trial]
fn an_open_flyout_builds_no_derived_values() {
    let tools = tools(&[1, 2]);
    let shown = 1_u8;
    let mut fixture = Fixture::new(move |ui| {
        FlyoutButton::new(&tools, &shown).show(ui);
    });
    fixture.room(Vec2::new(200.0, 200.0)).settle();
    let button = rect(&fixture, "Brush").center();
    fixture.frame(vec![secondary(button, true), secondary(button, false)]);
    fixture.settle();
    assert!(fixture.by_label("Pencil").is_some(), "the right click opened the flyout");
    for _ in 0..3 {
        fixture.frame(vec![]);
        assert!(last_frame_derived_builds(fixture.ctx()).iter().all(|(_, builds)| *builds == 0), "no derived value is built per pass");
    }
}

/// The room of the key-byte trial.
const KEY_BYTES_ROOM: Vec2 = Vec2::new(120.0, 400.0);

/// The key bytes of an idle pass of a rail of `count` slots, a quarter of whose tools are hidden and half remembered.
fn idle_key_bytes(count: u8) -> usize {
    let slots = Tracked::new((0..count).map(|id| ToolSlot::new(u32::from(id), tools(&[id]))).collect::<Vec<_>>());
    let hidden = Tracked::new((0..count / 4).collect::<Vec<u8>>());
    let mut last = LastUsed::default();
    for id in 0..count / 2 {
        last.set(u32::from(id), id);
    }
    let last = Tracked::new(last);
    let mut fixture = Fixture::new(move |ui| {
        ToolRail::new("Tools", &slots, &0, &last).hidden(&hidden).show_rail(ui, |_, _| {});
    });
    fixture.room(KEY_BYTES_ROOM).settle().idle(1);
    assert_eq!(last_frame_derived_builds(fixture.ctx()), Vec::new(), "an idle rail builds no derived value");
    last_pass_memo_key_bytes(fixture.ctx())
}

/// An idle rail feeds the same key bytes whatever its number of slots, with hidden and remembered tools in proportion.
#[trial]
fn a_tool_rails_idle_key_bytes_do_not_grow_with_its_slots() {
    assert_eq!(idle_key_bytes(10), idle_key_bytes(100), "the key follows revisions, not the slots, the hidden or the remembered tools");
}
