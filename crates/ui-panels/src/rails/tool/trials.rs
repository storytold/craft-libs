//! Trials of `ToolRail`: its columns for a width, its measure, a short room, its glyph role, its captions and its grid
//! in a scope of its own; and the search of its drawer.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use super::super::FlyoutButton;
use super::stories::tools;
use super::{DropAt, LastUsed, RailDensity, ToolRail, ToolSlot};
use crate::buttons::Button;
use crate::contract::Props as _;
use crate::contract::Show as _;
use crate::contract::ShowContainer as _;
use crate::contract::Tracked;
use crate::docking::{DockGrid, DockSizes};
use crate::layout_components::Stack;
use crate::runtime::env::ThemeScope;
use crate::style::StyleCx;
use crate::testing::pointer::secondary;
use crate::testing::pointer::{moved, wheel};
use crate::testing::relations::{above, grows_with};
use crate::testing::{Fixture, MemoRun, last_frame_derived_builds, last_frame_memo_runs, last_pass_memo_key_bytes, trial};
use craft_theme::Size;
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

/// A dense rail's buttons are smaller than a regular rail's, and its dock grid narrows with them: at the dense grid's
/// width of two steps the dense rail shows two columns, as wide as that grid, where a regular rail fits one.
#[trial]
fn a_dense_rail_and_its_grid_narrow_together() {
    let ctx = Fixture::new(|_| {}).ctx().clone();
    let sizes = DockSizes::of(&ctx);
    let (regular, dense) = (DockGrid::rail(LABELS.len()).resolved(&sizes), DockGrid::rail(LABELS.len()).density(RailDensity::Dense).resolved(&sizes));
    assert!(dense.width(2) < regular.width(2), "the dense grid is narrower: {dense:?} against {regular:?}");
    let shown = |density: RailDensity| {
        let (slots, last) = (Tracked::new(slots()), Tracked::new(LastUsed::default()));
        let mut fixture = Fixture::new(move |ui| {
            ToolRail::new("Tools", &slots, &0, &last).density(density).show_rail(ui, |_, _| {});
        });
        fixture.room(Vec2::new(dense.width(2), 800.0)).settle();
        (columns_of(&fixture), rect(&fixture, "Tools"), rect(&fixture, "Move"))
    };
    let ((dense_columns, rail, dense_button), (regular_columns, _, regular_button)) = (shown(RailDensity::Dense), shown(RailDensity::Regular));
    assert_eq!(dense_columns, 2, "the dense rail fills its grid's two steps");
    assert!(near(rail.width(), dense.width(2)), "the dense rail is as wide as its grid: {rail:?}");
    assert!(regular_columns < dense_columns, "a regular rail fits fewer columns in the same width");
    assert!(dense_button.width() < regular_button.width(), "a dense button ({dense_button:?}) is smaller than a regular one ({regular_button:?})");
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

/// A caption longer than any rail is wide.
const CAPTION: &str = "Painting, Retouching and Every Other Tool That Puts Pixels Down";

/// A rail with a caption on its first slot measures taller with captions shown than without, and as tall as a rail with
/// no caption while they are off; the shown caption is cut at the rail's width.
#[trial]
fn captions_take_room_only_while_the_rail_shows_them() {
    let width = DockGrid::rail(LABELS.len()).resolved(&DockSizes::of(Fixture::new(|_| {}).ctx())).width(1);
    let measured = Rc::new(Cell::new([Vec2::ZERO; 3]));
    let report = Rc::clone(&measured);
    let plain = Tracked::new(slots());
    let captioned =
        Tracked::new(slots().into_iter().enumerate().map(|(at, slot)| if at == 0 { slot.captioned(CAPTION) } else { slot }).collect::<Vec<_>>());
    let last = Tracked::new(LastUsed::default());
    let mut fixture = Fixture::new(move |ui| {
        // Three rails measured in one pass, each under an id of its own.
        let none = ToolRail::new("Tools", &plain, &0, &last).id_salt("none").measure_rail(ui, width, |_, _| {});
        let off = ToolRail::new("Tools", &captioned, &0, &last).id_salt("off").measure_rail(ui, width, |_, _| {});
        let on = ToolRail::new("Tools", &captioned, &0, &last).captions().id_salt("on").measure_rail(ui, width, |_, _| {});
        report.set([none, off, on]);
        ToolRail::new("Tools", &captioned, &0, &last).captions().show_rail(ui, |_, _| {});
    });
    fixture.room(Vec2::new(width, 800.0)).settle();
    let [none, off, on] = measured.get();
    assert!(near(off.y, none.y), "a caption takes no room while captions are off: {off:?} against {none:?}");
    assert!(on.y > off.y && !near(on.y, off.y), "a shown caption adds its row: {on:?} against {off:?}");
    let (rail, caption) = (rect(&fixture, "Tools"), rect(&fixture, CAPTION));
    assert!(rail.contains_rect(caption), "the caption {caption:?} is cut at the rail's width {rail:?}");
    assert!(above(caption, rect(&fixture, "Move")), "the caption heads its group");
}

/// A rail at `width` in a room 800 high under a comfortable root, its tokens resolved in `scope`: its columns, its rect
/// and its Move button.
fn rail_in_scope(width: f32, scope: ThemeScope) -> (usize, Rect, Rect) {
    let (slots, last) = (Tracked::new(slots()), Tracked::new(LastUsed::default()));
    let mut fixture = Fixture::new(move |ui| {
        ToolRail::new("Tools", &slots, &0, &last).theme(scope.clone()).show_rail(ui, |_, _| {});
    });
    fixture.density(Size::Comfortable).room(Vec2::new(width, 800.0)).settle();
    (columns_of(&fixture), rect(&fixture, "Tools"), rect(&fixture, "Move"))
}

/// A rail with a compact scope of its own fills the rail grid of that scope: its first step shows one column, its
/// second two, each of buttons smaller than a rail at the root's density shows; the compact grid is narrower than the
/// comfortable root's.
#[trial]
fn a_rail_with_a_compact_scope_of_its_own_fills_the_rail_grid_of_that_scope() {
    let mut probe = Fixture::new(|_| {});
    probe.density(Size::Comfortable).settle();
    let compact = ThemeScope::new().density(Size::Compact);
    let grid_in = |scope: &ThemeScope| DockGrid::rail(LABELS.len()).resolved(&StyleCx::with_root_scoped(probe.ctx(), scope, DockSizes::style));
    let (root, scoped) = (grid_in(&ThemeScope::new()), grid_in(&compact));
    assert!(
        scoped.width(1) < root.width(1) && scoped.width(2) - scoped.width(1) < root.width(2) - root.width(1),
        "the compact grid {scoped:?} is narrower than the root one {root:?}"
    );
    let (_, _, root_button) = rail_in_scope(root.width(1), ThemeScope::new());
    for steps in [1_u16, 2] {
        let (columns, rail, button) = rail_in_scope(scoped.width(steps), compact.clone());
        assert_eq!(columns, usize::from(steps), "the compact rail fills {steps} steps of its own grid");
        assert!(near(rail.width(), scoped.width(steps)), "{steps} steps: the rail is as wide as its compact columns: {rail:?}");
        assert!(button.width() < root_button.width(), "{steps} steps: a compact button {button:?} is smaller than a root one {root_button:?}");
    }
}

/// A searchable drawer over the Drawing (Move, Brush, Pencil) and Retouching (Eraser, Clone Stamp) groups, with the
/// clone stamp titled "Klonstempel" as a translation would show it, after typing `query` into its search field.
fn searched(query: &str) -> Fixture {
    use crate::rails::drawer::{DrawerGroup, DrawerKind, ToolbarDrawer};
    let mut fixture = Fixture::new(|ui| {
        let groups = [DrawerGroup::new("Drawing", tools(&[0, 1, 2])), DrawerGroup::new("Retouching", tools(&[3, 4]))];
        let rail = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(40.0, 400.0));
        ToolbarDrawer::new(rail, &groups, &[], None).kind(DrawerKind::Searchable).show(ui);
    });
    fixture.command_state(super::stories::StoryTools::Stamp, crate::runtime::commands::CommandState::new().title("Klonstempel"));
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    if !query.is_empty() {
        let field = fixture.by_label("Search Tools").expect("the drawer shows its search field");
        fixture.click(&field).type_text(query).settle();
    }
    fixture
}

/// The drawer finds a tool by its shown label, its English label or its command id, and keeps a group's heading only
/// while one of the group's tools matches; an empty query shows every row.
#[trial]
fn a_searchable_drawer_keeps_a_heading_only_with_a_matching_tool() {
    let shown = |fixture: &Fixture, label: &str| fixture.by_label(label).is_some();

    let all = searched("");
    for label in ["Drawing", "Move", "Brush", "Pencil", "Retouching", "Eraser", "Klonstempel"] {
        assert!(shown(&all, label), "an empty query shows {label}");
    }

    let eraser = searched("eraser");
    assert!(shown(&eraser, "Eraser") && shown(&eraser, "Retouching"), "the matching tool and its heading stay");
    for label in ["Klonstempel", "Drawing", "Move", "Brush", "Pencil"] {
        assert!(!shown(&eraser, label), "\"eraser\" hides {label}");
    }

    // Neither the shown label nor the command id (`railStory.stamp`) holds "clone": only the English label does.
    let english = searched("clone");
    assert!(shown(&english, "Klonstempel") && shown(&english, "Retouching"), "the English label finds the retitled stamp and keeps its heading");
    for label in ["Move", "Brush", "Pencil", "Drawing", "Eraser"] {
        assert!(!shown(&english, label), "\"clone\" hides {label}");
    }

    let titled = searched("klonstempel");
    assert!(shown(&titled, "Klonstempel") && shown(&titled, "Retouching"), "the shown label finds the stamp");
    assert!(!shown(&titled, "Drawing"), "\"klonstempel\" hides the other group's heading");

    let id = tools(&[3])[0].command.id.clone();
    let by_id = searched(&id);
    assert!(shown(&by_id, "Eraser") && shown(&by_id, "Retouching"), "the command id {id} finds the eraser");
    assert!(!shown(&by_id, "Drawing") && !shown(&by_id, "Brush"), "the command id {id} hides the other group");
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

/// While the toolbar is edited a drop off the buttons lands on the rail: on a blank toolbar as its first slot, and under
/// the last button after the last slot.
#[trial]
fn an_edited_rail_takes_a_drop_off_its_buttons_beside_the_nearest_one() {
    let dropped = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&dropped);
    let (slots, blank, last) = (Tracked::new(slots()), Tracked::new(Vec::new()), Tracked::new(LastUsed::default()));
    let mut fixture = Fixture::new(move |ui| {
        Stack::new().show(ui, |ui, _| {
            for (label, slots, edit) in [("Tools", &slots, "Edit Tools"), ("Blank", &blank, "Edit Blank")] {
                let out = ToolRail::new(label, slots, &0, &last).edit_toolbar(edit).editing().show_column(ui, |_, _| {});
                seen.borrow_mut().extend(out.dropped.map(|drop| (label, drop.at)));
            }
        });
    });
    fixture.room(Vec2::new(200.0, 800.0)).settle();
    let pencil = rect(&fixture, "Pencil").center();
    fixture.drag_at(pencil, rect(&fixture, "Blank").center() - pencil).settle();
    assert_eq!(dropped.borrow().as_slice(), [("Blank", DropAt::new_slot(0))], "a drop on a blank toolbar makes its first slot");
    dropped.borrow_mut().clear();
    fixture.drag_at(pencil, rect(&fixture, "Edit Tools").center() - pencil).settle();
    assert_eq!(dropped.borrow().as_slice(), [("Tools", DropAt::new_slot(LABELS.len()))], "under the last button it goes after the last slot");
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

/// A tool's own icon replaces its command's: the app's glyph paints the rail button that shows the tool and the tool's
/// row in the flyout, and a tool without one keeps its command's icon.
#[trial]
fn a_tools_own_icon_paints_its_button_and_its_flyout_row() {
    use crate::primitives::{Glyph, IconSpec};
    use crate::style::UiTokens;

    // An app's tool artwork: one ellipse across its rect, a shape nothing else on the rail paints, so the frame's
    // shapes say where the glyph painted.
    #[derive(crate::contract::MemoProps)]
    struct Artwork;
    impl Glyph for Artwork {
        fn paint(&self, painter: &egui::Painter, _: &UiTokens, rect: Rect, color: egui::Color32, _: bool) {
            painter.add(egui::Shape::ellipse_filled(rect.center(), rect.size() / 2.0, color));
        }
    }
    // Where the glyph painted in the fixture's next frame. A frame that replays a memoized view repeats its shapes, so
    // the shapes of one frame show every place the glyph is on screen.
    let painted = |fixture: &mut Fixture| -> Vec<Rect> {
        fixture
            .frame_shapes()
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Ellipse(ellipse) => Some(Rect::from_center_size(ellipse.center, ellipse.radius * 2.0)),
                _ => None,
            })
            .collect()
    };

    let mut group = tools(&[1, 2]);
    group[0] = group[0].clone().icon(IconSpec::glyph(&Artwork));
    let (slots, last) = (Tracked::new(vec![ToolSlot::new(0, group)]), Tracked::new(LastUsed::default()));
    let mut fixture = Fixture::new(move |ui| {
        ToolRail::new("Tools", &slots, &1, &last).show_rail(ui, |_, _| {});
    });
    fixture.room(Vec2::new(200.0, 400.0)).settle();
    let button = rect(&fixture, "Brush");
    let face = painted(&mut fixture);
    assert!(!face.is_empty() && face.iter().all(|at| button.contains_rect(*at)), "the glyph paints the Brush button: {face:?} in {button:?}");

    // With the flyout open the glyph paints the Brush row too, and the Pencil row keeps its command's icon.
    fixture.frame(vec![secondary(button.center(), true), secondary(button.center(), false)]);
    fixture.settle();
    let pencil = rect(&fixture, "Pencil");
    let shown = painted(&mut fixture);
    assert!(
        shown.iter().any(|at| !button.intersects(*at) && !pencil.contains(at.center())),
        "the open flyout's Brush row paints the glyph too: {shown:?}, button {button:?}, Pencil row {pencil:?}"
    );
    assert!(shown.iter().all(|at| !pencil.contains_rect(*at)), "the Pencil row keeps its command's icon: {shown:?} in {pencil:?}");
}
