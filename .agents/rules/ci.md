# Continuous Integration & Verification Protocol

This document defines the authoritative Continuous Integration (CI) standards, verification pipelines, and cross-application compatibility protocols across `craft-libs`.

---

## 1. Shared Library Quality Gates (`craft-libs`)

Every change in `craft-libs` must pass all local and automated quality gates with zero errors and zero warnings before review or merge.

### Gate Sequence

1. **Formatting**:
   ```bash
   cargo fmt --check
   ```
   Enforces consistent line formatting per `rustfmt.toml` (`max_width = 150`, `use_small_heuristics = "Max"`).

2. **Clippy Lint Suite**:
   ```bash
   cargo clippy --workspace --all-targets -- -D warnings
   ```
   Enforces compiler warning zero-tolerance, Never-Crash linting, and forbidden indexing on input-derived data.

3. **Workspace Test Suite**:
   ```bash
   cargo test --workspace
   ```
   Executes all unit, integration, and visual regression tests across crates (`craft-ui`, `craft-atomic`, `craft-settings`, `craft-quit`, `craft-print`, `craft-cms`).

4. **WebAssembly Compilation**:
   ```bash
   cargo check --target wasm32-unknown-unknown
   ```
   Guarantees that shared crates do not accidentally introduce non-WASM system calls or thread APIs without `#[cfg(not(target_arch = "wasm32"))]` conditional compilation and non-panicking fallbacks.

---

## 2. Cross-Application Compatibility Protocol (`cargo xtask compat`)

Shared library crates in `craft-libs` must never regress consuming applications (`photocraft`, `vectorcraft`, `filmcraft`, `effectcraft`, `lightcraft`, `pdfcraft`, `cadcraft`, `soundcraft`, `wordcraft`, `deckcraft`, `gridcraft`, `figcraft`, `designcraft`).

### The Compatibility Principle
Internal workspace tests verify only that `craft-libs` is self-consistent; they do not exercise consumer applications. Consumer CI pipelines pin immutable Git revisions and never build against unmerged branches. The cross-application compatibility runner (`cargo xtask compat`) bridges this gap.

### Running Compatibility Verification

```bash
# Validate against default consumer pair (PhotoCraft & VectorCraft)
cargo xtask compat

# Validate against specific candidate applications
cargo xtask compat --apps photocraft,designcraft

# Validate against all 13 consumer applications
cargo xtask compat --apps all
```

### Compatibility Invariants
1. **Worktree Isolation**: Baselines and candidates compile inside temporary ephemeral worktrees, guaranteeing that local repository state remains untouched.
2. **Provenance Reporting**: The runner captures exact Git commit SHAs, active feature sets, and toolchain versions.
3. **Non-Vacuous Verification**: Reject test runs where zero comparison tests execute.
4. **Regression Separation**: Distinguish pre-existing baseline failures on committed dependency pins from regressions introduced by the proposed shared candidate.

---

## 3. Conformance Verification Ladder

Verification in `craft-libs` is layered so that syntactic success is never mistaken for operational or cross-application validation:

- **C0 (Structural)**: Code formatting (`cargo fmt --check`), lint conformance (`cargo clippy --workspace --all-targets -- -D warnings`), and `#![forbid(unsafe_code)]` pass with zero errors and zero warnings.
- **C1 (Context & Specification)**: In-tree documentation, UI element maps (`docs/ui-elements-map.md`), and typed schemas (`docs/ui-schema.md`) reflect active crate implementations without drift.
- **C2 (Target Compilation)**: Clean compilation across all supported targets, including native host toolchains and WebAssembly (`wasm32-unknown-unknown`).
- **C3 (Operational Suite)**: Complete execution of all unit, workflow, and visual regression test suites (`cargo test --workspace`) with active assertions verified.
- **C4 (Cross-Application Compatibility)**: Ephemeral candidate validation against consuming Craft applications via `cargo xtask compat`, proving no consumer regressions on proposed revisions.
- **C5 (Empirical Resilience)**: Synthetic regression test proofs for every defect fix, verified failure capability prior to gate adoption, and explicit boundaries separating software ordering from hardware durability claims.
