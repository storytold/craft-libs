# Type System Contracts & Domain Invariants

This document outlines the numerical representations, type invariants, and error contracts enforced across `craft-libs`.

---

## 1. Non-Panicking Return Types

Shared crates eliminate implicit panics by representing fallibility exclusively through explicit return types:

- **Explicit `Result<T, E>` & `Option<T>`**:
  All operations that can encounter missing data, malformed inputs, full disks, or unsupported formats must return an explicit `Result` or `Option`.
- **Domain-Specific Error Enums**:
  Crates define strongly typed domain error hierarchies using `thiserror`. Never use generic string errors (`Result<T, String>`) in public crate surfaces.
- **Unsupported Feature Representation**:
  Unimplemented capabilities return explicit `Err(Error::Unsupported(...))` variants rather than calling `todo!()` or `unimplemented!()`.
- **Zero Panics in Shipped Code**:
  Prohibit `.unwrap()`, `.expect()`, and `panic!()`. Use `ok_or(..)?`, `let ... else { return Err(...); }`, or `unwrap_or_default()` / `unwrap_or_else()` where non-corrupting fallbacks are logically sound.

---

## 2. Signed Media Time (`craft-time`)

Time in media-oriented creative applications (video NLE, motion graphics, digital audio) is fundamentally signed:

- **The `Tick` Representation**:
  - Exact media timestamps are represented by the signed 64-bit integer newtype `Tick(i64)`.
  - The arithmetic contract covers the complete signed domain `[Tick::MIN, Tick::MAX]`.
  - Negative timestamps are valid domain constructs: audio lead-in / preroll buffers, lead-in pickup bars in music composition, and negative trim offsets.
- **Boundary Clamping**:
  - Shared library crates must **never** universally clamp timestamps to non-negative `[0, MAX]`.
  - Non-negative clamping is restricted to application-level playback transport and timeline scrubber boundaries.
- **Monotonic Division & Timebases**:
  - Timebase division must be checked for zero.
  - SMPTE timecode conversions (`HH:MM:SS:FF`) must handle drop-frame vs non-drop-frame calculations with exact integer arithmetic.

---

## 3. Hostile Numerical Inputs & Safe Arithmetic

Input data from external files, IPC channels, and foreign protocols must be treated as untrusted:

- **Safe Slicing & Indexing**:
  - Direct indexing (`v[i]`, `s[a..b]`) is forbidden on input-derived numbers.
  - Use `.get(index)` or `.get(range)` and handle `None`.
  - UTF-8 strings must be sliced only at valid character boundaries (`is_char_boundary`).
- **Checked & Saturating Arithmetic**:
  - Array lengths, offsets, tile coordinates, and media ticks must use `checked_*` or `saturating_*` arithmetic to prevent integer overflow.
  - Modular arithmetic and scaling ratios must guard against division by zero.
- **Finite Float Verification**:
  - Float values for dimensions, strokes, and zoom scales must be verified as finite (`f.is_finite()`).
  - Reject or clamp `NaN`, `+inf`, and `-inf` before converting floats to integers.

---

## 4. Logical Identity vs. Localized Presentation

User interface components, panels, and document entities must decouple internal identity from user-facing text:

- **Stable Logical IDs**:
  Tabs, panels, accordion headers, commands, and document layers use immutable IDs (`TabId`, `PanelId`, `LayerId`). Identifiers must never be derived from human-readable labels, which change across localizations and user renames.
- **Localization Resilience**:
  UI text and accessibility labels must be updateable without altering widget identity, focus paths, or saved layout state.
