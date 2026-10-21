# Consumer Compatibility Verification Recipe

To validate proposed changes in `craft-libs` against downstream consuming applications (such as PhotoCraft, VectorCraft, DesignCraft, or PdfCraft) without mutating committed lockfiles or working trees, follow this 5-step recipe.

## The 5-Step Recipe

### 1. Isolated Worktree
Create a detached Git worktree from the consumer repository so that local development files and committed lockfiles remain unmodified:

```sh
cd /path/to/consumer-repo
git worktree add --detach /tmp/compat-wt-consumer HEAD
cd /tmp/compat-wt-consumer
```

### 2. Baseline Test Run
Run the consumer's test suite against its committed dependencies to establish baseline status:

```sh
cargo test --locked --no-fail-fast -p <target-package>
```

Record whether all tests pass or if existing failures are present on the committed branch.

### 3. Complete Dependency Patching
Identify all `craft-libs` crates pinned in the consumer's `Cargo.lock` (for example, `craft-ui`, `craft-atomic`, `craft-settings`). Create a `.cargo/config.toml` in the worktree root patching every pinned crate to your candidate `craft-libs` tree:

```toml
[patch."https://github.com/storytold/craft-libs"]
craft-ui = { path = "/path/to/craft-libs/crates/ui" }
craft-atomic = { path = "/path/to/craft-libs/crates/atomic" }
craft-settings = { path = "/path/to/craft-libs/crates/settings" }
```

Ensure file paths use forward slashes (`/`) across all platforms to prevent path escaping issues on Windows.

### 4. Provenance Check
Verify that Cargo actually resolves the candidate paths rather than remote Git pins:

```sh
cargo tree --offline --workspace -i craft-ui
```

Confirm that the root package source exactly matches the full local candidate path (`/path/to/craft-libs/crates/ui`).

### 5. Candidate Test Run & Teardown
Execute the test suite against the candidate revision:

```sh
cargo test --no-fail-fast -p <target-package>
```

Compare candidate test outcomes against baseline:
- **`COMPATIBLE`**: Baseline tests pass and candidate tests pass.
- **`CANDIDATE_REGRESSION`**: Candidate fails tests that passed in baseline, or introduces additional failures beyond baseline.
- **`BASELINE_FAILURE`**: Identical failures occur in both baseline and candidate runs (pre-existing defect in consumer).
- **`FIXED_IN_CANDIDATE`**: Baseline fails but candidate passes.

Clean up the temporary worktree when finished:

```sh
cd /path/to/consumer-repo
git worktree remove --force /tmp/compat-wt-consumer
git worktree prune
```

---

## Automated Validation via xtask

The `cargo xtask compat` command automates this entire recipe:

```sh
# Validate against PhotoCraft and VectorCraft
cargo xtask compat --consumers photocraft,vectorcraft

# Validate against a specific consumer directory
cargo xtask compat --consumer ../designcraft --target-pkg designcraft-ui-egui

# Validate against a specific craft-libs commit or branch
cargo xtask compat --revision HEAD --consumers vectorcraft
```

The runner reads the lockfile from the recorded consumer commit and creates detached worktrees in an exclusively reserved per-run directory under the system temporary directory (`craft-compat-worktrees/`). Worktrees outside the candidate workspace prevent standalone consumers from inheriting its workspace membership or Cargo configuration. Baseline tests use committed pins with `--locked`; candidate tests use complete patches for pinned crates, including vendor crates under `[patch.crates-io]`. Existing consumer Cargo settings are retained. Conflicting patch tables fail closed and require manual reconciliation. Empty dependency coverage is rejected.

Both phases use `--no-fail-fast` to run all Cargo test executables, including integration tests and doctests for the selected package. Counts are aggregated across harnesses. Failure identities include the harness ordinal so identically named tests in different executables do not alias. Missing baseline test identities, fewer completed harnesses, interrupted harnesses, or incomplete failure-name extraction reject verification. Harness reordering or renaming can conservatively reject a comparison; the runner does not infer equivalence. Custom harness output that does not follow libtest conventions is unsupported.

Provenance queries use `cargo tree --offline --workspace -i <crate>` and require an exact full candidate path in the root package source, after slash and Windows verbatim-prefix normalization. A path containing `github.com/storytold` is a valid local path. Remote URLs, matching directory basenames, path prefixes, and matches in dependent lines cannot verify provenance. Required dependencies must already be cached for this offline query; missing cache entries report an infrastructure failure.

`HEAD` tests the current working tree and reports whether it is dirty. Any explicit revision other than the literal `HEAD` gets its own detached candidate worktree, even when it resolves to the same commit. Each run owns separate candidate and consumer paths; it refuses to replace existing worktrees.

### Reports and exit status

```sh
cargo xtask compat --consumer ../vectorcraft --target-pkg vectorcraft-ui-egui --json > report.json
cargo xtask compat --consumer ../vectorcraft --target-pkg vectorcraft-ui-egui --output report.json
```

`--json` writes one raw JSON document to stdout; progress and diagnostics go to stderr. `--output` writes the same report to a file. Missing, empty, or option-shaped values for value-taking CLI options are rejected before execution.

`INFRASTRUCTURE_FAILURE`, `INCOMPLETE_PATCH_COVERAGE`, and `VACUOUS_EXECUTION_REJECTED` are unverified results. A baseline build failure takes precedence over attributing a failure to the candidate. Named new candidate test failures remain regressions even if total failure counts are unchanged. A candidate-only build failure after a viable baseline fails as a candidate regression; compiler, dependency, and environment causes may still require diagnosis.

A missing requested consumer makes the invocation fail even if other consumers pass. With `--skip-baseline`, a passing candidate is `SKIPPED`, not a completed compatibility comparison, and cannot yield success. Zero completed comparisons always fail. The selected target package is the test scope, not the entire consumer workspace; choose additional packages in separate invocations when needed.

### Cleanup and interruption

Normal completion explicitly removes only worktrees registered by this run and records removal failures in the JSON `cleanup_errors` array, making the result fail. RAII cleanup also runs on ordinary Rust error returns. Worktrees are never recursively deleted as a fallback when Git removal fails.

There is no SIGINT/SIGTERM handler; abrupt termination can leave directories and registrations behind. After confirming a run has stopped, inspect `git worktree list` in both repositories, remove that run's worktrees with `git worktree remove --force <path>`, then prune stale registrations with `git worktree prune`. Git pruning removes stale registrations, not active worktree directories. Never remove paths belonging to another active run.

The runner executes code from trusted consumer and candidate repositories with the invoking user's permissions; Git worktrees provide checkout isolation, not a sandbox. No automatic compatibility CI workflow is added by this PR.
