//! Proptest configuration for property suites that must repeat the same cases on every run.
//!
//! [`config`] starts from proptest's `Config::default()`, which has already read the `PROPTEST_*`
//! variables, turns failure persistence off, and then:
//!
//! - uses the caller's case count unless `PROPTEST_CASES` is set, and
//! - pins the seed to [`SEED`] unless `PROPTEST_RNG_SEED` is set.
//!
//! When a variable is set, the field keeps the value proptest read from it. Proptest parses
//! `PROPTEST_CASES` as a `u32` and `PROPTEST_RNG_SEED` as a `u64`. For a value it cannot parse,
//! proptest prints a warning and keeps its own default: 256 cases, or a random seed. Setting
//! `PROPTEST_RNG_SEED` to a value that is not a number therefore runs the suite with random seeds.
//!
//! No file under `proptest-regressions/` is read or written. A minimal failure found with other
//! seeds or counts, for example with `PROPTEST_CASES=5000 PROPTEST_RNG_SEED=<n>`, is kept as a named
//! regression test in the suite.

use proptest::test_runner::{Config, RngSeed};

/// The seed [`config`] uses when `PROPTEST_RNG_SEED` is not set.
pub const SEED: u64 = 0x5eed_9a7f;

/// A proptest configuration that runs `cases` cases from [`SEED`] with failure persistence off.
///
/// `PROPTEST_CASES` replaces `cases` and `PROPTEST_RNG_SEED` replaces the seed when either is set.
/// Pass the result to `#![proptest_config(...)]` or to `TestRunner::new`.
pub fn config(cases: u32) -> Config {
    let mut c = Config { failure_persistence: None, ..Config::default() };
    if std::env::var_os("PROPTEST_CASES").is_none() {
        c.cases = cases;
    }
    if std::env::var_os("PROPTEST_RNG_SEED").is_none() {
        c.rng_seed = RngSeed::Fixed(SEED);
    }
    c
}
