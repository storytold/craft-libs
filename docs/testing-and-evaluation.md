# Craft Ecosystem Testing, Evaluation & Fallback Specification

> **Author:** Zach Shallbetter (`Zach Shallbetter <zachshallbetter@icloud.com>`)  
> **Status:** Authoritative Ecosystem Specification  
> **Scope:** In-tree synthetic generators, property-based fuzzing, hostile boundary evaluations (`evals.rs`), differential reference oracles, autonomous interaction harnesses, cross-application compatibility gates (`cargo xtask compat`), and fallback resilience across the 13 Craft applications and `mediacraft`.

---

## 1. Executive Summary & Epistemic Doctrine

Software development across the Craft ecosystem is guided by an absolute commitment to empirical verification, clean-room development, and defensiveness against hostile data.

### The Four-Tier Claim Taxonomy (`AGENTS.md`)

All assertions, benchmarks, compatibility declarations, and feature maturity claims adhere strictly to the Four-Tier Claim Taxonomy:

$$\text{Documented} \neq \text{Implemented} \neq \text{Tested} \neq \text{Empirically Validated}$$

- **Documented**: A specification, RFC, trait contract, or header defines the behavior. No runnable code exists.
- **Implemented**: Clean-room source code compiles cleanly under `cargo check` for both native host and `wasm32-unknown-unknown` targets with `#![forbid(unsafe_code)]`.
- **Tested**: Automated unit, property, and evaluation test suites pass deterministically in CI with assertions actively executing.
- **Empirically Validated**: The crate executes inside real consumer application pipelines under heavy workloads, verified by cross-application compatibility runs (`cargo xtask compat`) without crashes or regressions.

---

## 2. Test Architecture & Tiered Verification Matrix

Every crate in `craft-libs` and `mediacraft` implements a five-tier testing topology:

```text
┌──────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                     The Five Testing Tiers                                       │
├─────────────────────┬──────────────────┬──────────────────┬───────────────────┬──────────────────┤
│ 1. Unit Tests       │ 2. Synthetics    │ 3. Proptest      │ 4. Evals Suites   │ 5. Diff Oracles  │
├─────────────────────┼──────────────────┼──────────────────┼───────────────────┼──────────────────┤
│ • Pure math logic   │ • In-tree streams│ • Mutated inputs │ • Hostile limits  │ • ffmpeg/ffprobe │
│ • State machines    │ • Valid minimal  │ • Random flips   │ • Dimension caps  │ • Priming delays │
│ • Serde roundtrips  │   containers     │ • Zero panics    │ • Zip Slip budget │ • Loudness LUFS  │
│ • Error variants    │ • No external    │ • Deterministic  │ • Cyclic graphs   │ • Golden render  │
│   returned cleanly  │   test assets    │   random seeds   │ • Out-of-bounds   │   1 LSB parity   │
└─────────────────────┴──────────────────┴──────────────────┴───────────────────┴──────────────────┘
```

| Tier | File Location | Purpose | Execution Target | Invalidation Condition |
| :--- | :--- | :--- | :--- | :--- |
| **Tier 1: Unit & Domain** | `src/*.rs` or `src/tests.rs` | Internal state transitions, rational arithmetic, bit parsing | `cargo test` | Unhandled panics, broken contracts |
| **Tier 2: Synthetic Fixtures** | In-memory generators | Building conformant stream structures from first principles | In-tree helper modules | External proprietary asset dependencies |
| **Tier 3: Property Fuzzing** | `tests/fuzz.rs` | Randomized bitstream mutations, sign extensions, truncation | `proptest` / CI fuzz runs | Panic on malformed or truncated byte stream |
| **Tier 4: Hostile Evals** | `tests/evals.rs` | Enforcing defensive caps, resource budgets, causal timing | `cargo test --test evals` | OOM allocation, unbounded recursion, unhandled loops |
| **Tier 5: Differential Oracles**| `tests/oracles.rs` | Cross-checking against external reference verifiers | `cargo test -p mediacraft-oracles`| Output differs by more than 1 LSB tolerance |

---

## 3. Synthetic Test Fixture Generation

To maintain strict clean-room governance, **no proprietary test corpora, commercial presets, or copyrighted assets are committed to Git repositories**. All test fixtures must be generated programmatically in-tree from open specifications.

### 3.1 Audio Synthesis (`craft-audio-dsp`)
Test streams are constructed from mathematical equations rather than pre-recorded audio files:
- **Sine & Multi-Tone Synthesis**: Exact phase, amplitude, and frequency generators ($f = 440\text{ Hz}, 1\text{ kHz}, 10\text{ kHz}$) sampled at standard rates ($44.1\text{ kHz}, 48\text{ kHz}, 96\text{ kHz}, 192\text{ kHz}$).
- **Dirac Impulses**: Single non-zero sample ($1.0\text{f32}$) followed by silence to test finite impulse response (FIR) filtering, convolution latency, and ringing artifacts.
- **Pink & Gaussian Noise**: Seeded pseudo-random noise arrays passed through pole-zero filters to verify spectral balance and loudness weighting.

### 3.2 Container Synthesis (`craft-isobmff`, `craft-matroska`)
Minimal valid container structures built in memory:
- **ISOBMFF Synthesizer**: Constructs valid `ftyp` (brand `isom`/`mp42`), progressive `moov` with edit lists (`elst`), tracks with sample tables (`stts`, `stsz`, `stco`, `co64`, `ctts`), and interleaved `mdat` chunks.
- **Matroska Synthesizer**: Constructs EBML headers (DocType `matroska`/`webm`), Segment Info, Tracks (TrackType 1 for video, 2 for audio), and SimpleBlock Clusters with variable-length integers (VINT).

### 3.3 Layered Document Synthesis (`craft-psd`, `craft-pdf`)
- **PSD Synthesizer**: Constructs valid 26-byte headers, color mode data, Image Resources (`8BIM` signature blocks), layer records with PackBits RLE compression, and composite channel buffers.
- **PDF Synthesizer**: Constructs minimal valid ISO 32000 cross-reference tables (`xref`), stream dictionaries with Flate compression, and page tree hierarchies.

---

## 4. Property-Based Testing (`proptest`) & Fuzzing Standards

Every parser and decoder must prove panic-free behavior under mutated and malicious input streams.

```text
                                  ┌───────────────────────────────┐
                                  │   Valid Minimal Bitstream     │
                                  └──────────────┬────────────────┘
                                                 │
                                                 ▼
                                  ┌───────────────────────────────┐
                                  │   Proptest Mutation Strategy  │
                                  ├───────────────────────────────┤
                                  │ • Bit flips (0.1% - 10%)      │
                                  │ • Truncation at arbitrary N   │
                                  │ • Byte insertion & deletion   │
                                  │ • Sign bit manipulation       │
                                  │ • Chunk length fuzzing        │
                                  └──────────────┬────────────────┘
                                                 │
                                                 ▼
                                  ┌───────────────────────────────┐
                                  │   Target Pure-Rust Decoder    │
                                  └──────────────┬────────────────┘
                                                 │
                                                 ▼
                                  ┌───────────────────────────────┐
                                  │ MUST return Result::Err(...)  │
                                  │    ABSOLUTELY NO PANICS       │
                                  └───────────────────────────────┘
```

### 4.1 Fuzzing Rules
1. **Deterministic Pseudorandom Seeds**: Fuzz runs declare explicit seeds (`proptest::test_runner::Config`) to guarantee that any discovered defect can be reproduced bit-for-bit.
2. **Explicit Error Returns**: Parsers must convert corrupt inputs into structured domain error variants (`Error::Truncated`, `Error::InvalidHeader`, `Error::UnsupportedVersion`) rather than panicking, calling `unwrap()`, or indexing out of bounds.
3. **Execution Cap**: Mutation passes must complete within bounded wall-clock limits (maximum 10,000 iterations per suite in CI).

---

## 5. Evaluation Suites (`tests/evals.rs` Standards)

Every format decoder and container parser maintains a dedicated `evals.rs` integration suite verifying defenses against malicious, corrupted, or hostile inputs.

### 5.1 Dimension Explosion Defense
Attackers craft small files with header dimensions set to extreme values to induce catastrophic out-of-memory allocations.
- **Standard**: Crate decoders must inspect dimensions in the file header and enforce strict geometric ceilings **before** attempting pixel or channel buffer allocation.
- **Threshold**: Canvas dimensions exceeding $300,000 \times 300,000$ pixels (or uncompressed memory estimation exceeding $4\text{ GB}$) must be rejected immediately with an explicit `Error::TooLarge` or `Error::InvalidDimensions`.

```rust
// Standard Evaluation Pattern (tests/evals.rs)
#[test]
fn eval_rejects_dimension_explosion() {
    let mut hostile_header = valid_header();
    hostile_header.width = 500_000;
    hostile_header.height = 500_000;
    let res = Decoder::open(&hostile_header);
    assert!(matches!(res.unwrap_err(), Error::TooLarge(_) | Error::InvalidDimensions(_)));
}
```

### 5.2 Decompression Ratio & Zip Slip Defense (`ResourceBudget`)
Package decoders (`craft-archive`, `craft-pdf`) handling compressed containers (ZIP, TAR, Flate) must defend against archive bombs and directory traversal attacks:
- **Path Containment**: Every archive entry's normalized destination path must be verified to reside within the intended target directory. Entries with parent references (`..`) or absolute paths (`/etc/...`) must be rejected.
- **Decompression Expansion Ratio**: Output bytes must not exceed $100\times$ the compressed entry size.
- **Global Extraction Limits**: Enforced via `ResourceBudget`:
  - Maximum uncompressed bytes: Default 512 MB.
  - Maximum extracted files: Default 10,000 entries.
  - Exceeding any budget must abort extraction with `ArchiveError::DecompressionLimitExceeded`.

### 5.3 Cyclic Reference & Recursion Caps
Object graphs with reference cycles (PDF indirect objects, IDML parent/spread links, SVG `use` loops) can cause infinite call stacks.
- **Standard**: Recursive graph walks must maintain a visited-node set (`HashSet<ObjectId>`) and a recursion counter.
- **Cap**: Any traversal exceeding a depth of **64** must return `Error::CyclicReference` or `Error::NestingLimitExceeded`.

### 5.4 Causal Timeline Monotonicity & B-Frame Checks
Container demuxers (`craft-isobmff`, `craft-matroska`) must validate timestamp causality:
- **Causal Violation Warning**: When a packet's presentation timestamp is less than its decode timestamp ($\text{PTS} < \text{DTS}$), the diagnostic engine flags an explicit causal violation.
- **Backwards DTS Anomaly**: When $\text{DTS}_i < \text{DTS}_{i-1}$, the diagnostic report flags a non-monotonic DTS regression.

---

## 6. Differential Verification Oracles (`mediacraft-oracles`)

Where external reference binaries exist (`ffmpeg`, `ffprobe`, `vello_cpu`), integration test suites invoke them as external differential verifiers without linking their source code or accepting copyleft obligations.

```text
                                  ┌───────────────────────────────┐
                                  │      Synthetic Audio Stream   │
                                  └──────────────┬────────────────┘
                                                 │
                        ┌────────────────────────┴────────────────────────┐
                        ▼                                                 ▼
         ┌──────────────────────────────┐                  ┌──────────────────────────────┐
         │     Pure-Rust craft-aac      │                  │        FFmpeg Oracle         │
         │      Encoded Bitstream       │                  │       (Reference Tool)       │
         └──────────────┬───────────────┘                  └──────────────┬───────────────┘
                        │                                                 │
                        ▼                                                 ▼
         ┌──────────────────────────────┐                  ┌──────────────────────────────┐
         │     Decoded PCM Samples      │                  │     Decoded PCM Samples      │
         └──────────────┬───────────────┘                  └──────────────┬───────────────┘
                        │                                                 │
                        └────────────────────────┬────────────────────────┘
                                                 ▼
                                  ┌───────────────────────────────┐
                                  │ Assert Bit-Exact Match /      │
                                  │ True-Peak Within 1 LSB Tol.   │
                                  └───────────────────────────────┘
```

### 6.1 FFmpeg / FFprobe Conformance
- **Audio Codec Verification**: Audio encoded by `craft-aac`, `craft-flac`, and `craft-opus` must decode cleanly in `ffmpeg` without syntax warnings or sample count discrepancies.
- **Loudness Parity**: EBU R128 integrated loudness (LUFS) and true-peak (dBTP) computed by `craft-audio-dsp` must match `ffmpeg -af ebur128` within $0.1\text{ LU}$.
- **Faststart Multiplexing**: `mediacraft-oracles` validates that `moov` precedes `mdat` by inspecting byte offsets, proving progressive playback starts without initial seeking.
- **AAC Priming Delay Compensation**: Asserts that 1024-sample priming delays are correctly accounted for across edit lists (`elst`), eliminating audio/video sync drift.

### 6.2 Visual Rendering Oracles
- **Rasterizer Conformance**: CPU-rendered paths and vector shapes are compared against golden rendered outputs at 1 LSB tolerance per RGBA channel.

---

## 7. Autonomous Interaction & Control Testing

Every application is engineered to be driven autonomously by AI agents and automated scripts without human mouse interaction or a physical display.

### 7.1 The Headless-First Invariant (`apps/<app>-cli`)
Every application provides an `apps/<app>-cli` binary:
- **Operating Without Window Server**: Runs in minimal container environments without GPU, display drivers, or `xvfb`.
- **Scripted Operations**: Headless batch conversion, project compilation, asset cataloging, and transcode exports.

### 7.2 In-Process Agent MCP Server (`apps/<app>-cli mcp`)
Command-line interfaces expose a Model Context Protocol (MCP) server over `stdio` via JSON-RPC 2.0:
- Autonomous agents initialize document sessions, inspect object models, apply edits, and export deliverables in-process.

### 7.3 Loopback TCP UI Control Channel (`control.rs`)
Interactive desktop applications (`apps/<app>`) expose a local TCP control channel (port `7979`, configurable via `--control <port>`):
- **UI Thread Execution**: Incoming commands execute directly on the UI event loop, ensuring programmatic actions follow the identical state transitions, undo/redo stacks, and repaints as physical user interactions.
- **Repaint Activation**: Every programmatic command or menu trigger calls `repaint.request_repaint()` to prevent window dormancy.

### 7.4 Reflective Command Introspection (`CommandSpec`)
The UI control protocol supports dynamic introspection via `engine.commands`:
- Agents discover registered commands, human-readable labels, menu paths, keyboard shortcuts, and JSON parameter schemas at runtime.
- Commands declare explicit enablement preconditions; disabled commands return structured explanations (`disabled_reason`) preventing invalid state execution.

### 7.5 Visual Framebuffer Screenshot Capture (`ui.screenshot`)
The loopback control channel allows agents to capture the live window framebuffer:
- Returns PNG screenshots of the active window to allow agents to visually inspect layout, docking states, and modal dialogs without human intervention.

---

## 8. Cross-Application Compatibility Protocol (`cargo xtask compat`)

Shared library crates in `craft-libs` must never regress consuming applications (`photocraft`, `vectorcraft`, `filmcraft`, `effectcraft`, `lightcraft`, `pdfcraft`, `cadcraft`, `soundcraft`, `wordcraft`, `deckcraft`, `gridcraft`, `figcraft`, `designcraft`).

```text
                                  ┌───────────────────────────────┐
                                  │ Proposed craft-libs Revision  │
                                  └──────────────┬────────────────┘
                                                 │
                                                 ▼
                                  ┌───────────────────────────────┐
                                  │      cargo xtask compat       │
                                  └──────────────┬────────────────┘
                                                 │
                        ┌────────────────────────┴────────────────────────┐
                        ▼                                                 ▼
         ┌──────────────────────────────┐                  ┌──────────────────────────────┐
         │     Ephemeral Worktree 1     │                  │     Ephemeral Worktree 2     │
         │   (Baseline Committed Pin)   │                  │  (Candidate Proposed Patch)  │
         └──────────────┬───────────────┘                  └──────────────┬───────────────┘
                        │                                                 │
                        ▼ (cargo test consumer)                           ▼ (cargo test consumer)
         ┌──────────────────────────────┐                  ┌──────────────────────────────┐
         │       Baseline Results       │                  │      Candidate Results       │
         └──────────────┬───────────────┘                  └──────────────┬───────────────┘
                        │                                                 │
                        └────────────────────────┬────────────────────────┘
                                                 ▼
                                  ┌───────────────────────────────┐
                                  │ Assert Zero Regressions       │
                                  │ (Candidate Pass >= Baseline)  │
                                  └───────────────────────────────┘
```

### 8.1 Worktree Isolation Protocol
1. **Isolated Execution**: Builds execute inside isolated ephemeral Git worktrees, preserving developer working state and cleaning up automatically.
2. **Dirty-Tree Snapshotting**: Uncommitted changes are captured via isolated temporary Git indexes (`GIT_INDEX_FILE`), including untracked source files while respecting `.gitignore`.
3. **Non-Vacuous Success Enforcement**: Rejecting runs where zero comparisons complete; test assertions must actively execute.
4. **Regression Separation**: Distinguishing existing baseline failures from regressions introduced by the candidate revision.

---

## 9. Fallback Policies & Resilience Engineering

When encountering corrupted data, damaged containers, unsupported extensions, or platform limitations, engines degrade predictably and non-destructively:

### 9.1 Media Timeline Fallbacks (`TimelineFallbackPolicy`)
- **Backwards DTS Clamping**: If a demuxed packet contains a decode timestamp earlier than the previous packet (`DTS < last_dts`), DTS is clamped to `last_dts + 1` with a warning log, preserving causal playback without stalling playback clocks.
- **Causal PTS Correction**: If `PTS < DTS` (impossible causal ordering), PTS is corrected to `PTS = DTS`.
- **Audio Gap Concealment**: Dropped packet intervals inject mathematically silent buffers (`0.0f32`) rather than throwing audio tracks out of sync with video.

### 9.2 Storage & Configuration Fallbacks (`craft-atomic` & `craft-settings`)
- **Crash-Safe Write Ordering**: Enforces `sync_file < rename < sync_dir`.
- **Preservation of Corrupt Files (`move_aside`)**: Damaged primary configurations are never silently discarded or overwritten; they are renamed to `config.broken-<unix-ts>.toml` for forensic analysis.
- **Multi-Stage Backup Recovery**: Attempts syntax/schema parse on primary $\to$ falls back to `config.toml.bak` $\to$ falls back to compiled defaults only if both are corrupted.
- **Validated Backup Refresh**: `refresh_backup()` requires successful syntactic parse and schema bounds checks before replacing an existing backup.

### 9.3 Container & Parsing Fallbacks
- **Missing Faststart Recovery**: When an MP4 file has `moov` at the tail, streaming decoders fall back to an indexing pass without failing.
- **Asset Cataloging Fallback**: When an asset lacks a file extension, the scanner falls back to magic-byte sniffing (`8BPS`, `%PDF`, `RIFF`) before categorizing as an unknown binary facet.

### 9.4 Presentation & Color Fallbacks
- **Font Cascades**: Missing glyphs traverse fallback chains terminating in bundled clean-room fonts (`storytold/craft-fonts`).
- **Color Gamuts**: Out-of-gamut profiles fall back to relative colorimetric conversion to sRGB with black point compensation.

---

## 10. Quality Gate Checklist for Crate Authors

Before submitting or landing any crate in `craft-libs` or `mediacraft`:

- [ ] `#![forbid(unsafe_code)]` declared in `src/lib.rs`.
- [ ] `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]` declared in `src/lib.rs`.
- [ ] All indexing on input-derived numbers uses `.get()` or checked slices.
- [ ] Length and offset calculations use `checked_*` or `saturating_*` arithmetic.
- [ ] Lock poisoning is handled via `.unwrap_or_else(PoisonError::into_inner)`.
- [ ] Dedicated `evals.rs` suite added in `tests/evals.rs` covering dimension explosion, resource limits, and truncated streams.
- [ ] Structured diagnostic reporter implemented (`*DiagnosticReport`).
- [ ] `cargo fmt --check` passes with zero diffs.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes with zero warnings.
- [ ] `cargo test --workspace` passes with zero failures.
- [ ] `cargo check --target wasm32-unknown-unknown` compiles cleanly.
- [ ] Author attribution set to `Zach Shallbetter <zachshallbetter@icloud.com>` with zero AI attribution.
