//! The screen tests of craft-text, folded into the lib's test target.
// Unwrapping, expecting and panicking on failure is fine in tests, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod editor;
mod notes;
mod path_text;

mod ruler;
mod style_options;
mod style_panel;
mod tabs;

mod character;
