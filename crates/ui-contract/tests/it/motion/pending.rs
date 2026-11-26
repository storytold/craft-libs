//! `Remember::pending`: one submission per key, a result that lands on the next pass, one repaint.

use craft_behavior::Sense;
use craft_host::{AppInfo, Host, Jobs, ManualJobs};
use craft_ui::contract::{Common, Component, Cx, Job, Pending, Remember as _, Role, Show as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::memory::{Stage, global};
use craft_ui::testing::{Frames, measuring_run, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::Response;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Shows the pending result of a job that reports half done, then returns 7.
#[derive(Component)]
struct Thumbnail<'a> {
    #[common]
    common: Common,
    seen: &'a RefCell<Vec<Pending<u32, String>>>,
}

impl Component for Thumbnail<'_> {
    const ROLE: Role = Role::None;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ThumbnailStyleKey) -> PlainStyle {
        PlainStyle::min(egui::vec2(40.0, 20.0))
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let result = cx.pending("thumb", || {
            Job::new(|progress, _| {
                progress.report(0.5);
                Ok::<u32, String>(7)
            })
        });
        self.seen.borrow_mut().push(result.clone());
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn a_pending_result_arrives_on_the_next_pass_and_repaints_once() {
    let jobs = Arc::new(ManualJobs::new());
    let host = Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs.clone()).build());
    let seen = RefCell::new(Vec::new());
    let mut frames = Frames::new();
    frames.host(host);
    let mut show = |ui: &mut egui::Ui| {
        Thumbnail { common: Common::new(), seen: &seen }.show(ui);
    };
    frames.idle(3, &mut show);
    assert_eq!(jobs.waiting(), 1, "submitted once across frames");
    assert!(seen.borrow().iter().all(|p| matches!(p, Pending::Loading { progress: None, .. })), "loading until the job runs");
    assert!(!frames.repaint_requested(), "a loading result asks for nothing");
    jobs.run_next();
    assert!(frames.repaint_requested(), "the landed result asks for a pass");
    seen.borrow_mut().clear();
    frames.frame(Vec::new(), &mut show);
    assert_eq!(seen.borrow().last(), Some(&Pending::Ready(7)), "the next pass shows the result");
    frames.frame(Vec::new(), &mut show);
    assert!(!frames.repaint_requested(), "after egui's own follow-up pass nothing asks for more");
    frames.idle(2, &mut show);
    assert_eq!(jobs.waiting(), 0, "a landed result is not submitted again");
    assert_eq!(seen.borrow().last(), Some(&Pending::Ready(7)), "it stays");
}

fn host_over(jobs: Arc<dyn Jobs>) -> Arc<Host> {
    Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs).build())
}

/// The pending result of `"k"` under a stage of `id`, opened as a real pass or as the peek stage of a measuring run.
fn poll_stage(ui: &egui::Ui, sizing: bool, attempts: &Arc<AtomicUsize>, panic_first: bool) -> Pending<u32, String> {
    let id = global("pending.trial", "owner");
    let poll = || {
        let mut stage = if sizing { Stage::open_peek(ui.ctx(), id) } else { Stage::open(ui.ctx(), id) };
        let attempts = attempts.clone();
        let result = stage
            .pending("k", move || {
                let attempt = attempts.fetch_add(1, Ordering::Relaxed);
                Job::new(move |_, _| {
                    assert!(!(panic_first && attempt == 0), "the first attempt panics");
                    Ok::<u32, String>(7)
                })
            })
            .clone();
        stage.finish();
        result
    };
    if sizing { measuring_run(ui.ctx(), poll) } else { poll() }
}

fn polled(frames: &mut Frames, sizing: bool, attempts: &Arc<AtomicUsize>, panic_first: bool) -> Pending<u32, String> {
    let out = RefCell::new(Pending::default());
    frames.frame(Vec::new(), |ui| *out.borrow_mut() = poll_stage(ui, sizing, attempts, panic_first));
    out.into_inner()
}

// A measuring run between a result landing and the next real pass reads the result without taking it, and touches no
// service outside layout memory (the fixture's guard).
#[trial]
fn a_measuring_run_leaves_a_landed_result_for_the_next_real_pass() {
    let jobs = Arc::new(ManualJobs::new());
    let mut frames = Frames::new();
    frames.host(host_over(jobs.clone()));
    let attempts = Arc::new(AtomicUsize::new(0));
    polled(&mut frames, false, &attempts, false);
    jobs.run_all();
    polled(&mut frames, true, &attempts, false);
    assert_eq!(polled(&mut frames, false, &attempts, false), Pending::Ready(7));
    assert_eq!(attempts.load(Ordering::Relaxed), 1, "submitted once");
}
