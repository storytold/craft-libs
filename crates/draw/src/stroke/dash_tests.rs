//! Tests of dash alignment.

use craft_ui::testing::trial;
use kurbo::{BezPath, PathEl, Point};

use super::{DashAlign, DashUnit, StrokeStyle};

fn style() -> StrokeStyle {
    StrokeStyle { width: 1.0, dashes: vec![4.0, 2.0], dash_unit: DashUnit::Document, dash_align: DashAlign::Corners, ..StrokeStyle::default() }
}

/// The dashes of `path` as their points.
fn dashes(path: &BezPath) -> Vec<Vec<Point>> {
    let mut out: Vec<Vec<Point>> = Vec::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => out.push(vec![p]),
            PathEl::LineTo(p) => {
                if let Some(d) = out.last_mut() {
                    d.push(p);
                }
            }
            _ => {}
        }
    }
    out
}

fn length(d: &[Point]) -> f64 {
    d.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// Half a dash (2 at the given pattern) within the bound the fit allows.
fn half_dash(l: f64) -> bool {
    (1.5..2.5).contains(&l)
}

/// Whether a dash runs through `c` with half a dash on each side. The two halves may differ: each
/// run is fitted to whole periods on its own, so the runs on either side of a corner scale differently.
fn dash_on(dashes: &[Vec<Point>], c: Point) -> bool {
    dashes.iter().any(|d| {
        d.iter().position(|p| p.distance(c) < 1e-6).is_some_and(|i| {
            let (before, after) = (length(&d[..=i]), length(&d[i..]));
            i > 0 && i + 1 < d.len() && half_dash(before) && half_dash(after)
        })
    })
}

#[trial]
fn corners_alignment_puts_a_dash_on_each_corner() {
    let style = style();

    let (a, corner, b) = (Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 60.0));
    let mut open = BezPath::new();
    open.move_to(a);
    open.line_to(corner);
    open.line_to(b);
    let cut = dashes(&style.dashed(&open, 1.0));
    assert!(dash_on(&cut, corner), "a dash runs through the corner");
    let (head, tail) = (cut.first().expect("dashes"), cut.last().expect("dashes"));
    assert!(head[0].distance(a) < 1e-6 && half_dash(length(head)), "the path starts in the middle of a dash");
    assert!(tail[tail.len() - 1].distance(b) < 1e-6 && half_dash(length(tail)), "the path ends in the middle of a dash");
    let starts: Vec<f64> =
        cut.iter().filter(|d| d.len() == 2 && d[0].y.abs() < 1e-9 && d[1].y.abs() < 1e-9 && length(d) > 3.0).map(|d| d[0].x).collect();
    assert!(starts.len() > 2, "the first run holds whole dashes");
    let period = starts[1] - starts[0];
    assert!((5.0..7.0).contains(&period), "the pattern is scaled within a bound of the given lengths");
    assert!(starts.windows(2).all(|w| (w[1] - w[0] - period).abs() < 1e-6), "one period along the run");
    let periods = 100.0 / period;
    assert!((periods - periods.round()).abs() < 1e-6, "the run holds whole periods");

    let corners = [Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 60.0), Point::new(0.0, 60.0)];
    let mut closed = BezPath::new();
    closed.move_to(corners[0]);
    for c in &corners[1..] {
        closed.line_to(*c);
    }
    closed.close_path();
    let cut = dashes(&style.dashed(&closed, 1.0));
    for c in corners {
        assert!(dash_on(&cut, c), "a dash runs through the corner {c:?} of the rectangle");
    }
}
