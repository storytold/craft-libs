//! Deep graphs evaluate on an ordinary thread's stack: the evaluator keeps the nodes waiting for their inputs in a list
//! of its own instead of recursing along the graph, so a chain of any length needs no more stack than a short one. A
//! function graph runs recursively per pixel, so how deep one may be is bounded, and a deeper one is an error.

use craft_material_graph::graph::Graph;
use std::fmt::Write;

/// The stack of a thread spawned without asking for one (Rust's default), and of rayon's workers.
const NORMAL_STACK: usize = 2 << 20;

/// Stacks too small for a walk that recurses once per node: a frame takes at least tens of bytes (an unoptimised one about
/// a kilobyte), so the chains below overflow them if the evaluator recurses, and the chain length needs no more work than
/// that.
const SMALL_STACK: usize = 128 << 10;
const MEDIUM_STACK: usize = 512 << 10;

/// The stack a function as deep as allowed is run with: the normal one in an optimised build (measured: 2040 nodes in a
/// row take under 1 MB); an unoptimised build's frames are about fifteen times larger.
const FUNCTION_STACK: usize = if cfg!(debug_assertions) { 32 << 20 } else { NORMAL_STACK };

/// Runs `f` on the one thread of a new pool with `stack` bytes of stack and returns what it returns.
fn on_stack<R: Send + 'static>(stack: usize, f: impl FnOnce() -> R + Send + 'static) -> R {
    rayon::ThreadPoolBuilder::new().num_threads(1).stack_size(stack).build().expect("a pool").install(f)
}

/// `n` invert nodes in a row behind a gradient.
fn image_chain(n: u32) -> String {
    let mut t = String::from("size 16\nnode 0 gradient angle=0.1\n");
    for i in 1..=n {
        let _ = writeln!(t, "node {i} invert in=@{}", i - 1);
    }
    let _ = writeln!(t, "output o {n}");
    t
}

/// A pixel processor running a function of `n` additions in a row on the position.
fn function_chain(n: u32) -> String {
    let mut t = String::from("size 16\nsubgraph f\nmeta kind \"function\"\nnode 1 f_pos\nnode 2 f_swizzle a=@1 mask=\"x\"\nnode 3 f_float v=0.001\n");
    let mut last = 2;
    for i in 0..n {
        let id = 10 + i;
        let _ = writeln!(t, "node {id} f_add a=@{last} b=@3");
        last = id;
    }
    let _ = writeln!(t, "node 9 f_out in=@{last}\nend\nnode 1 pixel_processor fn=\"f\"\noutput o 1");
    t
}

/// Subgraphs `levels` deep, each an invert of the one below.
fn nested(levels: u32) -> String {
    let mut t = String::from("size 16\nsubgraph s0\nnode 1 input idx=0\nnode 2 invert in=@1\noutput o 2\nend\n");
    for l in 1..levels {
        let _ = write!(t, "subgraph s{l}\nnode 1 input idx=0\nnode 2 subgraph graph=\"s{}\" in0=@1\nnode 3 invert in=@2\noutput o 3\nend\n", l - 1);
    }
    let _ = write!(t, "node 1 gradient angle=0.1\nnode 2 subgraph graph=\"s{}\" in0=@1\noutput o 2\n", levels - 1);
    t
}

#[test]
fn a_long_chain_evaluates_on_a_small_stack() {
    let g = Graph::parse(&image_chain(10_000)).expect("parses");
    let shallow = Graph::parse(&image_chain(2)).expect("parses");
    let (deep, two) =
        on_stack(MEDIUM_STACK, move || (g.evaluate(None).map_err(|e| e.to_string()), shallow.evaluate(None).map_err(|e| e.to_string())));
    let (deep, two) = (deep.expect("evaluates"), two.expect("evaluates"));
    // an even number of inverts is the gradient again, as two are
    assert_eq!(deep["o"].px, two["o"].px);
}

#[test]
fn every_node_of_a_long_chain_gets_a_thumbnail_on_a_small_stack() {
    let g = Graph::parse(&image_chain(300)).expect("parses");
    let all = on_stack(SMALL_STACK, move || g.evaluate_all(16, &[300]));
    assert!(all.errors.is_empty(), "{:?}", all.errors.iter().next());
    assert_eq!(all.thumbs.len(), 301);
}

#[test]
fn subgraphs_nested_to_the_limit_evaluate_on_a_normal_stack() {
    let g = Graph::parse(&nested(8)).expect("parses");
    let r = on_stack(NORMAL_STACK, move || g.evaluate(None).map(|o| o.len()).map_err(|e| e.to_string()));
    assert_eq!(r, Ok(1));
    let g = Graph::parse(&nested(9)).expect("parses");
    let r = on_stack(NORMAL_STACK, move || g.evaluate(None).map_err(|e| e.to_string()));
    assert!(r.is_err_and(|e| e.contains("too deep")), "nine levels are refused");
}

#[test]
fn a_function_as_deep_as_allowed_runs_on_a_normal_stack() {
    let n = craft_material_graph::func::MAX_RUN_DEPTH as u32 - 4;
    let g = Graph::parse(&function_chain(n)).expect("parses");
    // (the pixels run on the pool's threads, so they get the same stack)
    let r = on_stack(FUNCTION_STACK, move || {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(4).stack_size(FUNCTION_STACK).build().expect("pool");
        pool.install(|| g.evaluate(None).map_err(|e| e.to_string()))
    });
    let o = r.expect("evaluates");
    let px = o["o"].px[0][0];
    assert!((px - (0.5 / 16.0 + n as f32 * 0.001)).abs() < 1e-3, "{px}");
}

#[test]
fn a_function_deeper_than_allowed_is_an_error() {
    let n = craft_material_graph::func::MAX_RUN_DEPTH as u32 + 10;
    let g = Graph::parse(&function_chain(n)).expect("parses");
    let r = on_stack(NORMAL_STACK, move || g.evaluate(None).map_err(|e| e.to_string()));
    assert!(r.as_ref().is_err_and(|e| e.contains("too deep")), "{:?}", r.map(|o| o.len()));
}

/// How much stack a function `TC_PROBE_N` additions deep takes: run with `TC_PROBE_STACK` bytes of stack (a crash is
/// too little). For choosing `MAX_RUN_DEPTH`.
#[test]
#[ignore]
fn probe_function_stack() {
    let n: u32 = std::env::var("TC_PROBE_N").ok().and_then(|v| v.parse().ok()).unwrap_or(1000);
    let stack: usize = std::env::var("TC_PROBE_STACK").ok().and_then(|v| v.parse().ok()).unwrap_or(NORMAL_STACK);
    let g = Graph::parse(&function_chain(n)).expect("parses");
    let r = on_stack(stack, move || {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(2).stack_size(stack).build().expect("pool");
        pool.install(|| g.evaluate(None).map(|_| ()).map_err(|e| e.to_string()))
    });
    eprintln!("probe n={n} stack={stack}: {r:?}");
}
