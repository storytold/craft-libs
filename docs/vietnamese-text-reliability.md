# Vietnamese Text Reliability Architecture & Roadmap
**A Contract-Driven, Verifiable Specification for Vietnamese Text Fidelity across the Storytold Craft Ecosystem**

* **Status:** Draft / Cross-App Architecture Specification
* **Target Location:** `storytold/craft-libs/docs/vietnamese-text-reliability.md`
* **Target Module:** `craft-i18n::encoding` (centralized in `craft-libs`, avoiding unneeded workspace crate proliferation)
* **Related References:** `craft-libs#198`, `craft-libs#480`, `craft-fonts#62`, `wordcraft#5189`, `cadcraft#1877`, `pdfcraft#5389`

---

## 1. Architectural Scope & Pipeline Overview

Reliable processing of Vietnamese text in high-performance desktop and WebAssembly Rust applications (`WordCraft`, `CADCraft`, `PDFCraft`, `GridCraft`, `DeckCraft`, `FigCraft`) involves navigating legacy 8-bit artifacts, Unicode canonical equivalences, input method state machines, and OpenType typographic nuances:

```
[ External Documents / Clipboard / Raw Bytes / OS IME ]
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ Tier 1: Input Boundary — Ingestion & Normalization          │  craft-i18n::encoding
│   - Decoupled Primitives: detect, decode, normalize         │  (Issue #480)
│   - Windows-1258 Official Unicode Mapping (CP1258.TXT)      │
│   - Explicit Evidence Hierarchy & Audited Byte Tables       │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ Tier 2: Text Core — Grapheme Cluster & Mutation Contract    │  craft-text::segment
│   - UAX #29 Extended Grapheme Cluster Navigation            │
│   - UTF-8 Boundary Safe Cursor & Selection Invariants      │
│   - Atomic Cluster Deletion (Isomorphic NFC/NFD Backspace)  │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ Tier 3: Input Boundary — IME Composition Contract           │  craft-ui / egui & winit forks
│   - Pre-edit vs. Committed Buffer Isolation                 │
│   - Non-destructive Transient Composition Handling          │
│   - Deferred Canonical Normalization on Commit Event        │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ Tier 4: Layout & Shaping Engine Integration                 │  Downstream Layout Engines
│   - Verified OpenType Default Shaping (ccmp / mark / mkmk)  │  (WordCraft, PDFCraft)
│   - Extended Grapheme Cluster Level Font Fallback           │
│   - Quantified Metric Compatibility & Perceptual Tolerances │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ Tier 5: Layout — Line Breaking (UAX #14) & Metric Hygiene   │  craft-text / Layout Engine
│   - Intra-Cluster Break Prohibition                         │
│   - Non-Breaking Space (U+00A0) Glue Semantics              │
│   - Typographic Orphan / Widow Decoupling                   │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ Tier 6: Asset Verification — Font & Outline Coverage Guard  │  craft-fonts-tests
│   - 134 Precomposed NFC Letter Coverage & Vector Outlines   │  (PR #62)
│   - Pinned Known Gaps (Caladea / Poppins 88 Codepoints)     │
│   - Continuous NFD Combining Mark Auditing (8 Marks)        │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Tiered Technical Specifications

---

### Tier 1: Input Boundary — Ingestion & Normalization

* **Target Location:** `crates/i18n/src/encoding/` in `craft-libs`
* **Current Status:** Proposal `craft-libs#480`
* **Workspace Convention:** Following Issue #198, encoding primitives belong to `craft-i18n` rather than creating a fragmented top-level crate, preserving cohesive dependencies across `craft-i18n`, `craft-text`, and `craft-ui`.
* **Existing Evidence:**
  * `cadcraft#1877` introduces an ad-hoc 8-bit transcoding profile for `.dxf`/`.dwg` text entities.
  * `pdfcraft#5389` duplicates NFC normalization via `unicode-normalization` for headers and watermarks.
  * `wordcraft#5189` reveals shaping failures when ingesting non-canonical or legacy-encoded text.

#### Architectural Separation of Concerns
To maintain strict testability and zero-side-effect execution, Tier 1 is decomposed into **three independent, single-responsibility primitives** and a high-level facade:
1. `detect_encoding`: Establishes encoding evidence without mutating data.
2. `decode`: Translates raw bytes of a verified encoding into a raw Unicode UTF-8 string without hidden normalization side effects.
3. `normalize_nfc`: Independent canonical normalization with verified memory allocation contracts.
4. `decode_text`: Consumer facade combining detection, decoding, and optional normalization.

#### Primitive 1: Detection & Evidence Hierarchy
Heuristic algorithms must never silently guess or mutate valid text. Evidence precedence and conflict resolution rules are strictly defined:

| Evidence Level | Source & Description | Conflict & Ambiguity Resolution |
| :--- | :--- | :--- |
| **`Explicit`** | Explicitly passed by the caller. Absolute priority. | Overrides all other signals. No heuristics run. |
| **`Metadata`** | Container headers or file format variables (e.g., DXF `$DWGCODEPAGE`). | Conflicting metadata returns `DetectionResult::Ambiguous`. |
| **`FontHint`** | Inferred from font name prefixes (`.Vn*` $\to$ TCVN3, `VNI-*` $\to$ VNI). | Advisory only. Conflicting font hints return `DetectionResult::Ambiguous`. |
| **`Heuristic`** | Statistical analysis of byte frequency distribution. | **Never overrides valid UTF-8.** If confidence is below threshold, returns `DetectionResult::Unknown`. |

**Font Hint Normalization:**
All `FontHint` string matching MUST be case-insensitive (`to_ascii_lowercase()`) and recognize historical legacy stems (e.g., `.vn*` $\to$ TCVN3, `.vnh` / `*h` $\to$ TCVN3H, `vni-` $\to$ VNI-Windows).

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LegacyEncoding {
    Tcvn3,
    Tcvn3H,
    Vni,
    Windows1258,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodingEvidence {
    Explicit,
    Metadata,
    FontHint,
    Heuristic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionResult {
    /// High-confidence detection backed by explicit evidence or verified signatures.
    HighConfidence {
        encoding: LegacyEncoding,
        evidence: EncodingEvidence,
    },
    /// Multiple candidate encodings match valid byte distributions equally, or metadata conflicts.
    Ambiguous(&'static [LegacyEncoding]),
    /// Byte patterns match no known Vietnamese encoding profile.
    Unknown,
}

pub struct EncodingMetadata<'a> {
    pub font_name: Option<&'a str>,
    pub codepage_hint: Option<u32>,
}

pub fn detect_encoding(
    input: &[u8],
    metadata: Option<&EncodingMetadata<'_>>,
) -> DetectionResult;
```

#### Primitive 2: Audited 8-Bit Decoding & Windows-1258 Invariants
Single-byte decoding tables map the high byte range `0x80..=0xFF`. Because some byte positions in legacy charsets are undefined, tables use `[Option<char>; 128]` to represent unmapped bytes explicitly without masking them into replacement characters:

```rust
pub type HighByteTable = [Option<char>; 128];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    InvalidUtf8 { offset: usize },
    UnmappedByte { byte: u8, offset: usize, encoding: LegacyEncoding },
    MalformedSequence { offset: usize, encoding: LegacyEncoding },
}

/// Pure decoding primitive: maps bytes strictly to Unicode codepoints without hidden normalization.
pub fn decode(
    input: &[u8],
    encoding: LegacyEncoding,
) -> Result<Cow<'_, str>, DecodeError>;
```

**Official Windows-1258 Specification Compliance (`CP1258.TXT`):**
Windows-1258 maps Latin-1 accented vowels directly, but uses five explicit combining diacritics in the high-byte space. Decoders must map these bytes strictly to their defined Unicode Combining Diacritical Marks:
* `0xCC` $\to$ `U+0300` (Combining Grave Accent / Huyền)
* `0xEC` $\to$ `U+0301` (Combining Acute Accent / Sắc)
* `0xD2` $\to$ `U+0309` (Combining Hook Above / Hỏi)
* `0xDE` $\to$ `U+0303` (Combining Tilde / Ngã)
* `0xF2` $\to$ `U+0323` (Combining Dot Below / Nặng)
* **9 Undefined / Unmapped Bytes:**
  `0x81, 0x8A, 0x8D, 0x8E, 0x8F, 0x90, 0x9A, 0x9D, 0x9E` are mapped to `None` in `HighByteTable`, returning `Err(DecodeError::UnmappedByte)`.

*Separation Invariant:* `decode(input, LegacyEncoding::Windows1258)` **does not perform implicit NFC normalization**. It outputs the exact sequence of base characters and combining marks. Canonical normalization is deferred to `normalize_nfc` or the `decode_text` facade when requested.

#### Practical vs. Strict Mode Contracts

To survive real-world documents (1995–2005), the decoder must support two modes:

1. **Strict Mode (Specification Only):** Maps only canonical specification bytes (TCVN 5712:1993, VNI standard, or CP1258.TXT). Throws `DecodeError::UnmappedByte` on any non-standard byte (e.g., CP1252 punctuation).
2. **Practical Mode (Default for Document Ingestion):** 
   - **CP1252 Fallback:** Maps the `0x80..=0x9F` range to Windows-1252 punctuation (curly quotes, en/em dashes, bullets, ellipses) to prevent losing punctuation in legacy Word/CAD documents.
   - **VNI CapsLock Handling:** Recognizes uppercase VNI tone marks (`0xD8` $\to$ `Ø`, `0xD9` $\to$ `Ù`, `0xDA` $\to$ `Ú`, `0xD5` $\to$ `Õ`, `0xCF` $\to$ `Ï`) and maps them correctly to uppercase Vietnamese characters.
   - **Orphan Diacritic Fallback:** If a VNI mark byte is not preceded by a valid base vowel, it is emitted as a standalone spacing accent (e.g., `\u{00B4}` for acute) rather than being dropped or causing a panic.
   - **Streaming State Machine:** The VNI decoder maintains a `pending_base` state to handle chunk boundaries without breaking composite characters (base + mark).

#### Font Hint Overrides (TCVN3H)
If metadata indicates an uppercase font (e.g., `.VnTimeH`), the decoder MUST use the `TCVN3_UPPER_MAP` instead of the standard lower map, ensuring legacy all-caps headers render correctly.


#### Primitive 3: Canonical Normalization & Allocation Contract
```rust
pub fn normalize_nfc(input: &str) -> Cow<'_, str>;
```
The fast-path memory allocation behavior is strictly defined:
| Input State | Result Type | Allocation Behavior |
| :--- | :--- | :--- |
| **Pure ASCII** | `Cow::Borrowed(&str)` | Zero allocation (`0` bytes allocated). |
| **UTF-8 Canonical NFC** | `Cow::Borrowed(&str)` | Zero allocation (`0` bytes allocated, checked via `is_nfc`). |
| **UTF-8 Decomposed NFD** | `Cow::Owned(String)` | Re-allocated only when normalization alters text. |
| **Legacy Transcoded Bytes** | `Cow::Owned(String)` | Allocated during byte-to-char transformation. |

#### Composite Facade (`decode_text`)
For consumer convenience, `craft-i18n` exposes a unified facade combining the three primitives:
```rust
#[derive(Debug, Clone, Copy)]
pub struct DecodeOptions<'a> {
    pub force_encoding: Option<LegacyEncoding>,
    pub metadata: Option<&'a EncodingMetadata<'a>>,
    pub normalize_nfc: bool,
}

impl Default for DecodeOptions<'_> {
    fn default() -> Self {
        Self {
            force_encoding: None,
            metadata: None,
            normalize_nfc: true, // Default to canonical NFC for safe consumption across Craft apps
        }
    }
}

pub fn decode_text<'a>(
    input: &'a [u8],
    options: &DecodeOptions<'a>,
) -> Result<Cow<'a, str>, DecodeError>;
```
* *Policy on Valid UTF-8:* If `options.force_encoding` is `None` and `input` is valid UTF-8, `decode_text` **never runs heuristic transcoding**. Valid UTF-8 is preserved as UTF-8, applying only `normalize_nfc` if requested.
* *Policy on Windows-1258:* If `options.normalize_nfc` is `true`, `decode_text` passes the decoded output through `normalize_nfc()`. If `false`, the semi-decomposed string is returned verbatim.

#### Tier 1 Extension: Robust Decoding Contract & Invariants

##### 1. Operating Modes: Strict vs. Practical
Consumers may configure the decoding pipeline with one of two behavioral modes:
* `DecodeMode::Strict`: Enforces rigid conformance to standard specification tables (TCVN 5712:1993, VNI, CP1258.TXT). Any unmapped byte, orphan diacritic, or truncated sequence immediately halts execution and returns an explicit `DecodeError`.
* `DecodeMode::Practical` (Default for Craft Apps): Optimized for real-world document tolerance (1995–2005 archival corpora).
  - Preserves Windows-1252 typographic punctuation in the `0x80..=0x9F` range (curly quotes `“ ”`, em-dash `—`, ellipsis `…`).
  - Flushes pending base vowels at EOF safely.
  - Converts orphan combining marks into standalone spacing accents rather than discarding bytes.
  - Emits non-fatal diagnostic warnings via `tracing` when encountering legacy VSCII-1 control codes.

##### 2. Contract 1: One-Way Canonical Normalization & Lossy Tolerance
* **The One-Way Bridge:** Decoding is formally defined as a one-way transformation from legacy byte streams to canonical UTF-8 NFC:
  $$\text{Legacy Bytes (TCVN3 / VNI / CP1258)} \xrightarrow{\text{Decode}} \text{Canonical UTF-8 NFC}$$
* **Lossy-Tolerant Invariant (Non-goal):** 
  * Reverse transcoding ($E(D(B)) = B$) is explicitly out of scope and non-deterministic due to NFC diacritic composition and lossy punctuation normalization.
  * Every string returned by `decode_text` is guaranteed to be in **Unicode Normalization Form C (NFC)** via `icu_normalizer` when `normalize_nfc` is enabled (the default). When `normalize_nfc` is set to `false`, raw decoded codepoints (such as CP1258 combining diacritics) are returned without composition. Downstream text shapers, search indices, and layout engines will never receive decomposed combining marks that NFC can compose. Marks without a precomposed form, such as U+0301 after `k`, can remain decomposed under the default configuration.

##### 3. Contract 2: Pending-at-EOF Policy & Truncated Sequence
VNI is a multi-byte composite encoding (Base character + Diacritic mark byte). When the data stream terminates (EOF) or encounters an anomalous sequence, the decoder follows a deterministic two-mode matrix:

| Situation at EOF | Mode: `Strict` | Mode: `Practical` (Default for Craft Apps) |
| :--- | :--- | :--- |
| **Pending base vowel at EOF**<br>*(e.g., stream terminates with byte `'a'`)* | **Flushes pending base vowel to output** safely as regular character (`'a'`). | **Flushes pending base vowel to output** safely as regular character (`'a'`). |
| **Orphan diacritic mark**<br>*(e.g., mark byte `0xF8` following a space or consonant `k`)* | Returns `Err(DecodeError::OrphanDiacritic { byte, offset })` | **Emits standalone spacing accent** (e.g. `0xF8` $\to$ `` ` `` `U+0060`), never panics or drops bytes. |
| **DOS control bytes (`0x01..=0x1F`)**<br>*(Legacy VSCII-1 vestiges in old CAD files)* | Returns `Err(DecodeError::StrictRejection { byte, offset })` | Emits non-fatal diagnostic warning via `tracing::warn!` and sanitizes as whitespace. |

##### 4. Contract 3: Ambiguity Rejection & Heuristic Threshold
To prevent autonomous detectors from over-confident false transcoding on short or ambiguous byte sequences:
* **Heuristic Activation Threshold:** Autonomous encoding detection requires an input length $\ge 16$ bytes and at least 2 distinct signature distributions (e.g., TCVN3 via pairs `0xA7`/`0xAE` or double vowels `0xCA`, `0xD4`; VNI via postfix byte pairs `0xF8`, `0xF9`, `0xEF`).
* **Short-String Invariant (< 16 bytes):** Strings shorter than 16 bytes automatically disable statistical byte-frequency heuristics to prevent false positives. Short strings rely strictly on the Evidence Hierarchy:
  1. Valid UTF-8 check (preserved as UTF-8).
  2. `FontHint` presence (e.g., `.VnTime`, `VNI-Times`) implies immediate decoding.
  3. If neither is present, return `DetectionResult::Unknown` (or fallback to CP1252 in Practical Mode) rather than speculative guessing.
* **Conflict & Safety Rules:** If candidate encodings (e.g., TCVN3 vs. VNI-Windows / CP1258) yield conflicting confidence scores differing by less than $30\%$, detection yields `DetectionResult::Ambiguous { candidates }`. The system must prompt caller intervention rather than executing speculative transcoding.

##### 5. Contract 4: Explicit Error Model (`DecodeError`)
Eliminates generic errors, providing unambiguous context for failures:
```rust
#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum DecodeError {
    #[error("Invalid UTF-8 sequence at offset {offset}")]
    InvalidUtf8 { offset: usize },

    #[error("Unmapped byte 0x{byte:02X} at offset {offset} in encoding {encoding:?}")]
    UnmappedByte { byte: u8, offset: usize, encoding: LegacyEncoding },

    #[error("Truncated composite sequence at EOF (pending base byte 0x{base:02X})")]
    TruncatedSequence { offset: usize, base: u8 },

    #[error("Orphan diacritic mark 0x{byte:02X} at offset {offset} without a preceding base vowel")]
    OrphanDiacritic { byte: u8, offset: usize },

    #[error("Ambiguous encoding detected between candidates {candidates:?}")]
    AmbiguousEncoding { candidates: &'static [LegacyEncoding] },

    #[error("Decoding mode Strict rejected non-standard byte 0x{byte:02X} at offset {offset}")]
    StrictRejection { byte: u8, offset: usize },
}
```

##### 6. Contract 5: Performance & Bounded Linear Allocation
* **Zero-Panic Policy:** `#![forbid(unsafe_code)]`. Memory safety invariants enforced across all paths; fuzz-tested with `cargo-fuzz` on arbitrary byte streams without unexpected panics.
* **Bounded Linear Allocation $O(N)$:** Decoder capacity is estimated once upfront (`String::with_capacity(input.len() * 6 / 5)`). No re-allocations during steady-state VNI composite streaming.

##### 7. Real-World Archival Test Corpora (Mandatory Fixtures)
The implementation must pass validation against authentic, anonymized test fixtures in `crates/i18n/src/encoding/fixtures/`:
1. `tcvn3_cad_dwg.txt`: Technical drawing text entities using `.VnTime` and `.VnTimeH`.
2. `vni_contract_1998.txt`: Business contract text employing uppercase/lowercase VNI-Times and mixed CapsLock diacritics (`0xD8..=0xD9`).
3. `cp1258_accounting.txt`: Tabular accounting exports containing hybrid precomposed and combining diacritics.
4. `clean_utf8_payload.txt`: Modern UTF-8 text, asserting zero byte alteration and zero memory allocation (`Cow::Borrowed`).

#### Verification & Test Fixtures
* **Semantic Verification over Byte-for-Byte Round-Trip:** Decoding is verified against curated fixtures (official TCVN 5712:1993, VNI standard, and CP1258 specification tables). Reverse encoding is tested only for representable subsets and semantic equivalence ($D(E(U)) = U$), recognizing that canonical normalization collapses non-unique legacy byte forms.
* **Full Byte Domain Audit:** Unit tests asserting every byte `0x00..=0xFF` produces either the exact Unicode mapping or `Err(DecodeError::UnmappedByte)`.
* **Criterion Benchmark Suite:** Measures throughput and heap allocation across:
  1. Pure ASCII payloads (assert 0 alloc).
  2. Canonical NFC payloads (assert 0 alloc).
  3. Decomposed NFD payloads (verify minimal re-alloc).
  4. Raw TCVN3 / VNI / Windows-1258 legacy bytes.

---

### Tier 2: Text Core — Grapheme Cluster & Mutation Contract

* **Target Crate:** `craft-text::segment` (in `craft-libs`)
* **Current Status:** Technical Specification
* **Existing Evidence:**
  * Document editors implementing deletion via `text.pop()` or single `char` steps peel combining marks off decomposed strings (e.g., deleting from `vi\u{0323}\u{0302}n` strips `\u{0302}`, leaving an invalid orphan mark `\u{0323}`).
  * Byte-index pointer arithmetic causes runtime panics when navigating across multibyte sequences.

#### Contract & Invariants
1. **UAX #29 Alignment:** Cursor navigation, selection boundaries, `Delete` (forward), and `Backspace` (backward) on committed document text must strictly snap to **Extended Grapheme Cluster** boundaries.
2. **Canonical Decomposition Ordering:** When verifying decomposed NFD clusters, combining marks must strictly adhere to Unicode Canonical Combining Class (ccc) ordering:
   $$\text{Base Vowel} + \text{ccc 220 (Combining Dot Below, } \texttt{U+0323}\text{)} + \text{ccc 230 (Combining Circumflex, } \texttt{U+0302}\text{)}$$
3. **Atomic Cluster Deletion:** Deleting a grapheme cluster on committed text removes the base consonant along with all attached combining diacritics as an indivisible unit, guaranteeing isomorphic mutation behavior between NFC and NFD strings.

#### Helper Function Signatures & Boundary Semantics
```rust
/// Returns the start byte offset of the extended grapheme cluster preceding `cursor`.
///
/// Preconditions:
/// - `cursor <= text.len()`
/// - `text.is_char_boundary(cursor)` is true.
///
/// Invariants:
/// - Always returns a valid UTF-8 char boundary.
/// - 0 <= return_value <= cursor.
pub fn prev_grapheme_offset(text: &str, cursor: usize) -> usize;

/// Returns the end byte offset of the extended grapheme cluster following `cursor`.
///
/// Preconditions:
/// - `cursor <= text.len()`
/// - `text.is_char_boundary(cursor)` is true.
///
/// Invariants:
/// - Always returns a valid UTF-8 char boundary.
/// - cursor <= return_value <= text.len().
pub fn next_grapheme_offset(text: &str, cursor: usize) -> usize;
```

**Cursor Position Boundary Matrix:**
The behavior of `prev` and `next` across cluster boundaries is strictly specified:

| Cursor Position | `prev_grapheme_offset(text, cursor)` | `next_grapheme_offset(text, cursor)` | Primary Use Case |
| :--- | :--- | :--- | :--- |
| **Start of cluster** | Start of preceding cluster (or `0` if at string start) | End of current cluster | Arrow Left / Delete |
| **Inside cluster** (e.g., between base vowel & combining mark) | Start of enclosing cluster | End of enclosing cluster | Recovery from unaligned cursor clicks |
| **End of cluster** | Start of cluster just finished | End of subsequent cluster (or `text.len()` if at string end) | Backspace / Arrow Right |

#### Verification
* Unit tests verifying atomic deletion across canonical NFD compound clusters (`vi\u{0323}\u{0302}n`).
* Fuzz testing cursor navigation across random UTF-8 slices to verify that `prev` and `next` never produce non-boundary offsets.

#### Non-Goals
* Diacritic unstacking / peel-back policies for committed document text.

---

### Tier 3: Input Boundary — IME Composition Contract

* **Target Crate:** `craft-ui` (in integration with `storytold` forks of `egui` and `winit`) [cite: 2.2.4]
* **Current Status:** Specification
* **Existing Evidence:**
  * Vietnamese IMEs (Unikey, OpenKey, EVKey, native OS IMEs) emit rapid sequences of simulated backspaces and replacement keys to compose accented words.
  * Running normalization or editor-level grapheme deletion over an active composition buffer disrupts the IME state machine, producing doubled characters (e.g., `vieên`) or dropped accents.

#### Contract
1. **Buffer Isolation:** Strict architectural separation between the **Pre-edit Buffer** (uncommitted text owned exclusively by the IME) and the **Committed Document Buffer** (owned by the editor core).
2. **Suspended Normalization:** Ingestion normalization (Tier 1) and cluster mutation rules (Tier 2) are suspended on text residing within the pre-edit buffer.
3. **Adapter Lifecycle States:** Platform adapters must map platform-specific window events into five discrete IME lifecycle events:
   * `PreeditStart`: Allocate/anchor composition region.
   * `PreeditUpdate`: Update transient composition string without polluting undo/redo history.
   * `PreeditCancel`: Discard transient composition cleanly.
   * `Commit`: Append finalized text to document buffer and trigger Tier 1 fast-path normalization.
   * `ReplaceRange`: Replace specified committed document range when requested by advanced IME engines.
4. **Composition Inhibition Invariant:**
   When the platform window backend signals an active composition context or receives simulated backspaces within an uncommitted text sequence, Tier 2 grapheme cluster deletion MUST be inhibited. Backspace semantics are deferred entirely to the active IME driver to prevent destroying transient composition states (e.g., Unikey sending fake backspaces to transform `a` into `á`).

#### Verification
* Synthetic mock event stream tests simulating rapid Telex/VNI key sequences.
* **Live OS Platform Integration Tests:** Verification against real OS IME event delivery on Windows (`Imm32`/`TSF`), Linux (`fcitx5`/`ibus`), and macOS Cocoa IME.

#### Non-Goals
* Bundling an internal Vietnamese IME engine inside application binaries.

---

### Tier 4: Layout & Shaping Engine Integration

* **Target Crate:** Application Layout Engines (`WordCraft`, `PDFCraft`, `DesignCraft`)
* **Current Status:** Research / Validation (`wordcraft#5189`)
* **Existing Evidence:**
  * When decomposed NFD sequences are shaped by fonts without OpenType GPOS `mkmk` (Mark-to-Mark) tables, secondary tone marks visually collide with vowel hats/horns instead of stacking cleanly.
  * Naive character-level font fallback can split an attached diacritic from its base glyph, routing the base to Face A and the combining mark to Face B.

#### Contract
1. **Shaping Output Verification:** Layout engines must preserve the applicable default shaping behavior for OpenType glyph composition (`ccmp`), mark-to-base positioning (`mark`), and mark-to-mark positioning (`mkmk`), verifying resulting glyph positions against representative Vietnamese NFC/NFD test fixtures.
2. **Cluster-Level Fallback:** When a face lacks a glyph, fallback evaluation must occur at the **Extended Grapheme Cluster** boundary to prevent isolating marks from base consonants across different fonts.
3. **Quantified Metric Compatibility:** Metric-compatible fallback substitution must verify that replacement faces preserve advance widths within $\pm 5\%$ and vertical metrics (ascender/descender/line gap) within $\pm 2\%$ of the original target face to avoid text reflow and vertical clipping.

#### Verification
* Golden-image visual regression tests comparing rasterized outputs against reference renders with explicit perceptual tolerances.
* Glyph placement verification checking that combining mark bounding boxes remain offset from base glyph contours.
* Fallback verification asserting zero `.notdef` emissions when rendering the 88 `Caladea`/`Poppins` gap codepoints.

#### Non-Goals
* Writing a custom font shaping engine (the ecosystem delegates shaping to HarfBuzz / Harfrust).

---

### Tier 5: Layout — Line Breaking (UAX #14) & Metric Hygiene

* **Target Crate:** `craft-text` & Application Layout Engines
* **Current Status:** Future Planning
* **Existing Evidence:**
  * Naive line wrappers splitting solely on whitespace can break lines across multi-codepoint grapheme clusters or leave single-syllable Vietnamese words orphaned at line edges.

#### Contract
1. **Intra-Cluster Invariant:** Line-breaking algorithms implementing **UAX #14** must strictly prohibit break opportunities inside any Extended Grapheme Cluster (a decomposed combining mark can never wrap to the subsequent line independently of its base).
2. **Non-Breaking Space Adherence:** Strict preservation of `U+00A0` (NBSP) break inhibition to permit manual layout grouping of Vietnamese compound structures.
3. **Typographic Decoupling:** General paragraph-level orphan and widow control policies belong to higher-level document pagination models and must not be conflated with low-level UAX #14 break opportunity determination.

#### Verification
* Automated wrapping tests positioning decomposed clusters across layout boundaries, asserting that entire clusters move cohesively.

#### Non-Goals
* Incorporating high-level Vietnamese compound semantic dictionaries into base layout primitives.

---

### Tier 6: Asset Verification — Font & Outline Coverage Guard

* **Target Crate:** `crates/craft-fonts-tests` (in `storytold/craft-fonts`)
* **Current Status:** PR `storytold/craft-fonts#62`
* **Existing Evidence:**
  * `fonts/latin-manifest.txt` catalogs 51 Latin faces across 16 families used by downstream typography engines (`WordCraft`, `DeckCraft`, `FigCraft`).
  * Empirical audits in `crates/craft-fonts-tests` revealed:
    - 44 of 51 faces cover all 134 precomposed NFC Vietnamese letters with verified vector outlines.
    - 7 faces (all 4 `Caladea` and all 3 `Poppins` styles) lack horn vowels `Ơ/ơ`, `Ư/ư` and 84 characters in `U+1EA0..=U+1EF9`.
    - Only 34 of 51 faces provide the 8 combining marks required for decomposed NFD strings (`U+0300`, `U+0301`, `U+0302`, `U+0303`, `U+0306`, `U+0309`, `U+031B`, `U+0323`).

#### Contract
1. **Vietnamese-Supported Faces:** Every face claiming full Vietnamese support must map all 134 precomposed NFC letters in `cmap` and resolve to drawable vector contours (`pen.0 > 0`).
2. **Known-Gap Faces:** The exact missing set for non-compliant faces must be explicitly pinned (`VIETNAMESE_CALADEA_POPPINS_GAP`, 88 codepoints). Any upstream font file replacement that alters this gap must immediately fail CI.
3. **NFD Combining Marks:** Missing combining marks must be explicitly documented per family (`VIETNAMESE_NFD_MARK_GAPS`) to inform downstream shaping and fallback expectations.

#### Verification
* `latin_manifest_fonts_cover_vietnamese_glyphs`: Asserts `cmap` mappings and verifies corpus uniqueness (`unique.len() == 134`).
* `latin_vietnamese_glyphs_have_outlines`: Verifies real vector outlines via `assert_glyph_outline` (rejecting `.notdef` or empty-contour stubs).
* `latin_vietnamese_nfd_marks_are_documented_per_family`: Audits standalone combining marks and their outlines.

#### Non-Goals
* Asserting runtime font fallback chains or glyph substitution policies.
* Validating OpenType mark positioning (`GPOS`) or dynamic diacritic stacking metrics.

---

## 3. Implementation Phasing & Practical Execution

To maintain reviewable PR sizes and prevent monolithic bloat, development is executed in six discrete phases:

| Phase | Milestone | Target Location | Status | Key Deliverable |
| :--- | :--- | :--- | :--- | :--- |
| **Phase 1** | **Tier 6: Font Coverage Guard** | `storytold/craft-fonts` | **PR #62 (In Review)** | Pin 44/51 NFC & 34/51 NFD baseline assertions |
| **Phase 2A** | **Tier 1: Windows-1258 & TCVN3** | `craft-i18n::encoding` | Implementation Ready | High-byte `[Option<char>; 128]` tables + pure decoding |
| **Phase 2B** | **Tier 1: VNI & Normalization** | `craft-i18n::encoding` | Implementation Ready | VNI table + zero-alloc `normalize_nfc` + facade |
| **Phase 3** | **Tier 2: Grapheme Boundary** | `craft-text::segment` | Specification Ready | `prev`/`next_grapheme_offset` helpers |
| **Phase 4** | **Tier 3: IME Lifecycle** | `craft-ui` / `egui` fork | Specification Ready | Pre-edit / Committed buffer adapter |
| **Phase 5** | **Tier 4: Layout & Fallback** | `WordCraft` / `PDFCraft` | Validated in `#5189` | Contextual shaping check & cluster fallback |
| **Phase 6** | **Tier 5: Line Breaking UAX #14** | `craft-text::wrap` | Future Milestone | Intra-cluster wrap protection |

---

## 4. Definition of Done & Exit Criteria

### Level A: Milestone Done (Component Level)
A discrete phase or PR is marked **Complete** if and only if:
1. **Contract Satisfaction:** The implementation enforces all invariants defined in its respective Contract section.
2. **Defensive Verification:** Crate-level automated unit, outline, or benchmark tests pass with zero warnings on CI.
3. **Zero Scope Creep:** No unreviewed dependencies are added; public APIs adhere strictly to `#![forbid(unsafe_code)]` and zero-panic policies.
4. **Upstream Merged:** Code is merged into upstream `main` (`craft-libs`, `craft-fonts`, or downstream application).

### Level B: Ecosystem Reliability Done (System Level)
The Vietnamese Text Reliability initiative is marked **Complete** across the Craft ecosystem if and only if:
1. **Downstream Integration:** Downstream applications (`WordCraft`, `CADCraft`, `PDFCraft`, `GridCraft`) retire bespoke transcoding and grapheme hacks, relying exclusively on `craft-libs` and `craft-fonts`.
2. **End-to-End User Invariants:**
   * Archival documents (`.dwg`, `.dxf`, `.doc`, `.xls`) containing legacy TCVN3/VNI open with clean, uncorrupted Vietnamese text.
   * Typing with any platform IME (Telex, VNI) does not produce doubled characters or displaced diacritics.
   * Pressing Backspace on any Vietnamese text removes exactly one visual cluster without leaving orphan diacritics.
   * Documents render across catalog fonts without tofu `.notdef` blocks or overlapping accents.
