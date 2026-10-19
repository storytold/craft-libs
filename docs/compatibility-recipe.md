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
cargo test --locked -p <target-package>
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
cargo tree --offline -p <target-package> -i craft-ui
```

Confirm that the output references the local candidate path (`/path/to/craft-libs/crates/ui`).

### 5. Candidate Test Run & Teardown
Execute the test suite against the candidate revision:

```sh
cargo test -p <target-package>
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

The runner creates isolated worktrees, checks patch completeness across all pinned crates, validates candidate provenance via `cargo tree`, compares baseline and candidate test counts, and cleans up temporary worktrees on exit.
