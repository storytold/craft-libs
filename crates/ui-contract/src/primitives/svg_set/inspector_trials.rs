//! The drawings the catalogue gained for the transform and layout panels, read from the generated catalogue itself:
//! each parses, its ink stays inside its view box, it draws along the axis or on the side its name gives in the
//! coordinates it renders in, it does not mirror, and no two of them are the same document in one set.
//!
//! `build.rs` checks the catalogue as data (its tables, a file for every listed name, no unlisted file, the stroke
//! lists) and reads no drawing. The catalogue's `svg` resolves a name and a set to the document the rasteriser draws,
//! the set's own file or the base set's, and this trial goes through that routing rather than a list of file names.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::{DIRECTIONAL, IconSet, find, svg};
use crate::primitives::svg_tree;
use crate::testing::trial;
use craft_svg::tree::usvg::tiny_skia_path::{self, PathSegment, Transform};
use craft_svg::tree::usvg::{Group, Node, Path};
use std::collections::BTreeMap;

/// How far an edge may drift from an axis, in the coordinates the drawing renders in, for the trial to count it as
/// along that axis.
const STRAIGHT: f32 = 0.01;

/// The slack the view-box check allows, in view-box units: a cap or a join just past the box is not a crop.
const SLACK: f32 = 1.0;

/// The length of the bar the two fixtures below draw, in view-box units.
const BAR: f32 = 16.0;

/// A drawing written the awkward way round: a bar that runs along x inside a group rotated 90 degrees about the centre
/// of its 24 unit box, inside another group that shifts it, so it draws down the box from the top to the bottom. Two
/// groups, because a reading that applied only the nearest transform and stopped there is a way to get this wrong.
const ROTATED: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><g transform="translate(2 2)"><g transform="rotate(90 12 12)"><path d="M4 8h16" fill="none" stroke="currentColor" stroke-width="2"/></g></g></svg>"#;

/// The same bar flattened: the edge written where those transforms land it, `M4 8h16` becoming `M18 6v16`, with no
/// transform at all. The two documents draw the same edge, so a reading of the drawing must give them the same axes.
const FLAT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path d="M18 6v16" fill="none" stroke="currentColor" stroke-width="2"/></svg>"#;

/// What a glyph's drawing must show. Each name is the axis or the side of the screen its control changes, never a
/// reading direction.
#[derive(Clone, Copy)]
enum Reads {
    /// The two marks the drawing is made of separate further along x than along y: a mirror about the vertical axis,
    /// or the gap between two items side by side. The ink's own box says nothing here (Lucide's flip is taller than it
    /// is wide), so the reading is the separation of those two marks.
    AlongX,
    /// The same along y.
    AlongY,
    /// The ink's box is wider than tall: one mark across the axis of a scale, either a bar with outward arrows or a box
    /// whose width outward arrows span.
    ExtentX,
    /// The same along y.
    ExtentY,
    /// The longest straight edge runs along x: a skew, whose leaning sides leave the top and bottom level. Tabler draws
    /// both of its skew files wider than tall, so their boxes cannot tell the pair apart; the straight edges can.
    LeansX,
    /// The same along y.
    LeansY,
    /// One filled band, off centre towards this side, which is a sign on x and y: a padding.
    Side(i32, i32),
}

/// The transform and layout glyphs this change adds, with the reading each one's drawing must have.
const ADDED: [(&str, Reads); 12] = [
    ("flip_horizontal", Reads::AlongX),
    ("flip_vertical", Reads::AlongY),
    ("scale_horizontal", Reads::ExtentX),
    ("scale_vertical", Reads::ExtentY),
    ("skew_horizontal", Reads::LeansX),
    ("skew_vertical", Reads::LeansY),
    ("padding_top", Reads::Side(0, -1)),
    ("padding_right", Reads::Side(1, 0)),
    ("padding_bottom", Reads::Side(0, 1)),
    ("padding_left", Reads::Side(-1, 0)),
    ("gap_horizontal", Reads::AlongX),
    ("gap_vertical", Reads::AlongY),
];

/// One painted path of a drawing: its box including the stroke, its longest straight edge along x and along y, and
/// whether it is filled.
struct Painted {
    rect: [f32; 4],
    straight: [f32; 2],
    filled: bool,
}

impl Painted {
    /// The centre of the box.
    fn centre(&self) -> (f32, f32) {
        ((self.rect[0] + self.rect[2]) / 2.0, (self.rect[1] + self.rect[3]) / 2.0)
    }

    fn area(&self) -> f32 {
        (self.rect[2] - self.rect[0]).max(0.0) * (self.rect[3] - self.rect[1]).max(0.0)
    }
}

/// The path nodes of `group` that paint, depth first. A path with neither a fill nor a stroke paints nothing (Tabler
/// ships one as a placeholder) and is left out, and so is one drawn hidden.
fn drawn_paths(group: &Group) -> Vec<&Path> {
    fn walk<'a>(group: &'a Group, out: &mut Vec<&'a Path>) {
        for node in group.children() {
            match node {
                Node::Group(inner) => walk(inner, out),
                Node::Path(path) if path.is_visible() && (path.fill().is_some() || path.stroke().is_some()) => out.push(path.as_ref()),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(group, &mut out);
    out
}

/// Every painted path of `group`, read: its box including the stroke, its straight edges, and whether it is filled.
fn painted(group: &Group) -> Vec<Painted> {
    drawn_paths(group)
        .into_iter()
        .map(|path| {
            let rect = path.abs_stroke_bounding_box();
            Painted { rect: [rect.left(), rect.top(), rect.right(), rect.bottom()], straight: straight_runs(path), filled: path.fill().is_some() }
        })
        .collect()
}

/// The longest straight edge of `path` along x and along y, in the coordinates the drawing renders in: its segments
/// placed by the path's absolute transform, the one the renderer draws it with.
fn straight_runs(path: &Path) -> [f32; 2] {
    straight_edges(path.data(), path.abs_transform())
}

/// The longest straight edge of `data` along x and along y once `transform` has placed its points, in the units that
/// transform gives. usvg keeps a path's segments in the element's own coordinates and leaves every transform above them,
/// a `<g transform>`, the element's own and the root view box's scale, on the node's absolute transform for the renderer
/// to apply, so `data` alone is not where the drawing lands: `usvg::Path::data`'s "all segments are in absolute
/// coordinates" means that the commands are absolute (`M`, `L`, not `m`, `l`), never that they are in canvas coordinates.
/// An edge is along an axis when, drawn, one of its deltas is zero; a curve is read as its end point, which no edge of
/// these drawings depends on. An identity `transform` reads the document as written. This is the reading
/// `craft_svg::tree::document` takes when it converts a parsed tree into a document.
fn straight_edges(data: &tiny_skia_path::Path, transform: Transform) -> [f32; 2] {
    let placed = |mut point: tiny_skia_path::Point| {
        transform.map_point(&mut point);
        (point.x, point.y)
    };
    let mut runs = [0.0f32; 2];
    let mut from = (0.0f32, 0.0f32);
    let mut start = from;
    for segment in data.segments() {
        let to = match segment {
            PathSegment::MoveTo(point) => {
                from = placed(point);
                start = from;
                continue;
            }
            PathSegment::Close => start,
            PathSegment::LineTo(point) | PathSegment::QuadTo(.., point) | PathSegment::CubicTo(.., point) => placed(point),
        };
        let (dx, dy) = ((to.0 - from.0).abs(), (to.1 - from.1).abs());
        if dy <= STRAIGHT {
            runs[0] = runs[0].max(dx);
        } else if dx <= STRAIGHT {
            runs[1] = runs[1].max(dy);
        }
        from = to;
    }
    runs
}

/// How far a drawing spreads along x and along y: the separation of its two largest painted paths (a flip and a gap are
/// two marks either side of the axis they name), or the box of its one path (a scale and a skew each draw one mark
/// across their axis).
fn spread(painted: &[Painted]) -> (f32, f32) {
    let mut ranked: Vec<&Painted> = painted.iter().collect();
    ranked.sort_by(|a, b| b.area().total_cmp(&a.area()));
    match ranked.as_slice() {
        [first, second, ..] => {
            let ((ax, ay), (bx, by)) = (first.centre(), second.centre());
            ((ax - bx).abs(), (ay - by).abs())
        }
        [only] => (only.rect[2] - only.rect[0], only.rect[3] - only.rect[1]),
        [] => (0.0, 0.0),
    }
}

/// Asserts that the drawing `name` gives in `set` reads as `reads` says, from its painted paths and its view box.
fn drawn_as(name: &str, set: &str, reads: Reads, ink: &[Painted], view: (f32, f32)) {
    let tenth = view.0.max(view.1) / 10.0;
    match reads {
        Reads::AlongX | Reads::AlongY => {
            let (dx, dy) = spread(ink);
            let along_x = matches!(reads, Reads::AlongX);
            let (along, across) = if along_x { (dx, dy) } else { (dy, dx) };
            assert!(along > across, "`{name}` in `{set}` spreads {dx} along x and {dy} along y, not along the axis its name gives");
            assert!(along >= tenth, "`{name}` in `{set}` spreads only {along} along its axis, under a tenth of the drawing");
        }
        Reads::ExtentX | Reads::ExtentY => {
            let left = ink.iter().map(|shape| shape.rect[0]).fold(f32::INFINITY, f32::min);
            let top = ink.iter().map(|shape| shape.rect[1]).fold(f32::INFINITY, f32::min);
            let right = ink.iter().map(|shape| shape.rect[2]).fold(f32::NEG_INFINITY, f32::max);
            let bottom = ink.iter().map(|shape| shape.rect[3]).fold(f32::NEG_INFINITY, f32::max);
            let (wide, tall) = (right - left, bottom - top);
            let along_x = matches!(reads, Reads::ExtentX);
            let (along, across) = if along_x { (wide, tall) } else { (tall, wide) };
            assert!(along > across, "`{name}` in `{set}` spans {wide} along x and {tall} along y, not the axis its name gives");
            assert!(along >= tenth, "`{name}` in `{set}` spans only {along} along its axis, under a tenth of the drawing");
        }
        Reads::LeansX | Reads::LeansY => {
            let leans_x = matches!(reads, Reads::LeansX);
            let (mut along, mut across) = (0.0f32, 0.0f32);
            for shape in ink {
                let (long, short) = if leans_x { (shape.straight[0], shape.straight[1]) } else { (shape.straight[1], shape.straight[0]) };
                along = along.max(long);
                across = across.max(short);
            }
            let axis = if leans_x { "x" } else { "y" };
            assert!(
                along > across,
                "`{name}` in `{set}` has its longest straight edges across the lean it names: {along} along {axis}, {across} the other way"
            );
            assert!(along >= tenth, "`{name}` in `{set}` has no straight edge along {axis} a tenth of the drawing long");
        }
        Reads::Side(dx, dy) => {
            let bands: Vec<&Painted> = ink.iter().filter(|shape| shape.filled).collect();
            assert_eq!(bands.len(), 1, "`{name}` in `{set}` fills {} parts: the space it names is one solid band", bands.len());
            let band = bands[0];
            assert!(band.area() > 0.0, "`{name}` in `{set}` fills an empty band");
            let (x, y) = band.centre();
            let (centre_x, centre_y) = (view.0 / 2.0, view.1 / 2.0);
            // A tenth of the drawing off centre towards its side, and a twentieth the other way: the band is on one
            // side rather than in a corner.
            let (reach, slack) = (view.0.min(view.1) / 10.0, view.0.min(view.1) / 20.0);
            let towards = if dx != 0 { (x - centre_x) * dx as f32 } else { (y - centre_y) * dy as f32 };
            let across = if dx != 0 { y - centre_y } else { x - centre_x };
            assert!(towards >= reach, "`{name}` in `{set}` puts its band {towards} off centre towards its side, short of {reach}");
            assert!(across.abs() <= slack, "`{name}` in `{set}` puts its band {} off the other axis", across.abs());
        }
    }
}

/// The reading above is taken where a drawing lands, not where its `d` is written, which is what a glyph drawn sideways
/// inside a `<g>` exposes: the bar written along x in `ROTATED` draws along y, and `straight_runs` must say so.
///
/// The bar's own coordinates still read along x, and `straight_edges` with an identity transform is kept as the reading
/// of the document as written; that is the reading this trial would compare against if the segments were not placed by
/// the path's absolute transform, so this case fails the old measurement and passes the drawn one. `FLAT` writes that
/// same edge where the rotation lands it, with no transform to apply, and the two documents must read alike.
fn edges_are_read_in_the_coordinates_the_drawing_renders_in() {
    let rotated = svg_tree::parse(ROTATED.as_bytes()).expect("the rotated fixture parses");
    let flat = svg_tree::parse(FLAT.as_bytes()).expect("the flattened fixture parses");
    let bars = drawn_paths(rotated.root());
    assert_eq!(bars.len(), 1, "the rotated fixture draws one bar");
    let drawn = straight_runs(bars[0]);
    assert!(
        drawn[1] >= BAR - STRAIGHT && drawn[0] <= STRAIGHT,
        "a bar written along x inside two transformed groups that rotate it draws along y, but its edges read {drawn:?}"
    );
    let written = straight_edges(bars[0].data(), Transform::identity());
    assert!(
        written[0] >= BAR - STRAIGHT && written[1] <= STRAIGHT,
        "the coordinates the rotated bar's `d` is written in read {written:?}, along x: that is not an axis the drawing has"
    );
    let flats = drawn_paths(flat.root());
    assert_eq!(flats.len(), 1, "the flattened fixture draws one bar");
    let flattened = straight_runs(flats[0]);
    assert!(
        flattened[1] >= BAR - STRAIGHT && (flattened[0] - drawn[0]).abs() <= STRAIGHT && (flattened[1] - drawn[1]).abs() <= STRAIGHT,
        "the bar written where the rotation lands it reads {flattened:?} and the rotated one {drawn:?}: both draw the same edge"
    );
    // The catalogue's own check on that drawing, which only reads right once the segments are placed: it fails on the
    // untransformed reading, where the longest straight edge is 16 along x and nothing along y.
    let view = (rotated.size().width(), rotated.size().height());
    drawn_as("the rotated bar", "a fixture", Reads::LeansY, &painted(rotated.root()), view);
}

#[trial]
fn the_added_transform_and_layout_glyphs_draw_where_their_names_say() {
    edges_are_read_in_the_coordinates_the_drawing_renders_in();
    for set in IconSet::ALL {
        let mut documents: BTreeMap<&'static [u8], &str> = BTreeMap::new();
        for (name, reads) in ADDED {
            let index = find(name).unwrap_or_else(|| panic!("`{name}` is not in the catalogue"));
            assert!(!DIRECTIONAL[index], "`{name}` names a side or an axis of the screen, so it must not mirror");
            let document = svg(index, set);
            let tree = svg_tree::parse(document).unwrap_or_else(|error| panic!("`{name}` in `{}`: {error}", set.id()));
            let view = tree.size();
            let ink = painted(tree.root());
            assert!(!ink.is_empty(), "`{name}` in `{}` paints nothing", set.id());
            // The ink, stroke and caps included: a cap past the view box is a cropped glyph.
            let bounds = tree.root().abs_stroke_bounding_box();
            assert!(
                bounds.left() >= -SLACK
                    && bounds.top() >= -SLACK
                    && bounds.right() <= view.width() + SLACK
                    && bounds.bottom() <= view.height() + SLACK,
                "`{name}` in `{}` paints outside its view box: {bounds:?} of {view:?}",
                set.id()
            );
            drawn_as(name, set.id(), reads, &ink, (view.width(), view.height()));
            if let Some(other) = documents.insert(document, name) {
                panic!("`{other}` and `{name}` are the same document in `{}`: a copied drawing?", set.id());
            }
        }
    }
}
