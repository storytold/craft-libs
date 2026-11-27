//! The replay audit (`CRAFT_UI_AUDIT_REPLAY=1`, `audit_replays`): a memoized view that would replay runs for real and
//! is compared with the replay. A stale view fails the frame with its name; honest views pass, and the audit changes
//! no count.

use craft_ui::buttons::Button;
use craft_ui::choice::{Radio, RadioGroup};
use craft_ui::contract::{
    Bind, ContainerProps, Entry, Lazy, MemoProps, MemoView, Props as _, Scroll, Show as _, ShowCollection as _, ShowContainer as _,
};
use craft_ui::form::Field;
use craft_ui::list::{List, ListRow};
use craft_ui::pickers::Select;
use craft_ui::runtime::lock_keys;
use craft_ui::style::SurfaceKind;
use craft_ui::testing::layout::{allocate, in_region};
use craft_ui::testing::pointer::{moved, wheel};
use craft_ui::testing::{
    Fixture, Frames, MemoRun, audit_replays, in_audit_run, last_frame_memo_runs, last_pass_memo_runs, last_pass_runs, last_pass_text_layouts, trial,
};
use craft_ui_test_kit::body::Slab;
use egui::{Color32, Pos2, Rect, Ui, Vec2};
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::Arc;

thread_local! {
    /// App state the stale views read outside their props.
    static HIDDEN: Cell<u32> = const { Cell::new(0) };
    /// The calls of the honest views' `show` on this trial's thread, audit runs included.
    static SHOWN: Cell<usize> = const { Cell::new(0) };
}

/// Idle frames that settle every view after a change, hover fades included.
const SETTLE: usize = 30;

fn region(at: usize) -> Rect {
    Rect::from_min_size(Pos2::new(10.0, 10.0 + 50.0 * at as f32), Vec2::new(160.0, 40.0))
}

/// The message of the panic `frame` ends in.
fn failure(frame: impl FnOnce()) -> String {
    let payload = catch_unwind(AssertUnwindSafe(frame)).expect_err("the audit fails the frame");
    payload.downcast_ref::<String>().cloned().or_else(|| payload.downcast_ref::<&str>().map(|text| (*text).to_owned())).unwrap_or_default()
}

/// A view whose label also shows `HIDDEN`, which its props do not hold.
#[derive(MemoProps)]
struct Untracked {
    label: &'static str,
}

fn untracked(props: &mut Untracked, ui: &mut Ui) {
    Button::new(format!("{} {}", props.label, HIDDEN.get())).show(ui);
}

/// A view that reads an untracked value fails the audit once the value changed, naming the view.
#[trial]
fn the_audit_names_a_view_that_reads_an_untracked_static() {
    HIDDEN.set(1);
    let mut frames = Frames::new();
    audit_replays(&frames.ctx, true);
    let frame = |frames: &mut Frames| {
        frames.frame(vec![], |ui| {
            in_region(ui, region(0), |ui| MemoView::new("untracked", Untracked { label: "Count" }, untracked).show(ui));
        })
    };
    for _ in 0..3 {
        frame(&mut frames);
    }
    HIDDEN.set(2);
    let message = failure(|| {
        frame(&mut frames);
    });
    assert!(message.contains("replay audit: memoized view") && message.contains("Untracked"), "names the view: {message}");
    assert!(message.contains("paint"), "names the paint difference: {message}");
}

/// A view that fills its rect with a colour from `HIDDEN`: its props, extent and output never change.
#[derive(MemoProps)]
struct Swatch {
    size: f32,
}

fn swatch(props: &mut Swatch, ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(props.size), egui::Sense::hover());
    let shade = u8::try_from(HIDDEN.get() % 256).unwrap_or(0);
    ui.painter().rect_filled(rect, 0.0, Color32::from_gray(shade));
}

/// A view whose paint alone went stale fails the audit, naming the view and the paint difference only.
#[trial]
fn the_audit_names_a_view_whose_paint_went_stale() {
    HIDDEN.set(10);
    let mut frames = Frames::new();
    audit_replays(&frames.ctx, true);
    let frame = |frames: &mut Frames| {
        frames.frame(vec![], |ui| {
            in_region(ui, region(0), |ui| MemoView::new("swatch", Swatch { size: 20.0 }, swatch).show(ui));
        })
    };
    for _ in 0..3 {
        frame(&mut frames);
    }
    HIDDEN.set(200);
    let message = failure(|| {
        frame(&mut frames);
    });
    assert!(message.contains("Swatch"), "names the view: {message}");
    assert!(message.contains("paint"), "names the paint difference: {message}");
    assert!(!message.contains("rect: left") && !message.contains("output:"), "extent and output are the same: {message}");
}

/// A view that returns a fraction of `HIDDEN`, which its props do not hold, and paints nothing from it.
#[derive(MemoProps)]
struct Fraction {
    size: f32,
}

fn fraction(props: &mut Fraction, ui: &mut Ui) -> f32 {
    ui.allocate_exact_size(Vec2::splat(props.size), egui::Sense::hover());
    HIDDEN.get() as f32 / 1000.0
}

/// A bare float a view returns compares exactly: a fraction stale by far less than the audit's position tolerance
/// fails the audit, naming the output only.
#[trial]
fn the_audit_names_a_view_whose_returned_fraction_went_stale() {
    HIDDEN.set(500);
    let mut frames = Frames::new();
    audit_replays(&frames.ctx, true);
    let frame = |frames: &mut Frames| {
        frames.frame(vec![], |ui| {
            in_region(ui, region(0), |ui| MemoView::new("fraction", Fraction { size: 20.0 }, fraction).show(ui));
        })
    };
    for _ in 0..3 {
        frame(&mut frames);
    }
    HIDDEN.set(501);
    let message = failure(|| {
        frame(&mut frames);
    });
    assert!(message.contains("Fraction") && message.contains("output:"), "names the view and its output: {message}");
    assert!(!message.contains("paint"), "the paint is the same: {message}");
}

/// A view that reads the Caps Lock state, a framework input, only while `HIDDEN` is odd, and paints the same either
/// way. An input read is noted in the run's facts only, never in its journal.
#[derive(MemoProps)]
struct InputReader {
    size: f32,
}

fn input_reader(props: &mut InputReader, ui: &mut Ui) {
    if HIDDEN.get() % 2 == 1 {
        let _ = lock_keys::caps_lock(ui.ctx());
    }
    ui.allocate_exact_size(Vec2::splat(props.size), egui::Sense::hover());
}

/// A kept run that missed a framework input the real run reads fails the audit, naming the input: a replay would note
/// the kept run's inputs, so a change of the missed one would not run the view.
#[trial]
fn the_audit_names_a_view_whose_kept_run_missed_an_input() {
    HIDDEN.set(2);
    let mut frames = Frames::new();
    audit_replays(&frames.ctx, true);
    let frame = |frames: &mut Frames| {
        frames.frame(vec![], |ui| {
            in_region(ui, region(0), |ui| MemoView::new("input reader", InputReader { size: 20.0 }, input_reader).show(ui));
        })
    };
    for _ in 0..3 {
        frame(&mut frames);
    }
    HIDDEN.set(3);
    let message = failure(|| {
        frame(&mut frames);
    });
    assert!(message.contains("InputReader") && message.contains("inputs:"), "names the view and the input: {message}");
    assert!(!message.contains("paint") && !message.contains("journal"), "paint and journal are the same: {message}");
}

#[derive(MemoProps)]
struct Title {
    title: String,
}

fn title(props: &mut Title, ui: &mut Ui) -> Rect {
    if !in_audit_run(ui.ctx()) {
        SHOWN.set(SHOWN.get() + 1);
    }
    Button::new(props.title.clone()).show(ui).response.rect
}

/// A view holding another: the inner view replays inside the outer one's runs.
#[derive(MemoProps)]
struct Outer {
    title: String,
    inner: String,
}

fn outer(props: &mut Outer, ui: &mut Ui) -> Option<Rect> {
    if !in_audit_run(ui.ctx()) {
        SHOWN.set(SHOWN.get() + 1);
    }
    Button::new(props.title.clone()).show(ui);
    MemoView::new("inner", Title { title: props.inner.clone() }, title).show(ui)
}

/// What one frame of the honest scene counted: how each view took part in every pass, the components that ran and the
/// text laid out in the last pass, and the passes.
type Counted = (Vec<(&'static str, MemoRun)>, Vec<(&'static str, MemoRun)>, u32, u32, usize);

/// Runs the honest scene (warm-up, the pointer over a view and away, a prop change, idle frames) with the audit `on`;
/// returns each frame's counts and the `show` calls outside audit runs.
fn honest_scene(on: bool) -> (Vec<Counted>, usize) {
    SHOWN.set(0);
    let mut frames = Frames::new();
    audit_replays(&frames.ctx, on);
    let mut counted = Vec::new();
    let mut title_text = "Title".to_owned();
    let script: Vec<(Vec<egui::Event>, Option<&str>)> = std::iter::repeat_n((vec![], None), 3)
        .chain([(vec![moved(region(0).center())], None), (vec![moved(Pos2::new(400.0, 400.0))], None)])
        .chain(std::iter::repeat_n((vec![], None), SETTLE))
        .chain([(vec![], Some("A longer title"))])
        .chain(std::iter::repeat_n((vec![], None), 3))
        .collect();
    for (events, retitle) in script {
        if let Some(text) = retitle {
            title_text = text.to_owned();
        }
        let passes = frames.frame(events, |ui| {
            in_region(ui, region(0), |ui| MemoView::new("title", Title { title: title_text.clone() }, title).show(ui));
            in_region(ui, region(1), |ui| MemoView::new("outer", Outer { title: "Outer".to_owned(), inner: "Inner".to_owned() }, outer).show(ui));
        });
        let ctx = &frames.ctx;
        let runs = |all: Vec<(&'static str, egui::Id, MemoRun)>| all.into_iter().map(|(name, _, run)| (name, run)).collect::<Vec<_>>();
        let components = last_pass_runs(ctx).iter().map(|(_, runs)| runs).sum();
        counted.push((runs(last_pass_memo_runs(ctx)), runs(last_frame_memo_runs(ctx)), components, last_pass_text_layouts(ctx), passes));
    }
    (counted, SHOWN.get())
}

/// Honest views pass the audit through hover, a prop change and idle frames, and the audit changes no count: with it
/// off, `show` runs exactly as often as the views are counted `Ran`; with it on, every count is the same.
#[trial]
fn honest_views_pass_the_audit_and_count_as_without_it() {
    let (off, shown_off) = honest_scene(false);
    let (on, shown_on) = honest_scene(true);
    let ran: usize = off.iter().map(|frame| frame.1.iter().filter(|(_, run)| *run == MemoRun::Ran).count()).sum();
    assert_eq!(shown_off, ran, "with the audit off, a view runs only when it is counted Ran");
    assert!(off.iter().any(|frame| frame.0.iter().any(|(_, run)| *run == MemoRun::Replayed)), "views replayed, so the audit had work");
    assert_eq!(on, off, "the audit changes no count");
    assert_eq!(shown_on, shown_off, "outside audit runs, `show` runs as often as without the audit");
}

/// A view that paints a paint callback.
#[derive(MemoProps)]
struct Gpu {
    size: f32,
}

fn gpu(props: &mut Gpu, ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(props.size), egui::Sense::hover());
    ui.painter().add(egui::PaintCallback { rect, callback: Arc::new(()) });
}

/// A view that paints a paint callback runs every pass: a replay could not repeat what the callback paints, nor could
/// the audit compare it.
#[trial]
fn a_view_painting_a_callback_runs_every_pass() {
    let mut frames = Frames::new();
    for frame in 0..5 {
        frames.frame(vec![], |ui| {
            in_region(ui, region(0), |ui| MemoView::new("gpu", Gpu { size: 20.0 }, gpu).show(ui));
        });
        let runs: Vec<MemoRun> = last_pass_memo_runs(&frames.ctx).into_iter().map(|(_, _, run)| run).collect();
        assert_eq!(runs, [MemoRun::Ran], "frame {frame}");
    }
}

/// A view holding a scrolling body of rows that run only in view.
#[derive(MemoProps)]
struct ScrolledRows {
    rows: usize,
}

fn scrolled_rows(props: &mut ScrolledRows, ui: &mut Ui) {
    Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("rows").show(ui, |ui, _| {
        for at in 0..props.rows {
            Lazy::new(at, 20.0).show(ui, |ui| {
                Button::new(format!("Row {at}")).show(ui);
            });
        }
    });
}

/// A view whose run measured the scrolling body inside it replays in passes that measure nothing: its record holds what
/// its real run registered, not what the body's measuring run did, so the audit finds the replay equal to a run.
#[trial]
fn a_view_whose_body_measured_in_its_run_replays_as_a_run_in_a_pass_that_does_not_measure() {
    let mut frames = Frames::new();
    audit_replays(&frames.ctx, true);
    let rows = Cell::new(12);
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(160.0, 100.0)), |ui| {
            MemoView::new("rows", ScrolledRows { rows: rows.get() }, scrolled_rows).show(ui);
        });
    };
    frames.idle(SETTLE, &mut show);
    rows.set(16);
    frames.idle(SETTLE, &mut show);
    let ran: Vec<_> = last_frame_memo_runs(&frames.ctx).into_iter().filter(|(_, _, run)| *run == MemoRun::Ran).collect();
    assert!(ran.is_empty(), "the view settles: {ran:?}");
}

/// A card, as the gallery's: a raised surface (painted snapped to the pixel grid) holding one pressable member of the
/// arena around it.
#[derive(MemoProps)]
struct Card {
    at: usize,
}

fn card(props: &mut Card, ui: &mut Ui) {
    Slab::new(ContainerProps::new().surface(SurfaceKind::Raised)).id_salt(("card", props.at)).show(ui, |ui, _| {
        Button::new(format!("Card {}", props.at)).show(ui);
    });
}

/// A card holding a scrolling body of its own, sized to its content, whose rows report their places to it.
fn scrolling_card(props: &mut Card, ui: &mut Ui) {
    let room = allocate(ui, Vec2::new(100.0, 80.0));
    in_region(ui, room, |ui| {
        Slab::new(ContainerProps::new().surface(SurfaceKind::Raised).scroll(Scroll::vertical())).id_salt(("inner", props.at)).show(ui, |ui, _| {
            for at in 0..6 {
                Lazy::new(at, 20.0).show(ui, |ui| {
                    Button::new(format!("Row {at}")).show(ui);
                });
            }
        });
    });
}

/// Where the pointer rests to scroll the cards: over their body, beside every card (in the lead above them, or beside
/// the narrow ones).
const BESIDE: Pos2 = Pos2::new(140.0, 50.0);

/// Cards shown by `body` in a scrolling arena body under a lead of `lead` points, as the gallery's grid shows them: each
/// in a row that runs only in view (`Lazy`).
fn cards(lead: f32, body: fn(&mut Card, &mut Ui)) -> impl FnMut(&mut Ui) {
    move |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(160.0, 200.0)), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical()).arena(true)).id_salt("cards").show(ui, |ui, _| {
                ui.add_space(lead);
                for at in 0..12 {
                    Lazy::new(("row", at), 40.0).show(ui, |ui| {
                        MemoView::new(("card", at), Card { at }, body).show(ui);
                    });
                }
            });
        });
    }
}

/// The cards that replayed in the last frame.
fn replayed(frames: &Frames) -> usize {
    last_frame_memo_runs(&frames.ctx).into_iter().filter(|(name, _, run)| name.contains("Card") && *run == MemoRun::Replayed).count()
}

/// A card holding a scrolling body sized to its content replays moved: the body's view is the content it measured,
/// never unbounded (no NaN in its bar parts), and the places its rows report are where the replay put them, from the
/// view of their own area, which moved with the card.
#[trial]
fn a_card_holding_a_scrolling_body_replays_moved_with_its_rows_places() {
    let mut frames = Frames::new();
    audit_replays(&frames.ctx, true);
    let mut show = cards(40.0, scrolling_card);
    frames.frame(vec![moved(BESIDE)], &mut show);
    frames.idle(SETTLE, &mut show);
    frames.frame(vec![wheel(-10.0)], &mut show);
    assert!(replayed(&frames) > 0, "the moved cards replayed");
    frames.idle(SETTLE, &mut show);
}

/// A card on half points moved by whole pixels replays while it keeps its side of zero, and runs when the move takes it
/// across: paint rounds a half pixel away from zero, so across zero the card snaps one pixel off the move. The audit
/// finds every replay equal to a run.
#[trial]
fn a_card_on_half_points_replays_moved_on_its_side_of_zero_and_runs_across_it() {
    let mut frames = Frames::new();
    audit_replays(&frames.ctx, true);
    let top = Cell::new(30.5);
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, top.get()), Vec2::new(100.0, 30.0)), |ui| {
            MemoView::new("card", Card { at: 0 }, card).show(ui);
        });
    };
    frames.idle(SETTLE, &mut show);
    top.set(10.5);
    frames.frame(vec![], &mut show);
    assert_eq!(replayed(&frames), 1, "a move that keeps the card below zero replays it");
    top.set(-9.5);
    frames.frame(vec![], &mut show);
    assert_eq!(replayed(&frames), 0, "a move across zero runs it");
    frames.idle(SETTLE, &mut show);
}

/// A view that holds a card in a room of its own, `gap` points down.
#[derive(MemoProps)]
struct Holder {
    gap: f32,
}

fn holder(props: &mut Holder, ui: &mut Ui) {
    in_region(ui, Rect::from_min_size(Pos2::new(20.0, 40.0 + props.gap), Vec2::new(100.0, 30.0)), |ui| {
        MemoView::new("held", Card { at: 0 }, card).show(ui);
    });
}

/// An inner view that replays moved inside the outer view's run is in the outer record where it replayed: the outer
/// view's later replay joins the card's member at the card's new place.
#[trial]
fn a_view_replayed_moved_inside_a_run_is_kept_at_its_new_place() {
    let mut frames = Frames::new();
    audit_replays(&frames.ctx, true);
    let gap = Cell::new(0.0);
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(160.0, 200.0)), |ui| {
            Slab::new(ContainerProps::new().arena(true)).id_salt("holder").show(ui, |ui, _| {
                MemoView::new("holder", Holder { gap: gap.get() }, holder).show(ui);
            });
        });
    };
    frames.idle(SETTLE, &mut show);
    gap.set(15.0);
    frames.frame(vec![], &mut show);
    let runs = last_frame_memo_runs(&frames.ctx);
    assert!(runs.iter().any(|(name, _, run)| name.contains("Holder") && *run == MemoRun::Ran), "the holder ran: {runs:?}");
    assert!(runs.iter().any(|(name, _, run)| name.contains("Card") && *run == MemoRun::Replayed), "the card replayed in it: {runs:?}");
    frames.idle(SETTLE, &mut show);
}

/// A view holding a capped list that reveals its last row on every run.
#[derive(MemoProps)]
struct RevealedRows {}

fn revealed_rows(_: &mut RevealedRows, ui: &mut Ui) {
    let keys: Vec<usize> = (0..40).collect();
    let source = craft_behavior::keyed(&keys, |key: &usize| *key);
    List::new("Rows").max_rows(8).reveal(39).show_items(ui, &source, &mut |key, _| ListRow::new(format!("Row {key}")));
}

/// A reveal moves the rows after the list declared its scroll bar at the old offset: the view runs again rather than
/// replay a run whose thumb stands where the rows no longer are, so its replays match a real run.
#[trial]
fn a_list_revealing_a_row_in_a_memoized_view_replays_as_it_runs() {
    let mut frames = Frames::new();
    audit_replays(&frames.ctx, true);
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 400.0)), |ui| {
            MemoView::new("revealed", RevealedRows {}, revealed_rows).show(ui);
        });
    };
    frames.idle(SETTLE, &mut show);
    let ran: Vec<_> = last_frame_memo_runs(&frames.ctx).into_iter().filter(|(_, _, run)| *run == MemoRun::Ran).collect();
    assert!(ran.is_empty(), "the view settles: {ran:?}");
}

/// A labelled select, as an options bar shows a mode.
#[derive(MemoProps)]
struct ModeField {
    label: &'static str,
}

fn mode_field(props: &mut ModeField, ui: &mut Ui) {
    let entries = [Entry::new(0u8, "Normal"), Entry::new(1, "Multiply")];
    Field::new(props.label).show(ui, |ui, _| Select::new(props.label, Bind::given(0u8), &entries).show(ui));
}

/// The screen y of the middle of the painted run `text` in `region`.
fn middle_of(fixture: &Fixture, text: &str, region: Rect) -> f32 {
    let run = fixture.texts().iter().find(|run| run.text == text && region.contains(run.rect.center()));
    run.unwrap_or_else(|| panic!("{text} is painted in {region:?}")).rect.center().y
}

/// A view back after frames away replays its kept run under the audit. The field's label sits by what its control
/// recorded the pass before, which layout memory dropped while the view was away: the audit run places the label at
/// the field's top and asks for a discard pass, so its pass shows nothing and is not compared. The frame shows the
/// label on the control's middle, as before the view went away.
#[trial]
fn a_view_back_after_frames_away_replays_under_the_audit() {
    let shown = Rc::new(Cell::new(true));
    let showing = Rc::clone(&shown);
    let mut fixture = Fixture::new(move |ui| {
        // A view that stays shown keeps the store's room for the absent one (the memo rule for absent entries).
        in_region(ui, region(1), |ui| MemoView::new("stays", ModeField { label: "Blend" }, mode_field).show(ui));
        if showing.get() {
            in_region(ui, region(0), |ui| MemoView::new("mode", ModeField { label: "Mode" }, mode_field).show(ui));
        }
    });
    fixture.room(Vec2::new(400.0, 200.0)).settle();
    audit_replays(fixture.ctx(), true);
    fixture.idle(2);
    let (label, control) = (middle_of(&fixture, "Mode", region(0)), middle_of(&fixture, "Normal", region(0)));
    assert!((label - control).abs() <= 0.5, "the label {label} sits on the control's middle {control}");
    shown.set(false);
    fixture.idle(4);
    shown.set(true);
    fixture.frame(vec![]);
    let ran = last_frame_memo_runs(fixture.ctx());
    assert!(!ran.iter().any(|(_, _, run)| *run == MemoRun::Ran), "the views replay: {ran:?}");
    assert!((middle_of(&fixture, "Mode", region(0)) - label).abs() <= 0.5, "the label shows where it was before the view went away");
}

/// An options bar's members: a caption, or a radio group in its place.
#[derive(MemoProps)]
struct Members {
    radios: bool,
}

fn members(props: &mut Members, ui: &mut Ui) {
    if !props.radios {
        Button::new("Zoom").show(ui);
        return;
    }
    RadioGroup::new("Type", Bind::given(1u8)).show(ui, |ui, _| {
        Radio::new(0u8).label("Proximity").show(ui);
        Radio::new(1u8).label("Content-Aware").show(ui);
    });
}

/// A radio group that first shows in a view's run names Tab's arrival (its selected member) from the members that run
/// registers: the run's record holds the arrival, so the view replays from the next frame, with no extra pass or run.
#[trial]
fn a_radio_group_new_in_a_run_replays_with_its_arrival() {
    let radios = Rc::new(Cell::new(false));
    let shown = Rc::clone(&radios);
    let mut fixture = Fixture::new(move |ui| {
        in_region(ui, region(0), |ui| MemoView::new("members", Members { radios: shown.get() }, members).show(ui));
    });
    fixture.room(Vec2::new(400.0, 200.0)).settle();
    audit_replays(fixture.ctx(), true);
    radios.set(true);
    fixture.frame(vec![]);
    assert_eq!(fixture.passes(), 1, "the frame the group appears in runs one pass");
    fixture.frame(vec![]);
    let ran = last_frame_memo_runs(fixture.ctx());
    assert!(!ran.iter().any(|(_, _, run)| *run == MemoRun::Ran), "the view replays in the next frame: {ran:?}");
    fixture.idle(3);
    let ran = last_frame_memo_runs(fixture.ctx());
    assert!(!ran.iter().any(|(_, _, run)| *run == MemoRun::Ran), "the view stays settled: {ran:?}");
}
