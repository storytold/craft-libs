# Shared Crate Contract (`CRAFT-CONTRACT-001`)

This document defines the mandatory architectural invariants, failure models, platform behaviors, and coordination policies for all shared libraries in the Craft ecosystem (`craft-libs`).

---

## 1. Non-Negotiable Invariants

Every crate in `craft-libs` must satisfy these substrate guarantees:

1. **Zero Application Dependencies**: Shared crates must be general-purpose, reusable across multiple applications, and must **never depend on any application crate** (`photocraft`, `vectorcraft`, etc.).
2. **Clean-Room Implementation**: All code is original, dual-licensed under `MIT OR Apache-2.0`. No proprietary code or assets.
3. **Memory Safety & No Unsafe Code**: `#![forbid(unsafe_code)]` across the entire workspace.
4. **Never Crash Policy**: In production code, panics and unhandled edge cases are forbidden. Every crate's `src/lib.rs` must start with:
   ```rust
   #![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
   ```
5. **Toolchain Baseline**: Rust Edition 2024, Cargo Resolver 3, Minimum Supported Rust Version (MSRV) 1.95.

---

## 2. Failure Semantics & Error Modeling

Every shared crate must publish an explicit failure model. Silent numerical corruption, hidden fallbacks, or swallow-and-continue patterns are strictly prohibited.

### 2.1 Two-Tier API Structure

When designing operations that can fail, crates must provide:
1. **Fallible APIs (`Result<T, E>` or `Option<T>`)**: The primary interface for safety-critical caller logic where failure must be handled or bubbled up.
2. **Total APIs (Operator Overloads & Saturated Fallbacks)**: Allowed only when necessary for standard trait implementations (such as `std::ops::Add`, `Div`), provided:
   - The fallback behavior is explicitly documented in the trait implementation.
   - Saturated bounds (`T::MAX` / `T::MIN`) or mathematical zero identities are used rather than arbitrary dummy values.
   - Arithmetic operations that overflow do not panic under `#![deny(clippy::panic)]`.

### 2.2 Post-Failure State Guarantees

Every fallible operation must document what state remains after a failure occurs:
- **Clean Invariant (All-or-Nothing)**: If an operation fails, the environment or object must remain in its pre-operation state.
  - *Example (`craft-atomic`)*: If staging, payload syncing, or rename fails, the destination file remains bit-for-bit identical to its state prior to the operation, and temporary staging files are unlinked.
- **Read-Only / Safe Degradation**: If persisted data is newer than the running code or partially corrupted, the subsystem must degrade safely rather than overwrite or corrupt data.
  - *Example (`craft-settings`)*: A settings file with a future `schema_version` opens in read-only mode so older application builds never truncate unknown fields.
- **Anti-Masking Rule**: Errors must never be hidden by returning empty collections, silent zeroes, or unlogged success codes. Upstream root causes must be preserved in error types.

---

## 3. Platform Behavioral Contracts

Operating systems differ in file locking, filesystem atomicity, window management, and signal dispatch. Shared crates that interact with OS boundaries must adhere to the following platform-specific contracts.

### 3.1 Atomic File Operations (`craft-atomic`)
| Dimension | Specification |
| :--- | :--- |
| **Atomicity** | Replacement must be atomic at the filesystem directory level via `std::fs::rename`. Replacing an existing file is an all-or-nothing pointer swap visible to other processes. |
| **Durability** | Atomicity $\neq$ Durability. File data must be explicitly flushed to physical media (`sync_all` on Windows/Linux, `F_FULLFSYNC` `fcntl` on macOS) *before* the directory rename. On Unix, the parent directory entry must be synced *after* the rename to guarantee power-loss persistence. |
| **Volume Boundaries** | Atomic rename cannot cross filesystem volume boundaries (`EXDEV` on Unix). Temporary staging files must reside on the exact same filesystem volume as the target file. Mount-point verification must resolve symlinks via canonical paths. |
| **Collision Resistance** | Staging filenames must use process-isolated, cryptographically random suffixes (`.tmp.<pid>.<rand>`) to prevent collisions during concurrent writer execution. |

### 3.2 Configuration & Settings (`craft-settings`)
| Dimension | Specification |
| :--- | :--- |
| **Corruption Isolation** | A malformed configuration file must never prevent application startup. Unparseable syntax must be quarantined to `config.broken-<timestamp>.toml`, preserving user work for manual recovery while resetting the active store to defaults. |
| **Partial Degradation** | When individual sections fail validation, only the invalid section falls back to defaults. Valid sibling sections must survive intact. |
| **Hand-Edit Preservation** | TOML document layout, comments, and unknown keys must be preserved across programmatic updates via document-preserving ASTs (`toml_edit`). |

### 3.3 Application Lifecycle & Shutdown (`craft-quit`)
| Dimension | Specification |
| :--- | :--- |
| **Unified Quit Pipeline** | Red window-close button and native application menu Quit (⌘Q) must funnel through the identical lifecycle pipeline: dispatching `egui::ViewportCommand::Close`. |
| **Unsaved-Change Cancellation** | Applications must have unconditional veto power over termination. If the application has unsaved changes, it emits `ViewportCommand::CancelClose`. Both window close and ⌘Q must respect this cancellation and remain interactive. |
| **Guaranteed Persistence** | When close is accepted, termination must dispatch through the platform's native path (`-[NSApplication terminate:]` on macOS) scheduled on the main event queue. In winit/eframe, `applicationWillTerminate:` guarantees `App::on_exit` executes to flush settings and document states before process exit. |

---

## 4. Multi-Tier Dependency Governance

As shared crates build on one another (e.g. `craft-atomic` $\to$ `craft-settings` $\to$ consumer applications), the following coordination protocol applies:

1. **Topological Order**: Upstream foundation crates (`craft-atomic`, `craft-time`) must stabilize and merge before second-order crates (`craft-settings`) merge into `main`.
2. **Lockstep Downstream Verification**: Any proposed breaking or semantic change to an upstream crate must be tested against all direct internal consumers (`craft-libs` crates) and all pinned consumer applications in the Craft ecosystem.
3. **Deprecation & Migration**: Shared APIs cannot be removed abruptly. They must follow a deprecation cycle:
   - Deprecated with alternative noted via `#[deprecated]`.
   - Consumer application pins updated across the 10 Craft applications.
   - Deprecated surface pruned only after all 10 applications have merged updated pins.
