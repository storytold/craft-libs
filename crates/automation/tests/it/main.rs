//! The screen tests of craft-automation, folded into the lib's test target.
// Unwrapping, expecting and panicking on failure is fine in tests, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub(crate) mod access;
pub(crate) mod actions;
pub(crate) mod agents;
pub(crate) mod control;
pub(crate) mod dialogs;
pub(crate) mod dock;
pub(crate) mod files;
pub(crate) mod screenshot;
pub(crate) mod synthetic;
