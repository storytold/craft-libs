//! The replay audit (`CRAFT_UI_AUDIT_REPLAY=1`, `audit_replays`): a memoized view that would replay runs for real and
//! is compared with the replay. A stale view fails the frame with its name; honest views pass, and the audit changes
//! no count.

use craft_ui::buttons::Button;
use craft_ui::contract::{MemoProps, MemoView, Show as _};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{
    Frames, MemoRun, audit_replays, in_audit_run, last_frame_memo_runs, last_pass_memo_runs, last_pass_runs, last_pass_text_layouts, trial,
};
use egui::{Color32, Pos2, Rect, Ui, Vec2};
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
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
    assert!(!message.contains("rect:") && !message.contains("output:"), "extent and output are the same: {message}");
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
