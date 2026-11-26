#![cfg(not(target_arch = "wasm32"))]

use craft_testkit::oracle::{self, Outcome};

/// No test or workflow sets this variable, so a missing program skips.
const UNSET_REQUIRED_VAR: &str = "CRAFT_TESTKIT_ORACLE_TEST_UNSET_REQUIRED";

#[test]
fn require_oracle_returns_from_the_test_when_the_program_is_missing() {
    let _oracle = craft_testkit::require_oracle!(oracle::find("craft-testkit-no-such-oracle-binary", "--version"), UNSET_REQUIRED_VAR);
    panic!("require_oracle! must return before this line when the program is missing");
}

#[test]
fn require_oracle_returns_the_given_value_when_the_program_is_missing() {
    fn probe() -> u32 {
        let _oracle = craft_testkit::require_oracle!(oracle::find("craft-testkit-no-such-oracle-binary", "--version"), UNSET_REQUIRED_VAR, 7);
        0
    }
    assert_eq!(probe(), 7);
}

#[test]
#[should_panic(expected = "oracle setup failed")]
fn require_oracle_fails_the_test_on_an_invalid_name() {
    let _oracle = craft_testkit::require_oracle!(oracle::find("../escape", "--version"), UNSET_REQUIRED_VAR);
}

#[test]
fn outcome_skips_only_a_missing_program() {
    let skipped = oracle::outcome(oracle::find("craft-testkit-no-such-oracle-binary", "--version"), UNSET_REQUIRED_VAR);
    assert!(matches!(skipped, Err(Outcome::Skip(ref line)) if line.starts_with("SKIPPED (oracle):")), "{skipped:?}");
}
