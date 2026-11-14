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
    // Craft patch (regression glob): a checkout path can hold glob
    // metacharacters such as `[`, and a run that finds no inputs is a failure.
    let pattern = format!("{}/tests/regression/**/*.txt", glob::Pattern::escape(&ws));
    let trials: Vec<Trial> = glob::glob(&pattern)
        .unwrap()
        .map(|p| {
            let p = p.unwrap();
            let name = input_path_base(&p).display().to_string();
            Trial::test(name, || generate_regression_test(p))
        })
        .collect();
    assert!(!trials.is_empty(), "no regression inputs under {ws}");
    trials
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
