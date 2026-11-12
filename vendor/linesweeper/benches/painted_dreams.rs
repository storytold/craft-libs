use gungraun::{library_benchmark, library_benchmark_group, main};
use kurbo::BezPath;

use linesweeper::{BinaryOp, FillRule, binary_op_with_eps};

fn painted_dreams_paths(eps: f64) -> (BezPath, BezPath, f64) {
    let path_a = BezPath::from_svg("M0,340C161.737914,383.575765 107.564182,490.730587 273,476 C419,463 481.741198,514.692273 481.333333,768 C481.333333,768 -0,768 -0,768 C-0,768 0,340 0,340 Z").unwrap();
    let path_b = BezPath::from_svg(
		"M458.370270,572.165771C428.525848,486.720093 368.618805,467.485992 273,476 C107.564178,490.730591 161.737915,383.575775 0,340 C0,340 0,689 0,689 C56,700 106.513901,779.342590 188,694.666687 C306.607422,571.416260 372.033966,552.205139 458.370270,572.165771 Z",
	).unwrap();
    (path_a, path_b, eps)
}

#[library_benchmark]
#[bench::inaccurate(setup = painted_dreams_paths, args = (1e-4))]
#[bench::accurate(setup = painted_dreams_paths, args = (1e-6))]
fn painted_dreams((a, b, eps): (BezPath, BezPath, f64)) {
    binary_op_with_eps(&a, &b, FillRule::EvenOdd, BinaryOp::Union, eps).unwrap();
}

library_benchmark_group!(
    name = bench_painted_dreams;
    benchmarks = painted_dreams,
);

main!(library_benchmark_groups = bench_painted_dreams);
