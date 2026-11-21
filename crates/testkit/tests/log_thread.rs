//! Per-thread capture. Every test in this binary captures per thread, so the logger runs in that
//! mode for the whole process.
#![cfg(not(target_arch = "wasm32"))]

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use craft_testkit::log::{CaptureError, CapturedRecord, MAX_BYTES, MAX_RECORDS, capture_global, records, warnings};
use log::{Level, LevelFilter};

#[test]
fn warnings_returns_the_result_and_the_warnings_and_errors() {
    let (out, lines) = warnings(|| {
        log::info!("not a warning");
        log::warn!("kept {} of {}", 3, 4);
        log::error!("an error");
        log::debug!("not a warning either");
        7
    })
    .expect("capture");
    assert_eq!(out, 7);
    assert_eq!(lines, ["kept 3 of 4", "an error"]);
}

#[test]
fn records_keep_level_and_target_at_the_requested_level() {
    let ((), got) = records(LevelFilter::Info, || {
        log::info!(target: "craft::test", "opened");
        log::debug!("dropped");
        log::warn!("short");
    })
    .expect("capture");
    assert_eq!(
        got,
        [
            CapturedRecord { level: Level::Info, target: "craft::test".into(), message: "opened".into() },
            CapturedRecord { level: Level::Warn, target: module_path!().into(), message: "short".into() },
        ]
    );
}

// No test in this binary raises the maximum level above `Info`, so the nested-capture test can
// check that a refused `Trace` capture leaves it below `Trace`.

#[test]
fn a_capture_does_not_lower_a_raised_maximum_level() {
    log::set_max_level(LevelFilter::Info);
    let ((), lines) = warnings(|| log::info!("below warn")).expect("capture");
    assert!(lines.is_empty());
    assert_eq!(log::max_level(), LevelFilter::Info);
}

#[test]
fn a_nested_capture_is_refused_and_the_outer_one_keeps_its_records() {
    let ran = AtomicBool::new(false);
    let (inner, lines) = warnings(|| {
        log::warn!("before");
        let inner = warnings(|| ran.store(true, Ordering::SeqCst));
        let deeper = records(LevelFilter::Trace, || ran.store(true, Ordering::SeqCst));
        log::warn!("after");
        (inner.map(|_| ()), deeper.map(|_| ()))
    })
    .expect("outer capture");
    assert_eq!(inner, (Err(CaptureError::NestedCapture), Err(CaptureError::NestedCapture)));
    assert!(!ran.load(Ordering::SeqCst), "a nested closure must not run");
    assert!(log::max_level() < LevelFilter::Trace, "a refused capture must not raise the maximum level");
    assert_eq!(lines, ["before", "after"]);
}

#[test]
fn a_panic_ends_the_capture_and_the_next_one_starts_empty() {
    let caught = catch_unwind(AssertUnwindSafe(|| {
        warnings(|| {
            log::warn!("left behind");
            panic!("closure failed");
        })
    }));
    assert!(caught.is_err());
    let ((), lines) = warnings(|| log::warn!("next")).expect("a new capture after the panic");
    assert_eq!(lines, ["next"]);
}

#[test]
fn records_from_other_threads_are_not_captured() {
    let ((), lines) = warnings(|| {
        std::thread::spawn(|| log::warn!("elsewhere")).join().expect("join");
        log::warn!("here");
    })
    .expect("capture");
    assert_eq!(lines, ["here"]);
}

#[test]
fn records_over_the_count_limit_are_reported() {
    let got = warnings(|| {
        for i in 0..=MAX_RECORDS {
            log::warn!("{i}");
        }
    });
    assert_eq!(got.map(|_| ()), Err(CaptureError::Overflow { dropped: 1 }));
    let ((), lines) = warnings(|| log::warn!("fits")).expect("a new capture after an overflow");
    assert_eq!(lines, ["fits"]);
}

#[test]
fn a_message_over_the_byte_limit_is_reported() {
    let long = "x".repeat(MAX_BYTES);
    let got = warnings(|| {
        log::warn!("short");
        log::warn!("{long}");
    });
    assert_eq!(got.map(|_| ()), Err(CaptureError::Overflow { dropped: 1 }));
}

/// Writes one byte at a time until a write fails, counting the writes that succeed.
struct Endless<'a>(&'a AtomicUsize);

impl fmt::Display for Endless<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for _ in 0..2 * MAX_BYTES {
            f.write_str("x")?;
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[test]
fn formatting_a_message_stops_at_the_byte_limit() {
    let written = AtomicUsize::new(0);
    let got = warnings(|| log::warn!("{}", Endless(&written)));
    assert_eq!(got.map(|_| ()), Err(CaptureError::Overflow { dropped: 1 }));
    assert_eq!(written.load(Ordering::SeqCst), MAX_BYTES);
}

#[test]
fn a_global_capture_is_refused_in_a_per_thread_process() {
    warnings(|| ()).expect("per-thread capture installs the logger");
    assert_eq!(capture_global(LevelFilter::Warn).map(|_| ()), Err(CaptureError::ModeConflict));
}
