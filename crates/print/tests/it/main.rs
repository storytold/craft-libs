//! The screen tests of craft-print, folded into the lib's test target.
// Unwrapping, expecting and panicking on failure is fine in tests, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub(crate) mod dialog;
pub(crate) mod panel;
pub(crate) mod print;
