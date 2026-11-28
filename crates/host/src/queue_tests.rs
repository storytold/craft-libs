//! `JobQueue` and `ManualJobs`: order, replacement, one job per key, cancellation, wakes and
//! inline runs. Every test drives lanes with `ManualJobs` (or `lanes: 0`), so nothing depends on
//! thread timing. The tests of `Cancel::wait` block a second thread and hand off over channels.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use crate::queue::{Cancel, Finished, JobQueue, Priority, QueueOptions, QueueOrder};
use crate::{ManualJobs, Wake};

fn queue(lanes: usize) -> (JobQueue<&'static str, &'static str>, Arc<ManualJobs>, Arc<AtomicUsize>) {
    let jobs = Arc::new(ManualJobs::new());
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&wakes);
    let wake = Wake::new(move || {
        counter.fetch_add(1, Ordering::Relaxed);
    });
    let queue = JobQueue::new(jobs.clone(), wake, QueueOptions { lanes, name: "test", ..QueueOptions::default() });
    (queue, jobs, wakes)
}

/// A queue without lanes in `order`: jobs run one at a time through `run_inline`.
fn inline_queue(order: QueueOrder) -> JobQueue<&'static str, &'static str> {
    JobQueue::new(Arc::new(ManualJobs::new()), Wake::new(|| {}), QueueOptions { lanes: 0, name: "test", order })
}

/// Runs the waiting jobs one `run_inline(1)` at a time and returns their results in run order.
fn run_one_by_one(queue: &JobQueue<&'static str, &'static str>) -> Vec<&'static str> {
    let mut ran = Vec::new();
    while queue.run_inline(1) == 1 {
        ran.extend(drain(queue));
    }
    ran
}

fn drain(queue: &JobQueue<&'static str, &'static str>) -> Vec<&'static str> {
    std::iter::from_fn(|| queue.try_recv()).map(|Finished { result, .. }| result).collect()
}

// A second submit for a waiting key runs only the second job; the first never runs.

// With two lanes: a key's second job waits until its first ends, while another key's job runs.
#[test]
fn queue_runs_one_job_per_key() {
    let (queue, jobs, _) = queue(2);
    let queue = Arc::new(queue);
    let inner = Arc::clone(&queue);
    let ran_meanwhile = Arc::new(AtomicUsize::new(usize::MAX));
    let seen = Arc::clone(&ran_meanwhile);
    queue.submit("a", Priority::VISIBLE, move |_| {
        inner.submit("a", Priority::VISIBLE, |_| "a2");
        inner.submit("b", Priority::BACKGROUND, |_| "b");
        // Only "b" may start while "a" runs, although "a2" ranks higher.
        seen.store(inner.run_inline(5), Ordering::Relaxed);
        "a1"
    });
    jobs.run_next();
    assert_eq!(ran_meanwhile.load(Ordering::Relaxed), 1);
    assert!(queue.contains(&"a"));
    jobs.run_all();
    assert_eq!(drain(&queue), ["b", "a1", "a2"]);
}

// `reprioritize` returning None drops waiting jobs and sets Cancel on running ones; neither result arrives.
#[test]
fn queue_dropped_keys_are_cancelled() {
    let (queue, jobs, wakes) = queue(1);
    let queue = Arc::new(queue);
    queue.submit("waiting", Priority::BACKGROUND, |_| "waiting");
    let inner = Arc::clone(&queue);
    let saw_cancel = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&saw_cancel);
    queue.submit("running", Priority::VISIBLE, move |cancel: &Cancel| {
        let dropped = inner.reprioritize(|_, _| None);
        assert_eq!(dropped, ["waiting", "running"]);
        seen.store(usize::from(cancel.is_cancelled()), Ordering::Relaxed);
        "running"
    });
    jobs.run_all();
    assert_eq!(saw_cancel.load(Ordering::Relaxed), 1);
    assert!(drain(&queue).is_empty());
    assert_eq!(wakes.load(Ordering::Relaxed), 0);
    assert!(!queue.contains(&"waiting") && !queue.contains(&"running"));
}

// `wait_timeout` reads the flag, a clone shares it, and `wait` returns at once on a set flag.
#[test]
fn cancel_wait_timeout_reports_the_flag() {
    let cancel = Cancel::new();
    let shared = cancel.clone();
    assert!(!cancel.wait_timeout(Duration::ZERO));
    shared.cancel();
    assert!(cancel.is_cancelled());
    assert!(cancel.wait_timeout(Duration::ZERO));
    cancel.wait();
}

// A thread blocked in `wait` returns once another thread cancels. The 60-second bound only fails
// the test when the wake never comes.
#[test]
fn cancel_wait_returns_once_another_thread_cancels() {
    let cancel = Cancel::new();
    let (started, has_started) = mpsc::channel();
    let (returned, has_returned) = mpsc::channel();
    let waiter = {
        let cancel = cancel.clone();
        thread::spawn(move || {
            let _ = started.send(());
            cancel.wait();
            let _ = returned.send(());
        })
    };
    has_started.recv().expect("the waiter starts");
    assert_eq!(has_returned.try_recv(), Err(TryRecvError::Empty), "wait returned before the cancel");
    cancel.cancel();
    has_returned.recv_timeout(Duration::from_secs(60)).expect("wait returns after the cancel");
    waiter.join().expect("the waiter ends");
}

// A running job blocked in `wait` ends when the queue drops its key, and its result never arrives.
#[test]
fn a_job_waiting_on_its_cancel_ends_when_its_key_is_dropped() {
    let (queue, jobs, wakes) = queue(1);
    let (started, has_started) = mpsc::channel();
    queue.submit("running", Priority::VISIBLE, move |cancel: &Cancel| {
        let _ = started.send(());
        cancel.wait();
        "running"
    });
    let (ran, has_run) = mpsc::channel();
    let lane = thread::spawn(move || {
        let _ = ran.send(jobs.run_all());
    });
    has_started.recv().expect("the job starts");
    assert_eq!(queue.reprioritize(|_, _| None), ["running"]);
    assert_eq!(has_run.recv_timeout(Duration::from_secs(60)), Ok(1), "the job ends after the cancel");
    lane.join().expect("the lane ends");
    assert!(drain(&queue).is_empty());
    assert_eq!(wakes.load(Ordering::Relaxed), 0);
}

// The wake is called once per finished job, never for a dropped or cancelled one.

// With `lanes == 0` nothing runs on submit; `run_inline(1)` runs the best job and returns 1.

// A `Jobs` that drops the lane unrun (a thread that could not start) does not leave the lane counted.
#[test]
fn queue_gives_back_a_lane_the_host_dropped() {
    use crate::Jobs;
    struct Refusing(AtomicUsize, ManualJobs);
    impl Jobs for Refusing {
        fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) {
            if self.0.load(Ordering::Relaxed) == 0 {
                drop(job);
            } else {
                self.1.spawn(name, job);
            }
        }
    }
    let jobs = Arc::new(Refusing(AtomicUsize::new(0), ManualJobs::new()));
    let queue = JobQueue::<&'static str, &'static str>::new(
        jobs.clone(),
        Wake::new(|| {}),
        QueueOptions { lanes: 1, name: "test", ..QueueOptions::default() },
    );
    queue.submit("a", Priority::VISIBLE, |_| "a");
    jobs.0.store(1, Ordering::Relaxed);
    queue.submit("b", Priority::VISIBLE, |_| "b");
    assert_eq!(jobs.1.waiting(), 1);
    jobs.1.run_all();
    assert_eq!(drain(&queue), ["b", "a"]);
}

// `ManualJobs::run_next` runs the oldest job only; a job spawned while it runs waits behind the others.

// `finish` runs every waiting job on the calling thread, the next job of a key included, and reports the queue done;
// the lanes the host was handed find nothing left.
#[test]
fn queue_finish_runs_the_waiting_jobs_inline() {
    let (queue, jobs, _) = queue(1);
    queue.submit("a", Priority::VISIBLE, |_| "a");
    queue.submit("b", Priority::BACKGROUND, |_| "b");
    assert!(queue.finish(std::time::Duration::ZERO), "nothing runs elsewhere, so nothing is left");
    assert_eq!(queue.counts().queued + queue.counts().running, 0);
    jobs.run_all();
    let mut done = drain(&queue);
    done.sort_unstable();
    assert_eq!(done, ["a", "b"], "each job ran once");
}

// Among equal priorities `OldestFirst` runs jobs in arrival order and the default runs the newest first; a tie rule
// flipped or ignored would reorder an app's file opens or its thumbnails.
#[test]
fn a_queue_ordered_oldest_first_runs_equal_priorities_in_arrival_order() {
    let oldest = inline_queue(QueueOrder::OldestFirst);
    let newest = inline_queue(QueueOrder::default());
    for queue in [&oldest, &newest] {
        queue.submit("one", Priority::NEAR, |_| "one");
        queue.submit("two", Priority::NEAR, |_| "two");
        queue.submit("three", Priority::NEAR, |_| "three");
    }
    assert_eq!(run_one_by_one(&oldest), ["one", "two", "three"]);
    assert_eq!(run_one_by_one(&newest), ["three", "two", "one"]);
}

// Priority decides before arrival in both orders: background work submitted first never runs before visible work
// submitted later, nor does newer background work outrank older visible work.
#[test]
fn a_higher_priority_runs_before_a_lower_one_in_either_order() {
    for order in [QueueOrder::NewestFirst, QueueOrder::OldestFirst] {
        let queue = inline_queue(order);
        queue.submit("background", Priority::BACKGROUND, |_| "background");
        queue.submit("visible", Priority::VISIBLE, |_| "visible");
        queue.submit("near", Priority::NEAR, |_| "near");
        assert_eq!(run_one_by_one(&queue), ["visible", "near", "background"], "{order:?}");
    }
}
