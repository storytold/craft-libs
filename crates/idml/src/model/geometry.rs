//! `PathGeometry`: the outline of a page item, as subpaths of anchor points with their
//! direction (Bézier control) points, in the item's inner coordinates.

use craft_xml::{El, Local};

use crate::value::{nums, point_str};

/// Most points one geometry keeps by default; points past it are dropped.
pub const MAX_PATH_POINTS: usize = 1_000_000;

/// An anchor point and its two direction points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathPoint {
    pub anchor: [f64; 2],
    /// `LeftDirection`: the control point before the anchor.
    pub left: [f64; 2],
    /// `RightDirection`: the control point after the anchor.
    pub right: [f64; 2],
}

impl PathPoint {
    /// A corner point: both direction points on the anchor.
    pub fn corner(anchor: [f64; 2]) -> PathPoint {
        PathPoint { anchor, left: anchor, right: anchor }
    }
}

/// One `GeometryPathType`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubPath {
    /// `PathOpen`: an open path is not closed back to its first point.
    pub open: bool,
    pub points: Vec<PathPoint>,
}

/// A `PathGeometry` element.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PathGeometry {
    pub paths: Vec<SubPath>,
}

/// A point `x y`; `None` when the value is not two finite numbers.
fn point(s: &str) -> Option<[f64; 2]> {
    match nums(s).as_slice() {
        [x, y] => Some([*x, *y]),
        _ => None,
    }
}

impl PathGeometry {
    /// The geometry of `pg` (a `PathGeometry` element), at most [`MAX_PATH_POINTS`] points.
    pub fn from_el(pg: &El) -> PathGeometry {
        Self::from_el_capped(pg, MAX_PATH_POINTS)
    }

    /// [`PathGeometry::from_el`] keeping at most `max` points over all subpaths.
    ///
    /// A point without a usable anchor is left out; a missing or unusable direction point is
    /// the anchor. A subpath with a value that is a number but not a finite one (`NaN`, `inf`)
    /// is left out whole: dropping only those points would change its shape.
    pub fn from_el_capped(pg: &El, max: usize) -> PathGeometry {
        let mut left = max;
        let mut paths = Vec::new();
        for g in pg.children(Local("GeometryPathType")) {
            if left == 0 {
                break;
            }
            let points_el: Vec<&El> =
                g.child(Local("PathPointArray")).map(|a| a.children(Local("PathPointType")).take(left).collect()).unwrap_or_default();
            let non_finite = |v: Option<&str>| v.is_some_and(|s| s.split_whitespace().any(|t| t.parse::<f64>().is_ok_and(|x| !x.is_finite())));
            if points_el.iter().any(|p| ["Anchor", "LeftDirection", "RightDirection"].iter().any(|k| non_finite(p.attr(k)))) {
                continue;
            }
            let mut points = Vec::with_capacity(points_el.len());
            for p in points_el {
                let Some(anchor) = p.attr("Anchor").and_then(point) else { continue };
                let left_dir = p.attr("LeftDirection").and_then(point).unwrap_or(anchor);
                let right_dir = p.attr("RightDirection").and_then(point).unwrap_or(anchor);
                points.push(PathPoint { anchor, left: left_dir, right: right_dir });
            }
            left = left.saturating_sub(points.len());
            paths.push(SubPath { open: g.attr("PathOpen") == Some("true"), points });
        }
        PathGeometry { paths }
    }

    /// The `PathGeometry` element.
    pub fn to_el(&self) -> El {
        let mut pg = El::new("PathGeometry");
        for sp in &self.paths {
            let mut arr = El::new("PathPointArray");
            for p in &sp.points {
                arr.push(
                    El::new("PathPointType")
                        .with_attr("Anchor", point_str(p.anchor[0], p.anchor[1]))
                        .with_attr("LeftDirection", point_str(p.left[0], p.left[1]))
                        .with_attr("RightDirection", point_str(p.right[0], p.right[1])),
                );
            }
            pg.push(El::new("GeometryPathType").with_attr("PathOpen", if sp.open { "true" } else { "false" }).with_child(arr));
        }
        pg
    }

    /// A closed rectangle from `top left bottom right`, corners clockwise from the top left.
    pub fn rectangle([top, left, bottom, right]: [f64; 4]) -> PathGeometry {
        let points = [[left, top], [left, bottom], [right, bottom], [right, top]].map(PathPoint::corner).to_vec();
        PathGeometry { paths: vec![SubPath { open: false, points }] }
    }

    /// The box around the anchors and direction points: `top left bottom right`. `None` without
    /// points.
    pub fn bounds(&self) -> Option<[f64; 4]> {
        let mut it = self.paths.iter().flat_map(|p| &p.points).flat_map(|p| [p.anchor, p.left, p.right]);
        let first = it.next()?;
        let [mut x0, mut y0, mut x1, mut y1] = [first[0], first[1], first[0], first[1]];
        for [x, y] in it {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        Some([y0, x0, y1, x1])
    }

    pub fn point_count(&self) -> usize {
        self.paths.iter().map(|p| p.points.len()).sum()
    }
}
