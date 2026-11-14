//! #492: a copy a package manager updates can leave the check at start out ([`STARTUP_VAR`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::tests::{newer, settle, updater};
use super::*;

/// An updater whose check finds a newer release and counts how often it asked.
fn counting_updater(startup_check: bool) -> (Updater, Arc<AtomicUsize>) {
    let asked = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&asked);
    let mut u = updater(Ok(None)).with_source(Arc::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(Some(newer()))
    }));
    u.startup_check = startup_check;
    (u, asked)
}

/// `v` as the run-time value of [`STARTUP_VAR`].
fn run(v: &str) -> Option<&OsStr> {
    Some(OsStr::new(v))
}

/// Every spelling of off and on, at run time and at build time; unset leaves the check on.
#[test]
fn the_variable_turns_the_startup_check_off_and_on() {
    for off in ["0", "false", "off", "no", "OFF", " No ", "False\n"] {
        assert!(!startup_check_from(run(off), None), "{off:?} at run time");
        assert!(!startup_check_from(None, Some(off)), "{off:?} at build time");
    }
    for on in ["1", "true", "on", "yes", "YES", " On "] {
        assert!(startup_check_from(run(on), None), "{on:?} at run time");
        assert!(startup_check_from(None, Some(on)), "{on:?} at build time");
    }
    // Unset everywhere: the check runs, as before.
    assert!(startup_check_from(None, None));
}

/// The run-time value overrides the build's both ways; an empty or unknown one decides nothing.
#[test]
fn the_run_time_value_wins_over_the_build_and_an_unusable_one_falls_through() {
    // A packager's build turns it off; the user turns it back on for one run, and the reverse.
    assert!(startup_check_from(run("1"), Some("0")));
    assert!(!startup_check_from(run("0"), Some("1")));
    // Empty or unrecognised at run time: the build decides, and without one the check runs.
    for unusable in ["", "  ", "2", "disabled", "of"] {
        assert!(!startup_check_from(run(unusable), Some("off")), "{unusable:?} over a build that is off");
        assert!(startup_check_from(run(unusable), Some("on")), "{unusable:?} over a build that is on");
        assert!(startup_check_from(run(unusable), None), "{unusable:?} alone");
        assert!(startup_check_from(None, Some(unusable)), "{unusable:?} at build time");
    }
}

/// A value that is not Unicode counts as unrecognised instead of panicking or matching.
#[test]
fn a_value_that_is_not_unicode_is_ignored() {
    #[cfg(unix)]
    let value = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(vec![b'0', 0xff])
    };
    #[cfg(windows)]
    let value = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[u16::from(b'0'), 0xd800])
    };
    assert!(value.to_str().is_none());
    assert!(startup_check_from(Some(&value), None));
    assert!(!startup_check_from(Some(&value), Some("0")));
}

/// Off: the check at start asks nothing and shows nothing, but a requested check still offers.
#[test]
fn with_the_startup_check_off_nothing_asks_the_network_until_the_user_does() {
    let ctx = egui::Context::default();
    let (mut u, asked) = counting_updater(false);
    u.check_at_start(&ctx);
    assert!(u.check.is_none(), "no check started");
    settle(&mut u, &ctx);
    assert_eq!(asked.load(Ordering::SeqCst), 0, "the network was asked at start");
    assert!(!u.prompt_open());
    assert_eq!(u.status(), &Status::Idle);
    // A second call stays quiet too.
    u.check_at_start(&ctx);
    assert!(u.check.is_none());

    // Help ▸ Check for Updates still checks, and offers what it finds.
    u.check_now(&ctx);
    assert_eq!(u.status(), &Status::Checking);
    settle(&mut u, &ctx);
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert!(u.prompt_open(), "a requested check offers the newer release");
}

/// Off: the first start after a failed update still says why it failed. Reading the outcome
/// record asks no network, so only the check itself is left out.
#[test]
fn with_the_startup_check_off_the_last_update_is_still_reported() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    self_update::record_failure(&paths, "disk full");
    let (u, asked) = counting_updater(false);
    let mut u = u.with_paths(paths);
    u.check_at_start(&ctx);
    assert_eq!(u.status(), &Status::InstallFailed("disk full".into()));
    assert!(u.check.is_none(), "no check started");
    settle(&mut u, &ctx);
    assert_eq!(asked.load(Ordering::SeqCst), 0, "the network was asked at start");
    assert!(!u.prompt_open());
}

/// On: the check at start asks once, however often it is called, and offers the release.
#[test]
fn with_the_startup_check_on_the_check_at_start_asks_once() {
    let ctx = egui::Context::default();
    let (mut u, asked) = counting_updater(true);
    u.check_at_start(&ctx);
    settle(&mut u, &ctx);
    u.check_at_start(&ctx);
    settle(&mut u, &ctx);
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert!(u.prompt_open());
}

/// A new updater follows the run-time value. Child processes isolate environment overrides from
/// parallel tests without unsafe set_var; an unusable value is left to the unit tests above, as
/// its outcome depends on the build's value.
#[test]
fn a_new_updater_reads_the_variable_from_its_environment() {
    const CHILD: &str = "CRAFT_UPDATE_STARTUP_TEST_EXPECT";
    if let Ok(expect) = std::env::var(CHILD) {
        let expect = expect == "on";
        assert_eq!(startup_check_enabled(), expect);
        assert_eq!(Updater::new(&crate::suite::PHOTOCRAFT, "0.1.0").startup_check, expect);
        return;
    }
    for (value, expect) in [("off", "off"), ("0", "off"), ("on", "on"), ("1", "on")] {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "updater::startup_tests::a_new_updater_reads_the_variable_from_its_environment", "--nocapture"])
            .env(CHILD, expect)
            .env(STARTUP_VAR, value)
            .status()
            .unwrap();
        assert!(status.success(), "{STARTUP_VAR}={value} should leave the check at start {expect}");
    }
}

/// A build made with `CRAFT_UPDATE_STARTUP=off` leaves the check at start off when the run sets
/// nothing. Only that build tests anything here (the update workflow runs it); the child process
/// has the run-time variable removed, so it sees only the compiled value.
#[test]
fn a_build_time_off_value_turns_the_startup_check_off() {
    const CHILD: &str = "CRAFT_UPDATE_STARTUP_TEST_BUILD_OFF";
    if option_env!("CRAFT_UPDATE_STARTUP") != Some("off") {
        return;
    }
    if std::env::var_os(CHILD).is_some() {
        assert!(!startup_check_enabled());
        assert!(!Updater::new(&crate::suite::PHOTOCRAFT, "0.1.0").startup_check);
        return;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "updater::startup_tests::a_build_time_off_value_turns_the_startup_check_off", "--nocapture"])
        .env(CHILD, "1")
        .env_remove(STARTUP_VAR)
        .status()
        .unwrap();
    assert!(status.success(), "a build made with {STARTUP_VAR}=off should leave the check at start off");
}
