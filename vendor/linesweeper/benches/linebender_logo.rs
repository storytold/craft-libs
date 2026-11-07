use gungraun::{library_benchmark, library_benchmark_group, main};
use std::hint::black_box;

use kurbo::BezPath;
use linesweeper::{BinaryOp, FillRule, Segments, binary_op, topology::Topology};
use linesweeper_util::svg_to_bezpaths;

fn linebender_path() -> BezPath {
    let input = include_str!("../examples/linebender.svg");
    let tree = usvg::Tree::from_str(input, &usvg::Options::default()).unwrap();
    svg_to_bezpaths(&tree)
        .into_iter()
        .flatten()
        .collect::<BezPath>()
}

fn linebender_paths() -> (BezPath, BezPath) {
    let input = include_str!("../examples/linebender.svg");
    let tree = usvg::Tree::from_str(input, &usvg::Options::default()).unwrap();
    let bezpaths = svg_to_bezpaths(&tree);

    let first_path = bezpaths.first().unwrap().clone();
    let second_path = bezpaths[1..]
        .iter()
        .flat_map(|b| b.elements().iter().cloned())
        .collect();
    (first_path, second_path)
}

fn linebender_segments() -> Segments {
    let mut segs = Segments::default();
    segs.add_bez_path(&linebender_path()).unwrap();
    segs
}

#[library_benchmark]
#[bench::bench(setup = linebender_segments)]
fn just_the_sweep(segs: Segments) {
    // If we don't collect the events, it seems like nothing gets measured (even
    // sprinkling black_box around).
    let mut events = Vec::new();
    linesweeper::sweep::sweep(&segs, 0.01, |_, ev| {
        events.push(ev);
    });
    black_box(events);
}

#[library_benchmark]
#[bench::bench(setup = linebender_path)]
fn build_topology(path: BezPath) {
    black_box(Topology::from_paths_binary(&path, &BezPath::new(), 0.01)).unwrap();
}

#[library_benchmark]
#[bench::bench(setup = linebender_paths)]
fn xor_linesweeper((first_path, second_path): (BezPath, BezPath)) {
    black_box(binary_op(
        &first_path,
        &second_path,
        FillRule::EvenOdd,
        BinaryOp::Xor,
    ))
    .unwrap();
}

library_benchmark_group!(
    name = bench_linebender_logo;
    benchmarks = just_the_sweep, build_topology, xor_linesweeper,
);
main!(library_benchmark_groups = bench_linebender_logo);
