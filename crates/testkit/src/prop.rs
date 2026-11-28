//! Proptest configuration for property suites that must repeat the same cases on every run.
//!
//! [`config`] starts from proptest's `Config::default()`, which has already read the `PROPTEST_*`
//! variables, turns failure persistence off, and then:
//!
//! - uses the caller's case count unless `PROPTEST_CASES` is set, and
//! - pins the seed to [`SEED`] unless `PROPTEST_RNG_SEED` is set.
//!
//! A set `PROPTEST_CASES` must parse as a `u32`, and a set `PROPTEST_RNG_SEED` must parse as a
//! `u64`. For any other value, `config` panics with a message that names the variable and its value,
//! and the suite fails. Proptest's own fallback of 256 cases or a random seed is not used, so every
//! run's seed is either [`SEED`] or the number in `PROPTEST_RNG_SEED`.
//!
//! No file under `proptest-regressions/` is read or written. A minimal failure found with other
//! seeds or counts, for example with `PROPTEST_CASES=5000 PROPTEST_RNG_SEED=<n>`, is kept as a named
//! regression test in the suite.

use std::ffi::OsStr;
use std::str::FromStr;

use proptest::test_runner::{Config, RngSeed};

/// The seed [`config`] uses when `PROPTEST_RNG_SEED` is not set.
pub const SEED: u64 = 0x5eed_9a7f;

/// A proptest configuration that runs `cases` cases from [`SEED`] with failure persistence off.
///
/// `PROPTEST_CASES` replaces `cases` and `PROPTEST_RNG_SEED` replaces the seed when either is set.
/// Pass the result to `#![proptest_config(...)]` or to `TestRunner::new`.
///
/// # Panics
///
/// Panics when `PROPTEST_CASES` is set to a value that is not a `u32`, or `PROPTEST_RNG_SEED` to a
/// value that is not a `u64`.
pub fn config(cases: u32) -> Config {
    let mut c = Config { failure_persistence: None, ..Config::default() };
    c.cases = match std::env::var_os("PROPTEST_CASES") {
        None => cases,
        Some(value) => parse_var("PROPTEST_CASES", &value, "a u32"),
    };
    c.rng_seed = match std::env::var_os("PROPTEST_RNG_SEED") {
        None => RngSeed::Fixed(SEED),
        Some(value) => RngSeed::Fixed(parse_var("PROPTEST_RNG_SEED", &value, "a u64")),
    };
    c
}

// Failing the calling test is what this function is for.
#[allow(clippy::panic)]
fn parse_var<T: FromStr>(name: &str, value: &OsStr, expected: &str) -> T {
    match value.to_str().and_then(|s| s.parse().ok()) {
        Some(parsed) => parsed,
        None => panic!("{name}={value:?} is not {expected}"),
    }
}
