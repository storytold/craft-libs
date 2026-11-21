//! Capture in a process where another logger was installed first.
#![cfg(not(target_arch = "wasm32"))]

use craft_testkit::log::{CaptureError, capture_global, records, warnings};
use log::{LevelFilter, Log, Metadata, Record};

struct Foreign;

impl Log for Foreign {
    fn enabled(&self, _: &Metadata) -> bool {
        true
    }

    fn log(&self, _: &Record) {}

    fn flush(&self) {}
}

static FOREIGN: Foreign = Foreign;

#[test]
fn every_capture_is_refused_and_nothing_changes() {
    // The first call installs the foreign logger; this is the only test in the binary.
    log::set_logger(&FOREIGN).expect("install the foreign logger");
    log::set_max_level(LevelFilter::Error);

    let mut ran = false;
    assert_eq!(warnings(|| ran = true).map(|_| ()), Err(CaptureError::LoggerAlreadySet));
    assert_eq!(records(LevelFilter::Trace, || ran = true).map(|_| ()), Err(CaptureError::LoggerAlreadySet));
    assert_eq!(capture_global(LevelFilter::Trace).map(|_| ()), Err(CaptureError::LoggerAlreadySet));
    assert!(!ran, "a refused capture must not run its closure");
    assert_eq!(log::max_level(), LevelFilter::Error);
}
