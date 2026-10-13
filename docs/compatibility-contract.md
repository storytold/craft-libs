# Cross-application compatibility validation

`cargo xtask compat` tests proposed changes in `craft-libs` against consuming Craft applications before those changes are adopted across the suite. It runs each application's existing test suite against both its committed dependencies and the proposed library revision, without modifying the application's source repository, committed dependency pins, or lockfiles.

## How it works

The runner operates in two distinct modes for each consuming application:

1. **Baseline**: Tests the application using its committed dependencies and lockfile. This establishes whether any test failures already exist on the consumer's current branch.
2. **Candidate**: Tests the application against a specified `craft-libs` commit, branch, or working tree using Cargo's native patch configuration (`--config 'patch."https://github.com/storytold/craft-libs".craft-ui.path="<candidate-path>"'`).

To protect consumer repositories from accidental modification or lockfile changes, candidate evaluation runs in an ephemeral Git worktree created under `target/compat/worktrees/`. A drop guard ensures that temporary worktrees are removed upon completion, interruption (`SIGINT`/`SIGTERM`), or failure.

Before executing any candidate tests, the runner runs `cargo tree -i craft-ui` in the worktree to verify that Cargo actually resolved the candidate path rather than the pinned Git revision. If the resolved source does not match the candidate, validation aborts immediately.

## Results classification

Every consumer check is evaluated across both runs:

| Baseline | Candidate | Classification | Description | Exit status |
| :---: | :---: | :--- | :--- | :---: |
| Pass | Pass | `COMPATIBLE` | Candidate revision preserves application compatibility (full test run) | 0 |
| Pass | Fail | `CANDIDATE_REGRESSION` | Candidate revision introduced a failure | 1 |
| Fail | Fail | `BASELINE_FAILURE` | Pre-existing failure on consumer's current branch | 0 |
| Fail | Pass | `FIXED_IN_CANDIDATE` | Candidate resolved an existing issue (observed outcome) | 0 |
| Dry Run | Dry Run | `PROVENANCE_VERIFIED` | Dependency resolution and worktree isolation verified without running tests | 0 |
| Error | — / Error | `INFRASTRUCTURE_FAILURE` | Git worktree, network, or environment failure | 1 |
| Incomplete | — | `INCOMPLETE` | Run cancelled or interrupted mid-execution | 1 |
| — | — | `SKIPPED` | Consumer directory not found or check skipped | 0 |

A run exits with a non-zero status if any candidate regression or infrastructure failure occurs. Baseline failures are reported in the summary table and machine-readable output but do not block library validation because the failure pre-existed in the consumer.

## Running

```sh
# Test default consumers (PhotoCraft and VectorCraft) against current working tree (HEAD)
cargo xtask compat

# Test all 9 known consumers across the Craft suite
cargo xtask compat --all-consumers

# Use fast dependency compilation (opt-level = 0 override)
cargo xtask compat --quick

# Test against a specific craft-libs commit or branch
cargo xtask compat --revision 5c874d51

# Test specific consumers
cargo xtask compat --consumers photocraft,vectorcraft

# Test all targets including doc-tests and integration tests (default: unit/lib only)
cargo xtask compat --all-targets

# Validate worktree and dependency resolution without running full test suites
cargo xtask compat --dry-run

# Output machine-readable JSON
cargo xtask compat --json
```

Detailed test outputs and compiler logs are saved to `target/compat/logs/<consumer>-<mode>.log`. A structured summary is written to `target/compat/report.json`.

## Report Schema (`target/compat/report.json`)

```json
{
  "version": "1.0",
  "candidate_revision": "778078e8e3d024f73fc68efab754e2d9e64257ad",
  "candidate_is_dirty": false,
  "timestamp": "1791701891",
  "toolchain": {
    "rustc": "rustc 1.99.0 (b940084d7 2026-09-28)",
    "cargo": "cargo 1.99.0 (5f94df478 2026-08-27)",
    "target": "aarch64-apple-darwin"
  },
  "overall_status": "PASS",
  "consumers": {
    "vectorcraft": {
      "repository": "vectorcraft",
      "target_package": "vectorcraft-ui-egui",
      "consumer_commit": "389f75fa7be31f24200f9024e1d3e5b2f69f6318",
      "consumer_is_dirty": false,
      "original_dependency_pin": "craft-ui v0.1.0 (https://github.com/storytold/craft-libs?rev=5c874d51...)",
      "candidate_revision": "778078e8e3d024f73fc68efab754e2d9e64257ad",
      "candidate_is_dirty": false,
      "resolved_candidate_dependency": "craft-ui v0.1.0 (.../crates/ui)",
      "baseline_command": "cargo test -p vectorcraft-ui-egui --lib",
      "baseline_exit_code": 0,
      "baseline_result": "PASS",
      "baseline_duration_seconds": 32.89,
      "candidate_command": "cargo test -p vectorcraft-ui-egui --lib --config patch...",
      "candidate_exit_code": 0,
      "candidate_result": "PASS",
      "candidate_duration_seconds": 25.10,
      "status": "COMPATIBLE",
      "total_duration_seconds": 58.12,
      "log_file": ".../target/compat/logs/vectorcraft-candidate.log",
      "notes": null
    }
  }
}
```
