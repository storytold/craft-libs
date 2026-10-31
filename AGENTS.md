# AGENTS.md

Shared Rust libraries for the Craft Apps.

- **Never crash.** Every crate's `lib.rs` starts with
  `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`.
  No `unsafe` (the workspace forbids it).
- **New crates** go in `crates/<name>`, named `craft-<name>`, inherit the workspace package fields
  and lints, and are listed in `[workspace.dependencies]`.
- **No app code.** A crate here must be useful to more than one app and must not depend on any app.
- **Time goes through `chrono`** (workspace dependency). Never hand-roll UTC/calendar conversion (days-to-date algorithms, leap-year math); a crate owns only its output format string. The one exception is range extension: past the years chrono can represent, shift the date by whole 400-year Gregorian cycles (146,097 days, after which the calendar repeats), let chrono compute the month and day, and add the cycles back to the year (splitting seconds into hours, minutes and seconds is plain arithmetic).
- **Before merging:** `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.
