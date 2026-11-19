//! The screen tests of craft-text, folded into the lib's test target.
// Unwrapping, expecting and panicking on failure is fine in tests, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub(crate) mod editor;
pub(crate) mod notes;
pub(crate) mod path_text;

pub(crate) mod ruler;
pub(crate) mod style_options;
pub(crate) mod style_panel;
pub(crate) mod tabs;
