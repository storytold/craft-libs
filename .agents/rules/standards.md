# Craft Ecosystem Core Engineering Standards & Doctrine

This document defines the authoritative engineering doctrine, architectural layering, non-negotiable invariants, and operational protocols for shared libraries in the Craft software ecosystem (`craft-libs`).

---

## 1. Ecosystem Architecture & Layering

The Craft software ecosystem is structured in three strictly decoupled, unidirectional architectural layers:

```
┌────────────────────────────────────────────────────────┐
│               1. DOCTRINE & STANDARDS                  │
│       Specifications (never-crash, clean-room, formats)│
└───────────────────────────┬────────────────────────────┘
                            │ informs
┌───────────────────────────▼────────────────────────────┐
│              2. SHARED FOUNDATION LIBRARIES            │
│  craft-libs (craft-ui, craft-atomic, craft-time, etc.) │
└───────────────────────────┬────────────────────────────┘
                            │ consumed by
┌───────────────────────────▼────────────────────────────┐
│              3. APPLICATION SUITE LAYER                │
│  PhotoCraft, VectorCraft, FilmCraft, EffectCraft, etc. │
└────────────────────────────────────────────────────────┘
```

1. **Doctrine & Standards Layer**: Defines suite-wide invariants, Never-Crash requirements, clean-room legal boundaries, and domain specifications.
2. **Shared Foundation Layer (`craft-libs`)**: Pure, headless, reusable Rust library crates shared across multiple applications. Must never depend on any application crate.
3. **Application Suite Layer**: Product implementations, canvas interactions, command registries, native window shells, and target application parity.

---

## 2. The 6 Non-Negotiable Invariants

Every change, commit, and implementation across the workspace must strictly satisfy these six core invariants:

### 1. Zero App Code
Shared crates in `craft-libs` must be useful to multiple applications and **must not depend on any application crate**. No imports of application crates (`photocraft-*`, `vectorcraft-*`, etc.).

### 2. Never Crash
In shipped library and production code: no panics, no unwraps, no expects, no todos, no unreachables. Every crate root must enforce:
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
All errors must be handled through explicit `Result<T, E>` and `Option<T>` returns. Unfinished capabilities return an explicit unsupported domain error.

### 3. Unsafe Code Forbidden
Unsafe code is strictly forbidden across workspaces:
```rust
#![forbid(unsafe_code)]
```

### 4. Never Break WASM
Every shared crate must compile cleanly for WebAssembly targets:
```bash
cargo check --target wasm32-unknown-unknown
```
Operating system file I/O, subprocess execution, and native threading must be gated behind `#[cfg(not(target_arch = "wasm32"))]` with graceful, non-panicking fallback stubs on WASM.

### 5. Defensive Engineering & Hostile Input Handling
All inputs from files, foreign protocols, IPC sockets, and user parameters are treated as hostile:
- **Indexing**: Use `.get()` and `.get_mut()` rather than direct indexing (`v[i]`).
- **String Slicing**: Slicing strings requires explicit character boundary validation (`is_char_boundary`).
- **Checked Arithmetic**: Use `checked_*` and `saturating_*` arithmetic for lengths, offsets, byte counts, and media ticks. Prevent integer overflow, divide-by-zero, and `NaN`/`inf` float conversions.
- **Resource Caps**: Cap memory allocations and loop iterations sized by untrusted data.
- **Recursion Bounds**: Enforce explicit depth limits on hierarchical or cyclic structures.
- **Lock Safety**: Handle lock poisoning gracefully via `lock().unwrap_or_else(PoisonError::into_inner)`. Real-time media timing paths must never block or allocate in steady state.
- **Regression Proofs**: Every bug or crash fix must include a synthetic regression test that failed prior to the fix.

### 6. Clean-Room Governance & Proprietary Product Independence
- **Dual-Licensed**: Dual MIT OR Apache-2.0. Clean-room implementations only.
- **Zero External Proprietary Product References**: No code, comments, docstrings, documentation, issue tickets, pull requests, commit messages, or agent prompts may reference proprietary commercial products outside the ArtCraft and Storytold ecosystem. All applications, tools, capabilities, and workflows must be defined strictly in terms of domain standards, mathematical/algorithmic specifications, public open formats (ISO 32000, ICC.1, SMPTE, Unicode, W3C), or native ArtCraft/Storytold application identities. Any contribution or request referencing non-ecosystem proprietary software must be rejected or sanitized immediately at triage.
- **Specification Boundary**: Work exclusively from published open standards (ISO 32000, ICC.1, SMPTE, Unicode, W3C) and observable black-box behavior. Never decompile, disassemble, or copy proprietary code, shaders, or presets. Never copy or link copyleft code (GPL/LGPL/AGPL).
- **Attribution**: Every bundled asset must have a corresponding entry in `ATTRIBUTION.md`.
- **Font Policy**: Font binaries are never committed to git (stored in `storytold/craft-fonts` via `CRAFT_FONTS_DIR`).
- **Test Corpora**: Test fixtures reside in external corpora (`storytold/photocraft-corpus`). Use synthetic generators and `proptest` for in-tree tests.

---

## 3. The 13 Consumer Applications

Shared crates in `craft-libs` are consumed across the 13 Craft applications:

| Project | Application Identity | Functional Domain | Role in Suite | `craft-ui` Pin Status |
| :--- | :--- | :--- | :--- | :--- |
| **PhotoCraft** | PhotoCraft | Professional raster image editing & color pipeline | Core creative app | Adopted (`rev = "5c874d5"`) |
| **VectorCraft** | VectorCraft | Precision vector illustration & geometry engine | Core creative app | Adopted (`rev = "5c874d5"`) |
| **DesignCraft** | DesignCraft | Page layout, desktop publishing & typography | Core creative app | Adopted (`rev = "5c874d5"`) |
| **PdfCraft** | PdfCraft | PDF document inspection, preflight & structural UI | Core creative app | Adopted (`rev = "5c874d5"`) |
| **LightCraft** | LightCraft | Non-destructive RAW image processing & grading | Non-destructive image app | Adopted (`rev = "5c874d5"`) |
| **FilmCraft** | FilmCraft | Nonlinear timeline video editing & multi-track media | Creative pro video | Adopted (`rev = "5c874d5"`) |
| **EffectCraft** | EffectCraft | Motion graphics, visual effects & compositing | Compositing app | Adopted (`rev = "5c874d5"`) |
| **CADCraft** | CADCraft | Precision 2D/3D CAD drafting & geometric constraints | Precision engineering app | Adopted (`rev = "5c874d5"`) |
| **SoundCraft** | SoundCraft | Multi-track digital audio workstation & DSP timing | Audio production app | Adopted (`rev = "5c874d5"`) |
| **WordCraft** | WordCraft | Structured document authoring & rich text processing | Document suite app | Component adoption (`ZoomBar`) |
| **DeckCraft** | DeckCraft | Slide presentations & visual storytelling | Presentation suite app | Component adoption (`ZoomBar`) |
| **GridCraft** | GridCraft | High-performance calculation engine & spreadsheets | Calculation suite app | Component adoption (`ZoomBar`) |
| **FigCraft** | FigCraft | Collaborative interface design & vector prototyping | Collaborative design app | Standalone UI crate |

---

## 4. Epistemic Integrity & Claim Taxonomy

Every status report, technical review, and verification log must enforce the strict Four-Tier Claim Taxonomy:

$$\text{Documented} \neq \text{Implemented} \neq \text{Tested} \neq \text{Empirically Validated}$$

- **Durability Ordering vs. Crash Durability**: The sequence `sync_file < rename < sync_dir` proves ordering and kernel dispatch. Software unit tests do not prove survival across physical power interruption or unbuffered drive controllers. State guarantees plainly without claiming unvalidated hardware durability.
- **Proposed Contracts vs. Project Policy**: Architectural RFCs and shared contracts must remain framed as proposals until maintainers explicitly adopt them.

---

## 5. Agent Output Integrity & Attribution Rules

- **Zero AI Attribution**: Strictly forbid `Co-Authored-By` trailers, "generated by AI" footers, assistant session identifiers, or model names anywhere in code, commits, documentation, or public pull requests.
- **Engineering Voice**: Direct, technical, active-voice human engineering prose. State problems, exact changes, verified facts, and concrete test numbers plainly without promotional language or filler.
- **Grounded References**: Always link directly to the exact pull request, commit, or issue (e.g. `[storytold/craft-libs#9](https://github.com/storytold/craft-libs/pull/9)`).
- **Natural Punctuation**: Avoid overusing em-dashes and academic thesis headings. Use natural, direct sentences with standard punctuation.
- **Natural Language Over Synthetic Identifiers**: Never invent artificial tracking codes. Use standard tooling and domain terminology.
