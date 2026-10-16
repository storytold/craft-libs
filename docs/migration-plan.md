# Craft Libraries: Proposed Crate Migration and Structure

> **Status:** Proposed architecture and migration plan; not an implementation record.  
> **Scope:** `storytold/craft-libs` and the 13 Craft applications.  
> **Basis:** [`ecosystem-architecture.md`](ecosystem-architecture.md), [`crate-audit.md`](crate-audit.md), [`engineering-methodology.md`](engineering-methodology.md), [`media-infrastructure.md`](media-infrastructure.md), and [`shared-infrastructure.md`](shared-infrastructure.md) updated October 2026.

## Purpose

Consolidate reusable functionality without forcing applications to share incompatible domain models. The audit reports 356 internal crates across 14 repositories in `craft-compat`. Similar architecture and names do **not** establish interchangeable implementations or verified downstream adoption.

**Status definitions:** **Existing** = documented in the audit as present upstream, although consumer integration may be unverified; **Extract** = candidate to move/adapt existing implementation; **New** = proposed shared contract; **Evaluate** = do not migrate until overlap and consumer demand are established.

## 1. Proposed migration matrix

| Priority | Group | Target crate | Existing source | Migration approach | Intended consumers |
|---|---|---|---|---|---|
| Existing / pending | Foundation | `craft-time` | Shared proposal | Qualify signed-time and timebase contracts before adoption | FilmCraft, EffectCraft, SoundCraft |
| Existing | Foundation | `craft-atomic` | `craft-libs` (PR #16 merged) | Preserve; PR #43 adds `PlatformDefault` new-file permissions; qualify downstream adoption | Document-based apps |
| Existing | Foundation | `craft-settings` | `craft-libs` (PR #22 merged) | Preserve; qualify downstream adoption and backup recovery | All apps |
| Existing | Application | `craft-quit` | `craft-libs` (PR #5, #21 merged) | Preserve primitive; leave dirty-document lifecycle to apps; decouple muda handler | Desktop apps |
| Existing / pending | Application | `craft-menu` | `craft-libs` (PR #65, #66) | macOS menu bar, muda event loop wakeup, AppKit input isolation; unblocks WordCraft #1254, #985, PDFCraft #904 | Desktop apps |
| Existing | Application | `craft-ui` | `craft-libs` (PR #6 merged) | Pinned dependency contracts (`5c874d5` $\to$ `tag = "v0.1.0"` in PR #41); PR #34 chrome unification; `ZoomBar` adopted in WordCraft (#1033), GridCraft (#779), DeckCraft (#267); docking adopted from `crates/ui/src/docking` (unblocks PhotoCraft #4903, VectorCraft #1834) | Desktop/web apps |
| P1 | Application | `craft-ui` widgets | PhotoCraft, VectorCraft, GridCraft, DesignCraft | **Extract Tier C widgets (Issue #67):** `NumericField` (bounds/scrubbing), `CanvasRuler` (zero point/units), `ColorPicker` (palette/recents), `ShortcutEditor` (keybind conflict map) | All 13 apps |
| Existing | Application | `craft-print` | `craft-libs` (PR #8, #28 merged) | Native in-process Windows spooler printing; extend adoption where print semantics match | DesignCraft, PDFCraft, PhotoCraft, VectorCraft |
| P1 | Media | `craft-media` | Shared media requirements | **New:** stream metadata, buffer contracts, capability descriptors, errors | FilmCraft, EffectCraft, SoundCraft |
| P1 | Media | `craft-bitstream` | `filmcraft-bitstream` | **Extract:** bounded bit-level read/write primitives (8 tests passed, 0 unsafe, wasm32 clean) | Codec and container crates |
| P1 | Media | `craft-aac` | `filmcraft-aac` | **Extract:** clean-room AAC-LC encoder & decoder; CBR & VBR, 1 to 8 channels, 1024-sample priming (31 tests passed, 0 unsafe, wasm32 clean) | FilmCraft, EffectCraft, SoundCraft |
| P1 | Media | `craft-isobmff` | `filmcraft-isobmff` | **Extract:** clean-room MP4/MOV demuxer & progressive faststart muxer with edit lists (62 tests passed, 0 unsafe, wasm32 clean) | FilmCraft, EffectCraft, SoundCraft |
| P2 | Interchange | `craft-metafile` | `vectorcraft-metafile` | **Extract:** pure-Rust WMF/EMF reader & writer; unblocks DeckCraft #327 and WordCraft #802 | DeckCraft, VectorCraft, WordCraft |
| P2 | Application | `craft-control-wire` | Per-app control server code | **Extract:** common wire schemas, authentication tokens (craft-libs#32), and error contracts | All 13 apps |
| P2 | Collaboration | `craft-collab` | `figcraft-collab`, `figcraft-collab-net` | **Evaluate/extract:** provider-neutral connection/presence primitives; inform PR #48 / #52 | FigCraft, prospective suite apps |
| P2 | Media | `craft-flac` | `filmcraft-flac`, SoundCraft audio I/O | **Evaluate/extract:** compare codec implementations and licenses | FilmCraft, SoundCraft |
| P2 | Media | `craft-opus` | `effectcraft-opusenc` | **Evaluate:** extract once a second consumer or stable public contract exists | EffectCraft, prospective FilmCraft |
| P2 | Media | `craft-audio-io` | `soundcraft-audio-io` | **Evaluate:** reusable file/buffer I/O only; keep DAW bounce workflow local | SoundCraft, prospective FilmCraft |
| P3 | Graphics | `craft-geom-core` | Ten geometry crates | **Extract:** only coordinate-neutral 2D math; retain domain units | Graphics/layout apps |
| P3 | Graphics | `craft-color-math` | Nine color crates | **Extract:** RGB/XYZ matrices, LUT interpolation; retain domain color semantics | Graphics/photo/video apps |
| P3 | Graphics | `craft-fonts` | Five font crates, `craft-fonts` repo | **Evaluate:** OpenType parsing/shaping overlap and license constraints | DesignCraft, WordCraft, PDFCraft, DeckCraft, FigCraft |
| P3 | Media | `craft-h264` | FilmCraft H.264 encoder | **Evaluate/extract:** codec-specific tests and second-consumer need | FilmCraft, EffectCraft |
| P3 | Media | `craft-prores` | FilmCraft ProRes implementation | **Evaluate/extract:** codec-specific conformance | FilmCraft, EffectCraft |
| P3 | Media | `craft-dnx` | FilmCraft DNx implementation | **Evaluate:** format behavior and consumers | FilmCraft, EffectCraft |
| P3 | Media | `craft-mxf` | FilmCraft MXF implementation | **Evaluate:** interchange and container requirements | FilmCraft, EffectCraft |
| P4 | Documents | `craft-document-core` | App document models | **Evaluate only:** stable identity/versioning primitives; not a shared document model | Document apps |
| P4 | Documents | `craft-text-core` | WordCraft, DesignCraft, PDFCraft | **Evaluate:** reusable text contracts without collapsing layout semantics | WordCraft, DesignCraft, PDFCraft |
| P4 | Graphics | `craft-image-core` | PhotoCraft, LightCraft, FilmCraft | **Evaluate:** pixel buffers/metadata without forcing renderer formats | PhotoCraft, LightCraft, FilmCraft |
| P4 | Application | `craft-command` | Thirteen command registries | **Evaluate:** common reflection/wire contract, not shared engine state | All 13 apps |

**Verification Note:** Source-level inspection and local test execution have verified: (1) Exactly 9 creative applications declare pinned dependencies on `craft-ui` (`5c874d5`), with migration targeted to `tag = "v0.1.0"` (PR #41), while Office-style apps (WordCraft #1033, GridCraft #779, DeckCraft #267) adopt shared components like `ZoomBar` and PR #34 chrome; (2) Candidate media crates (`filmcraft-bitstream`, `filmcraft-aac`, `filmcraft-isobmff`) pass **101 local tests with 0 failures**, zero `unsafe` blocks, and clean compilation for `wasm32-unknown-unknown`; (3) Maintainers (`aiongg`, `moonw0man`) in Discord explicitly invited an RFC for `craft-media-enc` referencing `craft-libs#2` and `craft-libs#24`; (4) `vectorcraft-metafile` provides a complete, tested pure-Rust EMF/WMF implementation ready to resolve DeckCraft #327 and WordCraft #802; (5) `figcraft-collab` and `figcraft-collab-net` provide an authoritative clean-room reference for document collaboration and presence. Level 3 adoption qualification (sample-exact decoded duration through SoundCraft Bounce Mix) remains to be empirically executed once upstream lands the crates. Pre-composed ticket drafts are maintained in `TICKET_DRAFTS.md`.

## 2. Proposed `craft-libs` workspace layout

The following is a **target layout**, not the existing filesystem. Cargo package names remain `craft-*`; directory moves must be independently reviewed for manifest, tooling, and consumer path impacts.

```text
craft-libs/
├── crates/
│   ├── foundation/
│   │   ├── time/               # craft-time (proposal/qualification)
│   │   ├── atomic/             # craft-atomic
│   │   └── settings/           # craft-settings
│   ├── application/
│   │   ├── ui/                 # craft-ui
│   │   ├── print/              # craft-print
│   │   ├── quit/               # craft-quit
│   │   ├── control-wire/       # craft-control-wire
│   │   └── command/            # craft-command (evaluate)
│   ├── media/
│   │   ├── core/               # craft-media
│   │   ├── bitstream/          # craft-bitstream
│   │   ├── aac/                # craft-aac
│   │   ├── flac/               # craft-flac
│   │   ├── opus/               # craft-opus
│   │   ├── audio-io/           # craft-audio-io
│   │   ├── isobmff/            # craft-isobmff
│   │   ├── h264/               # craft-h264
│   │   ├── prores/             # craft-prores
│   │   ├── dnx/                # craft-dnx
│   │   └── mxf/                # craft-mxf
│   ├── graphics/
│   │   ├── geom-core/          # craft-geom-core
│   │   ├── color-math/         # craft-color-math
│   │   ├── image-core/         # craft-image-core
│   │   └── fonts/              # craft-fonts
│   ├── documents/
│   │   ├── document-core/      # craft-document-core
│   │   └── text-core/          # craft-text-core
│   └── interchange/
│       └── metafile/          # craft-metafile
├── xtask/
│   ├── ci/
│   ├── compat/
│   ├── conformance/
│   └── adoption/
├── tests/
│   ├── fixtures/
│   ├── golden/
│   └── interoperability/
├── docs/
│   ├── architecture/
│   ├── contracts/
│   ├── migrations/
│   └── adoption/
└── Cargo.toml
```

The `xtask/` and `tests/` subdirectories above are conceptual organizational targets; the actual Rust workspace tooling layout should follow current repository conventions.

## 3. Migration phases and acceptance evidence

| Phase | Scope | Prerequisites | Required proof |
|---|---|---|---|
| **0: Baseline** | Existing shared crates (`craft-atomic`, `craft-settings`, `craft-quit`, `craft-print`, `craft-ui`) and consumers | Coordinate on `tag = "v0.1.0"` baseline (PR #41, PR #43); record build/test baseline | Reproducible multi-platform CI, lockfile alignment, and compat test reports |
| **1: Media foundation** | `craft-media`, `craft-bitstream` | Review time/buffer/error contracts and FilmCraft dependencies | Unit tests, WASM checks, no app dependencies |
| **2: First extraction** | `craft-aac`, bounded `craft-isobmff` | Phase 1 | Existing vectors; independent decoder and container checks |
| **3: First adoption** | SoundCraft AAC/M4A | Phase 2 | End-to-end bounce, duration/priming, metadata, quality |
| **4: Existing consumer migration** | FilmCraft and EffectCraft AAC imports | Phase 3 | Consumer baseline comparison and regression evidence |
| **5: Interoperability** | `craft-metafile`, `craft-control-wire` | Per-crate contract review | Cross-consumer fixtures and wire tests |
| **6: Additional media** | FLAC, Opus, H.264, ProRes, DNx, MXF | Demonstrated demand | Codec-specific conformance and export tests |
| **7: Shared math** | `craft-geom-core`, `craft-color-math` | Unit/precision semantics review | Differential tests against source implementations |
| **8: Higher-level reuse** | Fonts, image, text, document, commands | Multiple compatible consumers | Behavioral equivalence and migration tests |

Do **not** migrate all 16 EffectCraft dependencies on FilmCraft as a single batch. The initial migration is AAC-specific; evaluate other dependencies separately.

## 4. Application-owned components that should remain local

| Component | Decision | Boundary |
|---|---|---|
| `soundcraft-engine` | Keep | DAW session, bounce command, transport behavior |
| `filmcraft-engine` | Keep | Editing and timeline semantics |
| `effectcraft-engine` | Keep | Composition and animation behavior |
| `photocraft-compose` | Keep initially | PhotoCraft raster adjustment and compositing behavior |
| `cadcraft-geom` | Keep domain layer | CAD precision, coordinate systems, tolerances |
| `wordcraft-geom` | Keep domain layer | Office layout units and conversions |
| `designcraft-color` | Keep domain layer | Prepress separations and spot colors |
| `filmcraft-color` | Keep domain layer | Broadcast transfer/range semantics |
| App `doc` / `model` | Keep | Application document identity and persistence |
| App `ui-egui` | Keep | Domain interaction and presentation |
| App control servers | Keep adapters | Share wire types, not execution state |

## 5. Migration register template

Maintain one record per extraction and one per downstream adoption.

| Field | Required content |
|---|---|
| Source repository and revision | Exact immutable source provenance |
| Source crate | Original implementation and license |
| Target crate | `craft-libs` package and planned directory |
| Contract owner | Maintainer responsible for API acceptance |
| Dependency closure | Internal/external dependencies and platform requirements |
| Baseline tests | Source tests and golden vectors before extraction |
| Consumer adapters | Necessary app-local integration changes |
| Compatibility evidence | Builds, test logs, differential results, fixture versions |
| Adoption status | Proposed / implemented / verified / adopted |
| Rollback strategy | How to revert without breaking other consumers |

**Promotion lifecycle:** Discover → Validate → Propose → Implement → Prove → Adopt. A merged shared crate does not establish downstream adoption; a compiling consumer does not establish behavioral equivalence. All candidate crates progress through the **Three-Level Quality Gate Hierarchy**:
* **Level 1: Crate Qualification (Standup Gates)**: Zero-warning formatting, clippy lints, 100% unit tests, and clean WebAssembly compilation (`wasm32-unknown-unknown`).
* **Level 2: Extraction Qualification (Parity Gates)**: Satisfaction of the Extraction Provenance Contract, decoupling from application crates, bitstream conformance against reference oracles (`ffmpeg`).
* **Level 3: Adoption Qualification (Integration Gates)**: Consumer builds, sample-exact priming and duration compensation, SNR $> 45$ dB, and non-regression verification via `cargo xtask compat`.

## 6. First milestone: AAC/M4A proof

**Target:** SoundCraft AAC/M4A bounce using extracted FilmCraft functionality (`craft-aac`, `craft-isobmff`), with FilmCraft/EffectCraft non-regression checks.

1. **API & Feature Verification (Completed at source level)**:
   * `filmcraft-aac` contains both `Encoder` and `Decoder`.
   * CBR and VBR (1.0 to 5.0) are implemented via `BitrateMode`.
   * Channel layouts 1 to 8 (Mono up to 7.1/8ch) are supported.
   * Priming is explicitly calculated as 1024 samples (`FRAME_LEN`).
2. **Buffer & Contract Definition**:
   * Define `craft-media` buffer (`AudioBufferRef`), timestamp, and error contracts without silently adopting unresolved `craft-time` proposals.
3. **Extraction & Decoupling**:
   * Extract `craft-bitstream`, `craft-aac`, and `craft-isobmff` into `craft-libs`, preserving upstream test vectors and commit provenance under the Extraction Provenance Contract.
4. **SoundCraft Integration**:
   * Adapt SoundCraft planar audio buffers to `AudioBufferRef`, implement polyphase 2:1 decimation for 192 kHz sessions (down to 96 kHz), and wire the Bounce Mix dialog and `file.bounce_mix` engine command.
5. **Empirical Verification (Level 3 Adoption)**:
   * Verify independent decoding via `ffprobe`/`ffmpeg`, validate container box layout (`ftyp`, `moov/trak/edts/elst`), confirm sample-exact source duration and 1024-sample delay compensation, and verify objective SNR $> 45$ dB.
6. **Consumer Repinning**:
   * Repin FilmCraft and EffectCraft AAC/ISOBMFF imports to `craft-libs` and verify zero regressions via `cargo xtask compat`.

**Verification status:** Core codec scope (encoder, decoder, CBR/VBR, 1 to 8 channels, 1024-sample priming) and container muxing are verified at source level in FilmCraft. What remains to be proven under Level 3 Adoption Qualification is sample-exact decoded duration through SoundCraft's Bounce Mix end-to-end, container edit-list priming compensation, and perceptual audio quality in DAW export.

## 7. Companion documents

- [`shared-infrastructure.md`](shared-infrastructure.md): Comprehensive shared infrastructure specification covering all 18 candidate crates.
- [`media-infrastructure.md`](media-infrastructure.md): Focused media infrastructure specification and AAC/M4A proof plan.
- [`crate-audit.md`](crate-audit.md): Crate inventory, adoption claims, and extraction candidates.
- [`engineering-methodology.md`](engineering-methodology.md): Engineering methodology, testing hierarchy, and promotion lifecycle.
- [`downstream-adoption.md`](downstream-adoption.md): Downstream consumer adoption and integration guide.

All phases and target directories are **proposals**. No code migration or repository modification is implied by this document.
