//! The screen tests of craft-text, folded into the lib's test target.
// Unwrapping, expecting and panicking on failure is fine in tests, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod editor;
mod notes;
// PLAN: `mod path_text;` belongs here once tools-w4's `tests/it/path_text.rs` exists. It is not in this tree, so the declaration fails to compile.
mod ruler;
mod style_options;
mod style_panel;
mod tabs;
