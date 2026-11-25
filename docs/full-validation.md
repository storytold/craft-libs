# Full validation

The fast Workspace workflow intentionally samples hostile-input/proptest cases and primarily
runs all features. It is not evidence of full default-feature runtime or doctest coverage.

Add the `full-validation` label to a pull request to start the separate Full validation workflow.
Keep it on the pull request so each new push receives full checks. The workflow checks the PR
merge ref using `pull_request`, never `pull_request_target`, and does not inherit secrets.
A workflow dispatch validates the selected repository ref; it is not evidence for another PR SHA.

Before treating a candidate as fully validated, verify on its current head/merge ref:

- All three `Full native / gate` checks actually ran and succeeded (Linux, Windows, macOS).
- Both workspace test configurations ran with `cargo test`: default and all features, including
  their doctests and Cargo's ordinary example compilation/linking. Both clippy configurations
  and formatting checks also pass. No hostile divisor or proptest case override is set.
- All three `Full PDF contracts / gate` checks ran and succeeded, including the Unicode fixture
  cfg and the separate read-only-contract manifest. Linux-only generic tests do not replace them.
- The normal MSRV, WASM and applicable specialized workflow checks pass too. This optional
  workflow does not replace those checks, Sonar analysis, independent review, or branch rules.

An unlabelled PR's skipped Full validation jobs are not a full-validation pass. The label is an
opt-in mechanism, not a branch-protection change or a merge authorization. A new base/head may
need a fresh full run. Do not dispatch another copy when the matching run is already active.

The existing fast path and nightly workflow are left unchanged. This workflow is separate from
platform-selection and lint/cache optimization work in #2113. Its native commands deliberately
retain the repository's `cargo test --workspace` pre-merge requirement.
