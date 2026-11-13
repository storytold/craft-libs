use std::hint::black_box;

use gungraun::{library_benchmark, library_benchmark_group, main};

use kurbo::BezPath;
use linesweeper::{
    BinaryOp, FillRule, Point, binary_op,
    generators::{checkerboard, slanted_checkerboard, slanties},
};

fn to_bez(points: &[Vec<Point>]) -> BezPath {
    fn one(points: &[Point]) -> BezPath {
        let mut points = points.iter();
        let p = points.next().unwrap();
        let mut ret = BezPath::default();
        ret.move_to(p.to_kurbo());
        for q in points {
            ret.line_to(q.to_kurbo());
        }
        ret.line_to(p.to_kurbo());
        ret
    }

    let mut ret = BezPath::default();
    for ps in points {
        ret.extend(one(ps));
    }
    ret
}

fn generate_checkerboard(size: usize) -> (BezPath, BezPath) {
    let (contours_even, contours_odd) = checkerboard(size);
    (to_bez(&contours_even), to_bez(&contours_odd))
}

fn generate_slanted_checkerboard(size: usize) -> (BezPath, BezPath) {
    let (contours_even, contours_odd) = slanted_checkerboard(size);
    (to_bez(&contours_even), to_bez(&contours_odd))
}

fn generate_slanties(size: usize) -> (BezPath, BezPath) {
    let (contours_even, contours_odd) = slanties(size);
    (to_bez(&contours_even), to_bez(&contours_odd))
}

#[library_benchmark]
#[bench::small_checkerboard(setup = generate_checkerboard, args = (10))]
#[bench::medium_checkerboard(setup = generate_checkerboard, args = (20))]
#[bench::large_checkerboard(setup = generate_checkerboard, args = (50))]
#[bench::small_slanted_checkerboard(setup = generate_slanted_checkerboard, args = (10))]
#[bench::medium_slanted_checkerboard(setup = generate_slanted_checkerboard, args = (20))]
#[bench::large_slanted_checkerboard(setup = generate_slanted_checkerboard, args = (50))]
#[bench::small_slanties(setup = generate_slanties, args = (10))]
#[bench::medium_slanties(setup = generate_slanties, args = (20))]
#[bench::large_slanties(setup = generate_slanties, args = (50))]
fn xor((a, b): (BezPath, BezPath)) {
    black_box(binary_op(&a, &b, FillRule::EvenOdd, BinaryOp::Xor)).unwrap();
}

library_benchmark_group!(
    name = bench_synthetic;
    benchmarks = xor,
);

main!(library_benchmark_groups = bench_synthetic);
