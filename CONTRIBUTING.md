# Contributing to `craft-libs`

Welcome to `craft-libs`, the shared Rust libraries monorepo powering the 10 applications in the Storytold Craft creative suite.

This document outlines the architectural standards, development workflow, and verification protocol required for all contributions.

---

## 1. Principles & Non-Negotiables

Every contribution to `craft-libs` must satisfy six core invariants:

1. **Never Crash (`never-crash.md`)**:
   In shipped library code, no panics, unwraps, expects, todos, or unreachables are permitted. Every crate's root (`src/lib.rs`) must enforce:

   ```rust
   #![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
   ```

   All failure conditions must be returned explicitly through `Result<T, E>` or `Option<T>`.
2. **Zero App Code**:
   Crates in `craft-libs` must serve multiple applications and **must not depend on any application crate** (e.g. PhotoCraft, VectorCraft). Keep shared crates general, decoupled, and headless.
3. **Unsafe Code Forbidden**:
   Unsafe code is strictly forbidden across the entire workspace:

   ```rust
   #![forbid(unsafe_code)]
   ```

4. **Clean-Room Licensing**:
   Dual-licensed under MIT OR Apache-2.0. Clean-room implementations only. Work exclusively from published specifications (ISO 32000, ICC, SMPTE, Unicode, W3C) and observable behaviour. Never copy proprietary code, shaders, or assets from Adobe, Autodesk, Avid, or Microsoft. Never copy or link GPL/LGPL/AGPL code.
5. **Never Break WASM**:
   Every crate must compile cleanly for WebAssembly targets:

   ```bash
   cargo check --target wasm32-unknown-unknown
   ```

   Platform-specific I/O or threading must be guarded with `#[cfg(not(target_arch = "wasm32"))]` and provided with non-panicking fallback behavior.
6. **Toolchain & Standards**:
   - **Rust Edition**: 2024
   - **Cargo Resolver**: Version 3
   - **MSRV**: 1.95

---

## 2. Defensive Engineering & Hostile Input Handling

Shared library code processes untrusted files, foreign streams, IPC commands, and user inputs:

- **Safe Indexing & Slicing**: Use `.get()` rather than direct indexing (`v[i]`). Slice strings only at valid character boundaries (`is_char_boundary`).
- **Checked Arithmetic**: Use `checked_*` or `saturating_*` arithmetic for lengths, counts, byte offsets, and media ticks. Prevent integer overflow, divide-by-zero, and `NaN`/`inf` float conversions.
- **Resource Limits**: Cap memory allocations and loop iterations sized by untrusted data to avoid out-of-memory denial of service. Bound recursion on nested structures.
- **Concurrency Safety**: Handle mutex lock poisoning with `lock().unwrap_or_else(PoisonError::into_inner)`. Treat thread joins as fallible results. Real-time media timing paths must never block or allocate.
- **Regression Proofs**: Every bug or crash fix must include a small synthetic regression test that failed prior to the fix.

---

## 3. Asset & Licensing Governance

- **Attribution Policy**: Every bundled asset (fonts, icons, palettes, test vectors) must have an entry in `ATTRIBUTION.md` detailing path, author, source URL, and license.
- **Fonts Never Committed**: Font files are never committed to this repository. Fonts live in [`storytold/craft-fonts`](https://github.com/storytold/craft-fonts) and are referenced via the optional `CRAFT_FONTS_DIR` build variable.
- **Test Corpora**: Never commit large multi-megabyte binary fixtures (PSDs, AI files, video clips) directly to git. Pinned test corpora live in [`storytold/photocraft-corpus`](https://github.com/storytold/photocraft-corpus). Use programmatic synthetic generators and property tests (`proptest`) for in-tree tests.
- **Brand Assets**: Trademarks and logos in `docs/brand/` are proprietary and protected under `docs/brand/LICENSE-brand.txt`.

---

## 4. Monorepo Layout

```
craft-libs/
├── crates/             # Shared library crates
│   ├── atomic/         # craft-atomic: crash-safe atomic write & replace
│   ├── print/          # craft-print: cross-platform print discovery & submission
│   ├── quit/           # craft-quit: native macOS termination & lifecycle unhooks
│   ├── time/           # craft-time: exact media timebase and SMPTE timecode
│   └── ui/             # craft-ui: headless UI catalogs, docking, themes, widgets
├── docs/               # Architectural RFCs, shared contracts, and audit reports
├── xtask/              # Internal development automation runner
│   └── src/
│       ├── ci.rs       # Format, Clippy, and test runner
│       ├── compat.rs   # Isolated cross-application compatibility runner
│       └── main.rs
├── AGENTS.md           # Authoritative agent operating instructions
├── Cargo.toml          # Workspace root manifest
└── CONTRIBUTING.md     # This contributor guide
```

### Adding a New Crate

1. Create directory `crates/<name>`.
2. Name the crate `craft-<name>`.
3. Inherit workspace package metadata and lints in `crates/<name>/Cargo.toml`:

   ```toml
   [package]
   name = "craft-<name>"
   version.workspace = true
   edition.workspace = true
   license.workspace = true
   repository.workspace = true

   [lints]
   workspace = true
   ```

4. Register the crate in the root `Cargo.toml` under `[workspace.members]` and `[workspace.dependencies]`.

---

## 5. Contribution Workflow

### Step 1: Propose or Align on Shared Capabilities

Before writing substantial new shared primitives, ensure the capability is needed across at least two Craft applications. Open an RFC or tracking issue referencing:

- The target consuming applications (e.g. PhotoCraft, VectorCraft, FilmCraft).
- The duplicate implementations being superseded.
- The failure model and cross-platform boundaries.

### Step 2: Implement with Robust Error Handling

- Favor zero-allocation failure paths.
- Avoid hidden I/O side effects or implicit background threads.
- Document boundary differences from existing consumer applications plainly.

### Step 3: Write Comprehensive Tests

- **Unit Tests**: Place in `tests/` or submodules, exercising boundary cases (`i64::MIN`, `i64::MAX`, empty strings, missing folders).
- **Differential Tests**: When replacing application logic, add tests comparing the shared implementation against real test vectors from the consuming apps.
- **Property-Based Testing**: Use `proptest` for invariant verification (e.g. arithmetic monotonicity, timecode bijections).

### Step 4: Run Local CI Checks

Before submitting a pull request, run the workspace CI suite:

```bash
# Run format, clippy (-D warnings), and all workspace tests
cargo xtask ci

# Verify WebAssembly compatibility
cargo check --target wasm32-unknown-unknown
```

Every check must pass with 0 errors and 0 warnings.

### Step 5: Test Cross-Application Compatibility

When modifying a crate consumed by downstream applications, run the compatibility runner:

```bash
cargo xtask compat
```

The runner:

- Validates changes in ephemeral isolated worktrees without altering consumer checkouts or lockfiles.
- Captures uncommitted and untracked changes into a snapshot commit.
- Enforces non-vacuous success (runs must execute real comparisons).

---

## 6. Tickets, PRs, Commits & Merging Standards

To maintain high velocity and engineering clarity across the Craft ecosystem, all contributions follow structured git and communication protocols.

### 6.1 Issue & Ticket Standards ("Claim Before You Start")
- **Claim Before You Start**: Always comment on an existing issue or open a proposal before starting work. Unclaimed issues are frequently picked up within minutes.
- **Ticket Structure**:
  - **Problem Statement**: Describe the failure scenario, missing capability, or cross-application gap.
  - **Proposed Architecture**: Identify affected crates, API changes, and potential consumer blast radius.
  - **Acceptance Criteria**: Include a concrete checkbox list (`- [ ]`) specifying required tests and outcomes.

### 6.2 Branch Naming & Git Commit Standards
- **Branch Naming**: Use prefixes describing the change intent:
  - `feat/<short-name>` (e.g., `feat/craft-atomic`, `feat/timecode-display`)
  - `fix/<short-name>` (e.g., `fix/lock-poisoning-recovery`)
  - `docs/<short-name>` (e.g., `docs/contributor-guidelines`)
  - `refactor/<short-name>`
  - `perf/<short-name>`
- **Commit Message Conventions**:
  - **Subject Line**: 50–72 characters, written in the imperative mood, present tense (*"Add..."*, *"Fix..."*, *"Extract..."*, *"Preserve..."*, not *"Added"* or *"Fixes"*). Capitalize the first letter; no trailing period.
  - **Issue References**: Reference tracking issues in the subject or body of the target/squash commit: `(refs #37)` or `(fixes #7)`. **Never reference issues in intermediate scratch commits** pushed to remote branches; GitHub records an immutable timeline cross-reference for every pushed commit containing an issue tag, cluttering the issue thread even after the branch is squashed.
  - **Commit Body**: Explain the *why* and the *what*, separating the subject from the body with a blank line. Wrap body lines at 72–80 characters.
  - **Attribution**: Author must be `Zach Shallbetter <zachshallbetter@icloud.com>`. Strictly no `Co-Authored-By` AI trailers, no "generated by AI" footers, and no agent session identifiers.

### 6.3 Pull Request Structure & Status Honesty
PR descriptions must be structured, technical, and evidence-driven:
- **Title**: Imperative summary matching the intended squash-merge title: `Verb in imperative mood: short description (closes #XX)`.
- **Summary**: Concise explanation referencing the target issue (`Closes #XX` or `Refs #XX`).
- **What Changes**: Bulleted breakdown of architectural, API, and behavioral changes.
- **Verification Evidence**: Report exact test execution results:
  - `cargo fmt --check`: clean exit 0
  - `cargo clippy --workspace --all-targets -- -D warnings`: clean exit 0
  - `cargo test --workspace`: exact test count (`N passed; 0 failed`)
  - `cargo check --target wasm32-unknown-unknown`: clean exit 0
- **Four-Tier Claim Taxonomy**: Never conflate maturity:
  $$\text{Documented} \neq \text{Implemented} \neq \text{Tested} \neq \text{Empirically Validated}$$
  Clearly distinguish software invariants (e.g. kernel flush dispatch order) from physical guarantees (e.g. power-loss durability).

### 6.4 Squash Merging & Git History Protocol
- **Default Strategy**: Feature and bugfix pull requests are integrated into `main` using **Squash and Merge**.
- **Rationale**: Keeps `main` linear, clean, and easily bisectable with `git bisect`. Each commit on `main` represents a complete, green, atomically verified change.
- **Squash Commit Format**:
  - **Subject**: Defaults to `<PR Title> (#<PR Number>)`
  - **Body**: Curated PR summary and issue closure tag (`Closes #XX`). Scratch commits (*"wip"*, *"fix typo"*, *"address review"*) are squashed out of history.
- **Multi-Commit Rebase Exception**: Reserved strictly for large, multi-crate architectural extractions where each individual commit compiles independently, passes full CI, and represents a distinct logical milestone.

### 6.5 Review Comments & Technical Communication Style
- **Evidence-First**: Ground review feedback in exact file paths, line numbers, compiler outputs, or minimal reproduction cases.
- **Grounded References**: Never make unreferenced claims like *"the work I have been doing"*. Always link directly to the exact pull request, commit, or issue (e.g. `[storytold/craft-libs#9](https://github.com/storytold/craft-libs/pull/9)`).
- **Constructive & Specific**: When flagging an issue, suggest the preferred idiom or typed alternative (e.g., suggesting `.get()` over direct indexing or proposing an explicit error enum variant).
- **Direct Technical Voice**: Use clean, human engineering prose without promotional language, conversational fluff, or assistant identity markers. Avoid stylistic tells like em-dash (`—`) crutches and deep bullet hierarchies.
- **Small-Batch Landings**: Support small, focused, bisectable pull requests over massive multi-feature monoliths to preserve bisectability and prevent cross-PR regressions.


---

## 7. Verification Cheatsheet

| Command | Purpose |
| :--- | :--- |
| `cargo fmt --check` | Verify formatting across all crates and xtask. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Enforce zero compiler warnings and strict clippy lints. |
| `cargo test --workspace` | Execute all workspace unit and integration tests. |
| `cargo check --target wasm32-unknown-unknown` | Ensure shared crates compile cleanly for WebAssembly. |
| `cargo xtask ci` | Run standard format, clippy, and test pipeline. |
| `cargo xtask compat` | Test proposed shared crate against consuming applications. |

---

## 8. Common Contributor Pitfalls ("Getting Stuck?")

Practical advice for smooth contributions across the Craft repositories:

- **Claim Before You Start**: Always comment on an existing issue or open a proposal before starting work. The Craft repositories move rapidly, and unclaimed issues may be picked up by other contributors.
- **First PR Approval Gate**: For first-time contributors, GitHub Actions requires a repository maintainer to click *"Approve and run"*. This is standard GitHub security policy, not an infrastructure defect.
- **Checking Red CI Against Main**: If a test fails in CI that seems unrelated to your changes, check whether upstream `main` exhibits the same failure before debugging your branch, and mention it in your PR.
- **Previewing Merge Conflicts**: Run `git merge-tree --write-tree <branch> upstream/main` before pushing to preview potential merge conflicts without modifying your working branch.
- **Private Standards (`craftrules`)**: Repository documents reference `craftrules` for high-level ecosystem doctrine. Since `craftrules` is private, external contributors can ask maintainers directly on issue threads for the relevant specifications or clean-room guidance.

---

## 9. Architecture & Reference Documentation

For detailed specifications and contracts, consult the documents in `docs/`:

- **[CI & Quality Gates Protocol](docs/ci-and-verification.md)**: Quality gate sequence, WASM checks, multi-platform matrix, and compatibility runner.
- **[Type System Contracts & Domain Invariants](docs/type-system-contracts.md)**: Signed media time (`Tick`), safe indexing, checked arithmetic, and error hierarchies.
- **[Code Style & Quality Lints](docs/code-style-and-lints.md)**: Formatting (`rustfmt.toml`), clippy lint suites, unsafe prohibition, and clean-room policy.
- **[Shared Services Architecture](docs/shared-services-architecture.md)**: `craft-atomic` persistence, `craft-settings` TOML backups, and `craft-quit` lifecycle.
- **[Cross-Application Compatibility Contract](docs/compatibility-contract.md)**: Worktree isolation, dirty snapshotting, and regression classification.
- **[UI Docking System](docs/ui-docking.md)**: Surface geometry, tab strips, accordions, and workspace serialization.
- **[Theme Infrastructure](docs/theme-infrastructure.md)**: Design tokens, contrast palettes, and widget state styling.

---

## 10. Community & Contributor Roles (Discord)

Contributors across the Craft ecosystem can claim verified Discord roles recognizing their contributions:

### Claiming Your Role
1. In the Storytold Discord server, navigate to the contributor verification channel.
2. Click **Verify with GitHub** via the `TicketCraft` bot.
3. Authorize read-only GitHub identity confirmation.
   - The bot only reads your public GitHub username to tally merged pull requests across all `storytold/*` repositories.
   - It requests no write permissions and cannot modify your account.

### Role Tiers
- **@Junior Contributor**: Conferred automatically upon having **at least 1 merged pull request** into any `storytold/*` repository.
- **@Contributor**: Conferred automatically upon having **at least 10 merged pull requests** (or a major feature extraction) across the suite.
- **@Senior Contributor**: Conferred manually by the core team for substantial architectural leadership.

Your role upgrades automatically as additional pull requests are merged.


