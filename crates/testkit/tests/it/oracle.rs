#![cfg(not(target_arch = "wasm32"))]

use craft_testkit::oracle::{self, Outcome};
use std::cell::Cell;
use std::process::{Command, Output};

/// No test or workflow sets this variable, so a missing program skips.
const UNSET_REQUIRED_VAR: &str = "CRAFT_TESTKIT_ORACLE_TEST_UNSET_REQUIRED";
/// The required variable the child case reads; the parent sets it or removes it.
const CHILD_REQUIRED_VAR: &str = "CRAFT_TESTKIT_ORACLE_CHILD_REQUIRED";
/// Set only in the child process, where it makes `child_requires_a_missing_oracle` run.
const CHILD_CASE: &str = "CRAFT_TESTKIT_ORACLE_CHILD";
/// The line the child prints after `require_oracle!` returned from its probe.
const RETURNED: &str = "require_oracle returned";
const MISSING: &str = "craft-testkit-no-such-oracle-binary";

#[test]
fn require_oracle_returns_from_the_test_when_the_program_is_missing() {
    let _oracle = craft_testkit::require_oracle!(oracle::find(MISSING, "--version"), UNSET_REQUIRED_VAR);
    panic!("require_oracle! must return before this line when the program is missing");
}

#[test]
fn require_oracle_returns_the_given_value_when_the_program_is_missing() {
    fn probe() -> u32 {
        let _oracle = craft_testkit::require_oracle!(oracle::find(MISSING, "--version"), UNSET_REQUIRED_VAR, 7);
        0
    }
    assert_eq!(probe(), 7);
}

#[test]
fn require_oracle_evaluates_the_required_variable_once() {
    fn probe(calls: &Cell<u32>) -> u32 {
        let _oracle = craft_testkit::require_oracle!(
            oracle::find(MISSING, "--version"),
            {
                calls.set(calls.get() + 1);
                UNSET_REQUIRED_VAR
            },
            7
        );
        0
    }
    let calls = Cell::new(0);
    assert_eq!(probe(&calls), 7);
    assert_eq!(calls.get(), 1);
}

#[test]
#[should_panic(expected = "oracle setup failed")]
fn require_oracle_fails_the_test_on_an_invalid_name() {
    let _oracle = craft_testkit::require_oracle!(oracle::find("../escape", "--version"), UNSET_REQUIRED_VAR);
}

#[test]
#[should_panic(expected = "cannot name a variable")]
fn require_oracle_fails_the_test_on_an_invalid_required_variable_name() {
    let _oracle = craft_testkit::require_oracle!(oracle::find(MISSING, "--version"), "CRAFT=REQUIRED");
}

#[test]
fn outcome_sends_only_a_missing_program_to_the_skip_rule() {
    let missing = oracle::outcome(oracle::find(MISSING, "--version"), UNSET_REQUIRED_VAR);
    assert!(matches!(missing, Err(Outcome::Missing(ref reason)) if reason.contains(MISSING)), "{missing:?}");
    let invalid = oracle::outcome(oracle::find("../escape", "--version"), UNSET_REQUIRED_VAR);
    assert!(matches!(invalid, Err(Outcome::Fail(_))), "{invalid:?}");
}

/// Runs only in the child process that `run_child` starts.
#[test]
fn child_requires_a_missing_oracle() {
    if std::env::var_os(CHILD_CASE).is_none() {
        return;
    }
    let _oracle = craft_testkit::require_oracle!(oracle::find(MISSING, "--version"), CHILD_REQUIRED_VAR);
    eprintln!("{RETURNED}");
}

/// Runs `child_requires_a_missing_oracle` in a new process of this test binary with
/// `CHILD_REQUIRED_VAR` set to `required` or removed, so no test changes this process's
/// environment.
fn run_child(required: Option<&str>) -> Output {
    let exe = std::env::current_exe().expect("test binary path");
    let mut cmd = Command::new(exe);
    cmd.args(["oracle::child_requires_a_missing_oracle", "--exact", "--nocapture", "--test-threads=1"]).env(CHILD_CASE, "1");
    match required {
        Some(value) => cmd.env(CHILD_REQUIRED_VAR, value),
        None => cmd.env_remove(CHILD_REQUIRED_VAR),
    };
    cmd.output().expect("run the child test")
}

#[test]
fn a_missing_oracle_follows_the_skip_rule_for_the_required_variable() {
    for required in [None, Some("")] {
        let out = run_child(required);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{required:?}: {stderr}");
        assert!(stderr.contains(&format!("SKIPPED: `{MISSING}` was not found")), "{required:?}: {stderr}");
        assert!(!stderr.contains(RETURNED), "{required:?}: {stderr}");
    }
    let out = run_child(Some("1"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(stderr.contains(&format!("{CHILD_REQUIRED_VAR}=1 forbids skipping")), "{stderr}");
    assert!(!stderr.contains("SKIPPED:"), "{stderr}");
    // Every other value is a configuration error, `0` included.
    for value in ["0", "true", "yes", " 1"] {
        let out = run_child(Some(value));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{value:?}: {stderr}");
        assert!(stderr.contains(&format!("{CHILD_REQUIRED_VAR}={value:?} is not empty or 1")), "{value:?}: {stderr}");
        assert!(!stderr.contains("SKIPPED:"), "{value:?}: {stderr}");
    }
}
