//! Memoized views and the framework inputs they read outside their props (`runtime::inputs`): a change of one runs
//! only the views that read it. Count trials, as relations: how views took part in a pass (`last_pass_memo_runs`).

use craft_keymap::Overrides;
use craft_ui::buttons::Button;
use craft_ui::contract::{ContainerProps, MemoProps, MemoView, Props as _, Scroll, Show as _, ShowContainer as _, Sizing};
use craft_ui::runtime::commands::{self, CommandInput, CommandRef};
use craft_ui::runtime::env::{Env, EnvValue, with_env};
use craft_ui::runtime::{frame, pointer};
use craft_ui::testing::in_audit_run;
use craft_ui::testing::input::{key_press, text};
use craft_ui::testing::layout::{in_region, scroll_anchor, scroll_area};
use craft_ui::testing::{Frames, MemoRun, last_pass_memo_runs, trial};
use craft_ui_test_kit::body::Slab;
use craft_ui_test_kit::windows::Windows;
use egui::{Context, Key, Modifiers, Pos2, Rect, Ui, Vec2, ViewportInfo};
use std::cell::Cell;
use std::sync::Arc;

/// Idle frames that settle every view after a change, hover fades included.
const SETTLE: usize = 30;
const SIZE: Vec2 = Vec2::new(140.0, 40.0);
/// A place outside every view.
const AWAY: Pos2 = Pos2::new(400.0, 400.0);

/// Where the view `at` is shown, each in its own region.
fn region(at: usize) -> Rect {
    Rect::from_min_size(Pos2::new(10.0, 10.0 + 50.0 * at as f32), SIZE)
}

/// How the views whose props type is named `name` took part in the last pass.
fn runs_of(ctx: &Context, name: &str) -> Vec<MemoRun> {
    last_pass_memo_runs(ctx).into_iter().filter(|(of, _, _)| of.ends_with(name)).map(|(_, _, run)| run).collect()
}

/// A view that reads no framework input.
#[derive(MemoProps)]
struct Plain;

fn plain(_: &mut Plain, ui: &mut Ui) {
    Button::new("Plain").show(ui);
}

fn show_plain(ui: &mut Ui) {
    in_region(ui, region(0), |ui| MemoView::new("plain", Plain, plain).show(ui));
}

/// A view that shows whether Alt is held.
#[derive(MemoProps)]
struct Alt;

fn alt(_: &mut Alt, ui: &mut Ui) {
    let held = pointer::mods(ui.ctx()).alt;
    Button::new(if held { "Alt held" } else { "Alt up" }).show(ui);
}

/// Holding Alt runs only the view that reads the modifiers, once when it goes down and once when it comes up; a reader
/// that was not shown while Alt went down runs when it is shown again.
#[trial]
fn holding_alt_runs_only_the_views_that_read_modifiers() {
    let reader_shown = Cell::new(true);
    let mut show = |ui: &mut Ui| {
        show_plain(ui);
        if reader_shown.get() {
            in_region(ui, region(1), |ui| MemoView::new("alt", Alt, alt).show(ui));
        }
    };
    let mut frames = Frames::new();
    frames.frame(vec![pointer_away()], &mut show);
    frames.idle(SETTLE, &mut show);
    for held in [Modifiers::ALT, Modifiers::NONE] {
        frames.hold(held);
        frames.frame(vec![egui::Event::ModifiersChanged(held)], &mut show);
        assert_eq!(runs_of(&frames.ctx, "Alt"), [MemoRun::Ran], "the reader runs when Alt changes to {held:?}");
        assert_eq!(runs_of(&frames.ctx, "Plain"), [MemoRun::Replayed], "a view that does not read the modifiers replays");
        frames.frame(vec![], &mut show);
        assert_eq!(runs_of(&frames.ctx, "Alt"), [MemoRun::Replayed], "the reader settles on {held:?}");
    }
    reader_shown.set(false);
    frames.idle(2, &mut show);
    frames.hold(Modifiers::ALT);
    frames.frame(vec![egui::Event::ModifiersChanged(Modifiers::ALT)], &mut show);
    frames.idle(2, &mut show);
    reader_shown.set(true);
    frames.frame(vec![], &mut show);
    assert_eq!(runs_of(&frames.ctx, "Alt"), [MemoRun::Ran], "a reader absent while Alt went down runs when shown again");
}

fn pointer_away() -> egui::Event {
    craft_ui::testing::pointer::moved(AWAY)
}

/// A view that shows where the pointer is.
#[derive(MemoProps)]
struct Follow;

fn follow(_: &mut Follow, ui: &mut Ui) {
    let at = pointer::frame(ui.ctx()).pos.map(|pos| (pos.x.round(), pos.y.round()));
    Button::new(format!("{at:?}")).show(ui);
}

/// A pointer move outside every view runs the view that reads the pointer's frame; the others replay.
#[trial]
fn a_pointer_reader_runs_on_pointer_motion_and_the_others_replay() {
    let mut show = |ui: &mut Ui| {
        show_plain(ui);
        in_region(ui, region(1), |ui| MemoView::new("follow", Follow, follow).show(ui));
    };
    let mut frames = Frames::new();
    frames.frame(vec![pointer_away()], &mut show);
    frames.idle(SETTLE, &mut show);
    assert_eq!(runs_of(&frames.ctx, "Follow"), [MemoRun::Replayed], "a still pointer replays its reader");
    frames.frame(vec![craft_ui::testing::pointer::moved(AWAY + Vec2::new(20.0, 0.0))], &mut show);
    assert_eq!(runs_of(&frames.ctx, "Follow"), [MemoRun::Ran], "the pointer moved: its reader runs");
    assert_eq!(runs_of(&frames.ctx, "Plain"), [MemoRun::Replayed], "a view that does not read the pointer replays");
}

/// A view that shows the window's display state.
#[derive(MemoProps)]
struct WindowState;

fn window_state(_: &mut WindowState, ui: &mut Ui) {
    let state = Env::of(ui).view().shell().window.state;
    Button::new(format!("{state:?}")).show(ui);
}

/// Maximizing the window runs only the view that reads the window facts.
#[trial]
fn a_shell_change_runs_only_its_readers() {
    let windows = Windows::new();
    let mut show = |ui: &mut Ui| {
        show_plain(ui);
        in_region(ui, region(1), |ui| MemoView::new("window", WindowState, window_state).show(ui));
    };
    for _ in 0..SETTLE {
        windows.frame(vec![], vec![], &mut show);
    }
    assert_eq!(runs_of(&windows.ctx, "WindowState"), [MemoRun::Replayed], "an idle frame replays the reader");
    windows.root_info(ViewportInfo { maximized: Some(true), ..ViewportInfo::default() });
    windows.frame(vec![], vec![], &mut show);
    assert_eq!(runs_of(&windows.ctx, "WindowState"), [MemoRun::Ran], "the window was maximized: its reader runs");
    assert_eq!(runs_of(&windows.ctx, "Plain"), [MemoRun::Replayed], "a view that does not read the window facts replays");
    windows.frame(vec![], vec![], &mut show);
    assert_eq!(runs_of(&windows.ctx, "WindowState"), [MemoRun::Replayed], "the reader settles on the new facts");
}

/// A view that shows the character the Q key types.
#[derive(MemoProps)]
struct KeyCap;

fn key_cap(_: &mut KeyCap, ui: &mut Ui) {
    let typed = Env::of(ui).view().keyboard_layout().char_at(craft_keymap::Key::Q);
    Button::new(format!("{typed:?}")).show(ui);
}

/// Typing "a" on the Q key (an AZERTY layout) teaches the layout: only the view that reads the layout runs.
#[trial]
fn a_keyboard_layout_change_runs_only_its_readers() {
    let mut show = |ui: &mut Ui| {
        show_plain(ui);
        in_region(ui, region(1), |ui| MemoView::new("cap", KeyCap, key_cap).show(ui));
    };
    let mut frames = Frames::new();
    frames.frame(vec![pointer_away()], &mut show);
    frames.idle(SETTLE, &mut show);
    frames.frame(vec![key_press(Key::Q, Modifiers::NONE), text("a")], &mut show);
    assert_eq!(runs_of(&frames.ctx, "KeyCap"), [MemoRun::Ran], "the layout learned a key: its reader runs");
    assert_eq!(runs_of(&frames.ctx, "Plain"), [MemoRun::Replayed], "a view that does not read the layout replays");
    frames.frame(vec![], &mut show);
    assert_eq!(runs_of(&frames.ctx, "KeyCap"), [MemoRun::Replayed], "the reader settles on the new layout");
}

/// A view that shows the chord of Undo.
#[derive(MemoProps)]
struct UndoChord;

fn undo_chord(_: &mut UndoChord, ui: &mut Ui) {
    let chord = commands::chord(ui.ctx(), &CommandRef::new("edit.undo"));
    Button::new(format!("{chord:?}")).show(ui);
}

/// A new keymap (the user's overrides changed) runs only the view that shows a chord.
#[trial]
fn a_keymap_change_runs_only_views_that_show_chords() {
    let mut show = |ui: &mut Ui| {
        show_plain(ui);
        in_region(ui, region(1), |ui| MemoView::new("chord", UndoChord, undo_chord).show(ui));
    };
    let mut frames = Frames::new();
    frames.frame(vec![pointer_away()], &mut show);
    frames.idle(SETTLE, &mut show);
    frames.commands(CommandInput::new().overrides(Arc::new(Overrides::default())));
    frames.frame(vec![], &mut show);
    assert_eq!(runs_of(&frames.ctx, "UndoChord"), [MemoRun::Ran], "the keymap was built again: the chord's view runs");
    assert_eq!(runs_of(&frames.ctx, "Plain"), [MemoRun::Replayed], "a view that shows no chord replays");
    frames.frame(vec![], &mut show);
    assert_eq!(runs_of(&frames.ctx, "UndoChord"), [MemoRun::Replayed], "the chord's view settles on the new keymap");
}

thread_local! {
    static CLOCK_RUNS: Cell<usize> = const { Cell::new(0) };
}

/// A view that shows the whole seconds on the pass clock.
#[derive(MemoProps)]
struct Seconds;

fn seconds(_: &mut Seconds, ui: &mut Ui) {
    // An audit run is the replay audit's, not the pass's: the trials count none.
    if !in_audit_run(ui.ctx()) {
        CLOCK_RUNS.with(|runs| runs.set(runs.get() + 1));
    }
    let at = frame::stepped_clock(ui.ctx(), 1.0);
    Button::new(format!("{at} s")).show(ui);
}

/// A view that reads the clock in whole seconds runs once per second and replays in the frames between; the view
/// beside it never runs.
#[trial]
fn a_clock_reader_runs_only_at_its_wake_time() {
    const STEP: f64 = 0.25;
    const SECONDS: usize = 3;
    let mut show = |ui: &mut Ui| {
        show_plain(ui);
        in_region(ui, region(1), |ui| MemoView::new("seconds", Seconds, seconds).show(ui));
    };
    let mut frames = Frames::new();
    frames.still_clock();
    frames.frame(vec![pointer_away()], &mut show);
    frames.idle(SETTLE, &mut show);
    CLOCK_RUNS.with(|runs| runs.set(0));
    let mut ran_at = Vec::new();
    for frame in 0..SECONDS * 4 {
        frames.advance(STEP);
        frames.frame(vec![], &mut show);
        if runs_of(&frames.ctx, "Seconds") == [MemoRun::Ran] {
            ran_at.push(frame);
        }
        assert_eq!(runs_of(&frames.ctx, "Plain"), [MemoRun::Replayed], "the view that reads no clock replays in frame {frame}");
    }
    assert_eq!(CLOCK_RUNS.with(Cell::get), SECONDS, "one run per second: {ran_at:?}");
    assert_eq!(ran_at, [3, 7, 11], "the runs fall on the frames that reach a whole second");
}

/// An app value the scope around a view sets.
#[derive(Clone, Copy, Debug, PartialEq, MemoProps)]
struct Tint(u8);

impl EnvValue for Tint {
    fn root() -> Self {
        Tint(0)
    }
}

/// A view that shows the scope's tint and whether it is enabled.
#[derive(MemoProps)]
struct Scoped;

fn scoped(_: &mut Scoped, ui: &mut Ui) {
    let env = Env::of(ui);
    Button::new(format!("{:?} {}", env.view().get::<Tint>(), env.enabled)).show(ui);
}

/// A view's scope disabling it, or setting another app value, runs it once; then it replays.
#[trial]
fn a_scope_change_around_a_view_runs_it() {
    let enabled = Cell::new(true);
    let tint = Cell::new(Tint(1));
    let mut show = |ui: &mut Ui| {
        show_plain(ui);
        in_region(ui, region(1), |ui| {
            with_env(
                ui,
                |set| {
                    set.enabled = Some(enabled.get());
                    set.values.set(tint.get());
                },
                |ui| MemoView::new("scoped", Scoped, scoped).show(ui),
            )
        });
    };
    let mut frames = Frames::new();
    frames.frame(vec![pointer_away()], &mut show);
    frames.idle(SETTLE, &mut show);
    assert_eq!(runs_of(&frames.ctx, "Scoped"), [MemoRun::Replayed], "an unchanged scope replays the view");
    for change in [|_: &Cell<bool>, tint: &Cell<Tint>| tint.set(Tint(2)), |enabled: &Cell<bool>, _: &Cell<Tint>| enabled.set(false)] {
        change(&enabled, &tint);
        frames.frame(vec![], &mut show);
        assert_eq!(runs_of(&frames.ctx, "Scoped"), [MemoRun::Ran], "the scope changed: the view runs");
        assert_eq!(runs_of(&frames.ctx, "Plain"), [MemoRun::Replayed], "the view outside the scope replays");
        frames.frame(vec![], &mut show);
        assert_eq!(runs_of(&frames.ctx, "Scoped"), [MemoRun::Replayed], "the view settles in the new scope");
    }
}

thread_local! {
    static ROWS_RUNS: Cell<usize> = const { Cell::new(0) };
}

/// A view of rows that run only in view (`Lazy`), inside a scrolling body.
#[derive(MemoProps)]
struct Rows;

fn rows(_: &mut Rows, ui: &mut Ui) {
    const ROWS: usize = 30;
    // An audit run is the replay audit's, not the pass's: the trials count none.
    if !in_audit_run(ui.ctx()) {
        ROWS_RUNS.with(|runs| runs.set(runs.get() + 1));
    }
    for at in 0..ROWS {
        craft_ui::contract::Lazy::new(at, 40.0).show(ui, |ui| {
            Button::new(format!("Row {at}")).show(ui);
        });
    }
}

/// A view of lazy rows in a fixed box, inside a scrolling body that moves down the screen: the view's room, clip and
/// layer stay the same, but the body's view now meets other rows. It runs then, and replays after.
#[trial]
fn a_view_with_lazy_rows_runs_when_its_scroll_view_meets_other_rows() {
    let top = Cell::new(10.0);
    let fixed = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(100.0, 100.0));
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, top.get()), Vec2::new(300.0, 200.0)), |ui| {
            Slab::new(ContainerProps::new().sizing(Sizing::Fill).scroll(Scroll::vertical())).id_salt("area").show(ui, |ui, _| {
                in_region(ui, fixed, |ui| {
                    ui.set_clip_rect(fixed);
                    MemoView::new("rows", Rows, rows).show(ui);
                });
            });
        });
    };
    let mut frames = Frames::new();
    frames.frame(vec![pointer_away()], &mut show);
    frames.idle(SETTLE, &mut show);
    ROWS_RUNS.with(|runs| runs.set(0));
    frames.frame(vec![], &mut show);
    assert_eq!(ROWS_RUNS.with(Cell::get), 0, "an idle frame replays the rows");
    top.set(300.0);
    frames.frame(vec![], &mut show);
    assert!(ROWS_RUNS.with(Cell::get) > 0, "the body's view meets rows the last run left out: the view of rows ran");
    frames.idle(SETTLE, &mut show);
    ROWS_RUNS.with(|runs| runs.set(0));
    frames.frame(vec![], &mut show);
    assert_eq!(ROWS_RUNS.with(Cell::get), 0, "the view settles under the moved view");
}

/// A replayed view reports its nested rows' places as its run did: after a scroll, the scroll area keeps the row its
/// position is named by, frame after frame, while the view replays.
#[trial]
fn a_replayed_view_keeps_its_scroll_areas_anchor() {
    let area = Cell::new(None);
    let view = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 200.0));
    let mut show = |ui: &mut Ui| {
        in_region(ui, view, |ui| {
            Slab::new(ContainerProps::new().sizing(Sizing::Fill).scroll(Scroll::vertical())).id_salt("area").show(ui, |ui, _| {
                area.set(scroll_area(ui));
                MemoView::new("rows", Rows, rows).show(ui);
            });
        });
    };
    let beside = Pos2::new(view.max.x - 20.0, view.center().y);
    let mut frames = Frames::new();
    frames.frame(vec![craft_ui::testing::pointer::moved(beside)], &mut show);
    frames.idle(SETTLE, &mut show);
    frames.frame(vec![craft_ui::testing::pointer::wheel(-200.0)], &mut show);
    frames.frame(vec![pointer_away()], &mut show);
    frames.idle(SETTLE, &mut show);
    let area = area.get().expect("the scrolling body lays its content out for its scroll area");
    let named = scroll_anchor(&frames.ctx, area);
    assert!(named.is_some(), "the scrolled body names its position by a row");
    ROWS_RUNS.with(|runs| runs.set(0));
    for frame in 0..SETTLE {
        frames.frame(vec![], &mut show);
        assert_eq!(scroll_anchor(&frames.ctx, area), named, "frame {frame}: the anchor stays on the row the run found");
    }
    assert_eq!(ROWS_RUNS.with(Cell::get), 0, "the view replayed in every frame once the pointer left");
}
