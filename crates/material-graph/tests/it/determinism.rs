//! Evaluation gives the same bytes however it is run: on one worker thread or many, and from the node cache or computed
//! afresh. The graphs are the engine's own demo graphs (one per node kind, written for this repository), so nothing
//! here depends on outside data.

use craft_material_graph::context::EvalContext;
use craft_material_graph::graph::{self, Graph, NodeCache};
use craft_material_graph::tex::Tex;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Rendered at a size that is not the demo graphs' own (64), so sizes scale, and large enough that the parallel loops
/// split every image into several pieces.
const SIDE: usize = 96;

fn demo_graphs() -> Vec<(&'static str, Graph)> {
    graph::SPECS
        .iter()
        .filter_map(|s| graph::demo_graph_for(s.kind).map(|t| (s.kind, t)))
        .map(|(k, t)| (k, Graph::parse(&t).unwrap_or_else(|e| panic!("the demo graph of {k} does not parse: {e}"))))
        .collect()
}

fn bits(t: &Tex) -> Bits {
    (t.w, t.h, t.px.iter().flat_map(|p| p.map(f32::to_bits)).collect())
}

/// An image as (width, height, the bits of every component).
type Bits = (usize, usize, Vec<u32>);

/// Every output of `g`, or its error, as bits.
fn render(g: &Graph) -> Result<BTreeMap<String, Bits>, String> {
    g.evaluate(Some(SIDE)).map(|o| o.iter().map(|(k, t)| (k.clone(), bits(t))).collect()).map_err(|e| e.to_string())
}

fn in_pool<R: Send>(threads: usize, f: impl FnOnce() -> R + Send) -> R {
    rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("a thread pool").install(f)
}

#[test]
fn one_thread_and_eight_render_the_same_bytes() {
    let all = demo_graphs();
    assert!(all.len() > 100, "only {} demo graphs", all.len());
    let graphs = crate::sampled(all, 0);
    let one = in_pool(1, || graphs.iter().map(|(_, g)| render(g)).collect::<Vec<_>>());
    let eight = in_pool(8, || graphs.iter().map(|(_, g)| render(g)).collect::<Vec<_>>());
    let differ: Vec<&str> = graphs.iter().zip(one.iter().zip(&eight)).filter(|(_, (a, b))| a != b).map(|((k, _), _)| *k).collect();
    assert!(differ.is_empty(), "these kinds render differently on 1 and 8 threads: {differ:?}");
    assert!(one.iter().filter(|r| r.is_ok()).count() * 10 > graphs.len() * 8, "most demo graphs must render");
}

#[test]
fn cached_and_uncached_evaluations_give_the_same_bytes() {
    let cache = Arc::new(NodeCache::new(256));
    let mut differ = Vec::new();
    let mut not_reused = Vec::new();
    for (kind, g) in crate::sampled(demo_graphs(), 0) {
        let keep: Vec<u32> = g.nodes.keys().copied().collect();
        let fresh = g.evaluate_all(SIDE, &keep);
        let first = g.evaluate_all_cached(&EvalContext::default(), SIDE, &keep, &cache, None, &mut |_| {});
        // the second run takes every node from the cache
        let hits = cache.stats().0;
        let again = g.evaluate_all_cached(&EvalContext::default(), SIDE, &keep, &cache, None, &mut |_| {});
        if again.errors.len() < keep.len() && cache.stats().0 == hits {
            not_reused.push(kind);
        }
        for r in [&first, &again] {
            let same = keep.iter().all(|id| fresh.kept.get(id).map(|t| bits(t)) == r.kept.get(id).map(|t| bits(t)))
                && fresh.errors == r.errors
                && fresh.thumbs == r.thumbs;
            if !same {
                differ.push(kind);
            }
        }
        cache.clear();
    }
    assert!(differ.is_empty(), "the cache changes these kinds: {differ:?}");
    assert!(not_reused.is_empty(), "the second run took nothing from the cache for these kinds: {not_reused:?}");
}
