# The Craft Engineering Methodology: Testing, Interaction, Validation & Fallback Architecture

> **Author:** Zach Shallbetter (`Zach Shallbetter <zachshallbetter@icloud.com>`)  
> **Status:** Authoritative Shared Library Specification  
> **Scope:** Clean-room implementation, autonomous agent harnesses, testing oracles, compatibility validation, and fallback resilience across `craft-libs` and consumer applications.

---

## 1. Overview & System Invariants

The Craft application ecosystem consists of 13 primary desktop applications and the shared library repository `craft-libs`. 

To maintain clean-room integrity and prevent architectural drift across multi-gigabyte production workloads, all implementations adhere to four operational pillars:

```text
┌──────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                  The Four Operational Pillars                                    │
├──────────────────────────┬──────────────────────────┬─────────────────────────┬──────────────────┤
│        1. Testing        │      2. Interaction      │      3. Validation      │   4. Fallbacks   │
├──────────────────────────┼──────────────────────────┼─────────────────────────┼──────────────────┤
│ • Synthetic Generators   │ • Headless-First CLI     │ • Four-Tier Taxonomy    │ • Monotonic DTS  │
│ • Proptest Fuzzing       │ • In-Process Agent MCP   │ • Worktree Compat Gate  │ • Audio Conceal  │
│ • Hostile Evals Suites   │ • Loopback TCP (7979)    │ • Pre-Mutation Diag     │ • Atomic Backup  │
│ • Differential Oracles   │ • Command Reflection     │ • Defensive Arithmetic  │ • Move-Aside Fix │
│ • Failure-Capable Gates  │ • Framebuffer Capture    │ • WASM Target Parity    │ • Graceful Degr. │
└──────────────────────────┴──────────────────────────┴─────────────────────────┴──────────────────┘
```

---

## 2. Testing Architecture & Verification Oracles

Testing in the Craft ecosystem establishes empirical proof of correctness without depending on proprietary third-party test suites or committing non-redistributable project assets.

```text
                                  ┌───────────────────────────────┐
                                  │      Untrusted Test Input     │
                                  └──────────────┬────────────────┘
                                                 ▼
               ┌─────────────────────────────────┴─────────────────────────────────┐
               ▼                                 ▼                                 ▼
    ┌──────────────────────┐          ┌──────────────────────┐          ┌──────────────────────┐
    │ Synthetic Generators │          │   Proptest Fuzzing   │          │   Hostile Evals      │
    │ (Conformant Streams) │          │ (Randomized Bitflips)│          │ (Dimension Explos.)  │
    └──────────┬───────────┘          └──────────┬───────────┘          └──────────┬───────────┘
               │                                 │                                 │
               └─────────────────────────────────┼─────────────────────────────────┘
                                                 ▼
                                  ┌───────────────────────────────┐
                                  │   Target Pure-Rust Decoder    │
                                  └──────────────┬────────────────┘
                                                 │
                                                 ▼ (Decoded Buffer / Loudness)
                                  ┌───────────────────────────────┐
                                  │      Differential Oracle      │
                                  │   (Reference Tool / Decoders) │
                                  └──────────────┬────────────────┘
                                                 │
                                                 ▼
                                  ┌───────────────────────────────┐
                                  │ Assert Sample-Exact Parity    │
                                  │    Within 1 LSB Tolerance     │
                                  └───────────────────────────────┘
```

### 2.1 Synthetic In-Tree Generators
Because third-party proprietary assets cannot be stored in repositories, test fixtures are generated programmatically in-tree:
* **Audio Synthesis**: Pure sine waves, multi-tone bursts, band-limited pink noise, and Dirac impulses generated with exact mathematical frequencies and sample counts (`craft-audio-dsp`).
* **Container Synthesis**: Minimal valid container structures built dynamically in memory:
  - ISOBMFF: Synthesized `ftyp`, progressive `moov` with edit lists (`elst`), and fragmented `moof`/`mdat` sequences (`craft-isobmff`).
  - Matroska: Synthesized EBML headers, segment info, tracks, and SimpleBlock clusters (`craft-matroska`).
  - PSD: Header blocks, color mode data, synthetic resource blocks (signature `8BIM`), and RLE PackBits channel data (`craft-psd`).
  - PDF: Minimal ISO 32000 cross-reference tables, stream objects with Flate compression, and page resource dictionaries (`craft-pdf`).

### 2.2 Property-Based Testing (`proptest`) & Fuzzing
All bitstream parsers, packet decoders, and container demuxers include property-based test harnesses to guarantee panic-free resilience under hostile and mutated inputs:
* **Fuzzing Mutation Matrix**: Bit flips, byte truncations, random payload injection, header mangling, and sign extension attacks.
* **Deterministic Seeds**: Fuzz tests use explicit pseudorandom seeds to ensure reproducible regression vectors when a boundary condition fails.
* **Non-Panicking Guarantee**: Every parser must return an explicit `Result::Err(CrateError::...)` rather than aborting or panicking on corrupted inputs.

### 2.3 Evaluation Suites (`tests/evals.rs`)
Every crate maintains a dedicated `evals.rs` suite validating boundary limits and defensive protections:
* **Hostile Dimension Explosion Caps**: Rejecting maliciously crafted headers declaring absurd canvas sizes (e.g. PSD dimensions $> 300,000 \times 300,000$ pixels or PDF crop boxes $> 14,400 \times 14,400$ points) before memory allocation.
* **Decompression Ratio Defense (Zip Slip & Zip Bomb)**: Enforcing `ResourceBudget` limits: maximum uncompressed bytes ($100\times$ compressed size cap), maximum file count, and strict path containment within the destination directory (`craft-archive`).
* **Cyclic Reference Defense**: Tracking recursion depth and visited object sets during graph traversal (PDF indirect objects, publication spread references) with hard recursion bounds ($\le 64$).
* **Out-of-Bounds Layer Bounds**: Detecting layer coordinates placed completely outside valid document limits.

### 2.4 Differential Oracles
Where open-source reference implementations exist, they are invoked during test suites as external validation oracles rather than code dependencies:
* **External Reference Oracles**: Decoded audio buffers from pure-Rust decoders are compared against system reference decodes of identical inputs. Audio loudness (LUFS, LRA) and true-peak levels must match within 1 LSB tolerance.
* **Faststart Progressive Layout**: Verifying that container headers precede media payloads so progressive streaming starts with zero initial seeking.
* **Priming Compensation Oracle**: Proving that audio encoder priming delays (such as 1024-sample AAC priming) are correctly accounted for across edit lists to preserve sample-exact synchronization with video timestamps.

### 2.5 Failure-Capable Quality Gates
CI and test suites enforce the failure-capability doctrine:
* **Prove Failure Capability**: Every test assertion must be verified capable of failing when fed synthetic defective input before being trusted in CI.
* **No-Op Protection**: Test runs report the exact number of assertions executed; skipped or category-error tests cannot produce false-green results.

---

## 3. Interaction Architecture & Autonomous Agent Harnesses

Every application is engineered to be driven autonomously by AI agents and automated scripts without requiring a physical monitor, GPU, or manual human mouse interaction.

```text
               ┌─────────────────────────────────────────────────────────────┐
               │                    Autonomous AI Agent                      │
               └──────────────┬───────────────────────────────┬──────────────┘
                              │                               │
                      stdio JSON-RPC 2.0              loopback TCP (7979)
                              ▼                               ▼
               ┌──────────────────────────────┐┌──────────────────────────────┐
               │      Headless MCP Server     ││    Desktop Control Channel   │
               │      (apps/<app>-cli mcp)    ││    (crates/ui-egui/control)  │
               └──────────────┬───────────────┘└──────────────┬───────────────┘
                              │                               │
                              ▼                               ▼
               ┌──────────────────────────────┐┌──────────────────────────────┐
               │    In-Process Session /      ││       Live UI Thread &       │
               │       Offscreen Render       ││      Window Framebuffer      │
               └──────────────────────────────┘└──────────────────────────────┘
```

### 3.1 The Headless-First Invariant
Every application provides an `apps/<app>-cli` binary that operates completely detached from window servers and display drivers:
* **Scripted Execution**: Headless batch conversion, project compilation, asset cataloging, and transcode exports.
* **Headless Testing**: Executing full document workflows in CI environments without virtual framebuffers.

### 3.2 In-Process Agent MCP Server (`apps/<app>-cli mcp`)
Command-line interfaces expose a Model Context Protocol (MCP) server over `stdio` using JSON-RPC 2.0:
* **Engine Session Lifecycle**: Autonomous tools can initialize document sessions, load assets, query state, apply non-destructive edits, and export deliverables.
* **Off-Screen Pixel Rendering**: Rendering pixel buffers and document spreads directly in software memory without GPU requirements.

### 3.3 The Loopback TCP UI Control Channel (`control.rs` / `craft-control`)
Interactive desktop applications (`apps/<app>`) expose a local TCP control channel (default port `7979`, configurable via `--control <port>`):
* **Execution on the UI Thread**: Incoming commands are queued and drained on the primary UI event loop. This ensures automated actions follow the exact event handling, state transitions, undo/redo stacks, and repainting paths as manual user interactions.
* **Repaint Activation**: Every programmatic command or menu trigger calls `repaint.request_repaint()` to prevent window dormancy.
* **Shared Authentication Handshake (`craft-control`)**: Mandates authenticated caller tokens (256-bit random tokens from OS RNG, constant-time comparison) published to owner-only runtime descriptor locations to isolate localhost processes.

### 3.4 Reflective Command Introspection (`CommandSpec`)
The UI control protocol supports dynamic introspection via `engine.commands`:
* Agents discover registered commands, human-readable labels, menu paths, keyboard shortcuts, and JSON parameter schemas at runtime.
* Commands declare explicit enablement preconditions; disabled commands return structured explanations (`disabled_reason`) preventing invalid state execution.

### 3.5 Visual Verification (`ui.screenshot`)
The loopback control channel allows agents to capture the live window framebuffer:
* Returns PNG-encoded screenshots directly to the agent.
* Used to verify widget rendering, modal dialog appearance, layout stability, and visual docking layouts without human intervention.

### 3.6 Typed Socket Protocols
Background transcode and indexing daemons communicate with client applications over asynchronous Unix domain sockets or named pipes:
* **Non-Blocking Operation**: Real-time canvas editing and timeline playback never block on background transcode operations.
* **Atomic Job State**: Progress updates, transcode presets, hardware discovery, and error reports are exchanged via typed Serde-serialized messages.

---

## 4. Validation Protocols & Compatibility Gates

The Craft suite enforces a multi-tier qualification protocol to ensure that shared code remains decoupled, safe, and fully backward compatible.

### 4.1 The Four-Tier Claim Taxonomy
When discussing or documenting capabilities, all components enforce the strict claim hierarchy:

$$\text{Documented} \neq \text{Implemented} \neq \text{Tested} \neq \text{Empirically Validated}$$

1. **Documented**: The feature or format tag appears in documentation or specifications.
2. **Implemented**: Code exists in the repository without throwing unhandled exceptions.
3. **Tested**: Automated unit or synthetic proptests pass in the local test suite.
4. **Empirically Validated**: Bit-exact round-trip testing passes real-world production verification against external reference software.

### 4.2 Cross-Application Compatibility Protocol (`cargo xtask compat`)
Shared crates in `craft-libs` are validated against real consumer applications (PhotoCraft, VectorCraft, DesignCraft, etc.) before landing:
* **Candidate Revision Testing**: Testing proposed shared changes against real applications without altering committed lockfiles.
* **Worktree Isolation**: Candidate and baseline runs execute in ephemeral Git worktrees, isolated from the developer's working state and cleaned up automatically.
* **Dirty-Tree Snapshotting**: Uncommitted changes are captured via isolated temporary Git indexes (`GIT_INDEX_FILE`), including untracked source files while respecting `.gitignore`.
* **Non-Vacuous Success Enforcement**: Rejecting runs where zero comparisons complete; test assertions must actively execute.
* **Regression Separation**: Distinguishing existing baseline failures from regressions introduced by the candidate revision.

### 4.3 Structural Diagnostic Reports (`*DiagnosticReport`)
Before mutating document structures or media containers, engines generate detailed diagnostic reports:
* `PsdDiagnosticReport`: Checks canvas geometry, layer counts, out-of-bounds layer bounds, uncompressed memory estimation, and group nesting balance.
* `IsobmffDiagnosticReport`: Checks faststart progressive layout, keyframe intervals, B-frame reordering, edit lists, causal PTS/DTS order, and monotonic DTS progression.
* `MatroskaDiagnosticReport`: Validates EBML header elements, VINT lengths, and cluster timestamps.
* `PdfDiagnosticReport`: Verifies cross-reference tables, object cyclicity, and encryption dictionary revisions.
* `ScanReport`: Records files scanned, cataloged, skipped, duration, and error details during directory ingestion.

### 4.4 Defensive Engineering & Hostile Input Boundaries
All data originating from files, network streams, or foreign processes is treated as hostile:
* **Safe Slicing**: String slicing is gated by `.is_char_boundary(idx)`; byte buffers use `.get()` rather than unchecked indexing (`v[i]`).
* **Safe Arithmetic**: Arithmetic on lengths, offsets, byte counts, and media ticks uses `checked_*` or `saturating_*` operations, preventing integer overflow and divide-by-zero.
* **Poison Recovery**: Lock contention handles poisoning gracefully via `.unwrap_or_else(PoisonError::into_inner)`.
* **Zero Panics in Shipped Code**: `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]` is enforced at crate level.
* **Forbidden Unsafe**: `unsafe_code = "forbid"` is enforced across all domain and layout crates.

### 4.5 WebAssembly Conformance (`wasm32-unknown-unknown`)
All shared library crates (`craft-libs`) compile cleanly to WebAssembly:
* Verified via `cargo check --target wasm32-unknown-unknown`.
* Platform-specific I/O or multi-threading is strictly isolated behind `#[cfg(not(target_arch = "wasm32"))]` with non-panicking single-threaded fallback behavior.

---

## 5. Fallback Policies & Resilience Engineering

When encountering corrupted data, damaged containers, unsupported extensions, or platform limitations, Craft engines degrade predictably and non-destructively rather than failing catastrophically.

```text
                                  ┌───────────────────────────────┐
                                  │      Hostile/Corrupted Input  │
                                  └──────────────┬────────────────┘
                                                 ▼
               ┌─────────────────────────────────┴─────────────────────────────────┐
               ▼                                 ▼                                 ▼
    ┌──────────────────────┐          ┌──────────────────────┐          ┌──────────────────────┐
    │  Timeline Fallbacks  │          │   Storage Fallbacks  │          │   Format Fallbacks   │
    ├──────────────────────┤          ├──────────────────────┤          ├──────────────────────┤
    │ • Backwards DTS:     │          │ • Corrupt config:    │          │ • Missing moov:      │
    │   Clamp monotonic    │          │   move_aside()       │          │   Scan index rebuild │
    │ • Offset handling:   │          │ • Backup restore:    │          │ • Unknown extension: │
    │   Signed ctts check  │          │   Restore from .bak  │          │   Magic-byte sniff   │
    │ • Audio track gap:   │          │ • Multi-level check: │          │ • Missing font:      │
    │   Inject silence     │          │   Syntax + Version   │          │   System fallback    │
    └──────────────────────┘          └──────────────────────┘          └──────────────────────┘
```

### 5.1 Media Timeline Fallbacks (`TimelineFallbackPolicy`)
Media packet streams frequently contain timeline anomalies, out-of-order packets, or frame dropouts:
* **Backwards DTS Clamping**: If a demuxed packet contains a decode timestamp earlier than the previous packet (`DTS < last_dts`), the validator clamps DTS to `last_dts + 1` and logs a warning, maintaining monotonic decode progression without crashing playback clocks.
* **Composition Offset Normalization**: When container metadata lacks version 1 signed `ctts` support and encounters an unheralded negative composition offset, the presentation offset is normalized relative to the sequence anchor. In streams with valid signed composition offsets, negative PTS offsets are preserved to ensure frame reordering matches the encoder's presentation schedule.
* **Audio Gap Concealment**: When a multi-track audio session encounters dropped samples or missing packet intervals, the engine injects mathematically silent zero-buffers (`0.0f32`) rather than desynchronizing subsequent audio tracks.

### 5.2 Storage & Persistence Fallbacks (`craft-atomic` & `craft-settings`)
Document persistence and configuration files protect against data corruption during sudden power interruption:
* **Write Ordering**: Atomic persistence enforces `sync_file < rename < sync_dir`, ensuring data reaches non-volatile storage before filesystem directory pointers update.
* **Damaged File Preservation (`move_aside`)**: When a configuration or document fails syntax or schema validation, it is never silently overwritten. The corrupted file is renamed to `config.broken-<unix-ts>.toml` for diagnostic preservation.
* **Multi-Stage Backup Recovery**:
  1. Primary configuration file parsed and validated against schema version bounds.
  2. If primary is damaged, engine automatically attempts recovery from `config.toml.bak`.
  3. Only if both primary and backup are invalid does the engine fall back to compiled default settings.
* **Validated Backup Refresh**: `refresh_backup()` validates both syntactic correctness and schema bounds before replacing an existing backup, guaranteeing valid backups are never overwritten with invalid state.

### 5.3 Container & Parser Degradation Fallbacks
* **ISOBMFF Missing Faststart**: When a container has its header box located at the end of the file rather than the beginning, streaming decoders fall back to an index-rebuilding pass without aborting.
* **Truncated Archive Handling**: Archive unarchivers verify central directory headers and CRC32 checksums, failing early with explicit error variants rather than extracting partial, invalid files.
* **Asset Cataloging Fallback**: When an asset lacks a recognizable file extension, the catalog scanner falls back to magic-byte inspection (e.g. `8BPS` for PSD, `%PDF` for PDF, `RIFF....WAVE` for BWF). If format identification fails, the asset is preserved under a generic binary facet and logged in the `ScanReport`.

### 5.4 Presentation & Color Fallbacks
* **Font Fallback Cascades**: Missing glyphs in text layouts traverse a documented font fallback chain, terminating in embedded clean-room open fonts (`storytold/craft-fonts`) to eliminate missing-glyph boxes.
* **Unsupported Color Spaces**: High-gamut or specialized color profiles that cannot be rendered on the target display fall back to relative colorimetric conversion to sRGB with black point compensation, preserving perceptual tonal relationships.

---

## 6. The Six-Phase Shared Crate Promotion Lifecycle

Consolidating parallel application code into shared `craft-libs` crates follows a strict six-phase lifecycle:

$$\text{Discover} \longrightarrow \text{Validate} \longrightarrow \text{Propose} \longrightarrow \text{Implement} \longrightarrow \text{Prove} \longrightarrow \text{Adopt}$$

1. **Discover**: Identify genuine functional duplication across independent applications (e.g. WMF/EMF parsers across multiple apps, or ICC transform caches in PhotoCraft and FilmCraft).
2. **Validate**: Analyze domain invariants, precision requirements, coordinate spaces, and performance profiles to prove code is genuinely shareable, not merely superficially similar.
3. **Propose**: Frame shared architecture in formal proposals and RFCs on the issue tracker. Proposals remain proposals until maintainers approve them.
4. **Implement**: Clean-room implementation in `craft-libs` adhering to zero application dependencies, `forbid(unsafe_code)`, `never crash`, and clean WASM compilation (**Level 1: Crate Qualification**).
5. **Prove**: Satisfy the **Extraction Provenance Contract** against recorded baseline vectors, and verify non-breaking behavior via `cargo xtask compat` against consuming applications (**Level 2: Extraction Qualification**).
6. **Adopt**: Update downstream application `Cargo.toml` dependency pins in small, bisectable commits. Preserve consumer intent while proving sample-exact parity and zero regressions (**Level 3: Adoption Qualification**).

---

## 7. Quality Contract

By codifying these operational standards, `craft-libs` guarantees that:
1. **Testing is Empirical**: Relies on synthetic generation, proptest fuzzing, hostile evaluation vectors, and external reference oracles rather than unverified assertions.
2. **Interaction is Universal**: Every engine feature is accessible via headless CLI, in-process agent MCP, and live loopback TCP UI control.
3. **Validation is Rigorous**: Changes are verified against real consumer applications via isolated worktree compatibility runners (`cargo xtask compat`).
4. **Fallbacks are Safe**: Errors degrade gracefully and non-destructively, preserving user data and maintaining system stability under hostile conditions.
