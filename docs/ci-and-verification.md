# Continuous Integration & Quality Gates Protocol

This document defines the quality gates, verification commands, and cross-application compatibility protocols for `craft-libs`.

---

## 1. Quality Gate Sequence

Every pull request and candidate revision in `craft-libs` must pass all local and automated gates with zero errors and zero warnings:

```bash
# 1. Format verification
cargo fmt --check

# 2. Strict clippy compiler lints
cargo clippy --workspace --all-targets -- -D warnings

# 3. Complete workspace test suite
cargo test --workspace

# 4. WebAssembly compilation check
cargo check --target wasm32-unknown-unknown

# 5. Standard xtask CI runner (executes gates 1-3)
cargo xtask ci
```

---

## 2. WebAssembly Target Verification

Shared crates in `craft-libs` are designed to compile for WebAssembly (`wasm32-unknown-unknown`) to support browser-based Craft applications (e.g. PhotoCraft Web):

```bash
cargo check --target wasm32-unknown-unknown
```

### Invariants

- Operating system file I/O, native thread management, and platform event loops must be guarded behind `#[cfg(not(target_arch = "wasm32"))]`.
- Provide graceful, non-panicking unsupported fallback behavior on WASM targets.

---

## 3. Cross-Application Compatibility Protocol (`cargo xtask compat`)

Internal workspace tests (`cargo test --workspace`) verify only that `craft-libs` is self-consistent; they do not exercise consuming applications. Consumer CI pipelines pin immutable Git revisions and never build against unmerged branches.

`cargo xtask compat` bridges this gap:

```bash
# Validate candidate revision against default consumer pair (PhotoCraft & VectorCraft)
cargo xtask compat

# Validate candidate revision against specific consumers
cargo xtask compat --revision <SHA> --consumers photocraft,vectorcraft,filmcraft --consumers-dir ..

# Fast validation mode (targets primary UI crates)
cargo xtask compat --quick
```

### Invariants

1. **Immutable Consumer Repositories**: Never modify committed `Cargo.toml` dependency pins or `Cargo.lock` files in consumer trees.
2. **Worktree Isolation**: Candidate compilations execute in isolated temporary Git worktrees (`target/compat/worktrees/`) that clean up automatically on exit or interruption.
3. **Dirty-Tree Snapshotting**: Uncommitted changes are staged via an isolated temporary Git index (`GIT_INDEX_FILE`), capturing staged, unstaged, and untracked source files while respecting `.gitignore`.
4. **Regression Separation**: Pre-existing failures on the consumer's baseline pin (`BASELINE_FAILURE`) are reported but do not block candidate validation. Any failure introduced by the candidate (`CANDIDATE_REGRESSION`) causes a non-zero exit.
5. **Non-Vacuous Verification**: Runs where 0 tests executed or comparisons were skipped cannot produce a passing status.

---

## 4. Multi-Platform Targets & Matrix

`craft-libs` crates support the diverse platform deployments of the Craft suite:

- **macOS** (`aarch64-apple-darwin`, `x86_64-apple-darwin`): Metal rendering, AppKit menu integration, Touch Bar termination handling.
- **Linux** (`x86_64-unknown-linux-gnu`): Wayland/X11 rendering, GTK3 dialogs.
- **Windows** (`x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`): Win32 native windowing and MSVC ABI.
- **WebAssembly** (`wasm32-unknown-unknown`): WebGPU/WebGL web app runtimes.
- **FreeBSD**: Packaging lint and build compatibility.

---

## 5. Epistemic Claim Taxonomy

All PRs, CI logs, and commit messages must observe the **Four-Tier Claim Taxonomy**:

$$\text{Documented} \neq \text{Implemented} \neq \text{Tested} \neq \text{Empirically Validated}$$

- Stating a feature is *tested* requires active execution of assertions that prove capability to fail on defects.
- Stating *durability* requires distinguishing unit-tested syscall ordering (`sync_file < rename < sync_dir`) from unbuffered physical drive power-loss survival.
