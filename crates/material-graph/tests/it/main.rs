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

/// The gate's share of a sweep over `items`: every `craft_testkit::hostile::divisor()`-th one from `offset` (all of them
/// nightly, where the divisor is 1), so tests sweeping the same list with different offsets together reach more of it.
fn sampled<T>(items: impl IntoIterator<Item = T>, offset: usize) -> Vec<T> {
    let div = divisor();
    items.into_iter().enumerate().filter(|(i, _)| (i + offset).is_multiple_of(div)).map(|(_, t)| t).collect()
}

fn divisor() -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    {
        craft_testkit::hostile::divisor()
    }
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
}
