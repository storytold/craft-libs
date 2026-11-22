//! Each test runs this test binary again as a child with the `PROPTEST_*` variables it needs, so no
//! test changes the environment of the process that runs it. The child runs `child_reports_config`,
//! which prints one `PROPCFG` line describing `config(7)`.

#![cfg(not(target_arch = "wasm32"))]

use std::process::{Command, Output};

use craft_testkit::prop::{SEED, config};
use proptest::prelude::any;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

/// Set in a child's environment so that `child_reports_config` prints its line.
const CHILD: &str = "CRAFT_TESTKIT_PROP_CHILD";

/// The case count every child passes to `config`.
const CASES: u32 = 7;

/// Variables removed from every child's environment before a test sets its own.
const PROPTEST_VARS: [&str; 3] = ["PROPTEST_CASES", "PROPTEST_RNG_SEED", "PROPTEST_DISABLE_FAILURE_PERSISTENCE"];

#[test]
fn child_reports_config() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let c = config(CASES);
    let line = format!("cases={} seed={} persistence={}", c.cases, c.rng_seed, c.failure_persistence.is_some());
    let mut runner = TestRunner::new(c);
    let draws: Vec<String> = (0..4).map(|_| any::<u64>().new_tree(&mut runner).expect("draw a value").current().to_string()).collect();
    println!("PROPCFG {line} draws={}", draws.join(","));
}

/// The fields of one child's `PROPCFG` line.
#[derive(Debug, PartialEq)]
struct Report {
    cases: String,
    seed: String,
    persistence: String,
    draws: String,
}

fn spawn_child(vars: &[(&str, &str)]) -> Output {
    let exe = std::env::current_exe().expect("locate the test binary");
    let mut cmd = Command::new(exe);
    cmd.args(["--exact", "child_reports_config", "--nocapture", "--test-threads=1"]).env(CHILD, "1");
    for var in PROPTEST_VARS {
        cmd.env_remove(var);
    }
    for (name, value) in vars {
        cmd.env(name, value);
    }
    cmd.output().expect("run the child test")
}

/// Runs a child that `config` must reject, and returns its stderr.
fn rejected_child(vars: &[(&str, &str)]) -> String {
    let out = spawn_child(vars);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!out.status.success(), "child passed with {vars:?}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(!stdout.contains("PROPCFG "), "child printed a config with {vars:?}\nstdout:\n{stdout}");
    stderr
}

fn run_child(vars: &[(&str, &str)]) -> Report {
    let out = spawn_child(vars);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "child failed with {vars:?}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    let line = stdout
        .lines()
        .find_map(|l| l.split_once("PROPCFG ").map(|(_, rest)| rest))
        .unwrap_or_else(|| panic!("no PROPCFG line with {vars:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"));
    let field = |name: &str| {
        line.split(' ')
            .find_map(|part| part.strip_prefix(name).and_then(|rest| rest.strip_prefix('=')))
            .unwrap_or_else(|| panic!("no {name} in {line:?}"))
            .to_string()
    };
    Report { cases: field("cases"), seed: field("seed"), persistence: field("persistence"), draws: field("draws") }
}

#[test]
fn without_variables_the_caller_count_and_pinned_seed_apply() {
    let r = run_child(&[]);
    assert_eq!(r.cases, CASES.to_string());
    assert_eq!(r.seed, SEED.to_string());
    assert_eq!(r.persistence, "false");
}

#[test]
fn without_variables_every_run_draws_the_same_values() {
    let first = run_child(&[]);
    let second = run_child(&[]);
    assert_eq!(first.draws, second.draws);
}

#[test]
fn proptest_cases_replaces_the_caller_count() {
    let r = run_child(&[("PROPTEST_CASES", "19")]);
    assert_eq!(r.cases, "19");
    assert_eq!(r.seed, SEED.to_string());
    assert_eq!(r.persistence, "false");
}

#[test]
fn proptest_rng_seed_replaces_the_pinned_seed() {
    let pinned = run_child(&[]);
    let r = run_child(&[("PROPTEST_RNG_SEED", "12345")]);
    assert_eq!(r.cases, CASES.to_string());
    assert_eq!(r.seed, "12345");
    assert_eq!(r.persistence, "false");
    assert_ne!(r.draws, pinned.draws);
    assert_eq!(run_child(&[("PROPTEST_RNG_SEED", "12345")]).draws, r.draws);
}

#[test]
fn both_variables_apply_together() {
    let r = run_child(&[("PROPTEST_CASES", "19"), ("PROPTEST_RNG_SEED", "12345")]);
    assert_eq!(r.cases, "19");
    assert_eq!(r.seed, "12345");
    assert_eq!(r.persistence, "false");
}

#[test]
fn an_unparsable_case_count_is_rejected() {
    for value in ["many", "", "-1", "4294967296"] {
        let stderr = rejected_child(&[("PROPTEST_CASES", value)]);
        assert!(stderr.contains(&format!("PROPTEST_CASES={value:?} is not a u32")), "PROPTEST_CASES={value:?}\nstderr:\n{stderr}");
    }
}

#[test]
fn an_unparsable_seed_is_rejected() {
    for value in ["random", "", "-1", "18446744073709551616"] {
        let stderr = rejected_child(&[("PROPTEST_RNG_SEED", value)]);
        assert!(stderr.contains(&format!("PROPTEST_RNG_SEED={value:?} is not a u64")), "PROPTEST_RNG_SEED={value:?}\nstderr:\n{stderr}");
    }
}
