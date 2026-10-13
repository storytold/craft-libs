# Type System Architecture & Domain Invariants

This document defines the core type invariants, numerical representations, and error handling contracts across `craft-libs`.

---

## 1. Non-Panicking Return Types

Library and production code must completely eliminate implicit panics by modeling all fallibility through explicit Rust type signatures.

- **Explicit `Result<T, E>` & `Option<T>`**: Every operation that can encounter missing data, malformed inputs, full disks, or unsupported features must return an explicit `Result` or `Option`.
- **Domain-Specific Error Enums**: Each crate defines its own rich error enum (e.g. `PersistenceError`, `SettingsError`, `DockingError`) using `thiserror`. Never use generic string errors (`Result<T, String>`) or boxed trait objects (`Result<T, Box<dyn Error>>`) in public crate APIs.
- **Unsupported Feature Representation**: Unfinished features or missing format chunks must return an explicit `Err(Error::Unsupported(...))` variant rather than calling `todo!()` or `unimplemented!()`.
- **Never Unwrap**: No `.unwrap()`, `.expect()`, or `panic!()`. Use `ok_or(..)?`, `let ... else { return Err(...); }`, or `unwrap_or_default()` / `unwrap_or_else()` where a safe, non-corrupting fallback is logically guaranteed.

---

## 2. Signed Media Time (`craft-time`)

Time in creative media applications (audio, video, motion graphics) is fundamentally signed:

- **The `Tick` Representation**: Media time is represented by the signed 64-bit newtype `Tick(i64)`. The arithmetic contract holds across the full signed range `[Tick::MIN, Tick::MAX]`. Negative timestamps are valid domain constructs: audio lead-in / preroll, lead-in pickup bars in music composition, negative trim offsets, and negative timeline margins.
- **Boundary Clamping**: **Never** clamp time to non-negative `[0, MAX]` universally inside shared library crates. Non-negative clamping belongs strictly at application-level transport playback or interactive timeline scrubber boundaries.
- **Monotonic Division & Timebases**: Division by ticks or rates must be checked for zero. SMPTE timecode conversions (`HH:MM:SS:FF`) must explicitly handle drop-frame vs non-drop-frame calculations at exact integer boundaries.

---

## 3. Hostile Numerical Types & Safe Arithmetic

All numerical data received from files, foreign protocols, IPC sockets, or user parameters must be treated as hostile:

- **Safe Slicing & Indexing**: Direct indexing (`v[i]`, `s[a..b]`) is strictly forbidden on input-derived numbers. Always use `.get(index)` or `.get(range)` and handle `None`. Slicing strings requires explicit character boundary validation (`s.is_char_boundary(idx)`).
- **Checked & Saturating Arithmetic**: Byte lengths, offsets, tile coordinates, and media ticks must use `checked_*` or `saturating_*` arithmetic methods to prevent integer overflows. Guard against division by zero before executing modular arithmetic or scaling ratios.
- **Floating-Point Verification**: Check floating-point values for `NaN` and infinity before converting to integer coordinates or sending to rendering APIs (`f.is_finite()`). When parsing float user strings, clamp to known domain minimum/maximum bounds.

---

## 4. Serialization Contracts & Schema Evolution

All configuration structs, document models, and persisted preference state must support structured serialization:

- **Serde Gating**: Shared library domain structs derive `Serialize` and `Deserialize` (optionally gated under the `serde` Cargo feature).
- **Forward & Backward Compatibility**:
  - Unknown fields must be preserved on read/write cycles using `#[serde(flatten)]` or custom map storage.
  - Optional fields use `#[serde(default)]` to prevent desync during format upgrades.
- **Schema Migration**: Settings and state stores maintain an explicit integer `schema_version`. Migrations execute sequentially from version $N$ to $N+1$ before data ingestion.
