//! The screen tests of craft-automation, folded into the lib's test target.
// Unwrapping, expecting and panicking on failure is fine in tests, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod actions;
mod control;
mod dialogs;
mod screenshot;
mod synthetic;
