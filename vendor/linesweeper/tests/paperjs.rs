use kurbo::{BezPath, PathEl, Shape};
use libtest_mimic::{Arguments, Failed, Trial};
use linesweeper::{BinaryOp, FillRule, binary_op};
use serde::Deserialize;
use std::fs;

fn main() {
    let args = Arguments::from_args();
    let tests = paperjs_tests();
    libtest_mimic::run(&args, tests).exit();
}

fn paperjs_tests() -> Vec<Trial> {
    let mut tests = Vec::new();

    // Load all test files
    let test_files = vec![
        ("boolean-test/data/intersection.json", "intersection"),
        ("boolean-test/data/union.json", "union"),
        ("boolean-test/data/subtraction.json", "subtraction"),
    ];

    for (file_path, op_name) in test_files {
        if let Ok(file_tests) = load_test_file(file_path, op_name) {
            tests.extend(file_tests);
        }
    }

    tests
}

fn load_test_file(
    file_path: &str,
    op_name: &str,
) -> Result<Vec<Trial>, Box<dyn std::error::Error>> {
    let contents = fs::read_to_string(file_path)?;
    let test_file: PaperJsTestFile = serde_json::from_str(&contents)?;
    let operation = get_binary_op(&test_file.operation)
        .ok_or_else(|| format!("Unknown operation: {}", test_file.operation))?;

    let mut trials = Vec::new();

    for test_set in test_file.tests {
        let set_a = test_set.op1.clone();
        for case in test_set.cases {
            let test_name = format!("{}/{}:{}", op_name, test_set.name, case.name);
            let test_name_path = format!("{}__{}__{}", op_name, test_set.name, case.name);
            let set_a = set_a.clone();
            let set_b = case.op2.clone();
            let expected = case.res.clone();

            trials.push(Trial::test(test_name, move || {
                run_test_case(&test_name_path, &set_a, &set_b, &expected, operation)
            }));
        }
    }

    Ok(trials)
}

#[derive(Debug, Deserialize)]
struct PaperJsTestFile {
    #[serde(rename = "fn")]
    operation: String,
    tests: Vec<TestSet>,
}

#[derive(Debug, Deserialize)]
struct TestSet {
    name: String,
    op1: String, // set_a in SVG format
    cases: Vec<TestCase>,
}

#[derive(Debug, Deserialize)]
struct TestCase {
    name: String,
    op2: String, // set_b in SVG format
    res: String, // expected result in SVG format
}

/// Convert SVG string to BezPath
fn svg_to_path(svg: &str) -> Result<BezPath, Box<dyn std::error::Error>> {
    BezPath::from_svg(svg).map_err(|e| format!("Failed to parse SVG path: {}", e).into())
}

/// Convert Contours to a single BezPath by concatenating all contour paths
fn contours_to_bezpath(contours: &linesweeper::topology::Contours) -> BezPath {
    let mut combined = BezPath::new();
    for contour in contours.contours() {
        combined.extend(contour.path.clone());
    }
    combined
}

/// Map paper.js operation names to BinaryOp enum
fn get_binary_op(operation: &str) -> Option<BinaryOp> {
    match operation {
        "intersect" => Some(BinaryOp::Intersection),
        "unite" | "union" => Some(BinaryOp::Union),
        "subtract" | "difference" => Some(BinaryOp::Difference),
        "exclude" | "xor" => Some(BinaryOp::Xor),
        _ => None,
    }
}

fn to_tiny_skia(p: &BezPath) -> Option<tiny_skia::Path> {
    let mut b = tiny_skia::PathBuilder::new();
    for el in p.elements() {
        match el {
            PathEl::MoveTo(p) => b.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => b.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(p0, p1) => b.quad_to(p0.x as f32, p0.y as f32, p1.x as f32, p1.y as f32),
            PathEl::CurveTo(p0, p1, p2) => b.cubic_to(
                p0.x as f32,
                p0.y as f32,
                p1.x as f32,
                p1.y as f32,
                p2.x as f32,
                p2.y as f32,
            ),
            PathEl::ClosePath => b.close(),
        }
    }
    b.finish()
}

#[cfg(feature = "debug-svg")]
fn dump_svg(file_path: &str, svg: &BezPath) {
    let bbox = svg.bounding_box().inset(2.0);
    let doc = svg::Document::new().set(
        "viewBox",
        (bbox.min_x(), bbox.min_y(), bbox.width(), bbox.height()),
    );
    let path = svg::node::element::Path::new()
        .set("d", svg.to_svg())
        .set("stroke-width", "0")
        .set("fill", "black");
    let doc = doc.add(path);
    svg::save(file_path, &doc).unwrap();
}

fn pixmap_distance(a: &tiny_skia::Pixmap, b: &tiny_skia::Pixmap) -> f32 {
    let w = a.width().min(b.width());
    let h = a.height().min(b.height());
    let mut diff = 0.0;
    for x in 0..w {
        for y in 0..h {
            let ac = a.pixel(x, y).unwrap();
            let bc = b.pixel(x, y).unwrap();
            let rdiff = ac.red().abs_diff(bc.red());
            let gdiff = ac.green().abs_diff(bc.green());
            let bdiff = ac.blue().abs_diff(bc.blue());
            diff += rdiff.max(gdiff).max(bdiff) as f32 / 255.0;
        }
    }

    diff / (w * h) as f32
}

fn path_to_pixmap(bbox: kurbo::Rect, path: &BezPath) -> tiny_skia::Pixmap {
    let size = 256;
    let scale = size as f64 / bbox.width().max(bbox.height());
    let transform =
        tiny_skia::Transform::from_translate(-bbox.min_x() as f32, -bbox.min_y() as f32)
            .post_scale(scale as f32, scale as f32);

    let mut pixmap = tiny_skia::Pixmap::new(256, 256).unwrap();

    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::BLACK);
    if let Some(p) = to_tiny_skia(path) {
        pixmap.fill_path(&p, &paint, tiny_skia::FillRule::EvenOdd, transform, None);
    }
    pixmap
}

/// Run a single test case
fn run_test_case(
    test_name_path: &str,
    set_a_svg: &str,
    set_b_svg: &str,
    expected_svg: &str,
    operation: BinaryOp,
) -> Result<(), Failed> {
    let set_a = svg_to_path(set_a_svg).map_err(|e| format!("Failed to parse set_a: {}", e))?;
    let set_b = svg_to_path(set_b_svg).map_err(|e| format!("Failed to parse set_b: {}", e))?;
    let expected =
        svg_to_path(expected_svg).map_err(|e| format!("Failed to parse expected result: {}", e))?;

    // Run binary operation
    // If it's subtraction, we need to swap the order of the operands because paper.js does A - B but our library does A - B as well, so we need to do B - A to match paper.js results
    let (set_a, set_b) = if operation == BinaryOp::Difference {
        (set_b, set_a)
    } else {
        (set_a, set_b)
    };
    let result = binary_op(&set_a, &set_b, FillRule::NonZero, operation)
        .map_err(|e| format!("Binary operation failed: {:?}", e))?;
    let result_path = contours_to_bezpath(&result).reverse_subpaths();

    let bbox = set_a.bounding_box().union(set_b.bounding_box());

    let our_pixmap = path_to_pixmap(bbox, &result_path);
    let expected_pixmap = path_to_pixmap(bbox, &expected);
    let diff = pixmap_distance(&our_pixmap, &expected_pixmap);
    if diff > 0.0 {
        #[cfg(feature = "debug-svg")]
        {
            dump_svg(&format!("ignore/{test_name_path}_set_a.svg"), &set_a);
            dump_svg(&format!("ignore/{test_name_path}_set_b.svg"), &set_b);
            dump_svg(&format!("ignore/{test_name_path}_expected.svg"), &expected);
            dump_svg(&format!("ignore/{test_name_path}_result.svg"), &result_path);
        }
        // Silence the unused variable warning
        #[cfg(not(feature = "debug-svg"))]
        let _ = test_name_path;

        return Err(format!("pixmap distance {diff}, above threshold").into());
    }
    Ok(())
}
