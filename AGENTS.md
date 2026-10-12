# AGENTS.md — Craft Suite Shared Libraries (`craft-libs`)

This document defines the authoritative architecture, non-negotiable standards, and verification protocol for autonomous agents and contributors working in `craft-libs`.

The shared ecosystem standards are governed by [craftrules](https://github.com/storytold/craftrules) (`../../craftrules` on local dev environments): review `standards/never-crash.md`, `standards/engineering.md`, and `standards/licensing.md`.

### Reference Documentation
- **[CI & Quality Gates Protocol](docs/ci-and-verification.md)**: Gate sequence, WASM checks, multi-platform matrix, and compatibility runner.
- **[Type System Contracts & Domain Invariants](docs/type-system-contracts.md)**: Signed media time (`Tick`), safe indexing, checked arithmetic, and error hierarchies.
- **[Code Style & Quality Lints](docs/code-style-and-lints.md)**: Formatting (`rustfmt.toml`), clippy lint suites, unsafe prohibition, and clean-room policy.
- **[Shared Services Architecture](docs/shared-services-architecture.md)**: `craft-atomic` persistence, `craft-settings` TOML backups, and `craft-quit` lifecycle.
- **[Cross-Application Compatibility Contract](docs/compatibility-contract.md)**: Worktree isolation, dirty snapshotting, and regression classification.

---


## 1. Non-Negotiable Architecture Invariants

- **Never Crash**: In shipped library code, no panics, no unwraps, no expects, no todos, and no unreachables. Every crate `src/lib.rs` must begin with:
  ```rust
  #![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
  ```
  All errors must be handled through explicit `Result<T, E>` and `Option<T>` returns. Unfinished features return an explicit "unsupported" or "not yet implemented" error.
- **Zero App Code**: Crate code in `craft-libs` must be useful to multiple applications and **must never depend on any application crate** (e.g. PhotoCraft, VectorCraft, DesignCraft). Crates must remain strictly decoupled, headless, and generalizable.
- **Unsafe Code Forbidden**: Strictly forbidden across all workspace crates:
  ```rust
  #![forbid(unsafe_code)]
  ```
- **Clean-Room Licensing**: Dual-licensed MIT OR Apache-2.0. Clean-room implementations only. Work exclusively from public specifications (ISO 32000, ICC, SMPTE, Unicode, W3C) and observable behaviour. Never decompile, disassemble, or copy proprietary code, shaders, or presets from Adobe, Autodesk, Avid, or Microsoft. Never copy or link GPL/LGPL/AGPL code (e.g. Inkscape, lib2geom, LibreDWG).
- **Never Break WASM**: Every shared crate must compile cleanly for WebAssembly:
  ```bash
  cargo check --target wasm32-unknown-unknown
  ```
  File system, process, or platform threading logic must be gated behind `#[cfg(not(target_arch = "wasm32"))]` and provided with graceful, non-panicking unsupported fallbacks on WASM.
- **Toolchain & Standards**: Rust Edition 2024, Cargo Resolver 3, Minimum Supported Rust Version (MSRV) 1.95.
- **Workspace Inheritance**: Every new crate belongs under `crates/<name>`, named `craft-<name>`, and inherits package metadata and lints from the workspace (`license.workspace = true`, `edition.workspace = true`, `[lints] workspace = true`). Every crate must be registered under `[workspace.dependencies]` in the root `Cargo.toml`.

---

## 2. Defensive Engineering & Hostile Input Handling

All inputs from files, foreign protocols, IPC, and user commands are treated as hostile:

- **Safe Slicing & Indexing**: Use `.get()` rather than direct indexing (`v[i]`) on input-derived numbers. Slice strings only at validated character boundaries (`is_char_boundary`).
- **Checked & Saturating Arithmetic**: Use `checked_*` and `saturating_*` arithmetic for lengths, offsets, byte counts, and media ticks. Prevent integer overflow, division by zero, and invalid float-to-integer (`NaN`/`inf`) conversions.
- **Cap Allocations & Iterations**: Cap memory allocations and loop iterations sized by untrusted input to prevent out-of-memory denial of service.
- **Bound Recursion**: Always enforce explicit depth limits on hierarchical or cyclic structures.
- **Concurrency & Lock Safety**: Handle lock poisoning gracefully via `lock().unwrap_or_else(PoisonError::into_inner)`. Treat thread joins as fallible results. Real-time media timing paths must never block or allocate in steady-state operation.
- **Prove Crash Fixes**: Every crash or overflow fix must be accompanied by a synthetic regression test that failed or panicked before the fix.

---

## 3. Asset & Licensing Governance

- **Attribution Policy**: Every bundled asset (icons, test vectors, palettes) must have a corresponding entry in `ATTRIBUTION.md` detailing path, author, source URL, and permissive open-source license.
- **Fonts Never Committed**: Font binaries are never committed to this repository. Fonts live in [`storytold/craft-fonts`](https://github.com/storytold/craft-fonts) and are referenced via the optional `CRAFT_FONTS_DIR` build environment variable.
- **No Large Binary Corpora**: Large test files (multi-megabyte PSD, AI, or media files) belong in [`storytold/photocraft-corpus`](https://github.com/storytold/photocraft-corpus) or sibling corpora, fetched via xtask pins. Use synthetic generators and property tests (`proptest`) for in-tree testing.
- **Brand Protection**: Trademarks and logos in `docs/brand/` are proprietary and protected under `docs/brand/LICENSE-brand.txt`.

---

## 4. Authoritative Verification Commands

Always run and verify these commands before concluding any task:

```bash
# 1. Format check
cargo fmt --check

# 2. Strict workspace Clippy (must pass with 0 warnings)
cargo clippy --workspace --all-targets -- -D warnings

# 3. Complete workspace test suite
cargo test --workspace

# 4. WebAssembly compilation check
cargo check --target wasm32-unknown-unknown

# 5. Standard CI task (runs format, clippy, and tests)
cargo xtask ci

# 6. Cross-application compatibility runner
cargo xtask compat
```

---

## 5. Epistemic Integrity & Claim Taxonomy

Agents must enforce the strict **Four-Tier Claim Taxonomy**:

$$\text{Documented} \neq \text{Implemented} \neq \text{Tested} \neq \text{Empirically Validated}$$

- **Status Honesty**:
  - A feature documented in an RFC or doc comment is *documented*, not *implemented*.
  - A function with passing tests is *tested*, not *empirically validated*.
  - Passing simulated or mock fixtures does not constitute native platform validation.
  - Software tests running on isolated runners do not prove downstream application adoption or physical power-loss durability. Always state the weaker, verified claim.
- **Atomicity vs. Durability Guarantees**:
  - The sequence `sync_file < rename < sync_dir` proves kernel dispatch ordering and atomic namespace transition.
  - Software flushes do not guarantee survival across unbuffered physical disk controllers or sudden power cuts.
  - Staging crash cleanup (`.craft_tmp`) is explicitly delegated to consuming application startup routines; `craft-libs` does not run background cleanup daemons.
- **Architectural RFCs**:
  - Shared contracts and proposals remain proposals until repository maintainers formally merge and adopt them.

---

## 6. Cross-Application Compatibility Protocol (`cargo xtask compat`)

When modifying shared libraries consumed by the Craft application suite:

- **Worktree Isolation**: Always use ephemeral isolated Git worktrees for both baseline and candidate builds. Never modify consumer checkouts, working trees, or committed lockfiles.
- **Dirty-Tree Provenance**: Capture uncommitted source changes using an isolated Git index (`GIT_INDEX_FILE`), including untracked source files while respecting `.gitignore`.
- **Non-Vacuous Success Enforcement**: Reject runs where zero comparisons complete; ensure test assertions actively execute.
- **Distinguish Regressions**: Clearly separate baseline consumer failures (pre-existing issues on committed pins) from candidate regressions (failures introduced by proposed library changes).
- **Exact Provenance**: Report exact resolved Git commit SHAs recorded in `Cargo.lock` rather than branch names or query strings.
- **Small-Batch Landings**: Merges to `main` must land in small, bisectable batches rather than monolithic integration dumps to protect `git bisect` and isolate cross-PR regressions.
- **Issues vs Pull Requests Distinction**: Always verify GitHub endpoints before acting. Issues (e.g. DesignCraft #259, #602) are coordination hubs, not pull requests.

---

## 7. Domain Invariants by Crate

| Crate | Core Invariants & Boundaries |
| :--- | :--- |
| **`craft-atomic`** | Atomic replace and `write_new_with` via same-directory staging. Private Unix permissions (`0o600`). Best-effort Unix directory sync (`sync_dir`). Exponential backoff for Windows sharing/lock violations (`ERROR_SHARING_VIOLATION`/`ERROR_LOCK_VIOLATION`). Rejects non-existent parent directories (`NotFound`). Sweeping orphaned staging files left by abnormal termination is delegated to consuming applications. |
| **`craft-time`** | Exact rational media timebase (`TICKS_PER_SECOND = 254_016_000_000`). Signed arithmetic across `[Tick::MIN, Tick::MAX]`. Saturating division preserving monotonicity for positive and negative divisors. Document intentional differences from downstream applications (e.g. EffectCraft unpadded hours or fallback frame rates). |
| **`craft-quit`** | Native macOS AppKit termination harness and clean window/Touch Bar teardown without uncaught `NSRangeException` or invalidation crashes. Egui event loop close integration. |
| **`craft-ui`** | Headless SVG catalogs, theme infrastructure, dock surfaces, and accessible widget trees. Assets, brand styling, localization, and domain commands remain inside consuming apps. |
| **`craft-settings`** | Shared `config.toml` persistence. Multi-level pre-save validation (`check_disk`). Never overwrite `config.toml.bak` with unvalidated or corrupt data. Preserves damaged files to `config.broken-<unix-ts>.toml` before falling back to defaults. |

---

## 8. The 10 Mainline Craft Consumer Applications

Shared crates in `craft-libs` are consumed by the 10 Craft applications:

1. **PhotoCraft** (`storytold/photocraft`): Raster image editing & manipulation.
2. **VectorCraft** (`storytold/vectorcraft`): Precision vector illustration and geometry.
3. **DesignCraft** (`storytold/designcraft`): Multi-page publication layout and typography.
4. **PdfCraft** (`storytold/pdfcraft`): PDF inspection and document assembly.
5. **LightCraft** (`storytold/lightcraft`): Non-destructive RAW processing and color grading.
6. **FilmCraft** (`storytold/filmcraft`): Non-linear video timeline editing and media playback.
7. **EffectCraft** (`storytold/effectcraft`): Motion graphics, visual effects, and compositing.
8. **CADCraft** (`storytold/cadcraft`): Computer-aided design and parametric drafting.
9. **SoundCraft** (`storytold/soundcraft`): Digital audio workstation, multi-track timing, and DSP.
10. **WordCraft** (`storytold/wordcraft`): Structured rich text document processing.

---

## 9. Commit Attribution & Engineering Voice

- **Commit Author**: Zach Shallbetter (`Zach Shallbetter <zachshallbetter@icloud.com>`).
- **No AI Attribution**: Absolutely no `Co-Authored-By` trailers, no "generated by AI" footers, and no assistant session markers in code, documentation, pull requests, or git commits.
- **Tone**: Direct, technical, active-voice human engineering prose. State problems, exact changes, verified facts, and concrete test numbers plainly without promotional language or filler.
- **Grounded References**: Never make unreferenced claims like *"the work I have been doing"*. Always link directly to the exact pull request, commit, or issue (e.g. `[storytold/craft-libs#9](https://github.com/storytold/craft-libs/pull/9)`).
- **Natural Punctuation & Style**: Avoid stylistic tells such as overusing em-dashes (`—`), academic thesis headings, and nested bullet boilerplate. Use natural, direct sentences with standard punctuation.
- **Natural Language Over Synthetic Identifiers**: Never invent artificial tracking codes (e.g. `CRAFT-COMPAT-001`). Use real tooling and domain terms.
- **Git Hygiene & Zero-Noise Issue Timeline**: Never reference issues in intermediate scratch commits pushed to remote branches; GitHub permanently records timeline cross-references. Keep PR branches squashed to 1 commit with issue references only on the final commit and PR description.
