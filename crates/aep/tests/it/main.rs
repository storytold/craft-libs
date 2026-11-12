//! The integration tests of craft-aep, one binary: the container (chunk tree and typed view) and
//! the fixture access the tests share (`common`).

// Test tables are tuples of plain values; naming each shape would not make them clearer.
#![allow(clippy::type_complexity)]

#[macro_use]
mod common;

mod container;
