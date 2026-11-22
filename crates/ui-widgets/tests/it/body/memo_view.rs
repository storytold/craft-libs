//! Memoized views (`MemoView`): a view runs only when its props key or a settled rule changes. Count trials: how views
//! took part in a pass (`last_pass_memo_runs`), as relations.

use crate::values::kit::Scrub;
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{
    Bind, BodyCx, CollectionEvent, Common, Component, Container, ContainerProps, Cx, HeaderRegion, MemoProps, MemoView, Props as _, Role, Show as _,
    ShowCollection as _, ShowContainer as _, Tracked, request_rename,
};
use craft_ui::display::{Elide, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow};
use craft_ui::popover::Popover;
use craft_ui::runtime::env::{BodyMode, RevealEdge, ShellCx, WindowKey};
use craft_ui::runtime::focus::Initial;
use craft_ui::runtime::memory::{SaveKey, set_saved_value};
use craft_ui::runtime::overlay::{Backdrop, Rank};
use craft_ui::runtime::{frame, inspect};
use craft_ui::style::StyleCx;
use craft_ui::testing::access::{NodeKey, layer_root};
use craft_ui::testing::in_audit_run;
use craft_ui::testing::input::key_press;
use craft_ui::testing::layout::{allocate, in_region, measured_block};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{
    Frames, MemoRun, last_frame_memo_runs, last_pass_measuring_runs, last_pass_memo_key_bytes, last_pass_memo_runs, last_pass_runs,
    last_pass_text_layouts, measuring_run, trial,
};
use craft_ui::text::TextField;
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Id, Key, Modifiers, Pos2, Rect, Response, Ui, Vec2};
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

/// A content-sized container whose header shows the memoized view `title`.
#[derive(Component)]
#[component(container)]
struct Headed {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Component for Headed {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: HeadedStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Headed {
    fn header(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let _ = MemoView::new("heading", Title { title: "Heading".to_owned() }, title).show(ui);
    }
}

/// One frame of `Headed` with the body text `label`; returns the frame's passes and how many sizing passes its body ran
/// in.
fn headed_frame(frames: &mut Frames, label: &str) -> (usize, usize) {
    let sizing = Cell::new(0);
    let passes = frames.frame(vec![], |ui| {
        in_region(ui, STACK_ROOM, |ui| {
            let headed = Headed { common: Common::new(), props: ContainerProps::new().header(HeaderRegion::new()) };
            headed.show(ui, |ui, _| {
                if ui.is_sizing_pass() {
                    sizing.set(sizing.get() + 1);
                }
                Text::new(label.to_owned()).show(ui);
            });
        });
    });
    (passes, sizing.get())
}

/// A view in the header of a content-sized container replays when the container measures its regions again: its
/// measuring run lays the header out as a content column and the real run does not, and that is a fact of the place,
/// not of the view's inputs.
#[trial]
fn a_view_in_a_measured_header_replays_in_the_sizing_pass_and_the_real_run() {
    let mut frames = Frames::new();
    for _ in 0..SETTLE {
        headed_frame(&mut frames, "Short");
    }
    title_runs();
    headed_frame(&mut frames, "Short");
    assert_eq!(title_runs(), 0, "settled: an idle frame replays the header's view");
    // A second line, no wider than the first: the container's width, and with it the header's room, stays.
    let (passes, sizing) = headed_frame(&mut frames, "Short\nShort");
    assert!(sizing > 0, "the taller label made the container measure its regions again");
    assert_eq!(passes, 2, "the taller container asks for one discard pass");
    assert_eq!(title_runs(), 0, "the unchanged view replayed in every call: {:?}", last_frame_memo_runs(&frames.ctx));
    let heading = |runs: Vec<(&'static str, Id, MemoRun)>| -> Vec<MemoRun> {
        runs.into_iter().filter(|(name, _, _)| name.ends_with("::Title")).map(|(_, _, run)| run).collect()
    };
    assert_eq!(heading(last_pass_memo_runs(&frames.ctx)), [MemoRun::Replayed], "the discard pass's real run replayed the view");
    assert_eq!(
        heading(last_frame_memo_runs(&frames.ctx)),
        [MemoRun::Replayed; 3],
        "the view was called and replayed in the sizing pass and in the real run of each of the two passes"
    );
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

/// A view as large as the region `COVERED` gives it, under the popover of the covered trial.
#[derive(MemoProps)]
struct Board {
    title: String,
}

fn board(props: &mut Board, ui: &mut Ui) {
    Button::new(props.title.clone()).show(ui);
    allocate(ui, Vec2::new(300.0, 200.0));
}

/// Where the board view is: the popover hangs over its middle.
const COVERED: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(410.0, 260.0) };

/// A board view with an open popover over it whose slider edits a value the board does not show.
struct Covered {
    size: f32,
    /// Where the slider in the popover was shown in the last frame.
    slider: Option<Rect>,
}

impl Covered {
    fn show(&mut self, ui: &mut Ui) {
        in_region(ui, COVERED, |ui| MemoView::new("board", Board { title: "Board".to_owned() }, board).show(ui));
        let (size, slider) = (&mut self.size, &mut self.slider);
        Popover::new("Size", Bind::given(true)).at_point(Pos2::new(80.0, 80.0)).show(ui, |ui, _| {
            *slider = Some(Scrub::new(Bind::bound(size)).show(ui).response.rect);
        });
    }

    /// One frame with `events`; returns the views that ran in any of its passes, by props type name.
    fn frame(&mut self, frames: &mut Frames, events: Vec<egui::Event>) -> Vec<&'static str> {
        frames.frame(events, |ui| self.show(ui));
        let ran = last_frame_memo_runs(&frames.ctx).into_iter().filter(|(_, _, run)| *run == MemoRun::Ran);
        ran.map(|(name, _, _)| short_name(name)).collect()
    }
}

/// A drag of a slider on a popover over a memoized view leaves the view replaying: the press is on the popover, the
/// top layer where it started, so it engages nothing beneath it.
#[trial]
fn a_drag_on_a_popover_over_a_view_replays_the_view() {
    let mut scene = Covered { size: 10.0, slider: None };
    let mut frames = Frames::new();
    for _ in 0..SETTLE {
        scene.frame(&mut frames, vec![]);
    }
    let slider = scene.slider.expect("the popover shows its slider");
    let from = Pos2::new(slider.left() + 10.0, slider.center().y);
    assert!(COVERED.contains(from), "the slider lies over the board");
    scene.frame(&mut frames, vec![moved(from)]);
    assert_eq!(scene.frame(&mut frames, vec![press(from)]), Vec::<&str>::new(), "the press runs no view");
    let start = scene.size;
    let mut runs = Vec::new();
    for step in 1..=4 {
        runs.push(scene.frame(&mut frames, vec![moved(from + Vec2::new(12.0 * step as f32, 0.0))]));
    }
    scene.frame(&mut frames, vec![release(from + Vec2::new(48.0, 0.0))]);
    assert_ne!(scene.size, start, "the drag edited the size");
    assert!(runs.iter().all(Vec::is_empty), "the board replays in every drag frame: {runs:?}");
}

/// A view inside an open popover, after a button: the popover's body is on its own layer.
struct InPopover {
    /// Where the view's button was shown in the last frame.
    button: Option<Rect>,
}

impl InPopover {
    fn frame(&mut self, frames: &mut Frames, events: Vec<egui::Event>) -> Vec<&'static str> {
        let button = &mut self.button;
        frames.frame(events, |ui| {
            Popover::new("Presets", Bind::given(true)).at_point(Pos2::new(80.0, 80.0)).show(ui, |ui, _| {
                // The popover's first stop, outside the view, takes the focus on open.
                Button::new("Search").show(ui);
                *button = MemoView::new("presets", Title { title: "Inside".to_owned() }, title).show(ui);
            });
        });
        let ran = last_frame_memo_runs(&frames.ctx).into_iter().filter(|(_, _, run)| *run == MemoRun::Ran);
        ran.map(|(name, _, _)| short_name(name)).collect()
    }
}

/// A memoized view inside an open popover replays while nothing changes, still painted, and runs when the pointer
/// moves over it.
#[trial]
fn a_view_in_an_open_popover_replays_while_nothing_changes() {
    let mut scene = InPopover { button: None };
    let mut frames = Frames::new();
    for _ in 0..SETTLE {
        scene.frame(&mut frames, vec![]);
    }
    let idle: Vec<Vec<&str>> = (0..WARM).map(|_| scene.frame(&mut frames, vec![])).collect();
    assert!(idle.iter().all(Vec::is_empty), "the view in the popover ran in idle frames: {idle:?}");
    assert_eq!(runs_of(&frames, "Title"), [MemoRun::Replayed], "{:?}", last_pass_memo_runs(&frames.ctx));
    assert!(painted(&frames, "Inside"), "the replayed view is painted");
    let over = scene.button.expect("the popover shows the view").center();
    assert_eq!(scene.frame(&mut frames, vec![moved(over)]), ["Title"], "the pointer over the view runs it");
}

/// The id of the list in [`rows`], which a rename request names.
fn rows_id() -> Id {
    Id::NULL.with("memo_rows")
}

#[derive(MemoProps)]
struct Rows {}

/// A renamable list of three rows; returns how many renames it began.
fn rows(_: &mut Rows, ui: &mut Ui) -> usize {
    let keys = [0_usize, 1, 2];
    let source = craft_behavior::keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Owned(format!("Row {key}")));
    let out = List::new("Rows").id(rows_id()).rename(true).show_items(ui, &source, &mut |key, _| ListRow::new(format!("Row {key}")));
    out.events.iter().filter(|event| matches!(event, CollectionEvent::Rename(craft_behavior::EditEvent::Begin { .. }))).count()
}

/// A rename request from outside runs the view whose list takes it, once; without a request the view replays.
#[trial]
fn a_rename_request_runs_the_view_that_takes_it_once() {
    let mut began = 0;
    let mut frame = |frames: &mut Frames| {
        frames.frame(vec![], |ui| began = MemoView::new("rows", Rows {}, rows).show(ui).unwrap_or(0));
        let ran = last_frame_memo_runs(&frames.ctx).into_iter().filter(|(_, _, run)| *run == MemoRun::Ran);
        ran.map(|(name, _, _)| short_name(name)).collect::<Vec<_>>()
    };
    let mut frames = Frames::new();
    for _ in 0..SETTLE {
        frame(&mut frames);
    }
    assert!(frame(&mut frames).is_empty(), "with no request the view replays");
    request_rename(&frames.ctx, rows_id(), &7_usize);
    assert_eq!(frame(&mut frames), ["Rows"], "a request runs the view that takes it");
    assert!(frame(&mut frames).is_empty(), "a taken request runs it no more");
    request_rename(&frames.ctx, rows_id(), &1_usize);
    frame(&mut frames);
    drop(frame);
    assert_eq!(began, 1, "the taken request opens the rename field");
}

#[derive(MemoProps)]
struct Filling {}

/// A button over a rule that fills its width.
fn filling(_: &mut Filling, ui: &mut Ui) {
    Button::new("Inside").show(ui);
    craft_ui::display::Divider::horizontal().show(ui);
}

/// The width of an open popover at a point holding a stack with a button, under a header with a button over a rule
/// that fills its width, the header's content shown plainly or in a memoized view.
fn popover_width(memo: bool) -> f32 {
    let mut fixture = craft_ui::testing::Fixture::new(move |ui| {
        Popover::new("Filling", Bind::given(true)).at_point(Pos2::new(40.0, 40.0)).collection_body().show(ui, |ui, _| {
            Stack::new()
                .fill()
                .header(|ui, _| {
                    if memo {
                        let _ = MemoView::new("filling", Filling {}, filling).show(ui);
                    } else {
                        filling(&mut Filling {}, ui);
                    }
                })
                .show(ui, |ui, _| Button::new("Search").show(ui));
        });
    });
    fixture.room(Vec2::new(600.0, 400.0));
    fixture.settle();
    fixture.by_role(craft_ui::contract::Role::DIALOG).first().map(|target| target.rect.width()).expect("the popover shows")
}

/// A popover sizes from its content: a rule that fills its width, in the header of a stack held in the popover's
/// content-sized column, makes it as wide inside a memoized view as shown plainly, never as wide as the open room.
#[trial]
fn a_view_in_a_popover_fills_its_width_as_without_the_view() {
    let (plain, memo) = (popover_width(false), popover_width(true));
    assert!(plain > 0.0 && (memo - plain).abs() <= 0.5, "the popover is as wide with the view: {memo} against {plain}");
}

/// Where the popovers of the overlay trials open, and a point outside them.
const POPOVER_AT: Pos2 = Pos2::new(60.0, 60.0);
const AWAY: Pos2 = Pos2::new(700.0, 500.0);

/// Frames with motion off: a layer opens and closes in one frame, with no fade that runs its view.
fn still_frames() -> Frames {
    let mut frames = Frames::new();
    frames.root(craft_ui::runtime::env::RootEnv::default().motion(craft_behavior::MotionMode::Off));
    frames
}

/// A popover the view shows, holding a line of text and a button; the focus stays where it was, so nothing in it holds
/// the focus.
#[derive(MemoProps)]
struct Hint<'a> {
    open: Bind<'a, bool>,
}

/// Returns where the button in the popover was shown.
fn hint(props: &mut Hint<'_>, ui: &mut Ui) -> Option<Rect> {
    let mut row = None;
    Popover::new("Hint", props.open.reborrowed()).at_point(POPOVER_AT).initial_focus(Initial::KeepAnchor).show(ui, |ui, _| {
        Text::new("A hint").show(ui);
        row = Some(Button::new("Row").show(ui).response.rect);
    });
    row
}

/// The hint view, with whether its popover is open and where its row was shown.
struct HintScene {
    open: bool,
    row: Option<Rect>,
}

impl HintScene {
    /// One frame with `events`; returns the views that ran in any of its passes, by props type name.
    fn frame(&mut self, frames: &mut Frames, events: Vec<egui::Event>) -> Vec<&'static str> {
        let (open, row) = (&mut self.open, &mut self.row);
        frames.frame(events, |ui| *row = MemoView::new("hint", Hint { open: Bind::bound(open) }, hint).show(ui).flatten());
        let ran = last_frame_memo_runs(&frames.ctx).into_iter().filter(|(_, _, run)| *run == MemoRun::Ran);
        ran.map(|(name, _, _)| short_name(name)).collect()
    }

    /// An open popover after the frames that settle it. The scene stays where it is made: its binding's place is part
    /// of the view's key.
    fn settle(&mut self, frames: &mut Frames) {
        for _ in 0..SETTLE {
            self.frame(frames, vec![]);
        }
    }
}

fn any_open(frames: &Frames) -> bool {
    craft_ui::runtime::overlay::any_open(&frames.ctx, craft_ui::runtime::overlay::OpenFilter::Any)
}

/// A view that shows an open popover with no focus in it replays the popover while nothing reaches it: no component
/// runs, the layer stays open and on top under its row, and its AccessKit node keeps its id and parent.
#[trial]
fn an_open_popover_without_focus_replays_with_its_layer() {
    let mut frames = still_frames();
    let mut scene = HintScene { open: true, row: None };
    scene.settle(&mut frames);
    let row = scene.row.expect("the popover shows its row");
    let layer = frames.ctx.layer_id_at(row.center()).expect("the popover's layer is under its row");
    let dialog = |frames: &mut Frames| {
        let tree = frames.tree();
        let node = *tree.with_role(craft_ui::contract::Role::DIALOG).first().expect("the popover reports its node");
        (node, tree.parent_of_key(node))
    };
    let ran_with = dialog(&mut frames);
    for _ in 0..WARM {
        assert!(scene.frame(&mut frames, vec![]).is_empty(), "an idle frame replays the view");
        assert!(last_pass_runs(&frames.ctx).is_empty(), "no component runs: {:?}", last_pass_runs(&frames.ctx));
        assert!(any_open(&frames), "the replayed popover stays open");
        assert_eq!(frames.ctx.layer_id_at(row.center()), Some(layer), "the popover's layer is still on top under its row");
        assert_eq!(dialog(&mut frames), ran_with, "the popover's node keeps its id and parent");
    }
    assert!(painted(&frames, "A hint"), "the replayed popover is painted");
}

/// The pointer onto the popover's row runs the view in that frame.
#[trial]
fn the_pointer_over_a_replayed_popover_runs_its_view() {
    let mut frames = still_frames();
    let mut scene = HintScene { open: true, row: None };
    scene.settle(&mut frames);
    let row = scene.row.expect("the popover shows its row").center();
    assert_eq!(scene.frame(&mut frames, vec![moved(row)]), ["Hint"], "the pointer onto the row runs the view");
}

/// An outside press closes a replaying popover in the press frame; the view runs at most twice more and then replays
/// with no layer open.
#[trial]
fn an_outside_press_closes_a_replaying_popover() {
    let mut frames = still_frames();
    let mut scene = HintScene { open: true, row: None };
    scene.settle(&mut frames);
    scene.frame(&mut frames, vec![moved(AWAY)]);
    assert!(scene.frame(&mut frames, vec![]).is_empty(), "the view replays with the pointer away");
    assert_eq!(scene.frame(&mut frames, vec![press(AWAY), release(AWAY)]), ["Hint"], "the press runs the view");
    assert!(!scene.open, "the outside press closed the popover in its frame");
    // The first run after the close finds the layer's presence still open and settles it; the next one shows nothing.
    let after: Vec<Vec<&str>> = (0..=WARM).map(|_| scene.frame(&mut frames, vec![])).collect();
    assert!(after.iter().skip(2).all(Vec::is_empty), "the view runs at most twice after the close: {after:?}");
    assert!(!any_open(&frames), "no layer is open");
}

/// A close of every popup from outside the view (a command's) closes a replaying popover in the next frame.
#[trial]
fn closing_the_popups_closes_a_replaying_popover() {
    let mut frames = still_frames();
    let mut scene = HintScene { open: true, row: None };
    scene.settle(&mut frames);
    craft_ui::runtime::overlay::close_popups(&frames.ctx);
    assert_eq!(scene.frame(&mut frames, vec![]), ["Hint"], "the queued close runs the view");
    assert!(!scene.open, "the close reached the popover");
}

/// A window resize runs a view that shows a popover once, and the popover stays inside the new window; then it replays.
#[trial]
fn a_window_resize_runs_a_popover_view_once() {
    let mut frames = still_frames();
    let mut scene = HintScene { open: true, row: None };
    scene.settle(&mut frames);
    frames.screen(Vec2::new(1000.0, 700.0));
    let resized: Vec<Vec<&str>> = (0..WARM).map(|_| scene.frame(&mut frames, vec![])).collect();
    assert_eq!(resized.iter().filter(|ran| !ran.is_empty()).count(), 1, "the resize runs the view once: {resized:?}");
    assert!(resized[0] == ["Hint"], "it runs in the resize frame: {resized:?}");
    let row = scene.row.expect("the popover shows its row");
    assert!(frames.ctx.content_rect().contains_rect(row), "the popover stays inside the window");
}

/// A popover the view shows with a text field in it, which takes the focus when it opens.
#[derive(MemoProps)]
struct Typing<'a> {
    text: Bind<'a, String>,
}

fn typing(props: &mut Typing<'_>, ui: &mut Ui) {
    Popover::new("Typing", Bind::given(true)).at_point(POPOVER_AT).show(ui, |ui, _| {
        TextField::new("Name", props.text.reborrowed()).id_salt("typing").show(ui);
    });
}

/// With the focus in a replaying popover's text field, a typed character, a paste and an IME commit each reach the
/// field in their frame, and an idle frame after each replays the view.
#[trial]
fn events_sent_to_a_replaying_focused_field_reach_it_in_their_frame() {
    let mut frames = still_frames();
    let mut text = String::new();
    let frame = |frames: &mut Frames, events: Vec<egui::Event>, text: &mut String| {
        frames.frame(events, |ui| {
            MemoView::new("typing", Typing { text: Bind::bound(text) }, typing).show(ui);
        });
        let ran = last_frame_memo_runs(&frames.ctx).into_iter().filter(|(_, _, run)| *run == MemoRun::Ran);
        ran.map(|(name, _, _)| short_name(name)).collect::<Vec<_>>()
    };
    for _ in 0..SETTLE {
        frame(&mut frames, vec![], &mut text);
    }
    assert!(frame(&mut frames, vec![], &mut text).is_empty(), "an idle frame replays the view with the focus in its field");
    assert!(last_pass_runs(&frames.ctx).is_empty(), "no component runs: {:?}", last_pass_runs(&frames.ctx));
    let sent = [
        (craft_ui::testing::input::text("a"), "a"),
        (egui::Event::Paste("b".to_owned()), "ab"),
        (egui::Event::Ime(egui::ImeEvent::Commit("c".to_owned())), "abc"),
    ];
    for (event, after) in sent {
        let ran = frame(&mut frames, vec![event], &mut text);
        assert_eq!(text, after, "the event reached the field in its frame");
        assert!(ran.contains(&"Typing"), "the event ran the view: {ran:?}");
        frame(&mut frames, vec![], &mut text);
        assert!(frame(&mut frames, vec![], &mut text).is_empty(), "an idle frame after it replays the view");
    }
}

/// A popover the rooted view shows, open, and whether its layer is modal.
#[derive(MemoProps)]
struct Rooted<'a> {
    open: Bind<'a, bool>,
    modal: bool,
}

/// A popover the view shows, holding a line of text; returns the popover's id.
fn rooted(props: &mut Rooted<'_>, ui: &mut Ui) -> Option<Id> {
    let modal = props.modal;
    let popover = Popover::new("Rooted", props.open.reborrowed()).at_point(POPOVER_AT).initial_focus(Initial::KeepAnchor).overlay(|mut overlay| {
        if modal {
            overlay.rank = Rank::modal(Backdrop::None);
        }
        overlay
    });
    let shown = popover.show(ui, |ui, _| Text::new("Rooted").show(ui));
    shown.output.open.then_some(shown.output.response.id)
}

/// The nodes above `key` in `tree`, nearest first.
fn ancestors(tree: &craft_ui::testing::access::AccessTree, key: NodeKey) -> Vec<NodeKey> {
    let keys = tree.keys();
    let mut above = Vec::new();
    let mut at = key;
    while let Some(parent) = keys.iter().copied().find(|parent| tree.children_of(*parent).contains(&at)) {
        above.push(parent);
        at = parent;
    }
    above
}

/// A popover in a view hangs below its layer's root node, which has its own id, in the run and in each replay: the
/// pass's AccessKit parents hold no cycle (every trial's pass-end check), and a replay adds the nodes where the run did.
/// A modal layer's flag sits on the popover's own node, which carries its role, never on the root, a plain container.
#[trial]
fn a_popover_in_a_view_hangs_below_its_layer_root() {
    for modal in [false, true] {
        let mut frames = still_frames();
        let mut open = true;
        let mut popover = None;
        let mut frame = |frames: &mut Frames| {
            frames.frame(vec![], |ui| popover = MemoView::new("rooted", Rooted { open: Bind::bound(&mut open), modal }, rooted).show(ui).flatten());
            let ran = last_frame_memo_runs(&frames.ctx).into_iter().any(|(_, _, run)| run == MemoRun::Ran);
            let id = popover.expect("the popover is open");
            let tree = frames.tree();
            let (node, root) = (NodeKey::of(id), NodeKey::of(layer_root(id)));
            (ran, ancestors(&tree, node), root, (tree.is_modal(node), tree.is_modal(root)))
        };
        let (ran, above, root, flags) = frame(&mut frames);
        assert!(ran, "the first frame runs the view");
        assert!(above.contains(&root), "the popover hangs below its layer's root: {above:?}, root {root:?}");
        assert_eq!(flags, (modal, false), "the modal flag sits on the popover's node only (modal {modal})");
        for _ in 0..SETTLE {
            frame(&mut frames);
        }
        let (ran, replayed, _, flags) = frame(&mut frames);
        assert!(!ran, "an idle frame replays the view (modal {modal})");
        assert_eq!(replayed, above, "the replay adds the popover's node where the run did");
        assert_eq!(flags, (modal, false), "the replay keeps the modal flag on the popover's node (modal {modal})");
    }
}

/// What the toast view shows: the main window's toasts, over the view's room.
#[derive(MemoProps)]
struct Notices {
    corner: bool,
}

fn notices(_: &mut Notices, ui: &mut Ui) {
    let body = ui.max_rect();
    craft_ui::notices::ToastStack::show(ui, &WindowKey::main(), body);
}

/// A toast posted to a view that shows the toasts runs the view in the next frame, which opens the toast layer; then
/// the view replays the layer, and the layer stack and egui's layer order hold.
#[trial]
fn a_toast_opening_runs_its_view_in_its_frame_then_replays() {
    let mut frames = still_frames();
    let frame = |frames: &mut Frames| {
        frames.frame(vec![], |ui| {
            MemoView::new("notices", Notices { corner: true }, notices).show(ui);
        });
        last_frame_memo_runs(&frames.ctx).into_iter().filter(|(_, _, run)| *run == MemoRun::Ran).count()
    };
    for _ in 0..SETTLE {
        frame(&mut frames);
    }
    assert_eq!(frame(&mut frames), 0, "an idle frame with no toast replays the view");
    craft_ui::runtime::toast::post(&frames.ctx, &WindowKey::main(), craft_ui::runtime::toast::Toast::new("Saved").timeout(None));
    let runs: Vec<usize> = (0..=WARM).map(|_| frame(&mut frames)).collect();
    // The run that opens the toast asks for the discard pass that places the layer by its size, which runs it again.
    assert!(runs[0] > 0, "the post runs the view in the next frame: {runs:?}");
    assert!(runs[1..].iter().all(|ran| *ran == 0), "then the view replays: {runs:?}");
    assert!(painted(&frames, "Saved"), "the replayed toast is painted");
    let stack = |frames: &Frames| (craft_ui::runtime::overlay::top(&frames.ctx), frames.ctx.memory(|memory| memory.layer_ids().collect::<Vec<_>>()));
    let held = stack(&frames);
    assert!(held.0.is_some(), "the toast layer is open");
    for _ in 0..WARM {
        assert_eq!(frame(&mut frames), 0, "an idle frame replays the view");
        assert_eq!(stack(&frames), held, "the layer stack and egui's layer order hold");
    }
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

#[derive(MemoProps)]
struct Line {
    line: String,
}

/// One line of text cut with an ellipsis where its room is short.
fn line(props: &mut Line, ui: &mut Ui) -> Rect {
    Text::new(props.line.clone()).elide(Elide::End).show(ui).rect
}

/// A memoized view is one unit of the shrinking row around it: with room it is as wide as its content, and when a
/// sibling after it grows it narrows so the row fits, though the region it is offered stays the same. It replays while
/// the row's width for it is unchanged, in the row's measuring sweeps too, and runs once when that width changes.
#[trial]
fn a_view_in_a_row_narrows_with_it_and_runs_only_when_its_width_changes() {
    const ROOM: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(310.0, 50.0) };
    let mut frames = Frames::new();
    let frame = |frames: &mut Frames, after: &str| {
        let mut rects = (Rect::NOTHING, Rect::NOTHING);
        frames.frame(vec![], |ui| {
            in_region(ui, ROOM, |ui| {
                Row::new().fill_width().show(ui, |ui, _| {
                    let view = MemoView::new("line", Line { line: "A line of text long enough to be cut".to_owned() }, line).show(ui);
                    let after = Button::new(after.to_owned()).show(ui).response.rect;
                    rects = (view.unwrap_or(Rect::NOTHING), after);
                });
            });
        });
        rects
    };
    for _ in 0..WARM {
        frame(&mut frames, "B");
    }
    let (wide, _) = frame(&mut frames, "B");
    assert_eq!(runs_of(&frames, "Line"), [MemoRun::Replayed], "an idle frame replays the view");
    assert_eq!(measuring_runs_of(&frames, "Line"), 0, "an idle frame replays the view in the row's measuring sweeps too");
    let (narrow, after) = frame(&mut frames, "A much longer button label");
    assert_eq!(runs_of(&frames, "Line"), [MemoRun::Ran], "a new width runs the view");
    assert!(narrow.width() < wide.width(), "the view narrows: {narrow:?} from {wide:?}");
    assert!(narrow.right() <= after.left() && after.right() <= ROOM.right() + 0.5, "the row fits: {narrow:?}, {after:?} in {ROOM:?}");
    for _ in 0..WARM {
        frame(&mut frames, "A much longer button label");
    }
    assert_eq!(runs_of(&frames, "Line"), [MemoRun::Replayed], "at the same width it replays again");
    assert_eq!(measuring_runs_of(&frames, "Line"), 0, "at the same width the view replays in the row's measuring sweeps too");
}

/// How many times the views named `name` ran in the last frame outside the real run of its last pass: in sizing and
/// measuring runs.
fn measuring_runs_of(frames: &Frames, name: &str) -> usize {
    let ran =
        |runs: Vec<(&'static str, egui::Id, MemoRun)>| runs.into_iter().filter(|(of, _, run)| of.contains(name) && *run == MemoRun::Ran).count();
    ran(last_frame_memo_runs(&frames.ctx)) - ran(last_pass_memo_runs(&frames.ctx))
}
