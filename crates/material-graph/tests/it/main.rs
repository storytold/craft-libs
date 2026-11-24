//! The graph runtime's integration tests in one executable, with one module per feature.

// The build script's digest, at the root where its `source` module (see `kind_versions`) names it `crate::digest`.
#[allow(dead_code)]
#[path = "../../build/digest.rs"]
mod digest;

mod deep;
mod determinism;
mod format_doc;
mod func;
mod fxmap;
mod kind_versions;
mod nodes;
mod seam;
mod vector;
