//! `runtime::memory::fork_into`: a second context (a headless capture of a window) gets the window's UI memory, its
//! components' scroll offsets included, and keeps a runtime of its own: it runs passes inside the window's, and what it
//! does stays its own.

use craft_host::{AppInfo, Host, Jobs, ManualJobs};
use craft_ui::contract::{ContainerProps, Job, Pending, Props as _, Remember as _, Scroll, Show as _, ShowContainer as _};
use craft_ui::runtime::memory::{ForkError, fork_into};
use craft_ui::testing::layout::{Block, in_region};
use craft_ui::testing::memory::{Stage, global};
use craft_ui::testing::{Frames, pointer, trial};
use craft_ui_test_kit::body::Slab;
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

const VIEW: Vec2 = Vec2::new(300.0, 200.0);
const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const ROWS: usize = 40;
const ROW_H: f32 = 20.0;

/// A scrolling list of rows; `top` is where its first row showed.
fn list(ui: &mut Ui, top: &Cell<f32>) {
    in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
        Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("list").show(ui, |ui, _| {
            for row in 0..ROWS {
                let rect = Block::new(Vec2::new(VIEW.x, ROW_H)).show(ui).rect;
                if row == 0 {
                    top.set(rect.min.y);
                }
            }
        });
    });
}

fn scroll(frames: &mut Frames, top: &Cell<f32>, by: f32) {
    frames.frame(vec![pointer::moved(ORIGIN + VIEW / 2.0)], |ui| list(ui, top));
    frames.frame(vec![pointer::wheel(by)], |ui| list(ui, top));
    frames.idle(3, |ui| list(ui, top));
}

#[trial]
fn a_forked_context_shows_the_windows_scroll_offset_and_keeps_a_runtime_of_its_own() {
    let (window_top, shadow_top) = (Cell::new(0.0), Cell::new(0.0));
    let mut window = Frames::new();
    let mut shadow = Frames::new();
    window.idle(3, |ui| list(ui, &window_top));
    let unscrolled = window_top.get();
    scroll(&mut window, &window_top, -120.0);
    let scrolled = window_top.get();
    assert!(scrolled < unscrolled - 1.0, "the window's list scrolled: {unscrolled} to {scrolled}");

    fork_into(&window.ctx, &shadow.ctx).expect("the shadow runs no pass");
    // A headless capture draws the shadow inside one of the window's passes.
    window.frame(vec![], |ui| {
        list(ui, &window_top);
        shadow.frame(vec![], |ui| list(ui, &shadow_top));
    });
    assert!((shadow_top.get() - scrolled).abs() < 0.5, "the shadow shows the window's offset: {} for {scrolled}", shadow_top.get());

    // What the shadow does stays its own.
    scroll(&mut shadow, &shadow_top, -120.0);
    window.idle(2, |ui| list(ui, &window_top));
    assert!(shadow_top.get() < scrolled - 1.0, "the shadow scrolled further: {}", shadow_top.get());
    assert!((window_top.get() - scrolled).abs() < 0.5, "the window keeps its offset: {} for {scrolled}", window_top.get());
}

#[trial]
fn fork_into_a_context_running_a_pass_is_refused() {
    let window = Frames::new();
    let mut shadow = Frames::new();
    let refused = std::cell::RefCell::new(None);
    shadow.frame(vec![], |ui| {
        *refused.borrow_mut() = Some(fork_into(&window.ctx, ui.ctx()));
    });
    assert_eq!(refused.into_inner(), Some(Err(ForkError::DuringPass)));
}

fn host_over(jobs: Arc<dyn Jobs>) -> Arc<Host> {
    Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs).build())
}

/// The pending result of `"k"` under `id`'s stage, polled in a real pass. The job's answer is `answer`, so a slot of
/// another result type is a job of its own.
fn poll_type<T, E>(ui: &Ui, id: egui::Id, answer: T) -> Pending<T, E>
where
    T: Clone + Default + Send + Sync + 'static,
    E: Clone + Default + Send + Sync + 'static,
{
    let mut stage = Stage::open(ui.ctx(), id);
    let result = stage.pending("k", move || Job::new(move |_, _| Ok(answer))).clone();
    stage.finish();
    result
}

#[trial]
fn a_copied_pending_slot_takes_no_landed_result_from_the_context_it_was_forked_from() {
    let jobs = Arc::new(ManualJobs::new());
    let (first, second) = (global("fork.pending", "first"), global("fork.pending", "second"));
    let mut window = Frames::new();
    let mut shadow = Frames::new();
    window.host(host_over(jobs.clone()));
    shadow.host(host_over(jobs.clone()));
    let (seen, shadow_seen) = (RefCell::new(Pending::<u32, String>::default()), RefCell::new(Pending::<u32, String>::default()));
    let (other, shadow_other) = (RefCell::new(Pending::<u64, String>::default()), RefCell::new(Pending::<u64, String>::default()));
    window.frame(vec![], |ui| {
        *seen.borrow_mut() = poll_type(ui, first, 7u32);
        *other.borrow_mut() = poll_type(ui, second, 9u64);
    });
    assert!(matches!(*seen.borrow(), Pending::Loading { .. }), "the window waits for its jobs: {:?}", seen.borrow());
    jobs.run_all();

    fork_into(&window.ctx, &shadow.ctx).expect("the shadow runs no pass");
    // The shadow polls first, before the window takes what landed.
    shadow.frame(vec![], |ui| {
        *shadow_seen.borrow_mut() = poll_type(ui, first, 7u32);
        *shadow_other.borrow_mut() = poll_type(ui, second, 9u64);
    });
    window.frame(vec![], |ui| {
        *seen.borrow_mut() = poll_type(ui, first, 7u32);
        *other.borrow_mut() = poll_type(ui, second, 9u64);
    });
    assert_eq!(*seen.borrow(), Pending::Ready(7), "the window still takes its own landed result");
    assert_eq!(*other.borrow(), Pending::Ready(9), "and so does its slot of another result type");
    assert_eq!(*shadow_seen.borrow(), Pending::Loading { progress: None }, "the fork submits a job of its own");
    assert_eq!(*shadow_other.borrow(), Pending::Loading { progress: None }, "for each result type it copied");
}

#[trial]
fn fork_into_the_same_context_is_refused_and_leaves_its_own_state_alone() {
    let jobs = Arc::new(ManualJobs::new());
    let id = global("fork.pending", "same");
    let mut window = Frames::new();
    window.host(host_over(jobs.clone()));
    let seen = RefCell::new(Pending::<u32, String>::default());
    window.frame(vec![], |ui| *seen.borrow_mut() = poll_type(ui, id, 5u32));
    jobs.run_all();
    // The same context through a second handle.
    let alias = window.ctx.clone();
    assert_eq!(fork_into(&window.ctx, &alias), Err(ForkError::SameContext), "a context is not forked into itself");
    window.frame(vec![], |ui| *seen.borrow_mut() = poll_type(ui, id, 5u32));
    assert_eq!(*seen.borrow(), Pending::Ready(5), "the context still takes its own landed result");
}
