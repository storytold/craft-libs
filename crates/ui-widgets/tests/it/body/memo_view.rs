//! Memoized views (`MemoView`): a view runs only when its props key or a settled rule changes. Count trials: how views
//! took part in a pass (`last_pass_memo_runs`), as relations.

use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, MemoProps, MemoView, Show as _, Tracked};
use craft_ui::runtime::inspect;
use craft_ui::runtime::memory::{SaveKey, set_saved_value};
use craft_ui::testing::input::key_press;
use craft_ui::testing::layout::{in_region, measured_block};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{
    Frames, MemoRun, last_pass_memo_key_bytes, last_pass_memo_runs, last_pass_runs, last_pass_text_layouts, measuring_run, trial,
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

fn title(props: &mut Title, ui: &mut Ui) -> Vec2 {
    TITLE_RUNS.with(|runs| runs.set(runs.get() + 1));
    Button::new(props.title.clone()).show(ui).response.rect.size()
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
    out: (Option<Vec2>, Option<usize>, Option<u32>, Option<f32>),
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
    let over = region(0).min + scene.out.0.unwrap_or_default() / 2.0;
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
    TITLE_RUNS.with(|runs| runs.set(runs.get() + 1));
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
