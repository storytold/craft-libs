// SPDX-License-Identifier: MIT OR Apache-2.0
//! Versions of the evaluator for caches of results (in memory, on disk, or stored in a document). `build.rs` derives
//! them from the source, per node kind: a kind's version covers its match arm, what that reaches and what every kind
//! goes through (`build/source`). So changing a node changes the version of the kinds that reach the change, and only
//! theirs, with nothing to bump by hand.
//!
//! [`EPOCH`] is the one number a developer may have to change: for a change that alters results without changing any
//! Rust source of the evaluator (a data file read at run time, say). It is mixed into [`ALL`] and into every version
//! in [`KINDS`]. The kinds of the graphs that nodes run (function, pattern and FX-map nodes) have no entry there: they
//! report the fixed `hosted` string, and a change is covered through the node kinds that run them.

/// See the module docs.
pub const EPOCH: u32 = 1;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/kind_versions.rs"));
}

/// [`TOOLCHAIN`]: the compiler, its settings and the dependencies (the lock file). [`ALL`]: every file of the
/// evaluator. [`FILES`]: how many files that is. [`KINDS`]: (node kind, version), sorted by kind.
pub use generated::{ALL, FILES, KINDS, TOOLCHAIN};

/// What stands for a node kind in a function or a pattern program: such a graph is only ever run by the node holding it,
/// or by what every kind goes through, and those versions cover the code that runs it.
const HOSTED: &str = "hosted";

/// True for the kinds of the graphs that nodes run, not the evaluator.
fn is_hosted(kind: &str) -> bool {
    crate::func::FUNC_SPECS.iter().chain(crate::fxmap::FXM_SPECS).chain(crate::patterns::SPECS).any(|s| s.kind == kind)
}

/// The version of node kind `kind`. A kind nothing is known about goes by the version of all the evaluator's source: it
/// changes more often than it needs to, never less.
pub fn of_kind(kind: &str) -> &'static str {
    match KINDS.binary_search_by(|(k, _)| (*k).cmp(kind)) {
        Ok(at) => KINDS.get(at).map_or(ALL, |(_, v)| v),
        Err(_) if is_hosted(kind) => HOSTED,
        Err(_) => ALL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_with_an_arm_has_its_own_version_and_others_go_by_everything() {
        assert!(KINDS.is_sorted_by_key(|(k, _)| *k));
        assert_ne!(of_kind("blend"), ALL);
        assert_ne!(of_kind("blend"), of_kind("transform_matrix"));
        assert_eq!(of_kind("no such kind"), ALL);
    }

    /// A kind falling back to [`ALL`] would be recomputed for any change to any node.
    #[test]
    fn every_node_kind_has_a_version_of_its_own() {
        let specs = crate::graph::SPECS.iter().chain(crate::atomic::SPECS).chain(crate::compiled_ops::SPECS);
        let without: Vec<&str> = specs.map(|s| s.kind).filter(|k| of_kind(k) == ALL).collect();
        assert_eq!(without, Vec::<&str>::new());
        for kind in ["input", "subgraph", "pattern_program", "transform_matrix", "pixel_processor"] {
            assert_ne!(of_kind(kind), ALL, "{kind}");
            assert_ne!(of_kind(kind), HOSTED, "{kind}");
        }
        assert_eq!(of_kind("pp_out"), HOSTED);
        assert_eq!(of_kind("fxm_marker"), HOSTED);
    }
}
