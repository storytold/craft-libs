//! Memoized views (`MemoView`): a view runs only when its props key or a settled rule changes. Count trials: how views
//! took part in a pass (`last_pass_memo_runs`), as relations.

use crate::values::kit::Scrub;
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, MemoProps, MemoView, Props as _, Show as _, ShowContainer as _, Tracked};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::env::{BodyMode, RevealEdge, ShellCx, WindowKey};
use craft_ui::runtime::memory::{SaveKey, set_saved_value};
use craft_ui::runtime::{frame, inspect};
use craft_ui::testing::in_audit_run;
use craft_ui::testing::input::key_press;
use craft_ui::testing::layout::{in_region, measured_block};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{
    Frames, MemoRun, last_frame_memo_runs, last_pass_measuring_runs, last_pass_memo_key_bytes, last_pass_memo_runs, last_pass_runs,
    last_pass_text_layouts, measuring_run, trial,
};
use egui::{Id, Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::Cell;

thread_local! {
    /// The runs of `title` on this trial's thread, real and measuring: `last_pass_memo_runs` sees only the last pass.
    static TITLE_RUNS: Cell<usize> = const { Cell::new(0) };
}

/// Takes the runs of `title` counted since the last call.
fn title_runs() -> usize {
    TITLE_RUNS.with(|runs| runs.replace(0))
}

/// Idle frames that settle every view after a change.
const WARM: usize = 3;
/// Idle frames that settle every view after the pointer left it: its hover has faded.
const SETTLE: usize = 30;
const SIZE: Vec2 = Vec2::new(140.0, 40.0);

/// Where the view `at` is shown, each in its own region, so one view's size does not move another.
fn region(at: usize) -> Rect {
    Rect::from_min_size(Pos2::new(10.0, 10.0 + 50.0 * at as f32), SIZE)
}

/// Where the button outside every view is.
const OUTSIDE: Rect = Rect { min: Pos2::new(200.0, 10.0), max: Pos2::new(300.0, 40.0) };

#[derive(MemoProps)]
struct Title {
    title: String,
}

fn title(props: &mut Title, ui: &mut Ui) -> Rect {
    // An audit run is the replay audit's, not the pass's: the trials count none.
    if !in_audit_run(ui.ctx()) {
        TITLE_RUNS.with(|runs| runs.set(runs.get() + 1));
    }
    Button::new(props.title.clone()).show(ui).response.rect
}

#[derive(MemoProps)]
struct Names<'a> {
    names: &'a Tracked<Vec<String>>,
}

fn names(props: &mut Names<'_>, ui: &mut Ui) -> usize {
    for name in props.names.iter().take(3) {
        Button::new(name.clone()).show(ui);
    }
    props.names.len()
}

/// A view that brings its model to `wanted` when it runs: a write out of a view through its props.
#[derive(MemoProps)]
struct Counter<'a> {
    wanted: u32,
    model: &'a mut Tracked<u32>,
}

fn counter(props: &mut Counter<'_>, ui: &mut Ui) -> u32 {
    let wanted = props.wanted;
    if **props.model != wanted {
        props.model.edit(|value| *value = wanted);
    }
    Button::new(format!("Count {wanted}")).show(ui);
    wanted
}

/// A view holding a block whose measured width is `width`: a new width after a settled frame asks for a discard pass.
#[derive(MemoProps)]
struct Sized {
    width: f32,
}

fn sized(props: &mut Sized, ui: &mut Ui) -> f32 {
    measured_block(ui, "block", Vec2::new(props.width, 20.0));
    Button::new("Sized").show(ui);
    props.width
}

/// The app state the scene shows: a title, names, a counter's model and a sized block's width.
struct Scene {
    title: String,
    names: Tracked<Vec<String>>,
    wanted: u32,
    count: Tracked<u32>,
    width: f32,
    /// Whether only the button outside every view is shown.
    bare: bool,
    /// What each view returned in the last frame.
    out: (Option<Rect>, Option<usize>, Option<u32>, Option<f32>),
}

impl Scene {
    fn new() -> Scene {
        Scene {
            title: "Title".to_owned(),
            names: Tracked::new(vec!["a".to_owned(), "b".to_owned()]),
            wanted: 1,
            count: Tracked::new(0),
            width: 40.0,
            bare: false,
            out: (None, None, None, None),
        }
    }

    fn show(&mut self, ui: &mut Ui) {
        in_region(ui, OUTSIDE, |ui| Button::new("Outside").show(ui));
        if self.bare {
            return;
        }
        let title_out = in_region(ui, region(0), |ui| MemoView::new("title", Title { title: self.title.clone() }, title).show(ui));
        let names_out = in_region(ui, region(1), |ui| MemoView::new("names", Names { names: &self.names }, names).show(ui));
        let counter_out =
            in_region(ui, region(2), |ui| MemoView::new("counter", Counter { wanted: self.wanted, model: &mut self.count }, counter).show(ui));
        let sized_out = in_region(ui, region(3), |ui| MemoView::new("sized", Sized { width: self.width }, sized).show(ui));
        self.out = (title_out, names_out, counter_out, sized_out);
    }

    /// One frame with `events`; returns its passes.
    fn frame(&mut self, frames: &mut Frames, events: Vec<egui::Event>) -> usize {
        frames.frame(events, |ui| self.show(ui))
    }

    /// Idle frames until every view settled, hover fades included.
    fn settle(&mut self, frames: &mut Frames) {
        for _ in 0..SETTLE {
            self.frame(frames, vec![]);
        }
    }
}

/// How the views whose props type is named `name` took part in the last pass.
fn runs_of(frames: &Frames, name: &str) -> Vec<MemoRun> {
    last_pass_memo_runs(&frames.ctx).into_iter().filter(|(of, _, _)| of.contains(name)).map(|(_, _, run)| run).collect()
}

/// The views that ran in the last pass, by props type name.
fn ran(frames: &Frames) -> Vec<&'static str> {
    last_pass_memo_runs(&frames.ctx).into_iter().filter(|(_, _, run)| *run == MemoRun::Ran).map(|(name, _, _)| name).collect()
}

fn component_runs(frames: &Frames) -> u32 {
    last_pass_runs(&frames.ctx).iter().map(|(_, runs)| runs).sum()
}

/// After two warm-up frames an idle frame runs no view: the components that run, and the text laid out, are those outside every
/// view, and the frame takes one pass.
#[trial]
fn an_idle_frame_runs_no_memo_view() {
    const WARM_UP: usize = 2;
    let mut scene = Scene::new();
    let mut frames = Frames::new();
    let mut bare = Scene::new();
    bare.bare = true;
    let mut outside = Frames::new();
    for _ in 0..WARM_UP {
        scene.frame(&mut frames, vec![]);
        bare.frame(&mut outside, vec![]);
    }
    bare.frame(&mut outside, vec![]);
    let laid_out = last_pass_text_layouts(&outside.ctx);
    let passes = scene.frame(&mut frames, vec![]);
    assert!(ran(&frames).is_empty(), "an idle frame ran views: {:?}", last_pass_memo_runs(&frames.ctx));
    assert_eq!(runs_of(&frames, "Title"), [MemoRun::Replayed]);
    assert_eq!(component_runs(&frames), component_runs(&outside), "components ran inside views: {:?}", last_pass_runs(&frames.ctx));
    assert_eq!(passes, 1, "an idle frame takes one pass");
    assert_eq!(last_pass_text_layouts(&frames.ctx), laid_out, "an idle frame laid out text inside views");
}

/// A changed prop runs exactly its view once, then the view replays; what a replay returns equals what the run returned.
#[trial]
fn a_changed_prop_runs_its_view_once() {
    let mut scene = Scene::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    let before = scene.out;
    scene.title = "A longer title".to_owned();
    scene.frame(&mut frames, vec![]);
    assert_eq!(ran(&frames).iter().filter(|name| !name.contains("Title")).count(), 0, "only the title ran: {:?}", ran(&frames));
    assert_eq!(runs_of(&frames, "Title"), [MemoRun::Ran]);
    let run = scene.out;
    assert_ne!(run.0, before.0, "the run returned the new title's size");
    scene.frame(&mut frames, vec![]);
    assert_eq!(runs_of(&frames, "Title"), [MemoRun::Replayed], "it settles on the new props");
    assert_eq!(scene.out, run, "a replay returns what the run returned");
}

/// A pass egui will discard paints nothing, so a run there keeps no record: a settled view replays through it, and the
/// replay audit runs nothing in it; a view whose props change in it runs there and again in the shown pass, whose run
/// is the one kept. The audit is on, so each later replay is compared with a real run.
#[trial]
fn a_pass_egui_will_discard_keeps_no_record_and_replays_settled_views() {
    let mut frames = Frames::new();
    craft_ui::testing::audit_replays(&frames.ctx, true);
    let frame = |frames: &mut Frames, shown: &str, discard: bool| {
        let passes = frames.frame(vec![], |ui| {
            if discard && ui.ctx().current_pass_index() == 0 {
                frame::discard_pass(ui.ctx(), "trial");
            }
            in_region(ui, region(0), |ui| MemoView::new("title", Title { title: shown.to_owned() }, title).show(ui));
        });
        (passes, title_runs())
    };
    for _ in 0..WARM {
        frame(&mut frames, "A", false);
    }
    assert_eq!(frame(&mut frames, "A", false), (1, 0), "a settled view replays");
    assert_eq!(frame(&mut frames, "A", true), (2, 0), "it replays through the discarded pass and the shown one");
    assert_eq!(frame(&mut frames, "A", false), (1, 0), "its kept record is unchanged: a run gives what it replays");
    assert_eq!(frame(&mut frames, "B", true), (2, 2), "new props run it in the discarded pass and again in the shown one");
    assert_eq!(frame(&mut frames, "B", false), (1, 0), "then it replays the shown pass's run");
}

/// A kept run's text points into egui's font atlas; when egui rebuilds the atlas (it filled up, or the text options
/// changed) the kept glyphs name other glyphs, so a settled view runs in place of replaying, then settles again.
#[trial]
fn a_settled_view_runs_after_the_font_atlas_is_rebuilt() {
    let mut frames = Frames::new();
    let frame = |frames: &mut Frames| {
        frames.frame(vec![], |ui| {
            in_region(ui, region(0), |ui| MemoView::new("title", Title { title: "Atlas".to_owned() }, title).show(ui));
        });
        title_runs()
    };
    for _ in 0..WARM {
        frame(&mut frames);
    }
    assert_eq!(frame(&mut frames), 0, "a settled view replays");
    let before = frames.ctx.fonts(|fonts| fonts.atlas_generation());
    frames.ctx.all_styles_mut(|style| style.visuals.text_options.font_hinting = !style.visuals.text_options.font_hinting);
    assert!(frame(&mut frames) > 0, "the view runs in the first frame on the rebuilt atlas");
    assert_ne!(frames.ctx.fonts(|fonts| fonts.atlas_generation()), before, "the atlas was rebuilt");
    for _ in 0..WARM {
        frame(&mut frames);
    }
    assert_eq!(frame(&mut frames), 0, "then it replays again");
}

/// A view that writes its own `&mut Tracked` runs twice for the change that asked for the write, the writing run and
/// one that reads the written value, then replays.
#[trial]
fn a_views_own_tracked_write_runs_it_once_more_then_replays() {
    let mut scene = Scene::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    scene.wanted = 5;
    scene.frame(&mut frames, vec![]);
    assert_eq!(runs_of(&frames, "Counter"), [MemoRun::Ran], "the writing run: {:?}", last_pass_memo_runs(&frames.ctx));
    assert_eq!(*scene.count, 5, "the run wrote its model");
    scene.frame(&mut frames, vec![]);
    assert_eq!(runs_of(&frames, "Counter"), [MemoRun::Ran], "the run that reads the written value");
    for frame in 0..WARM {
        scene.frame(&mut frames, vec![]);
        assert_eq!(runs_of(&frames, "Counter"), [MemoRun::Replayed], "frame {frame} after its own write");
    }
}

/// A view that paints its model's value and clamps it.
#[derive(MemoProps)]
struct Clamped<'a> {
    model: &'a mut Tracked<i32>,
}

fn clamped(props: &mut Clamped<'_>, ui: &mut Ui) -> i32 {
    let shown = **props.model;
    Button::new(format!("Value {shown}")).show(ui);
    if shown > 10 {
        props.model.edit(|value| *value = 10);
    }
    shown
}

fn painted(frames: &Frames, text: &str) -> bool {
    frames.texts().iter().any(|run| run.text == text)
}

/// A view that clamps the model value it paints shows the clamped value once it settles, and replays that: never the
/// value it painted before its write.
#[trial]
fn a_view_that_clamps_what_it_paints_settles_on_the_clamped_value() {
    let mut model = Tracked::new(3);
    let mut frames = Frames::new();
    let frame = |frames: &mut Frames, model: &mut Tracked<i32>| {
        let mut out = None;
        frames.frame(vec![], |ui| out = in_region(ui, region(0), |ui| MemoView::new("clamped", Clamped { model: &mut *model }, clamped).show(ui)));
        (out, last_pass_memo_runs(&frames.ctx).into_iter().map(|(_, _, run)| run).collect::<Vec<_>>())
    };
    for _ in 0..WARM {
        frame(&mut frames, &mut model);
    }
    model.edit(|value| *value = 50);
    assert_eq!(frame(&mut frames, &mut model), (Some(50), vec![MemoRun::Ran]), "the run paints what it found, then clamps");
    assert_eq!(*model, 10);
    assert_eq!(frame(&mut frames, &mut model), (Some(10), vec![MemoRun::Ran]), "the next run reads the clamped value");
    for _ in 0..WARM {
        assert_eq!(frame(&mut frames, &mut model), (Some(10), vec![MemoRun::Replayed]));
        assert!(painted(&frames, "Value 10") && !painted(&frames, "Value 50"), "a replay paints the clamped value");
    }
}

/// An edit of a `Tracked` runs exactly the views whose props hold it, in that frame.
#[trial]
fn a_tracked_edit_runs_exactly_its_readers() {
    let mut scene = Scene::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    scene.names.edit(|names| names.push("c".to_owned()));
    scene.frame(&mut frames, vec![]);
    assert_eq!(ran(&frames).len(), 1, "one view ran: {:?}", ran(&frames));
    assert_eq!(runs_of(&frames, "Names"), [MemoRun::Ran], "{:?}", last_pass_memo_runs(&frames.ctx));
    assert_eq!(scene.out.1, Some(3));
}

/// The pointer over one view runs it and only it; a key press while focus is outside every view replays them all.
#[trial]
fn the_pointer_runs_its_view_and_a_key_elsewhere_runs_none() {
    let mut scene = Scene::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    let over = scene.out.0.map_or(region(0).min, |rect| rect.center());
    scene.frame(&mut frames, vec![moved(over)]);
    assert_eq!(ran(&frames), runs_of_names(&frames, "Title"), "only the view under the pointer ran: {:?}", ran(&frames));
    assert_eq!(runs_of(&frames, "Title"), [MemoRun::Ran]);

    let outside = OUTSIDE.center();
    scene.frame(&mut frames, vec![moved(outside), press(outside)]);
    scene.frame(&mut frames, vec![release(outside)]);
    scene.settle(&mut frames);
    scene.frame(&mut frames, vec![key_press(Key::A, Modifiers::NONE)]);
    assert!(ran(&frames).is_empty(), "a key press with focus outside ran views: {:?}", ran(&frames));
}

/// The names of the views of the last pass whose props type is named `name`.
fn runs_of_names(frames: &Frames, name: &str) -> Vec<&'static str> {
    last_pass_memo_runs(&frames.ctx).into_iter().filter(|(of, _, run)| of.contains(name) && *run == MemoRun::Ran).map(|(of, _, _)| of).collect()
}

/// A discard pass that one view's new size asks for replays every other view in pass 2, and runs the view that asked:
/// its first run laid out by the old measure.
#[trial]
fn a_discard_pass_replays_the_views_that_did_not_change() {
    let mut scene = Scene::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    // The title changes in the same frame: its view runs in pass 1 and replays that run in pass 2.
    scene.title = "Another title".to_owned();
    scene.width = 90.0;
    title_runs();
    let passes = scene.frame(&mut frames, vec![]);
    assert_eq!(passes, 2, "the new width asks for one discard pass");
    assert_eq!(title_runs(), 1, "the title ran in pass 1 and replayed that run in pass 2");
    let runs = last_pass_memo_runs(&frames.ctx);
    let others: Vec<_> = runs.iter().filter(|(name, _, _)| !name.contains("Sized")).map(|(_, _, run)| *run).collect();
    assert_eq!(others, [MemoRun::Replayed; 3], "pass 2 replays the other views: {runs:?}");
    assert_eq!(runs_of(&frames, "Sized"), [MemoRun::Ran], "the view that asked for the discard pass runs in it");
    scene.frame(&mut frames, vec![]);
    assert!(ran(&frames).is_empty(), "then every view replays: {:?}", ran(&frames));
}

#[derive(MemoProps)]
struct EmptyA {}

#[derive(MemoProps)]
struct EmptyB {}

fn one_a(_: &mut EmptyA, ui: &mut Ui) -> u32 {
    Button::new("One").show(ui);
    1
}

fn one_b(_: &mut EmptyB, ui: &mut Ui) -> u32 {
    Button::new("One").show(ui);
    1
}

fn two_a(_: &mut EmptyA, ui: &mut Ui) -> u32 {
    Button::new("Two").show(ui);
    2
}

/// Two views at one id with props that feed nothing do not replay each other: neither another props type nor another
/// `fn`.
#[trial]
fn views_at_one_id_key_apart_by_props_type_and_fn() {
    let mut frames = Frames::new();
    let shown = |frames: &mut Frames, which: usize| {
        let mut out = None;
        frames.frame(vec![], |ui| {
            out = in_region(ui, region(0), |ui| match which {
                0 => MemoView::new("same", EmptyA {}, one_a).show(ui),
                1 => MemoView::new("same", EmptyB {}, one_b).show(ui),
                _ => MemoView::new("same", EmptyA {}, two_a).show(ui),
            });
        });
        (out, last_pass_memo_runs(&frames.ctx).into_iter().map(|(_, _, run)| run).collect::<Vec<_>>())
    };
    for _ in 0..WARM {
        shown(&mut frames, 0);
    }
    assert_eq!(shown(&mut frames, 0), (Some(1), vec![MemoRun::Replayed]));
    assert_eq!(shown(&mut frames, 1).1, [MemoRun::Ran], "another props type runs");
    for _ in 0..WARM {
        shown(&mut frames, 0);
    }
    assert_eq!(shown(&mut frames, 2), (Some(2), vec![MemoRun::Ran]), "another fn runs");
}

/// An idle frame feeds a bounded number of key bytes per view, and a `Tracked` prop feeds the same bytes whatever the
/// size of its model.
#[trial]
fn key_bytes_per_view_are_bounded_and_independent_of_tracked_size() {
    const BOUND: usize = 128;
    let mut scene = Scene::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    scene.frame(&mut frames, vec![]);
    let views = last_pass_memo_runs(&frames.ctx).len();
    let small = last_pass_memo_key_bytes(&frames.ctx);
    assert!(small <= BOUND * views, "{small} key bytes for {views} views");
    scene.names.edit(|names| names.extend((0..10_000).map(|n| n.to_string())));
    scene.settle(&mut frames);
    scene.frame(&mut frames, vec![]);
    assert_eq!(last_pass_memo_key_bytes(&frames.ctx), small, "a large tracked model feeds the same key bytes");
}

/// A view shown again after a few frames of absence, with the same props, replays; absent views beyond twice the
/// shown ones are dropped, the longest absent first.
#[trial]
fn an_absent_view_keeps_its_run_within_twice_the_shown_count() {
    let mut frames = Frames::new();
    // The views in `shown` by their props (a title each); one view is always shown.
    let frame = |frames: &mut Frames, shown: &[&str]| {
        frames.frame(vec![], |ui| {
            in_region(ui, region(0), |ui| MemoView::new("always", Title { title: "Always".to_owned() }, title).show(ui));
            for (at, name) in shown.iter().enumerate() {
                in_region(ui, region(1 + at), |ui| MemoView::new(*name, Title { title: (*name).to_owned() }, title).show(ui));
            }
        });
        last_pass_memo_runs(&frames.ctx).into_iter().skip(1).map(|(_, _, run)| run).collect::<Vec<_>>()
    };
    for _ in 0..WARM {
        frame(&mut frames, &["a"]);
    }
    for _ in 0..WARM {
        frame(&mut frames, &[]);
    }
    assert_eq!(frame(&mut frames, &["a"]), [MemoRun::Replayed], "a view shown again with the same props replays");
    // Each of b, c and d shows once, at a's place, then leaves: a and b are older than twice the one shown view.
    for name in ["b", "c", "d"] {
        frame(&mut frames, &[name]);
        frame(&mut frames, &[]);
        frame(&mut frames, &[]);
    }
    assert_eq!(frame(&mut frames, &["d"]), [MemoRun::Replayed], "the newest absent view kept its run");
    frame(&mut frames, &[]);
    frame(&mut frames, &[]);
    assert_eq!(frame(&mut frames, &["a"]), [MemoRun::Ran], "the longest absent view was dropped");
}

/// A view that reads the record `POINTED`.
#[derive(MemoProps)]
struct Pointer {
    label: String,
}

const POINTED: &str = "memo-view-pointed";

fn pointer(props: &mut Pointer, ui: &mut Ui) -> Option<usize> {
    // An audit run is the replay audit's, not the pass's: the trials count none.
    if !in_audit_run(ui.ctx()) {
        TITLE_RUNS.with(|runs| runs.set(runs.get() + 1));
    }
    Button::new(props.label.clone()).show(ui);
    inspect::pointed(ui.ctx(), Id::NULL.with(POINTED))
}

/// A run of an earlier pass of the frame is not replayed when a value it read was edited after it started: the edit
/// comes later in the pass, asks for the discard pass, and the view runs in it.
#[trial]
fn an_edit_after_a_run_of_the_frame_runs_it_in_the_discard_pass() {
    let mut frames = Frames::new();
    let frame = |frames: &mut Frames, label: &str, point: Option<usize>| {
        frames.frame(vec![], |ui| {
            in_region(ui, region(0), |ui| MemoView::new("pointer", Pointer { label: label.to_owned() }, pointer).show(ui));
            if let Some(point) = point {
                inspect::point_at(ui.ctx(), Id::NULL.with(POINTED), Some(point));
            }
        })
    };
    for _ in 0..WARM {
        frame(&mut frames, "A", Some(1));
    }
    title_runs();
    assert_eq!(frame(&mut frames, "B", Some(1)), 1, "an unchanged record asks for no discard pass");
    assert_eq!(title_runs(), 1, "the new label runs the view once");
    assert_eq!(frame(&mut frames, "C", Some(2)), 2, "the edit after the view read it asks for the discard pass");
    assert_eq!(title_runs(), 2, "the view runs again in pass 2: the record changed after its pass-1 run started");
}

/// A measuring run over a settled view takes its kept extent and returns the kept value without running it.
#[trial]
fn a_measuring_run_returns_the_kept_value() {
    let mut frames = Frames::new();
    let frame = |frames: &mut Frames, measure: bool| {
        let mut out = (None, None);
        frames.frame(vec![], |ui| {
            in_region(ui, region(0), |ui| {
                let shown = MemoView::new("measured", Title { title: "Measured".to_owned() }, title).show(ui);
                // After the real run, so the view keeps its place; a measuring run has none to compare.
                let ctx = ui.ctx().clone();
                let measured =
                    measure.then(|| measuring_run(&ctx, || MemoView::new("measured", Title { title: "Measured".to_owned() }, title).show(ui)));
                out = (shown, measured.flatten());
            });
        });
        out
    };
    for _ in 0..WARM {
        frame(&mut frames, false);
    }
    title_runs();
    let (shown, measured) = frame(&mut frames, true);
    assert!(shown.is_some(), "the view replays");
    assert_eq!(measured, shown, "the measuring run returns the kept value");
    assert_eq!(title_runs(), 0, "neither run ran the view");
}

/// A view whose components' shown memory was evicted while it was absent (over 0.5 s and 3 passes) runs when it is
/// shown again, though its props key is equal: a replay would show state memory dropped.
#[trial]
fn a_view_back_after_its_memory_was_evicted_runs() {
    let mut frames = Frames::new();
    let frame = |frames: &mut Frames, shown: bool| {
        frames.frame(vec![], |ui| {
            in_region(ui, region(0), |ui| MemoView::new("always", Title { title: "Always".to_owned() }, title).show(ui));
            if shown {
                in_region(ui, region(1), |ui| MemoView::new("back", Title { title: "Back".to_owned() }, title).show(ui));
            }
        });
        last_pass_memo_runs(&frames.ctx).into_iter().skip(1).map(|(_, _, run)| run).collect::<Vec<_>>()
    };
    for _ in 0..WARM {
        frame(&mut frames, true);
    }
    for _ in 0..WARM {
        frame(&mut frames, false);
    }
    assert_eq!(frame(&mut frames, true), [MemoRun::Replayed], "back within the grace, the view replays");
    frame(&mut frames, false);
    frames.advance(1.0);
    for _ in 0..WARM {
        frame(&mut frames, false);
    }
    assert_eq!(frame(&mut frames, true), [MemoRun::Ran], "back after its memory was evicted, the view runs");
}

const FLAG: SaveKey = SaveKey::new("craft_tests.memo_flag");

/// A view showing a saved binding.
#[derive(MemoProps)]
struct Flag<'a> {
    flag: Bind<'a, bool>,
}

fn flag(props: &mut Flag<'_>, ui: &mut Ui) -> bool {
    Checkbox::new(props.flag.reborrowed()).show(ui);
    true
}

/// A write of a saved value from outside every view runs the view whose props bind it, and only that view.
#[trial]
fn a_saved_value_written_from_outside_runs_its_view() {
    let mut frames = Frames::new();
    let frame = |frames: &mut Frames| {
        frames.frame(vec![], |ui| {
            in_region(ui, region(0), |ui| MemoView::new("title", Title { title: "Title".to_owned() }, title).show(ui));
            in_region(ui, region(1), |ui| MemoView::new("flag", Flag { flag: Bind::kept().saved(FLAG) }, flag).show(ui));
        });
        last_pass_memo_runs(&frames.ctx).into_iter().filter(|(_, _, run)| *run == MemoRun::Ran).map(|(name, _, _)| name).collect::<Vec<_>>()
    };
    for _ in 0..WARM {
        frame(&mut frames);
    }
    assert!(frame(&mut frames).is_empty(), "settled");
    set_saved_value(&frames.ctx, &FLAG, true);
    let ran = frame(&mut frames);
    assert_eq!(ran.len(), 1, "one view ran: {ran:?}");
    assert!(ran[0].contains("Flag"), "the view that binds the value ran: {ran:?}");
    assert!(frame(&mut frames).is_empty(), "then it replays");
}

/// Where the stack of the sizing trials is shown: wider than every label it holds.
const STACK_ROOM: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(410.0, 210.0) };

/// One frame of a stack filling `STACK_ROOM` that holds the memoized view `title` showing `heading`, then the text
/// `label`. Returns the frame's passes, the sizing passes the stack's content ran in, the width the view took in the
/// last of them and what the view returned (its button's size) in the last real run.
fn stack_frame(frames: &mut Frames, label: &str, heading: &str) -> (usize, usize, f32, Option<Vec2>) {
    stack_frame_in(frames, STACK_ROOM, label, heading)
}

/// `stack_frame` with the stack in `room`.
fn stack_frame_in(frames: &mut Frames, room: Rect, label: &str, heading: &str) -> (usize, usize, f32, Option<Vec2>) {
    let (sizing, measured) = (Cell::new(0), Cell::new(0.0));
    let mut out = None;
    let passes = frames.frame(vec![], |ui| {
        in_region(ui, room, |ui| {
            let shown = Stack::new().fill().show(ui, |ui, _| {
                let out = MemoView::new("heading", Title { title: heading.to_owned() }, title).show(ui).map(|rect| rect.size());
                if ui.is_sizing_pass() {
                    sizing.set(sizing.get() + 1);
                    measured.set(ui.min_rect().width());
                }
                Text::new(label.to_owned()).show(ui);
                out
            });
            out = shown.inner.flatten();
        });
    });
    (passes, sizing.get(), measured.get(), out)
}

/// A view in a container that measures its content again (in an invisible sizing pass, after a sibling below it changed
/// size) replays there as in the real run: the sizing `Ui` is invisible, not disabled, so the view keys alike in both.
#[trial]
fn a_view_in_a_container_that_measures_again_replays_in_its_sizing_pass() {
    let mut frames = Frames::new();
    for _ in 0..SETTLE {
        stack_frame(&mut frames, "Short", "Heading");
    }
    title_runs();
    let (_, sizing, _, _) = stack_frame(&mut frames, "A much longer label than before", "Heading");
    assert!(sizing > 0, "the stack measured its content again in a sizing pass");
    assert_eq!(title_runs(), 0, "the view replayed in the sizing pass and in the real run");
}

/// A changed view runs in the real run of the pass it changed in; its new size asks for a discard pass, whose sizing
/// pass has no measure of the new content, so the view runs there once as a measuring run, and the discard pass's real
/// run replays it. The sizing pass measures the view at its new extent, not at the one it had before the change.
#[trial]
fn a_changed_view_is_measured_once_in_the_sizing_pass_its_new_size_asks_for() {
    let mut frames = Frames::new();
    for _ in 0..SETTLE {
        stack_frame(&mut frames, "Label", "Heading");
    }
    title_runs();
    let (passes, sizing, measured, button) = stack_frame(&mut frames, "Label", "A much longer heading than before");
    assert_eq!(passes, 2, "the new size asks for one discard pass");
    assert!(sizing > 0, "the discard pass measures the stack's content");
    assert_eq!(title_runs(), 2, "one real run in pass 1 and one measuring run in pass 2's sizing pass");
    assert_eq!(runs_of(&frames, "Title"), [MemoRun::Replayed], "pass 2's real run replays the pass-1 run");
    let button = button.expect("the view returned its button's size");
    assert!(measured + 0.5 >= button.x, "the sizing pass measured the new heading: {measured} for a button {button:?}");
    stack_frame(&mut frames, "Label", "A much longer heading than before");
    assert_eq!(title_runs(), 0, "then the view replays in every run");
}

/// Where a container measures before the real run of the pass the view changed in (here its room changed in the same
/// frame), the changed view runs in both runs of that pass: once as a measuring run in the sizing pass, whose run in an
/// invisible `Ui` can't be replayed in the real run, then in the real run. That is the same count as a change met by a
/// later sizing pass, in the other order, and no later pass of the frame runs it again.
#[trial]
fn a_changed_view_its_container_measures_first_runs_in_the_sizing_pass_and_the_real_run() {
    let mut frames = Frames::new();
    for _ in 0..SETTLE {
        stack_frame(&mut frames, "Label", "Heading");
    }
    title_runs();
    let wider = Rect::from_min_size(STACK_ROOM.min, STACK_ROOM.size() + Vec2::new(100.0, 0.0));
    let (passes, sizing, _, _) = stack_frame_in(&mut frames, wider, "Label", "A much longer heading");
    assert_eq!((passes, sizing), (1, 1), "the stack measures its content before the real run of the one pass");
    assert_eq!(
        last_frame_memo_runs(&frames.ctx).into_iter().map(|(_, _, run)| run).collect::<Vec<_>>(),
        [MemoRun::Ran, MemoRun::Ran],
        "one measuring run in the sizing pass, then one real run"
    );
    assert_eq!(title_runs(), 2, "and nothing else ran the view");
    stack_frame_in(&mut frames, wider, "Label", "A much longer heading");
    assert_eq!(title_runs(), 0, "then the view replays in every run");
}

/// A paragraph that wraps at its room's width.
#[derive(MemoProps)]
struct Paragraph {
    text: String,
}

fn paragraph(props: &mut Paragraph, ui: &mut Ui) -> Rect {
    Text::new(props.text.clone()).show(ui).rect
}

/// One frame of a stack `width` wide, sized from its content's height, holding the memoized `paragraph`. Returns the
/// sizing passes the stack's content ran in, the stack's rect and what the view returned (the paragraph's size) in the
/// last real run.
fn paragraph_frame(frames: &mut Frames, width: f32) -> (usize, Rect, Option<Vec2>) {
    let sizing = Cell::new(0);
    let mut out = (Rect::NOTHING, None);
    frames.frame(vec![], |ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(width, 400.0)), |ui| {
            let shown = Stack::new().fill_width().show(ui, |ui, _| {
                if ui.is_sizing_pass() {
                    sizing.set(sizing.get() + 1);
                }
                let text = "A paragraph long enough to wrap onto more lines in a narrow room".to_owned();
                MemoView::new("paragraph", Paragraph { text }, paragraph).show(ui).map(|rect| rect.size())
            });
            out = (shown.output.rect, shown.inner.flatten());
        });
    });
    (sizing.get(), out.0, out.1)
}

/// A view whose props are unchanged, offered another room in its container's sizing pass, is measured there once: its
/// last sizing extent was measured in the old room. The container's size follows the new extent (here the paragraph
/// wraps onto more lines), and the real run runs it too, for the new room is also its own.
#[trial]
fn a_view_offered_another_room_in_a_sizing_pass_is_measured_again() {
    let mut frames = Frames::new();
    for _ in 0..SETTLE {
        paragraph_frame(&mut frames, 600.0);
    }
    let (_, wide, wide_text) = paragraph_frame(&mut frames, 600.0);
    let (sizing, stack, text) = paragraph_frame(&mut frames, 150.0);
    assert!(sizing > 0, "the stack measured its content in a sizing pass");
    let text = text.expect("the view returned the paragraph's size");
    assert!(text.y > wide_text.map_or(0.0, |size| size.y), "the paragraph wraps onto more lines in the narrow room");
    let ran = last_frame_memo_runs(&frames.ctx).into_iter().filter(|(_, _, run)| *run == MemoRun::Ran).count();
    assert_eq!(ran, 2, "one measuring run in the sizing pass and one real run, both in the new room");
    assert!(stack.height() + 0.5 >= text.y, "the stack follows the new extent: {stack:?} for a paragraph {text:?} (wide: {wide:?})");
    paragraph_frame(&mut frames, 150.0);
    assert!(last_frame_memo_runs(&frames.ctx).iter().all(|(_, _, run)| *run == MemoRun::Replayed), "then the view replays in every run");
}

/// A block as tall as `height`.
#[derive(MemoProps)]
struct Tall {
    height: u32,
}

fn tall(props: &mut Tall, ui: &mut Ui) -> Rect {
    craft_ui::testing::layout::Block::new(Vec2::new(100.0, props.height as f32)).show(ui).rect
}

/// One frame of a two-column grid whose cells each hold a `Lazy` around a memoized block, the first 200 tall and the
/// second 40. Returns the passes and the rect each cell's `Lazy` took in the last real pass.
fn grid_frame(frames: &mut Frames) -> (usize, [Rect; 2]) {
    const TWO: &[craft_behavior::ColumnWidth] = &[craft_behavior::ColumnWidth::fill(1.0); 2];
    let took = [Cell::new(Rect::NOTHING), Cell::new(Rect::NOTHING)];
    let passes = frames.frame(vec![], |ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(400.0, 400.0)), |ui| {
            craft_ui::layout_components::Grid::new(TWO).show(ui, |ui, _| {
                for (at, height) in [200, 40].into_iter().enumerate() {
                    craft_ui::contract::Lazy::new(at, 0.0).show(ui, |ui| {
                        let _ = MemoView::new(("cell", at), Tall { height }, tall).show(ui);
                        if !ui.is_sizing_pass() {
                            took[at].set(ui.min_rect());
                        }
                    });
                }
            });
        });
    });
    (passes, took.map(Cell::into_inner))
}

/// A view in a grid cell replays at the extent it ran at, not at its row's height: a replay places its kept extent as
/// its run placed its content, so the body around it finds the measure its kept transcript holds and asks for no
/// discard pass.
#[trial]
fn a_view_in_a_grid_cell_replays_at_the_extent_it_ran_at() {
    let mut frames = Frames::new();
    let (_, ran) = grid_frame(&mut frames);
    assert!(!ran[0].intersects(ran[1]), "the cells sit beside each other: {ran:?}");
    for _ in 0..WARM {
        grid_frame(&mut frames);
    }
    let (passes, took) = grid_frame(&mut frames);
    assert!(last_frame_memo_runs(&frames.ctx).iter().all(|(_, _, run)| *run == MemoRun::Replayed), "an idle frame replays both views");
    assert_eq!(passes, 1, "a replay asks for no discard pass");
    assert_eq!(took, ran, "each cell takes the extent its view ran at");
}

/// A view whose run does not paint `generation`: a new one runs it with the same output.
#[derive(MemoProps)]
struct Moving {
    generation: u32,
}

/// The shell the moving view registers its revealed rect with.
fn moving_shell() -> ShellCx {
    ShellCx::detached(WindowKey::main(), BodyMode::Normal, Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0))).with_reveals_key(Id::unique("moving"))
}

fn moving(_: &mut Moving, ui: &mut Ui) -> Rect {
    let rect = Button::new("Moving").show(ui).response.rect;
    moving_shell().reveal_rect(ui.ctx(), RevealEdge::Start, rect);
    rect
}

/// What one frame of the moving view showed in its last pass: how it took part, the egui records of its region, the
/// revealed rects and what it returned.
struct MovingFrame {
    runs: Vec<MemoRun>,
    records: Option<egui::PassRecords>,
    revealed: Vec<(RevealEdge, Rect)>,
    out: Option<Rect>,
}

/// One frame of the moving view in `room`, shown through `clip` when given.
fn moving_frame(frames: &mut Frames, room: Rect, clip: Option<Rect>, generation: u32) -> MovingFrame {
    let (mut records, mut revealed, mut out) = (None, Vec::new(), None);
    frames.frame(vec![], |ui| {
        let mut show = |ui: &mut Ui| {
            let mark = ui.ctx().pass_mark();
            out = in_region(ui, room, |ui| MemoView::new("moving", Moving { generation }, moving).show(ui));
            records = Some(ui.ctx().pass_records_since(&mark));
            revealed = moving_shell().revealed_rects(ui.ctx());
        };
        match clip {
            Some(clip) => {
                ui.scope(|ui| {
                    ui.set_clip_rect(clip);
                    show(ui);
                });
            }
            None => show(ui),
        }
    });
    MovingFrame { runs: runs_of(frames, "Moving"), records, revealed, out }
}

/// The room of the moving view `down` points below its first place.
fn moving_room(down: f32) -> Rect {
    Rect::from_min_size(Pos2::new(10.0, 10.0 + down), SIZE)
}

/// A view whose room moved replays its record moved with it, and that is what a run there makes: the egui records of
/// its region (paint, widget rects with the stop's root widget, AccessKit nodes), the rect it registered with the
/// shell and what it returned. The focus transcript holds no place: a stop sits where its root widget is.
#[trial]
fn a_view_replayed_at_a_moved_place_equals_a_run_there() {
    let mut frames = Frames::new();
    for _ in 0..WARM {
        moving_frame(&mut frames, moving_room(0.0), None, 0);
    }
    let first = moving_frame(&mut frames, moving_room(0.0), None, 0);
    assert_eq!(first.runs, [MemoRun::Replayed], "the view settled");
    let replayed = moving_frame(&mut frames, moving_room(32.0), None, 0);
    assert_eq!(replayed.runs, [MemoRun::Replayed], "a view whose room moved replays");
    let ran = moving_frame(&mut frames, moving_room(32.0), None, 1);
    assert_eq!(ran.runs, [MemoRun::Ran], "a new generation runs it at the moved place");
    let moved = |rect: Option<Rect>| rect.map(|rect| rect.translate(Vec2::new(0.0, 32.0)));
    assert_eq!(replayed.out, moved(first.out), "the replay returns the run's rect moved with it");
    assert_eq!(replayed.out, ran.out, "and the run there returns the same");
    assert_eq!(replayed.revealed, vec![(RevealEdge::Start, replayed.out.expect("a rect"))], "the replay registers the moved rect");
    assert_eq!(replayed.revealed, ran.revealed, "as the run there does");
    let (replayed, ran) = (replayed.records.expect("records"), ran.records.expect("records"));
    assert!(!replayed.widgets().is_empty(), "the region placed widgets");
    let diff = replayed.first_difference(&ran, 0.0);
    assert!(diff.is_none(), "the moved replay differs from the run there: {}", diff.expect("a difference"));
}

/// A view whose room changed size runs: its extent depends on the room.
#[trial]
fn a_view_whose_room_changes_size_runs() {
    let mut frames = Frames::new();
    for _ in 0..WARM {
        moving_frame(&mut frames, moving_room(0.0), None, 0);
    }
    assert_eq!(moving_frame(&mut frames, moving_room(0.0), None, 0).runs, [MemoRun::Replayed]);
    let wider = Rect::from_min_size(moving_room(0.0).min, SIZE + Vec2::new(20.0, 0.0));
    assert_eq!(moving_frame(&mut frames, wider, None, 0).runs, [MemoRun::Ran]);
}

/// A run its clip cut lacks what the clip culled: moved under a clip that stayed, the view runs, until a run saw it
/// whole. Moved together with its clip, the cut record replays.
#[trial]
fn a_view_cut_by_its_clip_runs_when_it_moves_under_it() {
    let clip = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 100.0));
    let mut frames = Frames::new();
    let settle = |frames: &mut Frames, down: f32, generation: u32| {
        for _ in 0..WARM {
            moving_frame(frames, moving_room(down), Some(clip), generation);
        }
        moving_frame(frames, moving_room(down), Some(clip), generation).runs
    };
    assert_eq!(settle(&mut frames, 70.0, 0), [MemoRun::Replayed], "the cut view settled");
    assert_eq!(moving_frame(&mut frames, moving_room(62.0), Some(clip), 0).runs, [MemoRun::Ran], "a cut run moved under its clip runs");
    assert_eq!(settle(&mut frames, 20.0, 0), [MemoRun::Replayed], "it settles wholly inside its clip");
    assert_eq!(moving_frame(&mut frames, moving_room(70.0), Some(clip), 0).runs, [MemoRun::Replayed], "a whole run replays where the clip cuts it");
    assert_eq!(moving_frame(&mut frames, moving_room(70.0), Some(clip), 1).runs, [MemoRun::Ran], "a new generation runs it cut");
    assert_eq!(settle(&mut frames, 70.0, 1), [MemoRun::Replayed]);
    let both = clip.translate(Vec2::new(0.0, 32.0));
    assert_eq!(moving_frame(&mut frames, moving_room(102.0), Some(both), 1).runs, [MemoRun::Replayed], "a cut run moved with its clip replays");
}

/// A props type's name without its path and generics: `Reader` for `it::body::memo_settled::Reader<'_>`.
fn short_name(name: &'static str) -> &'static str {
    let name = name.split('<').next().unwrap_or(name);
    name.rsplit("::").next().unwrap_or(name)
}

/// A view that asks for a repaint in each run while `asks` holds.
#[derive(MemoProps)]
struct Asking {
    asks: bool,
}

fn asking(props: &mut Asking, ui: &mut Ui) {
    if props.asks {
        frame::request_repaint(ui.ctx());
    }
    Button::new("Asking").show(ui);
}

/// A view of the app's level.
#[derive(MemoProps)]
struct Level<'a> {
    level: &'a Tracked<f32>,
}

fn level(props: &mut Level<'_>, ui: &mut Ui) {
    Button::new(format!("Level {}", **props.level)).show(ui);
}

/// The app state of the repaint and edit trials: a level that a scrub outside every view edits as a given value, a view
/// of the level, a view that asks for repaints while `asks` holds, and the title view, which holds neither. The app
/// asks for a repaint outside every view while `app_asks` holds.
struct Repaints {
    level: Tracked<f32>,
    asks: bool,
    app_asks: bool,
}

impl Repaints {
    fn new() -> Repaints {
        Repaints { level: Tracked::new(10.0), asks: false, app_asks: false }
    }

    fn show(&mut self, ui: &mut Ui) {
        if self.app_asks {
            frame::request_repaint(ui.ctx());
        }
        let edited = Scrub::new(Bind::given(*self.level)).id_salt("level").show_at(ui, OUTSIDE);
        for event in edited.events {
            if let Some(value) = event.value().copied() {
                self.level.edit(|level| *level = value);
            }
        }
        in_region(ui, region(0), |ui| MemoView::new("title", Title { title: "Title".to_owned() }, title).show(ui));
        in_region(ui, region(1), |ui| MemoView::new("level", Level { level: &self.level }, level).show(ui));
        in_region(ui, region(2), |ui| MemoView::new("asking", Asking { asks: self.asks }, asking).show(ui));
    }

    /// One frame with `events`; returns the views that ran in any of its passes, by props type name, measuring runs
    /// included.
    fn frame(&mut self, frames: &mut Frames, events: Vec<egui::Event>) -> Vec<&'static str> {
        frames.frame(events, |ui| self.show(ui));
        let ran = last_frame_memo_runs(&frames.ctx).into_iter().filter(|(_, _, run)| *run == MemoRun::Ran);
        ran.map(|(name, _, _)| short_name(name)).collect()
    }

    fn settle(&mut self, frames: &mut Frames) {
        for _ in 0..SETTLE {
            self.frame(frames, vec![]);
        }
    }
}

/// An app repaint request outside every view runs none of them: every view replays in that frame and the next. What
/// the app changed reaches a view through its props.
#[trial]
fn an_app_repaint_outside_every_view_leaves_every_view_replaying() {
    let mut scene = Repaints::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    scene.app_asks = true;
    assert_eq!(scene.frame(&mut frames, vec![]), Vec::<&str>::new(), "the frame the app asks in runs no view");
    let replayed = last_pass_memo_runs(&frames.ctx).iter().filter(|(_, _, run)| *run == MemoRun::Replayed).count();
    assert_eq!(replayed, 3, "every view replays");
    scene.app_asks = false;
    assert_eq!(scene.frame(&mut frames, vec![]), Vec::<&str>::new(), "nor does the frame after it");
}

/// A repaint asked in a view's run runs that view again in the next frame, and only it; once it stops asking, it runs
/// once more for the props change and then replays.
#[trial]
fn a_repaint_asked_inside_a_view_runs_only_that_view() {
    let mut scene = Repaints::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    scene.asks = true;
    for frame in 0..4 {
        assert_eq!(scene.frame(&mut frames, vec![]), ["Asking"], "frame {frame}: only the view that asks runs");
    }
    scene.asks = false;
    assert_eq!(scene.frame(&mut frames, vec![]), ["Asking"], "the view runs for its props change");
    assert_eq!(scene.frame(&mut frames, vec![]), Vec::<&str>::new(), "and then replays");
}

/// A drag of a scrub outside every view that shows the app's level as a given value: the app writes each edit into
/// its `Tracked` level, which runs the view whose props hold it in the same frame; the other views replay.
#[trial]
fn a_given_value_edit_runs_only_the_views_whose_props_hold_it() {
    let mut scene = Repaints::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    let from = Pos2::new(OUTSIDE.left() + 10.0, OUTSIDE.center().y);
    scene.frame(&mut frames, vec![moved(from)]);
    assert_eq!(scene.frame(&mut frames, vec![press(from)]), Vec::<&str>::new(), "the press runs no view");
    let start = *scene.level;
    let mut runs = Vec::new();
    for step in 1..=4 {
        runs.push(scene.frame(&mut frames, vec![moved(from + Vec2::new(12.0 * step as f32, 0.0))]));
    }
    scene.frame(&mut frames, vec![release(from + Vec2::new(48.0, 0.0))]);
    assert_ne!(*scene.level, start, "the drag edited the level");
    assert!(runs.iter().all(|ran| ran.iter().all(|name| *name == "Level")), "only the level's view runs in a drag frame: {runs:?}");
    assert!(runs[1..].iter().all(|ran| !ran.is_empty()), "the level's view runs in each drag frame that edits: {runs:?}");
}

/// A view whose content is measured in a stack; `extra` is read by nothing inside, so a change of it runs the view and
/// leaves the stack's inputs as they were.
#[derive(MemoProps)]
struct Measured {
    extra: u32,
}

fn measured(props: &mut Measured, ui: &mut Ui) -> u32 {
    Stack::new().show(ui, |ui, _| {
        Text::new("A measured line").show(ui);
        Text::new("Another measured line").show(ui);
    });
    props.extra
}

/// The memoized view `measured` with `extra`, in `STACK_ROOM`: the frame's measuring runs.
fn measured_frame(frames: &mut Frames, extra: u32) -> usize {
    frames.frame(vec![], |ui| {
        in_region(ui, STACK_ROOM, |ui| {
            MemoView::new("measured", Measured { extra }, measured).show(ui);
        });
    });
    last_pass_measuring_runs(&frames.ctx).len()
}

/// A view that replays for frames keeps the layout memory its run kept: when it runs again with its content's inputs
/// unchanged, its content is measured as little as in a run right after a run.
#[trial]
fn a_view_replayed_for_frames_keeps_the_layout_memory_it_kept() {
    let mut frames = Frames::new();
    for _ in 0..WARM {
        measured_frame(&mut frames, 0);
    }
    let after_a_run = measured_frame(&mut frames, 1);
    assert_eq!(runs_of(&frames, "Measured"), [MemoRun::Ran], "a new prop runs the view");
    for _ in 0..WARM + 3 {
        measured_frame(&mut frames, 1);
        assert_eq!(runs_of(&frames, "Measured"), [MemoRun::Replayed], "an idle frame replays the view");
    }
    let after_replays = measured_frame(&mut frames, 2);
    assert_eq!(runs_of(&frames, "Measured"), [MemoRun::Ran], "a new prop runs the view");
    assert_eq!(after_replays, after_a_run, "the replays kept the stack's measurements alive");
}

#[derive(MemoProps)]
struct Centred {
    label: String,
}

/// A row that fills its width, around one button.
fn centred(props: &mut Centred, ui: &mut Ui) {
    Row::new().fill_width().show(ui, |ui, _| {
        Button::new(props.label.clone()).show(ui);
    });
}

/// A memoized view in a column that takes its width from its content (beside a sibling that fills a row) lays out its
/// fill-width row as the same content outside a view does: the column stays as narrow as its button, and the sibling
/// takes the rest of the row.
#[trial]
fn a_view_in_a_content_sized_column_lays_out_its_fill_width_row_as_outside_a_view() {
    let shown = |memo: bool| {
        let mut frames = Frames::new();
        let mut rects = (Rect::NOTHING, Rect::NOTHING);
        for _ in 0..WARM {
            frames.frame(vec![], |ui| {
                in_region(ui, OUTSIDE.union(region(0)), |ui| {
                    Row::new().fill().show(ui, |ui, _| {
                        let column = Stack::new().show(ui, |ui, _| {
                            let mut props = Centred { label: "Open".to_owned() };
                            if memo {
                                MemoView::new("centred", props, centred).show(ui);
                            } else {
                                centred(&mut props, ui);
                            }
                        });
                        let rest = Stack::new().fill().show(ui, |ui, _| Button::new("Rest").show(ui));
                        rects = (column.output.rect, rest.output.rect);
                    });
                });
            });
        }
        rects
    };
    let (outside, inside) = (shown(false), shown(true));
    assert!(outside.0.width() < outside.1.width(), "the column outside a view is as narrow as its button: {outside:?}");
    assert_eq!(inside, outside, "the view's column and its sibling lie as without the view");
}
