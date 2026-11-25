//! Shared property-test configuration for the pathops suites.

use proptest::test_runner::{Config, RngSeed};

/// Seed the property suites run with unless `PROPTEST_RNG_SEED` is set.
const SEED: u64 = 0x5eed_9a7f;

/// Reproducible property-test config: `cases` cases from a fixed seed, so a CI run can never fail
/// at random. Explore further locally with `PROPTEST_CASES=5000 PROPTEST_RNG_SEED=<n>`; keep
/// every minimal failure found that way as a test in `regressions.rs`.
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
