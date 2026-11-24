use kurbo::{Affine, BezPath, ParamCurve as _, Point, Rect, Vec2};
use libtest_mimic::{Arguments, Failed, Trial};
use linesweeper::{
    Segment, Segments,
    sweep::{SweepLineBuffers, SweepLineRange, SweepLineRangeBuffers, Sweeper, Correctness},
    topology::Topology,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tiny_skia::Pixmap;

fn main() {
    let args = Arguments::from_args();
    let mut tests = sweep_snapshot_diffs();
    tests.extend(position_snapshot_diffs());
    tests.extend(output_snapshot_diffs());

    libtest_mimic::run(&args, tests).exit();
}

fn path_color(idx: usize) -> tiny_skia::Color {
    let palette = [
        tiny_skia::Color::from_rgba8(0x00, 0x5F, 0x73, 0xFF),
        tiny_skia::Color::from_rgba8(0x94, 0xD2, 0xBD, 0xFF),
        tiny_skia::Color::from_rgba8(0xE9, 0xD8, 0xA6, 0xFF),
        tiny_skia::Color::from_rgba8(0xEE, 0x9B, 0x00, 0xFF),
        tiny_skia::Color::from_rgba8(0xCA, 0x67, 0x02, 0xFF),
        tiny_skia::Color::from_rgba8(0xBB, 0x3E, 0x03, 0xFF),
        tiny_skia::Color::from_rgba8(0xAE, 0x20, 0x12, 0xFF),
    ];
    palette[idx % palette.len()]
}

fn sweep_line_color() -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(0x9B, 0x22, 0x26, 0xFF)
}

fn sweep_snapshot_diffs() -> Vec<Trial> {
    let ws = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let paths = glob::glob(&format!("{ws}/tests/snapshots/inputs/sweep/**/*.svg")).unwrap();
    paths
        .into_iter()
        .map(|p| {
            let p = p.unwrap();
            let name = input_path_base(&p).display().to_string();
            Trial::test(name, || generate_sweep_snapshot(p))
        })
        .collect()
}

fn position_snapshot_diffs() -> Vec<Trial> {
    let ws = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let paths = glob::glob(&format!("{ws}/tests/snapshots/inputs/position/**/*.svg")).unwrap();

    paths
        .into_iter()
        .map(|p| {
            let p = p.unwrap();
            let name = input_path_base(&p).display().to_string();
            Trial::test(name, || {
                generate_position_snapshot(p.clone(), false).and(generate_position_snapshot(p, true))
            })
        })
        .collect()
}

fn output_snapshot_diffs() -> Vec<Trial> {
    let ws = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let paths = glob::glob(&format!("{ws}/tests/snapshots/inputs/output/**/*.svg")).unwrap();

    paths
        .into_iter()
        .map(|p| {
            let p = p.unwrap();
            let name = input_path_base(&p).display().to_string();
            Trial::test(name, || {
                generate_output_snapshot(p.clone(), false).and(generate_output_snapshot(p, true))
            })
        })
        .collect()
}

fn input_path_base(input_path: &Path) -> &Path {
    let ws = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let base = format!("{ws}/tests/snapshots/inputs");
    input_path.strip_prefix(base).unwrap()
}

fn output_path_for(input_path: &Path, correct: bool) -> PathBuf {
    let mut ws: PathBuf = std::env::var_os("CARGO_MANIFEST_DIR").unwrap().into();
    ws.push("target/snapshots/snapshots");
    if correct {
        ws.push("correct");
    }
    ws.push(input_path);
    ws.set_extension("png");
    ws
}

fn saved_snapshot_path_for(input_path: &Path, correct: bool) -> PathBuf {
    let mut ws: PathBuf = std::env::var_os("CARGO_MANIFEST_DIR").unwrap().into();
    ws.push("tests/snapshots/snapshots");
    if correct {
        ws.push("correct");
    }
    ws.push(input_path);
    ws.set_extension("png");
    ws
}

fn skia_path(elts: impl IntoIterator<Item = kurbo::PathEl>) -> tiny_skia::Path {
    let mut pb = tiny_skia::PathBuilder::new();
    for elt in elts {
        match elt {
            kurbo::PathEl::MoveTo(p) => pb.move_to(p.x as f32, p.y as f32),
            kurbo::PathEl::LineTo(p) => pb.line_to(p.x as f32, p.y as f32),
            kurbo::PathEl::QuadTo(p0, p1) => {
                pb.quad_to(p0.x as f32, p0.y as f32, p1.x as f32, p1.y as f32)
            }
            kurbo::PathEl::CurveTo(p0, p1, p2) => pb.cubic_to(
                p0.x as f32,
                p0.y as f32,
                p1.x as f32,
                p1.y as f32,
                p2.x as f32,
                p2.y as f32,
            ),
            kurbo::PathEl::ClosePath => pb.close(),
        }
    }
    pb.finish().unwrap()
}

fn skia_kurbo_seg(seg: kurbo::PathSeg) -> tiny_skia::Path {
    let mut pb = tiny_skia::PathBuilder::new();
    let start = seg.start();
    pb.move_to(start.x as f32, start.y as f32);
    match seg {
        kurbo::PathSeg::Line(ell) => pb.line_to(ell.p1.x as f32, ell.p1.y as f32),
        kurbo::PathSeg::Quad(q) => {
            pb.quad_to(q.p1.x as f32, q.p1.y as f32, q.p2.x as f32, q.p2.y as f32)
        }
        kurbo::PathSeg::Cubic(c) => pb.cubic_to(
            c.p1.x as f32,
            c.p1.y as f32,
            c.p2.x as f32,
            c.p2.y as f32,
            c.p3.x as f32,
            c.p3.y as f32,
        ),
    }
    pb.finish().unwrap()
}

fn skia_cubic(s: &kurbo::CubicBez) -> tiny_skia::Path {
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(s.p0.x as f32, s.p0.y as f32);
    pb.cubic_to(
        s.p1.x as f32,
        s.p1.y as f32,
        s.p2.x as f32,
        s.p2.y as f32,
        s.p3.x as f32,
        s.p3.y as f32,
    );
    pb.finish().unwrap()
}

fn skia_segment(s: &Segment) -> tiny_skia::Path {
    skia_cubic(&s.to_kurbo_cubic())
}

fn line(p: impl Into<Point>, q: impl Into<Point>) -> tiny_skia::Path {
    let mut pb = tiny_skia::PathBuilder::new();
    let p = p.into();
    pb.move_to(p.x as f32, p.y as f32);
    let q = q.into();
    pb.line_to(q.x as f32, q.y as f32);
    pb.finish().unwrap()
}

fn two_lines(p: impl Into<Point>, q: impl Into<Point>, r: impl Into<Point>) -> tiny_skia::Path {
    let mut pb = tiny_skia::PathBuilder::new();
    let p = p.into();
    pb.move_to(p.x as f32, p.y as f32);
    let q = q.into();
    pb.line_to(q.x as f32, q.y as f32);
    let r = r.into();
    pb.line_to(r.x as f32, r.y as f32);
    pb.finish().unwrap()
}

fn draw_orig_path(pixmap: &mut Pixmap, path: &BezPath, offset: kurbo::Point) {
    let p = skia_path(path);
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(0, 0, 0, 255);
    let stroke = tiny_skia::Stroke {
        width: 1.0,
        ..Default::default()
    };
    let transform = tiny_skia::Transform::from_translate(offset.x as f32, offset.y as f32);

    pixmap.stroke_path(&p, &paint, &stroke, transform, None);
}

fn adjust_x_positions(orig: &[f64], padding: f64, min_x: f64, max_x: f64) -> Vec<f64> {
    if orig.is_empty() {
        return Vec::new();
    }

    let mut padded = Vec::with_capacity(orig.len());
    let mut max_so_far = f64::NEG_INFINITY;
    for &x in orig {
        let x = x.max(max_so_far + padding);
        padded.push(x);
        max_so_far = x;
    }

    let orig_first = *orig.first().unwrap();
    let orig_last = *orig.last().unwrap();
    let padded_min = *padded.first().unwrap();
    let padded_max = *padded.last().unwrap();

    let mut mid_shift = (orig_first + orig_last - padded_min - padded_max) / 2.0;
    mid_shift = mid_shift.clamp(min_x - padded_min, max_x - padded_max);
    if padded_max - padded_min > max_x - min_x {
        mid_shift = (max_x + min_x - padded_max - padded_min) / 2.0;
    }
    for x in &mut padded {
        *x += mid_shift;
    }
    padded
}

fn color(c: tiny_skia::Color) -> tiny_skia::Paint<'static> {
    let mut p = tiny_skia::Paint::default();
    p.set_color(c);
    p
}

fn draw_sweep_line_range(
    pixmap: &mut Pixmap,
    segments: &Segments,
    range: SweepLineRange<'_, '_, '_>,
    bbox: kurbo::Rect,
    padding: f64,
) {
    let y = range.line().y();
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(sweep_line_color());
    let stroke = tiny_skia::Stroke {
        width: 2.0,
        ..Default::default()
    };
    let thick_stroke = tiny_skia::Stroke {
        width: 4.0,
        ..Default::default()
    };
    let s_line = line(
        (bbox.min_x() - padding, bbox.min_y() + y),
        (bbox.max_x() + padding, bbox.min_y() + y),
    );

    pixmap.stroke_path(
        &s_line,
        &paint,
        &stroke,
        tiny_skia::Transform::identity(),
        None,
    );

    let old_segs: Vec<_> = range.old_segment_range().collect();
    let new_segs: Vec<_> = range.segment_range().collect();
    let really_new_segs = new_segs.iter().filter(|s| !old_segs.contains(s));
    let seg_color: HashMap<_, _> = old_segs
        .iter()
        .chain(really_new_segs.clone())
        .enumerate()
        .map(|(color_idx, seg_idx)| (seg_idx, color_idx))
        .collect();
    let origin = bbox.origin();

    for seg_idx in old_segs.iter().chain(really_new_segs) {
        let color_idx = seg_color[seg_idx];
        let seg = &segments[*seg_idx];
        let p = skia_segment(seg);
        let mut paint = tiny_skia::Paint::default();
        paint.set_color(path_color(color_idx));
        let transform = tiny_skia::Transform::from_translate(origin.x as f32, origin.y as f32);
        pixmap.stroke_path(&p, &paint, &stroke, transform, None);
    }

    let old_seg_positions: Vec<_> = old_segs.iter().map(|s| segments[*s].at_y(y)).collect();
    let padded_old_seg_positions: Vec<_> = adjust_x_positions(
        &old_seg_positions,
        padding / 2.0,
        bbox.min_x() - padding,
        bbox.max_x() + padding,
    );

    for ((&px, &x), seg_idx) in padded_old_seg_positions
        .iter()
        .zip(&old_seg_positions)
        .zip(&old_segs)
    {
        let p = two_lines(
            (bbox.min_x() + px, bbox.min_y()),
            (bbox.min_x() + px, bbox.min_y() + y - padding / 2.0),
            (bbox.min_x() + x, bbox.min_y() + y),
        );
        let color_idx = seg_color[seg_idx];
        let c = path_color(color_idx);

        pixmap.stroke_path(
            &p,
            &color(c),
            &thick_stroke,
            tiny_skia::Transform::identity(),
            None,
        );
    }
    let new_seg_positions: Vec<_> = new_segs.iter().map(|s| segments[*s].at_y(y)).collect();
    let padded_new_seg_positions: Vec<_> = adjust_x_positions(
        &new_seg_positions,
        padding / 2.0,
        bbox.min_x() - padding,
        bbox.max_x() + padding,
    );
    for ((&px, &x), seg_idx) in padded_new_seg_positions
        .iter()
        .zip(&new_seg_positions)
        .zip(&new_segs)
    {
        let p = two_lines(
            (bbox.min_x() + x, bbox.min_y() + y),
            (bbox.min_x() + px, bbox.min_y() + y + padding / 2.0),
            (bbox.min_x() + px, bbox.max_y()),
        );
        let color_idx = seg_color[seg_idx];
        let c = path_color(color_idx);

        pixmap.stroke_path(
            &p,
            &color(c),
            &thick_stroke,
            tiny_skia::Transform::identity(),
            None,
        );
    }
}

fn generate_sweep_snapshot(path: PathBuf) -> Result<(), Failed> {
    let input = std::fs::read_to_string(&path).unwrap();
    let tree = usvg::Tree::from_str(&input, &usvg::Options::default()).unwrap();
    let bezs = linesweeper_util::svg_to_bezpaths(&tree);
    let bbox = linesweeper_util::bezier_bounding_box(bezs.iter());
    let bez: BezPath = bezs
        .into_iter()
        .flat_map(|p| Affine::translate(-bbox.origin().to_vec2()) * p)
        .collect();

    let mut segments = Segments::default();
    segments.add_bez_path(&bez).unwrap();
    segments.check_invariants();

    let eps = 16.0;
    let mut range_bufs = SweepLineRangeBuffers::default();
    let mut line_bufs = SweepLineBuffers::default();

    // Run through once just to count the ranges.
    let mut sweep_state = Sweeper::new(&segments, eps);
    let mut num_ranges = 0;
    while let Some(mut line) = sweep_state.next_line(&mut line_bufs) {
        while line.next_range(&mut range_bufs, &segments).is_some() {
            num_ranges += 1;
        }
    }

    let pad = 32.0;
    let mut sweep_state = Sweeper::new(&segments, eps);
    let mut pixmap = Pixmap::new(
        (bbox.width() + 2.0 * pad).ceil() as u32,
        ((bbox.height() + pad) * num_ranges as f64 + pad).ceil() as u32,
    )
    .unwrap();

    let mut b = Rect::new(pad, pad, pad + bbox.width(), pad + bbox.height());
    while let Some(mut line) = sweep_state.next_line(&mut line_bufs) {
        while let Some(range) = line.next_range(&mut range_bufs, &segments) {
            draw_orig_path(&mut pixmap, &bez, b.origin());
            draw_sweep_line_range(&mut pixmap, &segments, range, b, pad);

            b = b + Vec2::new(0.0, bbox.height() + pad);
        }
    }

    let base_path = input_path_base(&path);
    let out_path = output_path_for(base_path, false);
    std::fs::create_dir_all(out_path.parent().unwrap()).unwrap();
    pixmap.save_png(&out_path).unwrap();

    let new_image = kompari::load_image(&out_path)?;
    let snapshot = kompari::load_image(&saved_snapshot_path_for(base_path, false))?;
    match kompari::compare_images(&snapshot, &new_image) {
        kompari::ImageDifference::None => Ok(()),
        _ => Err("image comparison failed".into()),
    }
}

fn generate_position_snapshot(path: PathBuf, correct: bool) -> Result<(), Failed> {
    let input = std::fs::read_to_string(&path).unwrap();
    let tree = usvg::Tree::from_str(&input, &usvg::Options::default()).unwrap();
    let bezs = linesweeper_util::svg_to_bezpaths(&tree);
    let bbox = linesweeper_util::bezier_bounding_box(bezs.iter());
    let bez: BezPath = bezs
        .into_iter()
        .flat_map(|p| Affine::translate(-bbox.origin().to_vec2()) * p)
        .collect();

    let eps = 16.0;
    let correctness = if correct { Correctness::ReallyCorrect } else { Correctness::GoodEnough };
    let top =
        Topology::<i32>::from_paths_with_correctness([(&bez, ())], eps, correctness).unwrap();
    let out_paths = top.compute_positions();

    let pad = 2.0 * eps;
    let bbox = top.bounding_box();
    let mut pixmap = Pixmap::new(
        (bbox.width() + 2.0 * pad).ceil() as u32,
        (bbox.height() + 2.0 * pad).ceil() as u32,
    )
    .unwrap();
    let pad_transform = tiny_skia::Transform::from_translate(
        (pad - bbox.min_x()) as f32,
        (pad - bbox.min_y()) as f32,
    );

    let stroke = tiny_skia::Stroke {
        width: 1.0,
        ..Default::default()
    };
    for out_idx in top.segment_indices() {
        let path = &out_paths[out_idx].path;
        let far_idx = &out_paths[out_idx].copied_idx;
        for (idx, seg) in path.segments().enumerate() {
            let skia_seg = skia_kurbo_seg(seg);

            let c = if far_idx == &Some(idx) {
                path_color(0)
            } else {
                path_color(3)
            };

            pixmap.stroke_path(&skia_seg, &color(c), &stroke, pad_transform, None);

            let p0 = seg.start();
            let p0 = tiny_skia::PathBuilder::from_circle(p0.x as f32, p0.y as f32, 2.0).unwrap();
            let p1 = seg.end();
            let p1 = tiny_skia::PathBuilder::from_circle(p1.x as f32, p1.y as f32, 2.0).unwrap();
            let black = color(tiny_skia::Color::BLACK);
            pixmap.fill_path(
                &p0,
                &black,
                tiny_skia::FillRule::Winding,
                pad_transform,
                None,
            );
            pixmap.fill_path(
                &p1,
                &black,
                tiny_skia::FillRule::Winding,
                pad_transform,
                None,
            );
        }
    }
    let base_path = input_path_base(&path);
    let out_path = output_path_for(base_path, correct);
    std::fs::create_dir_all(out_path.parent().unwrap()).unwrap();
    pixmap.save_png(&out_path).unwrap();

    let new_image = kompari::load_image(&out_path)?;
    let snapshot = kompari::load_image(&saved_snapshot_path_for(base_path, correct))?;
    match kompari::compare_images(&snapshot, &new_image) {
        kompari::ImageDifference::None => Ok(()),
        _ => Err("image comparison failed".into()),
    }
}

fn generate_output_snapshot(path: PathBuf, correct: bool) -> Result<(), Failed> {
    let input = std::fs::read_to_string(&path).unwrap();
    let tree = usvg::Tree::from_str(&input, &usvg::Options::default()).unwrap();
    let bezs = linesweeper_util::svg_to_bezpaths(&tree);
    let bbox = linesweeper_util::bezier_bounding_box(bezs.iter());
    let bez: BezPath = bezs
        .into_iter()
        .flat_map(|p| Affine::translate(-bbox.origin().to_vec2()) * p)
        .collect();

    let eps = 1.0;
    let correctness = if correct { Correctness::ReallyCorrect } else { Correctness::GoodEnough };
    let top =
        Topology::<i32>::from_paths_with_correctness([(&bez, ())], eps, correctness).unwrap();
    let contours = top.contours(|w| w % 2 == 0);

    let pad = 2.0 * eps;
    let bbox = top.bounding_box();
    let mut pixmap = Pixmap::new(
        (bbox.width() + 2.0 * pad).ceil() as u32,
        (bbox.height() + 2.0 * pad).ceil() as u32,
    )
    .unwrap();
    let pad_transform = tiny_skia::Transform::from_translate(
        (pad - bbox.min_x()) as f32,
        (pad - bbox.min_y()) as f32,
    );

    let stroke = tiny_skia::Stroke {
        width: 1.0,
        ..Default::default()
    };
    for contour in contours.contours() {
        for seg in contour.path.segments() {
            let skia_seg = skia_kurbo_seg(seg);
            pixmap.stroke_path(
                &skia_seg,
                &color(path_color(0)),
                &stroke,
                pad_transform,
                None,
            );

            let p0 = seg.start();
            let p0 = tiny_skia::PathBuilder::from_circle(p0.x as f32, p0.y as f32, 2.0).unwrap();
            let p1 = seg.end();
            let p1 = tiny_skia::PathBuilder::from_circle(p1.x as f32, p1.y as f32, 2.0).unwrap();
            let black = color(tiny_skia::Color::BLACK);
            pixmap.fill_path(
                &p0,
                &black,
                tiny_skia::FillRule::Winding,
                pad_transform,
                None,
            );
            pixmap.fill_path(
                &p1,
                &black,
                tiny_skia::FillRule::Winding,
                pad_transform,
                None,
            );
        }
    }
    let base_path = input_path_base(&path);
    let out_path = output_path_for(base_path, correct);
    std::fs::create_dir_all(out_path.parent().unwrap()).unwrap();
    pixmap.save_png(&out_path).unwrap();

    let new_image = kompari::load_image(&out_path)?;
    let snapshot = kompari::load_image(&saved_snapshot_path_for(base_path, correct))?;
    match kompari::compare_images(&snapshot, &new_image) {
        kompari::ImageDifference::None => Ok(()),
        _ => Err("image comparison failed".into()),
    }
}
