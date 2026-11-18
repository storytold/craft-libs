use kurbo::BezPath;
use libtest_mimic::{Arguments, Failed, Trial};
use linesweeper::binary_op;
use std::path::{Path, PathBuf};

fn main() {
    let args = Arguments::from_args();
    let tests = regression_tests();

    libtest_mimic::run(&args, tests).exit();
}

fn regression_tests() -> Vec<Trial> {
    let ws = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let paths = glob::glob(&format!("{ws}/tests/regression/**/*.txt")).unwrap();
    paths
        .into_iter()
        .map(|p| {
            let p = p.unwrap();
            let name = input_path_base(&p).display().to_string();
            Trial::test(name, || generate_regression_test(p))
        })
        .collect()
}

fn input_path_base(input_path: &Path) -> &Path {
    let ws = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let base = format!("{ws}/tests/regression");
    input_path.strip_prefix(base).unwrap()
}

fn generate_regression_test(path: PathBuf) -> Result<(), Failed> {
    let input = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<_> = input.lines().collect();
    assert_eq!(lines.len(), 2);
    let p0 = BezPath::from_svg(lines[0]).unwrap();
    let p1 = BezPath::from_svg(lines[1]).unwrap();
    binary_op(
        &p0,
        &p1,
        linesweeper::FillRule::EvenOdd,
        linesweeper::BinaryOp::Xor,
    )
    .unwrap();
    Ok(())
}
