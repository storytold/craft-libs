//! `hostile::hammer` as the media crates' tests call it.

use std::collections::BTreeSet;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_media_testkit::hostile::{divisor, hammer};

/// Set only in the child processes of `run_child`: the value names what `child_hammer` does.
const CHILD_VARIABLE: &str = "CRAFT_MEDIA_TESTKIT_HOSTILE_CHILD";

/// Overrides the two-minute limit on each child process of `run_child`. The limit does not read
/// `CRAFT_TEST_TIMEOUT_SECS`, which `a_hang_fails_at_the_deadline` sets for the child.
const WATCHDOG_VARIABLE: &str = "CRAFT_MEDIA_TESTKIT_CHILD_WATCHDOG_SECS";

/// Runs `child_hammer` in a child process with `vars` set, and returns what it printed. The child
/// must fail. A child that never returns fails the test after two minutes.
fn run_child(vars: &'static [(&'static str, &'static str)]) -> String {
    let limit = craft_testkit::DeadlineLimit::new(Duration::from_secs(120)).with_variable(WATCHDOG_VARIABLE);
    craft_testkit::with_deadline(limit, "the child test", move || {
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child_hammer", "--nocapture", "--test-threads=1"])
            .envs(vars.iter().copied())
            .output()
            .unwrap();
        let printed = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(printed.contains("running 1 test"), "the child did not run the test:\n{printed}");
        assert!(!out.status.success(), "the child passed:\n{printed}");
        printed
    })
}

/// Runs in the child processes of `run_child`. Without `CHILD_VARIABLE` it hammers a small seed and
/// passes; with `hang`, its parser never returns.
#[test]
fn child_hammer() {
    if std::env::var_os(CHILD_VARIABLE).is_some_and(|v| v == "hang") {
        hammer("hang", &[1, 2, 3], 0, |_| {
            loop {
                std::thread::park();
            }
        });
    } else {
        hammer("small", &[1, 2, 3], 10, |_| {});
    }
}

#[test]
fn a_hang_fails_at_the_deadline() {
    let printed = run_child(&[(CHILD_VARIABLE, "hang"), ("CRAFT_TEST_TIMEOUT_SECS", "1")]);
    assert!(printed.contains("hang: the unmodified seed (len 3) did not finish within 1s, set by CRAFT_TEST_TIMEOUT_SECS"), "{printed}");
}

#[test]
fn a_bad_multiplier_names_the_variable() {
    let printed = run_child(&[("CRAFT_MEDIA_HOSTILE_MULT", "ten")]);
    assert!(printed.contains(r#"small: CRAFT_MEDIA_HOSTILE_MULT="ten" is not a whole number written in digits"#), "{printed}");
}

#[test]
fn a_panic_keeps_its_message() {
    // Truncation to 0 runs under any CRAFT_HOSTILE_DIVISOR.
    let payload = std::panic::catch_unwind(|| hammer("demo", &[9, 8, 7, 6], 0, |d| assert!(!d.is_empty(), "boom at {}", d.len()))).unwrap_err();
    let message = payload.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.starts_with("demo: truncation to 0 (len 0): the parser panicked: "), "{message}");
    assert!(message.contains("boom at 0"), "{message}");
}

#[test]
fn every_truncation_of_a_large_seed_runs() {
    let seed: Vec<u8> = (0..8_192u32).map(|i| i as u8).collect();
    let lengths = Arc::new(Mutex::new(BTreeSet::new()));
    let record = Arc::clone(&lengths);
    hammer("large", &seed, 0, move |d| {
        record.lock().unwrap().insert(d.len());
    });
    // The seed and its 8,192 truncations, every N-th of them under CRAFT_HOSTILE_DIVISOR=N.
    assert_eq!(lengths.lock().unwrap().len(), 1 + 8_192usize.div_ceil(divisor()));
}

#[test]
fn the_child_timeout_variable_does_not_set_the_watchdog() {
    // `a_hang_fails_at_the_deadline` sets `CRAFT_TEST_TIMEOUT_SECS` for its child. Run it with a value
    // that `with_deadline` rejects; it passes only if `run_child`'s watchdog reads another variable.
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "a_hang_fails_at_the_deadline", "--nocapture", "--test-threads=1"])
        .env("CRAFT_TEST_TIMEOUT_SECS", "not a number")
        .output()
        .unwrap();
    let printed = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(printed.contains("running 1 test"), "the test did not run:\n{printed}");
    assert!(out.status.success(), "{printed}");
}
