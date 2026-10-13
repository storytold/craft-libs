# Code Style & Engineering Standards

This document defines the authoritative coding conventions, compiler lint requirements, and formatting rules across `craft-libs`.

---

## 1. Toolchain & Workspace Standards

- **Rust Edition**: 2024
- **Cargo Resolver**: Version 3
- **Minimum Supported Rust Version (MSRV)**: 1.95
- **Monorepo Crate Inheritance**: New crates inherit workspace package properties and lints:
  ```toml
  [package]
  edition.workspace = true
  license.workspace = true
  rust-version.workspace = true

  [lints]
  workspace = true
  ```

---

## 2. Formatting & Editor Standards (`rustfmt.toml`, `.editorconfig`)

All Rust code must adhere to the ecosystem formatting configuration:

```toml
max_width = 150
use_small_heuristics = "Max"
```

- **Imports**: Group imports cleanly: standard library (`std::`), external crates, internal workspace crates, and local module items (`super::`, `self::`).
- **Trailing Commas**: Prefer trailing commas in multiline struct definitions, enum variants, and function arguments to minimize git diff churn.
- **Whitespace & Indentation (`.editorconfig`)**:
  - Rust source files (`*.rs`): 4-space indentation.
  - Manifests and documentation (`*.toml`, `*.json`, `*.yml`, `*.md`): 2-space indentation.
  - Mandatory trailing newline and whitespace trimming across all formats.

---

## 3. Strict Quality Lints & Never-Crash Policy

### Workspace Compiler Lints (`Cargo.toml`)

Centralized workspace lints apply across all crates via `[lints] workspace = true`:

```toml
[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
dbg_macro = "warn"
todo = "warn"
needless_borrow = "warn"
match_same_arms = "warn"
empty_line_after_outer_attr = "warn"
filter_map_next = "warn"
flat_map_option = "warn"
manual_assert = "warn"
manual_string_new = "warn"
needless_continue = "warn"
```

### Crate-Level Never-Crash Lints

Every crate root (`src/lib.rs`) must explicitly enforce the Never-Crash compiler lints on shipping library code:

```rust
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
```

Workspace-wide pedantic and correctness expectations:
- `#![deny(clippy::indexing_slicing)]` on all input parsing layers.
- Safe arithmetic bounds or checked math on media tick calculations.
- Clamping and explicit bounds on numeric conversions.

---

## 4. Unsafe Restrictions & Concurrency Safety

- **Unsafe Code Forbidden**: In shared crates, unsafe code is forbidden (`#![forbid(unsafe_code)]`).
- **Graceful Lock Recovery**: Mutex and RwLock poisoning must never crash a process:
  ```rust
  let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
  ```
- **Thread Management**: Background workers, filesystem flushes, and export jobs must yield thread handles wrapped in fallible `std::thread::JoinHandle` results rather than unmonitored detached threads. Real-time media timing paths must never block or allocate in steady state.

---

## 5. Defensive Implementation Invariants

- **Bounds Checks Before Allocation**: Never allocate vectors or buffers sized directly from untrusted input files without a verified ceiling.
- **Cycle Detection**: Traversal of parent/child layers, clip groupings, or document object graphs must maintain a depth budget or visited set to detect cycles.
- **Synthetic Regression Proofs**: Every bug or crash fix must include a synthetic regression test that reproduced the failure prior to the patch.

---

## 6. Clean-Room Implementation & Proprietary Name Prohibition

- **Zero External Proprietary Mentions**: Code, comments, identifier names, docstrings, and commit messages must never reference proprietary commercial software outside the ArtCraft and Storytold ecosystem.
- **Domain-First Terminology**: Refer to concepts by their generic domain definitions (such as "layer blending", "cubic Bézier path", "soft-proof transform", "gamut mapping", "ICC profile evaluation") or open international standards (ISO 32000, ICC.1, SMPTE, Unicode, W3C).
- **Prohibited Sources**: Never inspect, copy, decompile, or adapt proprietary source code, headers, presets, or shaders from third-party commercial applications. Clean-room engineering proceeds strictly from open public specifications and synthetic black-box test fixtures.
