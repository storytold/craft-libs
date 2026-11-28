//! Process-wide capture, in a binary without the test harness so the cases run one after another
//! on the main thread, as a `harness = false` consumer runs them. The process captures process
//! wide from the first case on. The module is native only, so on `wasm32` the binary runs nothing.

fn main() {
    // nextest lists custom-harness tests with `--list`; this binary is one test.
    if std::env::args().any(|a| a == "--list") {
        println!("log_global: test");
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    cases::run();
}

#[cfg(not(target_arch = "wasm32"))]
mod cases {
    use std::fmt;
    use std::sync::mpsc::{self, Receiver, Sender};

    use craft_testkit::log::{CaptureError, CapturedRecord, MAX_RECORDS, capture_global, records, warnings};
    use log::{Level, LevelFilter};

    pub fn run() {
        a_record_from_another_thread_is_drained();
        the_level_filters_and_a_drain_empties_the_capture();
        a_second_live_capture_is_refused_and_a_new_one_starts_after_drop();
        a_record_checked_against_one_capture_is_not_kept_by_the_next();
        an_undrained_capture_reports_what_it_dropped();
        a_capture_does_not_lower_a_raised_maximum_level();
        per_thread_captures_are_refused_in_a_process_wide_process();
        println!("log_global: 7 cases passed");
    }

    fn a_record_from_another_thread_is_drained() {
        let capture = capture_global(LevelFilter::Warn).expect("capture");
        std::thread::Builder::new()
            .name("audio".into())
            .spawn(|| log::warn!(target: "clap_host", "editor dropped off the main thread"))
            .expect("spawn")
            .join()
            .expect("join");
        assert_eq!(
            capture.drain().expect("drain"),
            [CapturedRecord { level: Level::Warn, target: "clap_host".into(), message: "editor dropped off the main thread".into() }]
        );
    }

    fn the_level_filters_and_a_drain_empties_the_capture() {
        let capture = capture_global(LevelFilter::Info).expect("capture");
        log::debug!("below info");
        log::info!("info");
        log::error!("error");
        let messages: Vec<String> = capture.drain().expect("drain").into_iter().map(|r| r.message).collect();
        assert_eq!(messages, ["info", "error"]);
        assert!(capture.drain().expect("second drain").is_empty());
    }

    fn a_second_live_capture_is_refused_and_a_new_one_starts_after_drop() {
        let first = capture_global(LevelFilter::Warn).expect("capture");
        log::warn!("held by the first capture");
        assert_eq!(capture_global(LevelFilter::Warn).map(|_| ()), Err(CaptureError::GlobalCaptureActive));
        drop(first);
        log::warn!("between captures");
        let second = capture_global(LevelFilter::Warn).expect("a new capture after drop");
        log::warn!("held by the second capture");
        let messages: Vec<String> = second.drain().expect("drain").into_iter().map(|r| r.message).collect();
        assert_eq!(messages, ["held by the second capture"]);
    }

    /// Signals on `started` when formatting begins, then waits on `go` before writing its text.
    struct Gate {
        started: Sender<()>,
        go: Receiver<()>,
    }

    impl fmt::Display for Gate {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.started.send(()).expect("signal the start");
            self.go.recv().expect("wait for go");
            f.write_str("checked against the first capture")
        }
    }

    fn a_record_checked_against_one_capture_is_not_kept_by_the_next() {
        let first = capture_global(LevelFilter::Warn).expect("capture");
        let (started_tx, started_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel();
        let gate = Gate { started: started_tx, go: go_rx };
        let logger = std::thread::spawn(move || log::warn!("{gate}"));
        started_rx.recv().expect("formatting started");
        drop(first);
        let second = capture_global(LevelFilter::Warn).expect("a new capture");
        go_tx.send(()).expect("release the formatter");
        logger.join().expect("join");
        assert!(second.drain().expect("drain").is_empty());
    }

    fn an_undrained_capture_reports_what_it_dropped() {
        let capture = capture_global(LevelFilter::Warn).expect("capture");
        for i in 0..MAX_RECORDS + 2 {
            log::warn!("{i}");
        }
        assert_eq!(capture.drain(), Err(CaptureError::Overflow { dropped: 2 }));
        log::warn!("after the overflow");
        let messages: Vec<String> = capture.drain().expect("drain after an overflow").into_iter().map(|r| r.message).collect();
        assert_eq!(messages, ["after the overflow"]);
    }

    fn a_capture_does_not_lower_a_raised_maximum_level() {
        log::set_max_level(LevelFilter::Trace);
        let capture = capture_global(LevelFilter::Warn).expect("capture");
        log::info!("below warn");
        assert_eq!(log::max_level(), LevelFilter::Trace);
        assert!(capture.drain().expect("drain").is_empty());
    }

    fn per_thread_captures_are_refused_in_a_process_wide_process() {
        let mut ran = false;
        assert_eq!(warnings(|| ran = true).map(|_| ()), Err(CaptureError::ModeConflict));
        assert_eq!(records(LevelFilter::Trace, || ran = true).map(|_| ()), Err(CaptureError::ModeConflict));
        assert!(!ran, "a refused capture must not run its closure");
    }
}
