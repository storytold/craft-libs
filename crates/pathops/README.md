# craft-pathops

Path operations on [`kurbo::BezPath`](https://docs.rs/kurbo): curve-preserving booleans, the Pathfinder
set (unite, minus front, intersect, exclude, divide, trim, merge, crop, outline, minus back), planar
regions, offset path, outline stroke, simplify and join. A path is a `BezPath`; the crate depends on
`kurbo`, `linesweeper` and `thiserror` and on no application.

The crate is carved from VectorCraft's `crates/pathops` (`vectorcraft-pathops`) at
[`storytold/vectorcraft` commit `ff9deee46d1c59dc9ecef0b668c25e57cd80b55e`](https://github.com/storytold/vectorcraft/tree/ff9deee46d1c59dc9ecef0b668c25e57cd80b55e/crates/pathops),
and every source file says so in its header. VectorCraft keeps its anchor-based `PathData` model; here
the same algorithms read and write `BezPath`. Diagram, vector and slide editors all need path booleans,
offsets and strokes, and can share it. The crate offers the building blocks; what each app's own command
means in terms of them is the app's to decide.

Licence: MIT OR Apache-2.0, as VectorCraft and this repository.

## Booleans (curve-preserving)

```rust
pub enum FillRule { NonZero, EvenOdd }
pub enum BoolOp { Union, Intersect, Difference, Xor }
pub fn boolean(a: &BezPath, a_rule: FillRule, b: &BezPath, b_rule: FillRule, op: BoolOp) -> BezPath;
pub fn try_boolean(.., op: BoolOp, precision: f64) -> Result<BezPath, PathOpsError>; // errors on NaN, huge, too many segments
pub fn boolean_n(paths: &[(&BezPath, FillRule)], pred: impl Fn(&[bool]) -> bool) -> BezPath;
pub fn unite_all(paths: &[(&BezPath, FillRule)]) -> BezPath;  // fast many-shape union
pub fn normalize(path: &BezPath, rule: FillRule) -> BezPath;  // resolve self-overlaps
pub fn area(path: &BezPath, rule: FillRule) -> f64;           // exact filled area
```

Each also has a `try_` form (`try_boolean_n`, `try_unite_all`, `try_normalize`, `try_area`) that reports why
it refused. The engine is [`linesweeper`](https://crates.io/crates/linesweeper), a robust sweep line that
works directly on cubic Béziers, so nothing is flattened to polygons. The sweep splits curves at
intersections and y-extrema. Afterwards:

- pieces that lie on one input cubic are rebuilt exactly as a single sub-curve of that cubic;
- other smooth joints (for example, in stroker output) are refitted by least squares within
  `DEFAULT_PRECISION` (0.01 path units);
- collinear line pieces are merged;
- the inputs' own anchors are always kept;
- slivers narrower than the precision are dropped.

Output contours are simple and consistently oriented: outer contours and holes wind in opposite
directions, so the result fills the same under either fill rule. Open subpaths are closed implicitly,
because a filled path is painted closed. Quadratic segments are raised to cubics on the way in.

## Pathfinder and regions

```rust
pub struct Shape { pub path: BezPath, pub rule: FillRule, pub key: u64 } // key = the caller's paint or object id
pub enum PathfinderOp { Unite, MinusFront, Intersect, Exclude, Divide, Trim, Merge, Crop, Outline, MinusBack }
pub fn pathfinder(op: PathfinderOp, shapes: &[Shape] /* back → front */) -> Vec<Shape>;
pub fn pathfinder_with(op: PathfinderOp, shapes: &[Shape], precision: f64) -> Vec<Shape>;
pub fn try_pathfinder(op: PathfinderOp, shapes: &[Shape]) -> Result<Vec<Shape>, PathOpsError>; // why there is no result
pub fn try_pathfinder_with(op: PathfinderOp, shapes: &[Shape], precision: f64) -> Result<Vec<Shape>, PathOpsError>;

pub struct Region { pub path: BezPath, pub sources: Vec<usize> }
pub fn regions(shapes: &[Shape]) -> Vec<Region>;          // all faces of the arrangement
pub fn region_at(shapes: &[Shape], p: Point) -> Option<Region>;
pub fn merge_regions(regions: &[&Region]) -> BezPath;     // their union
```

Each result takes its key as follows:

| Operation | Result |
|---|---|
| Unite, Intersect, Exclude, Minus Back | Takes the front-most key |
| Minus Front | Takes the back-most key |
| Divide | One shape per face, keyed by the front-most shape that covers the face. Open paths cut the faces they cross but give no shape of their own |
| Trim | One shape for each object that is still visible |
| Merge | Joins touching visible parts that share a key; each connected component becomes one shape |
| Crop | Keeps the visible parts of the lower objects inside the front-most object |
| Outline | Returns open edge paths, split at every junction; coincident edges are de-duplicated, and each keeps the front-most key. A closed subpath gives the edges of its outline, an open one the edges of the path itself (no closing edge; one of three anchors or more that ends where it starts is a ring, and counts as closed, and so does one cubic that loops round an area, closed or not), and where an open path crosses another is a junction. The crossings of open paths come from the planar map (below), which is built for up to `SHAPE_BUILDER_MAX_SEGMENTS` segments: past that, or when no map can be built, or when finding the junctions on each edge takes more than `MAX_SEARCH_WORK` steps (Limits, below), there are no shapes, since the filled areas, where an open path's way there and back cancel, do not see where it crosses another. A junction of the map that lies on one path at one place only is no cut (the two passes of an open path part by more than the map's precision where it runs nearly level, and leave vertices along it that nothing meets): the Outline cuts a path where it meets another path or itself, and not in the middle of a path that nothing meets (Known limits: it can lose a cut, never add one; the shape builder's lines and live paint's edges are not so held) |

An operation that cannot sweep its input (see Limits) returns no shapes.

## Planar maps

```rust
pub fn live_paint(shapes: &[Shape]) -> (Vec<Region>, Vec<Shape>);                // (faces, edges)
pub fn shape_builder(shapes: &[Shape], edges: bool) -> BuilderArrangement;       // regions, line pieces, edge pieces
pub fn try_shape_builder(shapes: &[Shape], edges: bool) -> Result<BuilderArrangement, PathOpsError>; // no fallback
pub fn has_open_path(shapes: &[Shape]) -> bool;
pub fn interior_point(path: &BezPath) -> Option<Point>;
```

Unlike `regions`, which splits the filled areas of closed shapes, `live_paint` treats every path as an
edge, open paths included. Three crossing lines enclose a triangle, a line across a rectangle splits it
in two, and an area enclosed by paths becomes a face even when nothing fills it. Each face lists the inputs
whose fill covers it (empty for an area only enclosed by paths); an open subpath of one cubic that comes back
to its start round an area is the outline of that loop and fills as a closed one does, here and in the fallback
below (other open paths fill nothing, rings of more anchors among them). Each edge is a piece of a path, split
wherever another path meets it, and keeps the key of the front-most input it lies on.
`shape_builder` gives the regions an editor offers for a set of shapes: with closed paths only they are the
`regions` (plus the gaps closed shapes enclose); once an open path is among them they are the planar faces, so a
stroke-only line across a shape splits it. Inputs with more than `SHAPE_BUILDER_MAX_SEGMENTS` segments fall back
to the filled areas.

The sweep takes an open path's two passes (there and back) for cancelling out where the path is level, so the
map is swept in a frame sheared (`y + k x`) so that no open path is level in it after the paths are snapped to the
sweep's tolerance, which a shape far away widens. The first of ten shears (1/32, 1/64, 1/16, ...) that leaves no
side level is used (a side of a segment's chord or control polygon). A shear that leaves a side longer than 8
tolerances of the sweep level is not used. A side from 2 to 8 tolerances long does not rule a shear out, but the
shears are tried on for one that does not level it either, and where none leaves every such side unlevelled the one
that levels the fewest is used: beside a shape at 1e11 (a tolerance of 1.5e-3), a horizontal edge 0.009 long is
level in the usual shear and kept in the shear of 1/4. When each of the ten leaves a long side level, one more shear
is tried, between the slopes of the long sides, where none of them is; when the long sides leave no shear free,
there is no map, as for an input the sweep cannot take. A side of 2 tolerances or less is level in every shear and
not looked at: a stub, a point repeated a hair apart or a handle a nanometre long does not empty the map, and a
feature that small may be lost from it (the sweep cannot tell it from a point). A shear in which the paths pass the
largest coordinates the sweep takes (near 7e13) is passed over for the next.

## Offset and stroke

```rust
pub use kurbo::{Cap, Join};
pub fn offset_path(path: &BezPath, delta: f64, join: Join, miter_limit: f64) -> BezPath; // delta < 0 insets
pub fn try_offset_path(..) -> Result<BezPath, PathOpsError>;
pub fn outline_stroke(path: &BezPath, width: f64, cap: Cap, join: Join, miter_limit: f64) -> BezPath;
pub fn try_outline_stroke(..) -> Result<BezPath, PathOpsError>;
pub fn stroke_region(outlines: &[BezPath]) -> BezPath; // the union of stroke outlines
```

`outline_stroke` passes the path to `kurbo::stroke` and then normalizes the result, which removes the
overlaps at joins. `offset_path` computes `fill ∪ stroke(2|d|)` for a positive offset and `fill − stroke(2|d|)`
for a negative one; an open subpath is outlined with a stroke of width `2|d|` (butt caps), and an open
subpath whose last point is on its first is offset as a closed one. When the arrangement is too complex,
`offset_path` returns the input unchanged. The quality of offsets depends on kurbo's parallel-curve stroker;
its output is refitted, so offset results have more anchors than boolean results.

## Path editing

```rust
pub fn simplify(path: &BezPath, tolerance: f64) -> BezPath;
pub fn simplify_with(path: &BezPath, opts: &SimplifyOptions) -> BezPath; // tolerance, corner_angle_deg, straight_lines, smooth
// SimplifyOptions::default().tolerance(1.0).straight_lines(true): #[non_exhaustive], so start from the default
pub fn remove_redundant_points(path: &BezPath, tolerance: f64) -> BezPath;
pub fn join(paths: &[BezPath], tolerance: f64) -> BezPath;
```

`simplify` fits least-squares cubics with fixed end tangents, then reparameterizes with Newton steps and
splits at the point of worst error. Any point where the path turns more than the corner threshold (30° by
default) stays a corner, and a subpath never gets more points than it had (except with `straight_lines`, which
only draws lines). `join` joins the open subpaths of
all its inputs nearest-endpoint-first (ends closer than the tolerance become one point, otherwise a straight
segment joins them); a single open subpath is closed instead, and closed subpaths pass through.

## Limits

The operations take hostile input. The crate denies `unwrap`, `expect` and `panic` outside tests, and the
work of each operation is bounded before it starts, in counts the tests assert on:

- coordinates that are not finite are `PathOpsError::NonFinite`; coordinates so large that the sweep's
  tolerance would pass a unit are `PathOpsError::Degenerate`;
- more than `MAX_ELEMENTS` (524,288) path elements (every move, line, curve and close) in all the inputs of
  one operation is `PathOpsError::TooComplex`, checked first, in one length read;
- so is a sweep that would take too long: more than `MAX_SEGMENT_PAIRS` (4,194,304) pairs of segments whose
  bounds overlap (`MAX_OFFSET_SEGMENT_PAIRS`, 524,288, for an offset), or more than `MAX_SWEEP_LINE_PAIRS`
  (268,435,456) pairs of pieces of the input at overlapping heights. The sweep line is a list of the pieces
  it crosses and each new piece is placed by scanning it, so a row of dots on one line, whose bounds never
  touch, costs the square of its length. Pieces of no height (the collinear vertices along a rectangle's top)
  are not on the line, and count only against the pieces that cross their height. Each crossing, and each
  end of a segment, also scans that line, so the pairs of segments whose bounds overlap (pairs of curves
  nine times, a line and a curve three) times the most pieces on the line at one height are held to
  `MAX_SWEEP_EVENT_WORK` (2^32, 4,294,967,296; where `usize` is 32 bits, on the web, the most it holds, 4,294,967,295). All of them are counted in O(n log n) before the sweep;
- that count takes every pair of segments whose bounds overlap for a crossing, which prices a hatch (a fifth of
  its pairs cross) and under-prices a polygon whose every edge crosses nearly every other: a star of n points,
  each joined to the one nearly opposite, crosses in n (n - 3) / 2 places, every pair of its edges overlaps, and the
  sweep scans a line of n pieces for each. 2,001 edges (2.0 million pairs, 4.0e9 searches) passed the limits above
  and took 19 s and 3.5 GB (VectorCraft: 26 s and 3.7 GB). So where the pairs of bounds (priced as below: a pair
  of lines one, a line and a curve six, two curves eighteen, and two for each curve) times the most pieces on the line
  could pass `MAX_CROSSED_WORK` (2^30), they are tested (no more than `MAX_SEGMENT_PAIRS` of them), and the
  places where segments really cross, away from their ends, times those pieces are held to it: 1,001 edges take
  2.4 s and 0.9 GB, 1,291 (the most) 4.3 s, and 1,293 or more, 2,001 among them (in 14 ms) and 3,001 (by the pairs, in
  2 ms), are `TooComplex`;
- the sweep's time and memory follow its crossings themselves (about 3 microseconds and 1.2 to 1.6 KB for each place
  where lines cross), and the count above times the pieces on the sweep line is small where the shapes that cross a
  great deal are stacked so that their heights do not overlap: 206 stars of 201 points in one path, each above and
  beside the last (41,818 elements, 4.1 million pairs of bounds, 201 pieces on the line, 4.1 million crossings),
  passed every limit above and took 13 s and 6.5 GB (and a `wasm32` build aborts), and 100 such stars whose edges are
  bowed cubics (20,300 elements; curves were not counted at all) 14.5 s and 3.7 GB. So the places where segments really
  cross are held to `MAX_REAL_CROSSINGS` (2^20, 1,048,576) whatever the pieces on the line. The count is a bound
  that is never below the number there are, and is made, whenever the most it can come to (priced as below) could
  pass the limit or the limit on the crossings times the pieces on the line, by testing the pairs of bounds that
  overlap (already held to `MAX_SEGMENT_PAIRS`):
  - two lines are tested, and two nearly parallel ones count one unless the ends of one lie on one side of the other.
    Nearly parallel is the rounding's word and not an angle's: where the cross product of the directions is no more than
    16 epsilon of the product of the lengths (an angle of 3.6e-15), where the lines cross, or whether, cannot be told,
    and the pair counts one unless both ends of either lie to one side of the other by more than the rounding, or the
    parts of them along one another are no longer than the slack (lines that touch end to end). Above
    that the crossing is computed and its place along each line is known to within 16 epsilon of the largest distance in
    play over the sine of the angle: a crossing that near an end counts as away from it, and in the work of lines
    through one point crossings that near one another count as one point;
  - a line and a cubic count the changes of sign of the distances of the cubic's control points from the line
    (at most three: a cubic crosses a line no more often than its control polygon does), and none where they meet
    only at an end they share and the line leaves it outside the cone of the rays to the cubic's control points;
  - two cubics count one where the directions of their tangents are apart (as lines, not as ways: two arcs that cross
    twice both have a tangent along the chord between the crossings, so arcs whose tangents never run along one
    line cross once at most), by pieces of an eighth where those of the whole curves are not, and nine (the most two
    cubics can cross at) for each pair of pieces that are not apart, and none where they meet only at an end they
    share and the cones of the rays from it to their control points do not meet (a path's joints, the ends of a fan);
  - a cubic whose tangents lie in no cone narrower than a half plane may loop and cross itself, once at the most, and
    counts for it.

  A place where a curve crosses counts for two, for the sweep takes about twice as long over it and half
  the memory. The bound of a star of cubics is about a third over its crossings. 52 stars of 201 points
  (1,034,748 crossings, the most; 53 have 1,054,647) took 3.2 s and 1.6 GB before the metered sweep, which makes 50 of them (a
  work of 4.14e9 of its budget's 4.2e9, 2.3 million pieces) and refuses 51, and 19 of bowed cubics (the most) take 2.6 s and 0.5
  GB; 53 and 206 stars (the latter in 0.08 s) and 20 and 100 of bowed cubics (in 0.02 and 0.09 s) are `TooComplex`. A
  single star of bowed cubics is made up to 871 points (3.2 s), and 881 are refused. A pair of cubics whose tangents
  run along one line anywhere (arcs close and nearly parallel, or crossing at a shallow angle, whose pieces' cones
  overlap) counts nine, so 58 thousand such pairs overlapping are refused, whatever they do;
- what is done with the output of a sweep is bounded as well: a Pathfinder Outline finds the junctions on each
  segment among those inside the bounds of the segment (a grid of 100 and 100 open lines makes 10,000 of them,
  and every arch round it holds them all in its bounds with the curve nowhere near), and an inset checks
  each segment of its subpaths against the boundary of their fill. Each step is charged before it is made, to
  `MAX_SEARCH_WORK` (2^30 steps: a junction looked at is one, a block of 64 of them one, the nearest point of a
  curve to a point 64, a third of a microsecond; about 5 s). Past it a Pathfinder Outline is `TooComplex` (no
  shapes, for `pathfinder`), and an inset takes the boundary of the fill, which is right for any input;
- segments that meet at one point cost about the fourth power of their number, which none of those counts
  (a pie of k slices has 2k ends at its centre): more than `MAX_ENDS_AT_A_POINT` (512) ends at one point is
  `TooComplex`. The ends are counted in a hash at the sweep's tolerance, which every point of every element
  sets, control points too (one cubic with control points at 1e12 makes it 0.014, and 400 spikes whose apexes
  are 0.004 apart one point to the sweep, which took 4.4 s). A pie of 256 slices takes 0.5 to 0.9 s, 257 are
  refused in a few milliseconds. Many points that each hold that many are held by `MAX_ENDS_WORK` (4e9
  nanoseconds), the sum over the points of what the sweep takes at each (the greater of 5.6 `n`³ and 0.019 `n`⁴
  nanoseconds for `n` ends, which is what pies of 64 to 256 slices took): pies of 256 slices stacked on a diagonal took 1.3,
  2.2, 4.8, 15.7 and 65.8 s for 1, 2, 4, 8 and 30, and 3 are admitted, 4 and more refused in a few milliseconds. Three
  things cost more than that fit, and are priced on their own:
  - **ends of copies:** a stack of one shape one on another (every edge the same segment `c` times) costs the cube of its ends
    at 18.5 nanoseconds, in proportion to the pairs of ends there that are copies of one segment: 230 squares took 9.6 s
    where the star of 1,291 points took 6.4 beside it, 255 triangles 10 s beside 7, 21 stacks of 128 copies of a segment 21 s
    beside 5.9. The most admitted are 185 squares (2.9 s beside 4.5), 205 triangles, 220 copies of a segment and 6 stacks of
    128, none over the star; a wheel of triangles that share their sides (two copies of each) costs as a pie;
  - **ends moved a few tolerances apart:** the ends of segments longer than 256 tolerances that lie within 64 tolerances of
    one another (squares of 64, and the squares round them) are one cluster, priced at 19.6 `n`³ where they are not at one
    point (the fit's 5.6, 3.5 times over, for the memory: the sweep takes about 8 bytes for the cube of the ends), so that the
    pie of 256 slices whose apexes are each moved by up to 10 tolerances (1.5 s and 1.1 GB, an exact pie 88 MB) is made, and 2
    of them (4 took 4.5 GB, a pie of 400 slices 4.9 GB, under a limit of 3 GB both abort) are refused. How far they must be
    moved to cost nothing was measured: the memory of a pie of 256 slices is 650 MB at 2 tolerances, 1.2 GB at 8, 0.96 GB at
    15, 0.41 GB at 30, 0.16 GB at 60 and 0.1 GB at 100. Short segments are left out (100,000 cubics round a circle of
    radius 0.09, whose joints are 5 tolerances apart, took 0.4 s);
- lines that cross at one point, with no end there, cost about the cube of their number (150 take 0.2 s, 300
  1.7 s, 450 8 s, and about a thousand passed every other limit and would take a minute and a half). A line
  that crosses `m` others at one point counts `m`², so `c` lines through a point count `c` (`c` - 1)², and
  `MAX_CROSSING_WORK` (2^24) is at 256 lines through a point, which take about a second (257 are `TooComplex`
  in a few milliseconds). The crossings are computed, for the lines whose bounds overlap, only when that
  many pairs could reach the limit if they were at one point, and then once for each pair (in O(pairs log
  n), the pairs being held to `MAX_SEGMENT_PAIRS` already). Lines that end at the point are ends, above. Where two
  lines cross is known to the rounding of the arithmetic over the sine of their angle (a crossing of lines 1e10
  long at 1e-13 radians is placed to within about 360 of their length), and crossings are taken as one point when they lie within
  the tolerance and that uncertainty of each other. Lines whose angle is too small to place a crossing at all (under 16
  epsilon of the product of their lengths) count as crossing somewhere along their length, unless both ends of one lie on
  one side of the other, and so add to the work of every group of their lines: 450 lines through a point at angles of
  1e-13 or 1e-16 (lines 2e7 to 1e10 long, ends a tolerance or ten apart), which took 25 to 41 s, are refused in 6 ms.
  Lines that lie on one line and overlap are of that kind: 250 and 500 on a diagonal (subpaths each from `(i, i)` to
  `(i + 1000, i + 1000)`) took 13 s and over a minute and a half, and are refused now, in 2 and 6 ms; level ones (the same height at
  both ends) are not counted, for pieces of no height are not on the sweep line (250 take 16 ms, 1,000 0.3 s and 0.6 GB). An end
  that two lines share exactly does not make them meet only there if they lie on one line: a fan of 250 segments from one point down
  a diagonal took 11 s and down a vertical 9 s (all of them share their start and overlap), and is refused now in a few
  milliseconds, while the same fan along a level takes 11 ms. The same line twice (both ends shared, as the closing segment of a
  subpath of one line, or stacked copies of a shape) is not counted: its cost is in the ends at its two points, priced as
  copies (above). The curves count too. A curve that is a line to the sweep's tolerance (every control point within 2 of
  the line through its ends) is a line here, the span of its control points: 256 straight cubics from one point down a diagonal,
  each closed by a line back, took 7.8 s where the star of 1,291 points took 5.3 beside them (12.5 s at another load) and
  counted nothing of work, and are refused now in 2 ms. The other curves are told where one runs along another, as pieces of
  one cubic do (a fan of them from its start, windows of it that share no end): of the curves whose tangents may run along
  each one's own (the pairs the cones above count nine), up to 24 are tried, by sampling the one at nine places and asking
  whether three in a row are within 64 tolerances of the other (a stretch of a quarter of it), and as many as that fraction
  of them run along it, counted squared and 1.5 times, as many as the lines on one line: 256 pieces of an arc from its start
  took 7.2 s beside 5.7, 256 windows of it 7.3 s beside 5.9, and are refused in 4 to 14 ms; 150 pieces, 160 windows, 100 or
  128 straight cubics take 0.3 to 0.9 s and are made. Crossings along one line within 64 tolerances (and the uncertainty
  of each) of one another are one point's (they were 2): 300 lines through a point, each moved sideways by up to 10
  tolerances, took 2.6 GB and aborted under a limit of 3 GB. Where they are within a thousand tolerances but not at one point
  (to the rounding) they count 1.7 times, for the memory: 256 lines moved 2 tolerances took 2.7 GB, 215 took 1.6 GB (the
  metered sweep refuses both, at 7.9 and 4.6 million pieces), 256 exactly through a point 0.8 s and 58 MB (refused by it too, as a work
  of 5.2e9; 200 are made);
- a boolean operation asks its predicate once for each distinct winding of a side of a segment, and hands it a
  flag for each input. That is the winding of every input, not the set of flags it gives: shapes nested and
  wound the same way are windings 1 and 2 under the non-zero rule, which are the same flags and are asked
  about separately, so a predicate that counts its calls sees more than one for a set of flags (and it must
  answer by its flags alone). There are at most two windings for each segment, so the flags are at
  most twice the segments times the inputs, which is more than the sweep's own work when most of many
  inputs are empty: past `MASK_WORK` (268,435,456) of them, counted as four for each segment and input
  before the sweep, it is `TooComplex`. The contour pass asks about the same side again and again (it
  scans west through the segments that are no boundary for every contour it starts), so the answers are
  found where the windings are stored, by their address, and no winding is hashed again: the work of the
  pass does not grow with the scans, however long the windings of nested shapes are. Divide and `regions`
  list the inputs that cover each side of each edge, one entry for each covering input and each side, which is near
  the number of segments when shapes lie apart (100 shapes apart walk 1,200 entries, 400 walk 4,800), and give
  nothing past the same number of entries. The union of many inputs, and of the outlines of a stroke, stops
  as soon as the contours made pass `MAX_ELEMENTS`, and does not make the rest; and the boundary of each sweep
  of an input is counted before its contours are made (they are made in one call, which cannot be stopped): a
  sweep whose boundary has more segments than the elements left makes none, and the union is `TooComplex`.
  What is made is then at most three elements for each boundary segment (a boundary piece is one element or
  less, a contour adds a move and a close), and the elements made are checked exactly after;
- an arrangement of several inputs keeps, for each side of each piece the sweep cuts a segment into, the
  inputs that cover it. For shapes nested n deep that is about 4n(n + 1) entries (2,000 nested squares hold 16
  million, 270 MB), and the sweep cuts every side where others cross it, so n nested squares crossed by m
  thin bars hold about 3n²m (200 and 200: 430 MB; 300 and 300: 1.3 GB). The entries are estimated before
  the sweep, from the pieces: a segment is cut into one more than the lines that cross it (found by
  computing the crossings of the lines whose bounds overlap, and of a line and a curve; a curve is cut
  three more for its turns and itself, and once by each curve when an eighth of the one lies within the
  bounds of an eighth of the other), each piece having
  an entry on each side for every input whose bounds overlap the segment. Past `MAX_COVERAGE_ENTRIES`
  (33,554,432) the operation is `TooComplex` (nested 2,896 deep or more, 2,895 take 550 MB; 201 and 201
  bars pass, 202 and 202 do not; without it 8,000 deep needs about 4.5 GB, which ends a web page). The
  estimate is at least what lines hold and under twice it in the nested and barred squares tested, but it
  is not a bound for curves that cross each other more than once;
- Merge, Divide, Trim and Crop colour the classes of faces into batches whose faces share no vertex and
  make one contour pass of the whole arrangement for each batch; classes that meet at one point are a batch
  each. The batches times the segments are counted before the first pass, and past `MAX_PASS_WORK`
  (33,554,432) the operation gives no shapes (a fan of 209 shapes at a point beside 40,000 squares is refused in
  0.7 s; 208 shapes there take 2.6 s to merge and 3.5 s to divide, where the squares alone take 0.4 s);
- a planar map (`live_paint`, `shape_builder`, Divide with an open path) finds what fills each face by
  winding a point of it against each subpath whose bounds hold it, and the face each hole is in by winding
  the hole against each face whose bounds hold it, so contours nested n deep in one shape cost about the
  square of n (rectangles nested 4,000 deep, about 96 million elements wound). Each test is charged before
  it is made, and past `MAX_FILL_WORK` (67,108,864 elements) the map is not built: `live_paint` gives
  nothing, and `shape_builder` and Divide with an open path fall back to the filled areas, as past
  `SHAPE_BUILDER_MAX_SEGMENTS`: not `TooComplex` or no shapes, but an answer that is not the arrangement
  asked for, the faces unsplit by the open paths (rectangles nested 3,300 deep pass, in 0.75 s; 3,400 do not).
  `try_shape_builder` and `try_pathfinder` give `TooComplex` there instead, so a caller can tell, and also
  where the faces of closed shapes are past `mask_work` or `pass_work` (Divide gives no shapes and
  `shape_builder` no regions of the filled areas there, as `regions` gives none), which they do not hide as
  an empty drawing or a map with the filled areas missing; they check the shapes as given as well, before
  the fill leaves out a loop of one cubic whose handle is no number (`NonFinite`, or `Degenerate` for one
  past what the sweep takes);
- the winding of a point against a path is made by `src/winding.rs` (see "Differences from VectorCraft"): a segment
  whose box the point is outside is turned away without a step, and a cubic takes at most 12 steps of Newton's method
  and 64 halvings for each of its (at most three) pieces, so the charge stays one element for each segment wound. A
  winding is faster than kurbo's where most segments are turned away by their boxes, as in a map of shapes (a circle
  or a polygon of many segments takes 0.07 to 0.6 of kurbo's time, and the nested rectangles above take 0.14 / 0.36
  / 0.75 s where they took 0.20 / 0.58 / 1.34 s), and slower for a cubic that crosses the point's height: against one
  arch or one general cubic with the point inside its box, 1.25 and 0.9 times kurbo's time as one reviewer measured it
  and 1.5 and 1.3 here on a loaded machine, and, where the height is near a flat end of a piece (a cubic whose height is
  `t^3`, so that Newton's method gives up and 62 halvings follow, 76 evaluations at the most), about 5 times (480
  nanoseconds against 97);
- a stroke's outline is counted as it is made (a subpath of more than 256 segments is stroked in windows of
  256, so that a width that is huge against the tolerance cannot make millions of curves first) and the
  operation is `TooComplex` past `MAX_ELEMENTS` elements of outline. The union of the outlines in
  `stroke_region`, and of the paths in `unite_all`, counts the boundary of its sweep before its contours are
  made and the contours before they are written, and is `TooComplex` (or empty) past the element limit too
  (outlines that each pass cross each other a great deal);
- the operations without a `Result` answer an empty path, no shapes, or the input unchanged, as their
  documentation says. `simplify` stops fitting at `MAX_FIT_POINTS` samples of one run and at `FIT_WORK` in
  all, `remove_redundant_points` at `REDUNDANT_WORK` (the joints it reads in its search, the points of a run it
  checks one by one against the line that replaces it, and the points a run of curves is refitted from) and
  `join` at `JOIN_WORK`, and leaves the rest as it is;
- Most of `REDUNDANT_WORK` goes to the refits (24 for each point, about a hundred points for a run of a few
  segments at a tight tolerance), so a path of hundreds of thousands of short pieces that each need one is
  left partly merged: 450,000 cubics in arcs of 9 between corners of 30 degrees come out at 1.5 to 2.8 times
  the elements e2351f5 gave (242,013 against 150,001 for arcs of radius 1,000 turning 0.5, at a tolerance of
  0.01, in 1.3 s), and at 100,001 (fewer than e2351f5) with `redundant_work` raised to 2^34 (on a 64-bit target), in 5 s. What is
  merged on the way is within the tolerance all the same;
- lengths are measured to a relative accuracy (kurbo's takes an accuracy in the units of the path, which a cubic of
  large coordinates meets only by subdividing the most it ever does), at any size the sweep accepts, and an
  inset compares the lengths of the closed subpaths and of the boundary of their fill for at most 4,096
  curves: past that it strokes the boundary, which is right for any input;
- the tidy that merges the pieces the sweep split joins at most 1,024 pieces into one line and 32 into one
  curve at a time (each piece added tries the whole run again), so a straight run of more comes out as
  lines of that many; the work is proportional to the pieces;
- the angle an edge leaves a vertex at is read from the edge flattened to a millionth of the distance to the
  nearest other end at the vertex, which the sweep's stubs make tiny: a segment is flattened whole (the same
  lines VectorCraft makes, which the order of edges leaving along one tangent depends on) up to about a
  million lines each and 67 million for a map, and past that only the start of it that reaches that
  distance is, a few hundred lines;
- `interior_point` and the faces of a planar map test a handful of points against the path itself
  (windings of a comb of a thousand teeth are not one pass over the path each).

A panic inside `linesweeper`, which can happen on rare near-degenerate input, is caught with
`std::panic::catch_unwind` at the crate's public functions and becomes `PathOpsError::Degenerate` or an
empty result. That is all it does: the panic hook still runs and prints the message; in a build with
`panic = "abort"` (WebAssembly, or an app that sets it) nothing is caught and the process ends; and a panic
in a closure the caller passes (the predicate of `boolean_n`) is caught the same way, so it is reported as an
error and not raised. Apps that run on the web should keep their coordinates well inside the range the sweep
accepts (under about 7e13) and their input well inside the limits above.

### The metered dry sweep

The counts above price a sweep from its geometry, and each was set from the worst input found for it. Review found shapes they do not
see after every round, and an automated scan of our own commit 3814284 found two more: copies were found by the bits of their
points, so copies a unit in the last place apart were none to the count of ends, and where curves run along one another was sampled
(24 partners to a curve, at fixed places in the list). More rules would not end that, so under them the crate runs the sweep itself, before
the operation sweeps, and stops it at a budget, `Limits::sweep_work` (`MAX_SWEEP_WORK`, 4,200,000,000). The counts above stay as the fast first
filter (an input they refuse is refused in milliseconds, without sweeping); the meter is the backstop under them.

- **The dry sweep.** `src/meter.rs` builds the segments the topology is built from (the monotone pieces of every path, all the paths of
  an operation in one arena, by the same call of `linesweeper`) and runs `linesweeper`'s `Sweeper` over them, at the same tolerance, through
  its public interface, in the loop `Topology::from_paths` runs: each sweep line, each range of it, each position of the range, the pieces
  that go down from it and the horizontal fragments that start there. It builds nothing from what it sees, and counts. Nothing in
  `vendor/linesweeper` is patched, so an app that takes `linesweeper` 0.5 from the registry gets the same sweep. It is the same sweep, and tests
  show it: for 21 inputs (the fixtures of the suites, quadratics, a path that starts with no move, several paths) the segments, their
  entrances and exits, and the number of pieces the dry sweep counts are those of the topology, whose `compute_positions` has an entry for every
  output segment made (the coincident ones that are merged out afterwards too).
- **The units.** The unit is the nanosecond of sweep that a count is worth on the machine the weights were measured on (one thread, a release build),
  so that the budget reads as a time. The work of a sweep is the greater of the time it will take and the memory it will hold (also counted in
  nanoseconds: a piece of the topology is 400 to 700 bytes, and is worth 1.8 microseconds, so that the budget is 2.3 million pieces, 0.9 to 1.6 GB: the star holds
  1.96 million and 0.9 GB, 50 stars stacked 2.3 million and 1.6 GB; and a pair of segments that start or end together, which the sweep's cache of comparisons
  holds, is worth 1.1 microseconds more, below). The time is a
  sum of five terms, each priced for the kind of input that makes it large (weights fitted to about 180 inputs of 30 kinds):

  | Term | Counted as | Weight | What makes it large |
  |---|---|---|---|
  | output pieces | each piece of a segment between two of the sweep's points, and each horizontal fragment (a piece of a curve counts 4,000 more) | 600 ns | the crossings of many segments; every shape (about 0.6 microseconds a piece for a circle of 100,000 cubics) |
  | scan | each piece times the segments that span the sweep line at its height | 1.05 ns | the star of 1,291 points, a hatch of 400 strokes, a bundle of 1,500 lines that cross along their length: the sweep scans the line for each piece, and a nanosecond for each segment of it is what they all take |
  | tangle | each piece times the square of its cluster: the most of the segments of its range that lie within 8 tolerances of one another across | 0.23 ns | pencils of lines, curves that run along one another, copies of a shape: where many segments lie within a few tolerances of each other the sweep takes time for every pair of them |
  | ends that start together | the fourth power of the segments that start, and of those that end, within 8 tolerances of one another in a grid (across and up, in four grids, each moved half a cell: a cluster of 4 tolerances lies in one cell of one of them) | 0.07 ns | stacks of copies, pies, fans: what the sweep does at a height it does in one step, which nothing counted as the sweep goes can stop in time (a stack of copies of one shape is a dozen steps and nine seconds), so these are counted from the segments before the sweep; copies a unit in the last place or a fraction of a tolerance apart start together as copies one on another do |
  | segments that start at one height | the square of those that start (and of those that end) at one height, wherever they are across | 3 ns | a row of dots on one line: their shapes put their pieces into the sweep line in one step |

  The star of 1,291 points counts 3.8e9 (91% of the budget), a hatch of 400 and 400 strokes 3.1e9.
- **The cache of comparisons.** `linesweeper` keeps the order of each pair of segments it compares in two tables that it never clears (`ComparisonCache` in its
  `order.rs`: the sweep's own, and a second, conservative one), and segments that start or end together are compared with one another: a pie of k slices has 2k - 1 ends
  at its centre, which make (2k - 1)(k - 1) entries. An entry takes about 650 bytes once its table has grown to hold it, and a table grows by doubling.
  2,200 pies of 30 slices (264,000 elements, 3.76 million counted pairs) aborted a process held to 3 GB after 7 to 9 s, in the allocation of the next table of the second cache
  (877 MB, at about 1.8 million entries, which for a pie is about 0.49 of the pairs counted), and so did 2,250, and 5,200 pies of 20 slices (3.85 million); with the meter off,
  2,150 pies of 30 (3.68 million) and 5,000 of 20 (3.71 million, RSS 2.5 GB) ran. The first meter did not count them; it does: before the sweep, the pairs of the starts and ends
  of the segments that lie within 8 tolerances of one another across and up (by cell where many lie together, and for few by a pass of the points in order across, so that the
  vertices of a polygon of 262,000 sides are a pair each and no more) are charged 1.1 microseconds each, with the memory of the pieces, and 1.18 microseconds each against the
  budget alone, so that the budget admits 3.56 million of them whatever else the sweep holds: for pies, 4% under the cliff. 2,000 pies of 30 slices (3.4 million pairs), 4,500 of 20,
  900 of 45 and 700 of 50 are made. 2,100 and more of 30, 1,000 of 45, 800 of 50 and 5,000 and more of 20 are refused in 0.03 to 1 s, before the sweep; 2,080 of 30 and 4,550 and more of 20
  are refused by the dry sweep, after 4 to 5 s. A child process held by `ulimit -v` ran the most admitted with limits of about 2.4 GB (700 pies of 50 slices), 2.5 GB (900 of 45),
  2.6 GB (2,000 of 30) and 2.8 GB (4,500 of 20): 0.2 to 0.6 GB under 3 GB (the least for pies of 20, as the 2,000 pies of 30 that were always made fix the edge, and 20-slice pies have
  more pieces for their pairs). The refusals of the dry sweep near the edge hold up to about 2.7 GB. Past the edge a sweep that fits does not fit by much: 800 pies of 50 slices, 3.88 million
  pairs, need 2.9 GB, and 530 of 60 and 190 of 100 need 2.9 GB, which is why the pairs have a limit of their own and not only their weight with the pieces'. The shapes that make entries
  by their crossings and not by their ends (a mesh, a grid of squares, rings: 4 to 11 for each piece, 1 or so for most shapes) make them in proportion to their pieces and are held by the
  piece's weight and by the counts of crossings (a mesh of 180 cells, the most the default limits admit, makes 2.1 million, in 1.3 GB).
- **The budget.** The work is held to `MAX_SWEEP_WORK` as it is counted: after each range of each sweep line the sweep stops, refusing the input
  as `TooComplex`, if the work counted so far is past it, so an input that is refused has cost about the budget of sweep at most. The counts made
  before the sweep refuse stacks of copies, fans and pies in 0.3 to 40 ms; the other holes were refused in 0.14 to 1.5 s, and in 3.8 to 5.1 s
  where the budget is nearly spent (the zigzags of 1,500 and 2,040 lines, a hatch of 500, a star of 1,501: the dry sweep of the star of 1,291 points
  takes 4 s). A budget of `usize::MAX` leaves the meter off, and the dry sweep is not run. The budget was set so that the most the meter admits takes
  about as long as the star of 1,291 points beside it: of the kinds measured, the inputs it admits nearest the budget take 0.07 to 0.7 of its time (with the meter on for both: six stacks of 128 copies 3.0 s beside the star's 8.1, a hatch of 400 5.6 beside 8.4, 50 stars stacked 5.6 beside 8.1, three pies of 256 slices 4.7 beside 8.8).
- **The gate.** The dry sweep is run only for an input of more than 192 monotone pieces (the pieces the curves are cut into where their height turns, and the lines,
  the level ones too: the length of the arena the sweep is made of, which an input cannot keep low whatever its shapes). Under that no input costs more than a few
  tenths of a second in the sweep: the worst of 22 shapes that cost most for their pieces (fans and stacks of copies, pies, pencils and zigzags of lines, stars whose every edge
  crosses every other, curves that run along one another), each at the largest size under the gate, takes 0.2 s or less in the sweep (a fan of 96 straight cubics) and up to 1.0 s in a planar map (a star of 92 bowed cubics), a test times them all. Past it every input
  pays for the dry sweep, which costs about as much as the sweep it guards: `try_area` with the meter on against off, back to back (a release build, best of two, the star of 1,291 points 4.6 s and 9.1 s; a hatch of 400 strokes each way 3.1 and 5.8 s; 15,000 rings 1.5 and 2.4 s; 40,000 S-strokes 1.2 and 1.6 s; a circle of 100,000 cubics 0.37 and 0.49 s; a polygon of 262,000 sides 0.85 and 1.1 s; a mesh of 170 by 170 cells 2.3 and 4.2 s; 2,000 pies of 30 slices 4.2 and 7.8 s) is 1.2 to 2.0 times the time (1.1 to 2.2 in the recheck of the commit before this one, over the rows of the table of costs), in the same memory but for 4% more at the most (2,000 pies of 30 slices 2.35 and 2.45 GB). The counts before the sweep, for inputs of 250,000 segments, take 0.1 s of it.
- **What it bounds.** The time of the sweep and, with the 1.8 microseconds a piece, the memory of the topology (the star holds 1.96 million pieces and 0.9 GB; 2.3 million
  are the most that are admitted). The dry sweep's own memory is that of the sweep it is, up to 0.7 GB for the star and the zigzags at their refusal, and is freed before the
  operation sweeps (2,000 pies of 30 slices hold 2.4 GB in it, which the cache's term bounds). Every input a review found, and the two classes the scan named, are refused by the meter alone, with the other limits lifted (tests in `tests/it/meter.rs`;
  the table of costs gives the times).

What it does not bound is in Known limits: a step of the sweep is not interrupted, the weights are a fit (near the budget the time is 0.4 to 1.4 of the work, and far under
it for the kinds the memory and the stacks of copies are tuned to refuse), and the stages after the sweep.

### Choosing the limits

The constants above are the defaults of `Limits`. An app that knows its machine, or runs an operation where it
can wait, chooses its own for the operations it calls:

```rust
let mut limits = Limits::default();
limits.sweep_line_pairs = 1 << 30;
let outline = with_limits(limits, || try_outline_stroke(&path, 3.0, Cap::Butt, Join::Miter, 4.0));
```

`with_limits` puts the limits in force on the calling thread for the closure and puts the caller's back when
it returns, also if it unwinds; scopes nest, and another thread keeps its own. `Limits` has a field for each
bound (`elements`, `segment_pairs`, `offset_segment_pairs`, `sweep_line_pairs`, `shape_builder_segments`,
`fit_points`, `fit_work`, `join_work`, `redundant_work`, `mask_work`, `sweep_event_work`, `crossed_work`, `real_crossings`, `pass_work`,
`fill_work`, `search_work`, `ends_at_a_point`, `ends_work`, `crossing_work`, `coverage_entries`, `sweep_work`) and is `#[non_exhaustive]`, so start from
`Limits::default()`. Time and memory grow with a limit raised, and a limit lowered refuses more ordinary art.

### Ordinary input the defaults refuse

`MAX_SWEEP_LINE_PAIRS` is a worst case: it prices a pair at 25 ns, and the sweep takes 1 to 25 ns for one,
so some input that would finish in a few seconds is refused. Measured in a release build, with the
sweep-line limit at its default and raised to 2^30:

| Input | By default | With `sweep_line_pairs` = 2^30 |
|---|---|---|
| `try_outline_stroke`, 3 wide, of a wavy open path of cubics 10 long and 3 high | 3,000 cubics: 1.0 s; 3,100 and more: `TooComplex` in 0.3 s | 4,000: 1.7 s; 6,000: 2.7 s; 8,000: `TooComplex` |
| `pathfinder(Divide, ..)` of one comb polygon (a base with teeth 100 high and 1 wide, 10 apart) | 6,500 teeth: 0.5 s; 7,000 and more: no shapes, in 4 ms | 10,000: 0.6 s; 20,000: no shapes |
| `unite_all` of a row of dots on one line | 5,500 dots: 3.0 s; 5,800 and more: `TooComplex` | |

`MAX_SWEEP_EVENT_WORK` is also a worst case, priced at some nanoseconds a search of the sweep line: a hatch of
400 thin strokes slanting each way across 400 others (3,200 elements, none of them level) takes 3.1 s, and
one of 408 or more is `TooComplex`. Strokes that are curves count more than lines (a pair of curves crosses up
to nine times, a line and a curve three), so a hatch of curves is refused at fewer. A boolean operation of more
than a few thousand inputs, most of them empty, is `TooComplex` past `MASK_WORK`.

Nine more bounds refuse input a drawing can hold, none of it common: a polygon of more than 1,291 edges that cross nearly every other (a star of 1,293 points, joined to the one nearly opposite) by `MAX_CROSSED_WORK`, more than a million places where segments cross, as in 53 stars of 201 points stacked (or 20 of bowed cubics) by `MAX_REAL_CROSSINGS`, a grid of 300 and 300 open lines under 100 arches whose bounds hold it (an Outline: by the counts of crossings, in 18 ms; 200 arches, 41 ms), a pie of more than 256 slices (or any
more than 512 segment ends at one point; a fan of triangles has two at the point for each, so 257 of them) by `MAX_ENDS_AT_A_POINT`, four pies of 256 slices on a diagonal (the work of the ends summed over the points) by `MAX_ENDS_WORK`, 257 lines
through one point by `MAX_CROSSING_WORK`, shapes nested 2,896 deep, or 202 deep and crossed by 202 bars, by
`MAX_COVERAGE_ENTRIES`, Merge, Divide, Trim or Crop of a fan of 209 or more shapes at a point beside 40,000
other squares by `MAX_PASS_WORK`, and the planar map of thousands of contours nested in one shape (3,400
rectangles a thousand times as tall as they are wide) by `MAX_FILL_WORK`. Each is a worst case measured in
seconds or in half a gigabyte (the pie of 256 takes 0.5 to 0.9 s, 256 lines through a point 1.1 s, 2,895 nested
squares 0.2 s and 550 MB, 3,300 nested rectangles in one shape 0.75 s, the star of 1,291 4.3 s, 52 stars of 201 points 3.2 s
and 1.6 GB, 19 of bowed cubics 2.6 s, the Outline of 100 and 100 lines
under 100 arches 1.4 s), so an app that can wait or has the memory raises it in `Limits`.

The metered sweep (`MAX_SWEEP_WORK`) is a worst case as well, a fit to the time of the sweep and to the memory of its pieces, and
refuses some input that would finish in a few seconds: pencils of more than about 240 lines through one point (256, which took 0.7 s),
arc fans of 200 pieces (2.2 s) and 200 windows of an arc that share a tenth of it (1.8 s), 180 small squares a unit in the last place
apart (3.0 s; 150 are made in 1.3 s), 51 stacked stars of 201 points (52 took 3.2 s), 2,100 pies of 30 slices and 4,800 of 20 (their cache of comparisons: 3.6 million pairs that start or end together; they took 4.0 s and 2.4 GB, and 4.0 s and 2.5 GB, with the meter off), and pencils and pies whose apexes were moved a
few tolerances, past 2.3 million pieces (200 lines moved by 10 tolerances took 0.6 s and 0.8 GB, a pie of 256 slices moved by 10, 1.4 s
and 1.1 GB). A planar map (`shape_builder`, `live_paint`, Divide with an open path, an Outline) is built for a quarter of the pieces a
sweep is: not for a star of 711 points (the shape builder of 801 aborted under a limit of 3 GB) or a hatch of 300 strokes each way
(9 s and 1.3 GB of map before the fill limit refused it). `Limits::sweep_work` is the knob, and `usize::MAX` leaves the meter off.

The element limit refuses a drawing of more than 524,288 elements in one operation (a traced image of half a
million points), and `shape_builder` gives only the filled areas for more than 65,536 segments.

### Known limits

The limits count before the sweep and stand for a time or a size; no proof ties the sweep's time to the counts, and
each was set from the worst input found for it (by measuring and by review). Under them the metered dry sweep counts what
the sweep itself does and refuses past its budget, which bounds what the counts miss, but its weights are a fit too
(see its section for what it bounds and what it does not). What is known to pass them and still cost more than a second
or so, measured once in a release build:

- 100,000 straight cubics round a circle of radius 1,000 at (1e12, 1e12) whose handles lie 1 or 2 units in the last place from
  their anchors are `TooComplex` (50,000 are made; 1d3e413 made them in 0.43 s): a handle shorter than twice the rounding of
  its point gives the cone of its joint every direction, so each of the 100,000 joints counts as crossing (18 each). Handles
  that short are not drawn (handles of a drawn cubic are a thousandth of its chord or more).
- Where two curves run along one another is looked for by the counts before the sweep among up to 24 of the curves whose tangents
  may run along each one's own, over a stretch of a quarter of the one (nine samples), so a stretch shorter than a quarter of
  each (windows of one arc that share a tenth of it) is not seen by them, and a clique of curves that run along one another in
  ones and twos is counted by its tried partners; the clusters of ends and of crossings they price are 64 tolerances wide, and
  crossings count more within a thousand. The metered sweep does not look: windows of one arc that share a tenth of it are
  refused by it alone (300 of them, in 1.5 s), and ends and crossings moved 100 tolerances apart are the separate points they
  nearly are to it as well (a pie moved 100 tolerances took 0.2 s, 300 lines moved 200 tolerances 0.2 s).
- Stacks of copies are priced by the cube of their ends, fitted to squares, triangles and segments one on another; polygons of
  many sides cost less for each corner (100 copies of a polygon of 20 sides are priced 3.0e9 and took 0.9 s), so a stack of those
  is refused at fewer than it could be. The ends of short segments (under 256 tolerances across) that were copies a unit in the
  last place, or a fraction of a tolerance, apart were priced as a pie's, a third of what copies cost (240 triangles 2e-4 across,
  each an ulp from the last, took 5.3 s beside the star of 1,291 points at 4.6, 230 squares moved by tenths of a tolerance 5.7 s
  beside 4.5): the metered sweep counts them as it counts copies one on another, and refuses both before it sweeps.
- The metered sweep cannot stop a step of the sweep. What a step did is counted when it is over: n lines through exactly one point
  cross in one step, and that step took 1.4 s of the 3 to 4 s the sweep of 400 of them took. What starts at one height is counted from
  the segments before the sweep (a stack of copies is a dozen steps and nine seconds), and the counts before the sweep price lines
  through one point exactly; curves through one point exactly are priced by neither before their step is made. With the counts before the sweep
  lifted (`real_crossings`, `crossed_work` and the rest), 600 closed loops of two cubics through one point exactly (1,200 cubics, `scurves`) are
  refused by the meter only after 33 s and 1.0 GB, and 800 (1,600 cubics) after 77 s and 1.9 GB (400 take 7.2 s and 0.5 GB, 300 3.8 s): the step in which they cross is
  made before it is counted. By default the counts of crossings refuse them, in 1 to 20 ms.
- The weights of the metered sweep are a fit to about 180 inputs of 30 kinds, on one machine, under a load of 5 to 25: near the
  budget the time of the operation is 0.4 to 1.4 of the work for the kinds measured (0.4 for an arc fan of 150 pieces, 1.4 for six
  stacks of 128 copies, which take 1.1 times the star, the most the meter admits of the kinds measured), the work is far over the
  time where the pieces are many and cheap (a pie of 256 slices whose apexes were moved by 10 tolerances took 1.4 s and 1.1 GB, which
  the 2.3 million pieces hold, not the time) and where the counts are tuned to refuse a stack of copies (pencils of 256 lines near
  one point, 0.7 s, are refused; 200 are made), and under the time for stacked pies of 4 and 5 slices of 256 (5.3 s, 1.2 times the
  star; the counts before the sweep refuse them). The work of pies of 30 slices is far over their time: 2,000 took 3.6 s of sweep (a time of 0.34e9)
  for a work of 4.1e9, 97% of the budget, which is the cache of comparisons they fill (2.4 GB).
  The false refusals it makes are in "Ordinary input the defaults refuse". The sweep's comparison cache is counted by the pairs of
  segments that start or end together, and not by the comparisons the sweep makes: a shape whose segments cross many others compares each with those it meets (a mesh of 100 cells
  made 635,000 entries for 60,000 pieces, a grid of 100 squares 240,000 for 40,000, 15,000 rings 490,000 for 120,000), which the weight of a piece covers and the counts of crossings bound
  (a mesh of 180 cells, the most the defaults admit, makes 2.1 million entries in 1.3 GB; 190 is refused), but no input is known that fills the second cache's table, at about 1.8 million entries, with fewer than the 3.56 million
  pairs that start or end together that the budget admits. The cliff is that table's step, which holds about 0.49 of the pairs of a pie: the pies that aborted had 3.76 million pairs
  (2,200 of 30 slices) and 3.85 million (5,200 of 20), those that ran with the meter off 3.68 and 3.71 million, and the edge leaves 4% for pies. Pies of 12 to 28 slices at and past their
  edges, bowed pies, jittered pies, wide shallow lines, slivers and lenses were measured too, and none aborts.
- What follows the sweep is not metered but for the planar map. The operations on the arrangement take a multiple of the sweep that
  is linear in its pieces (Divide and Merge of the star of 1,291 points 14.7 and 13.2 s, three times the 4.5 s of its area; the
  contour pass after the sweep takes up to 2.6 times the sweep for stacks of copies: six stacks of 128, 4.9 s for 1.9 s). A planar map
  (`live_paint`, `shape_builder`, Divide with an open path, an Outline's) takes 6 to 16 microseconds and 1 to 4.4 kilobytes for each
  piece, 8 to 15 times the sweep (the shape builder of a star of 501 points took 4.7 s and 1.3 GB, of 801 aborted under a limit of 3 GB,
  of a hatch of 100, 200 and 300 strokes 1.3, 3.9 and 9.4 s), so a map is built for a quarter of the pieces a sweep is: 583,000: a star of 705 points
  takes 10 to 12 s and 2.6 GB to map, 0.4 GB under a limit of 3 GB, and 711 and more are refused. The searches of an Outline and the fills of a map are
  bounded by their own limits as they are made, and not from the counts of the dry sweep: a bound from the pieces and the junctions
  (every curve times every junction) would refuse drawings of thousands of curves that take milliseconds.
- Curves that pass through one point with no end there are not counted, only lines are (and the event limit
  prices a pair of curves at nine crossings, a worst case, so it refuses a stack of curves before it is
  slow). Lenses whose two cubics each pass through one point (the cubics, not lines) take 0.17 s for 100, 0.7 s for
  200, 3.3 s for 400 and 4.1 s for 450; 500 are refused by the event limit. The cost grows with about the
  square of their number there, not the cube. Lines that pass through that point are counted, as if the curves
  were not there.
- The planar map, and what is made of it (the Outline of open paths, `shape_builder`'s lines and faces, `live_paint`'s edges and faces), can lose a crossing, or cut
  a path where nothing crosses it, where curves cross at a shallow angle or one of them runs nearly level. The map traces an open path out and back,
  so four curves meet at a crossing, and the sweep, which finds the height of a cut to a tolerance and so its place along a level curve to that tolerance
  over the slope, cuts them at points up to a unit apart (0.17 and 0.78 were seen) and joins the cuts with pieces that run level; copies of one path part
  by about 5e-3 at the top of a flat arc. The map reads the input of each piece from the outlines that lie within 16 tolerances of its middle, and drops
  a piece that has none, joins the ends of a stub only if they lie within a sixteenth of the default precision, and merges two edges between the same
  points only within the precision, all of which those cuts outrun: the crossing is no vertex, or a vertex is where nothing meets. The sweep records which inputs a
  piece is made of (the difference of the crossings on its two sides, in `linesweeper`'s public interface), which the map does not use. Two open cubics that
  cross twice (`M50.0175,87.4777 C91.0932,74.4057 61.7557,89.6998 73.7868,37.2415` and
  `M40.8294,97.1229 C86.9075,78.6158 58.564,64.895 99.0986,87.5071`) came back from the Outline as 1 piece each and not 3, at a precision of 1e-5 and of 0.01, and
  closed into shapes `live_paint` gave no face; turned by 30 or 90 degrees they are right. These are measured against the crossings found by flattening the
  paths to polylines (200 steps a cubic), crossing every pair of segments and cutting the plane into slabs, which uses no sweep (a count of points on a grid agrees with it where it was
  checked), on three runs of 400 stacks of two to four paths in a drawing 100 across, a stack counting as wrong where a count of cuts or of faces, or an area, differs by more than
  the flattening and the precision explain. The counts are the same when the drawing is made 10 or 100 times as large (a tenth: the Outline of open smooth paths is wrong in 18 of 300 and
  of closed shapes in 44 of 400, where at 1 it is 2 of 300 and 1 of 400):
  - open paths of two or three random cubics each (1,200 stacks, mostly paths that cross themselves): the Outline lacks cuts in 349 (4,516 cuts) and has 72 more than places (half of them
    where two crossings lie closer than 5e-3, so right); `shape_builder`'s lines and `live_paint`'s edges are wrong in 924 (3,061 cuts lost and 2,221 more than places); their faces in 512
    (3,232 faces lost and 672 extra, an area off by up to 1,907);
  - open paths of three or four anchors, as a drawing has them (1,200 stacks): the Outline is wrong in 11 (28 cuts lost, 8 more, all of the 8 where crossings lie closer than 5e-3),
    the lines and edges in 372 (17 lost, 887 more), the faces in 6 (10 lost);
  - closed shapes that do not cross themselves (four to six anchors round a centre, 1,200 stacks): Divide, Trim, Merge, the four booleans and `shape_builder`'s faces are right in all;
    `live_paint`'s faces are wrong in 4 (11 faces lost and 4 extra; in one, two faces of 119 and 3,156 square units came back as one of 3,274), and the Outline of closed shapes in 3 (2 cuts lost, 1 more);
  - closed shapes that cross themselves (random closed cubics of two or three each, mixed fill rules, 1,200 stacks; 2,400 results of Merge, 3,589 of Trim): a wrong area, by up to 463 square units,
    in 5.8 to 8.7% of the pairs for the four booleans, in 7.2% of Divide, 10% of Merge and 10.8% of Trim, whatever the size or the turn of the drawing; the cause is not known, and is not this one.

  VectorCraft's `shape_builder` and `live_paint` are this code and give the same results on the same stacks (open: 924 and 372 wrong lines and edges, 512 and 6 faces; closed shapes that do
  not cross themselves: 4 faces and 3 Outlines), and its closed self-crossing results are worse (128 to 164 of 1,200 booleans, 121 of Divide, 311 of 2,400 Merge, 458 of 3,589 Trim). Its Outline of open
  paths cuts them at the vertices of the filled areas (their implicit closing lines included) and not at their crossings, so it is not compared. None of this is changed here: the map's reading of the sweep and the Outline's cuts at its
  vertices, and the self-crossing results, are left for a later change. Lines that cross lines, and a line across an arc or a circle, are cut at every crossing (`tests/it/outline_cuts.rs`).
  The vertices in the middle of a path that the Outline does not cut at (the next limit) are still cuts of the shape builder's lines and live paint's edges.
- The Outline of open paths keeps a junction of the planar map only if it lies within `reach` (a ten-millionth of the drawing's size, at least 1e-6) of two paths or of two places on one, and the map's
  vertex can lie on one pass and 2e-5 to 9e-5 from the other, so a real cut can be lost; it never adds one. Against the crossings found by flattening, 4,000 random single open cubics (492 of them loop):
  488 of the 492 loops are cut as they cross (4 come back as 1 piece, not 3: one crosses itself at 3 degrees, 2.3e-5 from the double point), the 3,474 without a loop all are (before this rule it made 483
  spurious cuts, and 71 of the loops were not cut as they cross); in random stacks of open cubics 12 of about 49,000 crossing points are lost, in 6,268 of lines none; no case of 3,000 has more pieces than
  before the rule; arcs crossing at 0.1 to 30 degrees (60 pairs) and lines (56) were all cut. The Outline of closed shapes does not ask for it (its junction points lie too far from one pass:
  asking would change 317 of 1,500 random closed rings, a ring cut in 3 coming back in 2), and a test pins a closed ring of three cubics that crosses itself, cut in three.
- Near the edge of the cache term (above): a refusal by the dry sweep holds up to about 2.7 GB, 0.3 GB under a limit of 3 GB, and bowed pies at the edge run at 1.7 to 1.9 times the 1,291-point star
  beside them (1,800 pies of 30 slices of two cubic sides took 15.3 s against 8.1 s, of three cubic sides 19.1 s against 11.0 s), in 2.55 to 2.68 GB.
- A Pathfinder Outline refuses after the planar map is built: `MAX_SEARCH_WORK` counts the searches for the junctions
  on each edge once the map exists, so its refusal comes after the time the map took. The grid of 300 and 300 open lines under arches that was
  measured for it (the bounds of every arch hold all 90,000 junctions, with the curve nowhere near) does not reach it by default: the counts of
  crossings (`MAX_CROSSED_WORK` and `MAX_REAL_CROSSINGS`, each alone) refuse it first, with 100 arches in 18 ms and with 200 in 41 ms, and 100 and 100
  lines under 100 arches are made, in 1.4 s. With those two lifted, the meter makes the Outline of 300 and 300 lines under 100 arches (7.7 s, a work under the
  quarter of its budget that a map is held to) and refuses 200 arches in 0.12 s, before the map (the meter alone refuses them in 3 ms with every other limit lifted); the search limit alone refuses 200 after 9.6 s. No input that
  the other limits admit is known to reach `MAX_SEARCH_WORK` by default.
- In a build with debug assertions `linesweeper` checks the order it gives two curves against their positions where it changes (the two pairs of `debug_assert!`s of `intersect_cubics_rec` in `curve/mod.rs`, lines 1301 to 1302 and 1305 to 1306 of linesweeper 0.5.0), and the check fails for some curves whose outline is far from them. The outline stroke of an open run of two arcs (each of chord 300 and bow 100), taken
  1e7 on either side (a width of 2e7), fails it for 1 of 700 such runs laid out on a grid (the one at (3,000, 4,000)), and the same strokes at 3e7, 1e8, 1e9 and 1e10 on
  either side fail it for 13, 259, 327 and 275 of the 700. The crate catches the panic, so a debug build (`cargo test` too) answers `Degenerate` for them. A release build makes no
  check and goes on, and made the same outline for every one of the 700 runs at every distance from 1e6 to 1e10: no panic, 11 ms a run at the most, areas within 1e-9 of one
  another up to 1e8 on either side, 4e-9 at 1e9 and 3e-6 at 1e10 (the rounding of an area that large). So the answer of a release build is the right one where it can be checked, and a
  debug build refuses what it would have made; a test pins both (`tests/it/far_out.rs`). No cheap check before or after the sweep sees the condition (it is the sweep's own comparison
  of two curves), so a release build is not guarded.
- The estimate of the entries of the windings counts one crossing for each pair of curves when an eighth of the
  one lies within the bounds of an eighth of the other, where two cubics may cross up to nine times, so curves
  that cross each other several times can hold more than the limit's half a gigabyte. It was at least the
  entries held in every arrangement of lines and of circles that the tests built, and is not a bound for
  curves that cross several times each.
- The event limit and the limit on pairs of pieces at overlapping heights are worst cases priced in
  nanoseconds, and the slowest rows of the Costs table are what they admit: 3.1 s for the largest hatch,
  2.8 s for the largest row of dots, 4.3 s for the largest star, 5.4 s for an outline stroke of cubics a thousand
  wide, 1.8 s and a gigabyte for two polygons at the element limit.
- `interior_point` drops the edges that run back over each other exactly (an edge and the same edge the other
  way, as an out-and-back path has) before it looks at the few heights it tries, and takes those heights from
  the vertices where the path turns, not those it passes straight through, so a stub out and back along a
  line, through the same vertices or others, costs it nothing. A retrace along a curve flattened at other
  points each way (a vertex at each turn of the flattening, and no edge the same) is not dropped, and its
  vertices use up the heights: a unit square beside a long curved stub of that kind can be missed.
- A planar map is built before the fills of its faces are tested, and the fill limit stops only the tests:
  rectangles nested in one shape, each a thousand times as tall as it is wide (so that the bounds of their sides
  overlap only at the corners, and the limits counted before the sweep admit over 8,000 of them), are refused
  by `MAX_FILL_WORK` past 3,300, in 0.95 s at 4,000 and 1.9 s at 8,000 (answering 8,000 took 9.2 s before the limit).
- A line of segments each shorter than 16 tolerances of the sweep (1.6e-5 at ordinary coordinates, 0.023 beside
  a shape at 1e11) is lost from a planar map in any frame: the map takes pieces whose ends lie within that of
  each other for copies of one edge (which the sweep cuts a little apart) and joins their ends. Its pieces are
  under the noise the sweep can tell from a point; a line of longer segments is kept.
- `simplify` with `straight_lines` samples a curve in as many intervals as keep the chords of its samples
  within a tenth of the tolerance of it, up to 65,536 for one segment (a quarter circle of radius 1e9 at a
  tolerance of 1), and leaves a run that needs more as it is; the polyline reduction then holds the samples to
  the tolerance less how far the curve can be from them between two (the bound the intervals are chosen by),
  so a curve that peaks between two samples is within the tolerance too. A refit curve is held by the same
  kind of bound, which is larger than the distances it bounds: a fit that comes close to the tolerance at the
  samples is split further, or the run left as it is, where it may in fact have been within it.
- `remove_redundant_points` does not take a run of cubics whole where the rounding of lengths is wider than the
  tolerance. A run is merged when it is shown to be within the tolerance of the cubic that replaces it, and the proof
  takes the length of each stretch of the run between two of its sample points as long as rounding may have made it (4
  units in the last place, `LENGTH_ROUNDING`), so that the lens it lies in is never thinner than 3e-8 of the stretch
  (a path as long as its chord to the last place may still be off it by that much). The samples are as many as the
  curvature of the cubic asks for, and not fewer than keep that lens within a quarter of the tolerance (up to 1,024),
  so the longest run merged is about 1,024 stretches of tolerance / 3e-8 each: 3e8 long at a tolerance of 0.01, in
  proportion to the tolerance. 100,000 cubics 1e4 long along a parabola of radius 1e16 (each off its chord by 1e-9, the
  whole a line within 1e-6 over a length of 1e9) come out as 5 elements, in 14 ms, not 2 (266 in 0.1 s before the
  samples were chosen by the lens as well: eight stretches of 4.7e5 had a lens of 0.014), and as 8, 3, 2, 2, 2 and 2 at
  tolerances of 0.005, 0.02, 0.05, 0.1, 0.3 and 1; 100,000 straight cubics 1e4 long at 0.01 come out as 4 (1c1f05c gave 2,
  0bdd962 55); radii of 1e11, 1e12, 1e13, 1e14 and 1e15 give 21, 8, 5, 5 and 5 (161 at 1e14 before, where the work limit
  ended the pass, and 362 at 1e15). 61d528a gave 132 at 1e16 and tolerance 0.01 with an allowance of 8 units for each of
  the three lengths of a segment and each segment of a stretch, and counts that did not follow the tolerance (160, 132,
  97, 104, 119, 171 and 89 at 0.005 to 1).
- An inset widens a slit along its sides however narrow, but not at its closed end where that end is as short
  as the stroker's tolerance (1e-3): a slit 0.001 wide from 20 to 100 in a square 100 across, inset by 5, is a notch
  75 long (to y = 20, not 15), as a slit 0.002 wide is not (80). Measured, and not looked into further. Beside a shape at 1e11 a slit under the sweep's tolerance there (1.4e-3) is lost by the sweep, before the
  inset sees it.
- The counts themselves are made before the sweep and cost time: up to 0.3 to 0.5 s on an input of 250,000
  segments (they are a few sweeps of the bounds), with 64,000 dots as separate inputs the most.

An operation within the limits that takes longer than the seconds above, or more than a gigabyte, is not
known; the inputs above were found by testing and by independent review, and review has found more after each
round, so there may be others.

## Costs

Within the limits an operation can still take seconds and about a gigabyte, so run the heavy ones off a UI
thread and ask first with the `try_` forms. Measured on one machine, in a release build, **before the metered sweep**: for an input of more
than 192 monotone pieces the meter now runs a dry sweep first, so that with it on (the default) an operation takes 1.1 to 2.2 times the time
below (1.2 to 2.0 over 15 of these rows, re-measured with it on and off back to back: a circle of 100,000 cubics 0.45 and 0.58 s, the star of 1,291
points 5.9 and 10.7 s, 50 stars 3.6 and 6.7 s, a hatch of 400 3.8 and 7.2 s, 2,000 pies of 30 slices 5.3 and 9.8 s, a mesh of 170 cells 3.0 and 5.2 s),
in the same memory but for 4% more at the most, and an input it refuses is refused in the time of the dry sweep (up to 4 s, for the budget is the
most of sweep it makes). The star of 1,291 points, the measure of the budget, took 4.3 s in this table; with the meter on and off back to back, under
a load of 15, it took 10.7 and 5.9 s.

| Operation | Time |
|---|---|
| `try_boolean` of two polygons of 262,000 sides each (the element limit; peak memory 1.0 GB) | 1.8 s |
| `unite_all` of a column of 16,000 / 32,000 / 64,000 dots (64,000 / 128,000 / 256,000 elements) | 0.5 / 1.2 / 2.1 s |
| `unite_all` of a row of 5,500 dots on one line (the sweep-line limit is at 5,800) | 2.8 s |
| `unite_all` of a grid of 16,000 / 32,000 lenses with one more 10 million away | 0.6 / 1.4 s (as without the far one) |
| `try_area` of a pie of 256 slices (512 segment ends at its centre, the limit) | 0.5 to 0.9 s |
| `try_area` of 3 / 4 pies of 256 slices on a diagonal, so that their heights do not overlap (the work of the ends is 3.9e9 / 5.2e9 against the limit of 4e9) | 3.3 to 3.7 s, 270 MB / refused in 1 ms (8 and 30 pies, 15.7 s and 65.8 s before the limit, in 2 and 8 ms) |
| `try_area` of 450 lines through one point at angles under 1e-10 (2e7 to 1e10 long, their ends 1.6 to 10 tolerances apart; counted as no crossing at all before) / of 2,040 nearly parallel lines that cross along their length (2.08 million crossings) | refused in 6 ms (31 to 41 s before the limit) / in 50 ms (14 to 21 s before) |
| `try_area` of a bundle of 450 / 2,040 nearly parallel lines that cross nowhere | 2 / 10 ms, as before |
| `try_area` of a circle of 100,000 cubics, radius 1,000, at the origin / at (1e12, 1e12) | 0.5 s / 0.56 s |
| `try_area` of 256 straight cubics from one point down a diagonal (each closed by a line back: 512 segments on one line) / of 256 pieces of one arc from its start / of 256 windows of one arc that share no end | refused in 2 / 17 / 7 ms (they took 7.8 / 7.2 / 7.3 s, beside the star of 1,291 points at 5.3 / 5.7 / 5.9) |
| `try_area` of 100 / 128 straight cubics down a diagonal, 150 pieces of an arc from its start, 160 windows of an arc | 0.27 / 0.83 / 0.78 / 0.94 s |
| `try_area` of 300 lines through a point, each moved sideways by up to 10 / 40 tolerances | refused in 3 ms (2.6 GB and aborting under a limit of 3 GB / 0.6 s and 0.4 GB) |
| `try_area` of 215 such lines moved 2 tolerances / of 300 moved 200 tolerances / of 256 exactly through a point | refused by the metered sweep in 0.2 s (1.7 s and 1.6 GB without it) / 0.24 s and 62 MB / refused by it in 0.5 s (0.8 s and 58 MB without it; 200 are made) |
| `try_area` of a pie of 256 slices, each apex moved by up to 10 tolerances / of 2, and of 4 such pies, and of one of 400 slices | refused by the metered sweep in 0.2 s (1.4 s and 1.1 GB without it, an exact pie 0.7 s and 86 MB) / refused in 1 to 3 ms (4 took 4.5 GB, the pie of 400 slices 4.9 GB) |
| `try_area` of 150 / 185 squares one on another (4 corners of 300 / 370 ends), 200 triangles, 200 copies of a segment, 6 stacks of 128 copies of a segment | 1.0 s / 2.9 s (the star 4.5 beside it), 1.5 s, 2.0 s, 1.9 s |
| `try_area` of 190 / 230 squares one on another, 255 triangles, 255 copies of a segment, 21 stacks of 128, 4 stacks of 200 | refused in 0.3 to 10 ms (230 squares took 9.6 s beside the star's 6.4, 255 triangles 10 s beside 7, 21 stacks of 128 21 s beside 5.9, 4 stacks of 200 22 s beside 8.1) |
| `try_area` of a star of 1,001 / 1,291 points, each joined to the one nearly opposite (0.5 / 0.83 million crossings; 1,291 is the limit on them) | 2.4 s, 0.9 GB / 4.3 s, 0.9 GB |
| `try_area` of the star of 1,293 / 2,001 / 3,001 points | refused in 6 / 14 / 2 ms |
| `try_area` of 50 stars of 201 points, stacked so that their heights do not overlap (the most the metered sweep admits: 2.3 million pieces; 52, the limit on their crossings, took 3.2 s and 1.6 GB without it) / of 19 stars of bowed cubics (the most the bound admits) | 3.1 s, 1.6 GB (5.6 s with it) / 2.6 s, 0.5 GB |
| `try_area` of 53 / 206 such stars of lines, of 20 / 100 of bowed cubics | refused in 0.02 / 0.08 s, in 0.02 / 0.09 s |
| `pathfinder(Outline, ..)` of 100 and 100 open lines across one another (one shape; 10,000 junctions) / of 300 and 300 (90,000) | 0.19 s / 2.8 s, 430 MB |
| the same 100 lines with 100 arches round them whose bounds hold every junction (a search apiece: 1 million) / 300 with 100 and with 200 arches | 1.4 s / refused in 18 and 41 ms by the counts of crossings (the 90,000 junctions are 9 million searches) |
| `try_area` of 256 thin triangles whose long edges cross at one point (the limit on lines crossing at one point) | 1.1 s |
| `try_area` of 400 / 450 lenses whose two cubics each pass through one point (curves through a point are not counted: Known limits) | 3.3 / 4.1 s |
| `pathfinder(Exclude, ..)` of 200 squares nested and crossed by 200 bars (the estimate of the entries is at the limit) | 0.2 s, 430 MB |
| `try_boolean_n` of 2,000 squares in a column, each an input, with one path of 2,000 tall rectangles to their west | 0.25 s (with the rectangles to the east, 0.08 s) |
| `pathfinder(Exclude / Divide, ..)` of 2,000 / 2,895 squares nested (16 / 33 million coverage entries; 2,895 is the limit) | 0.11 / 0.25 s and 270 / 300 MB for 2,000; 0.2 s and 550 MB for 2,895 |
| `pathfinder(Merge / Divide, ..)` of a fan of 208 shapes at a point beside 40,000 squares (the pass limit) | 2.6 / 3.5 s |
| `try_area` of a rectangle with 20,000 collinear vertices on its top and bottom (40,000 pieces of no height) | 24 ms |
| `try_outline_stroke` of 3,000 wavy cubics 3 wide (the default limit is at about 3,100) | 1.0 s |
| `try_outline_stroke` of 1,000 cubics 1,000 wide (22,000 elements of outline, much of it crossing itself) | 5.4 s |
| `try_outline_stroke` of 200,000 cubics 100 million wide | refused in 0.3 s |
| `simplify` of 100,000 segments | 1.7 s (the `FIT_WORK` limit) |
| `join` of 4,096 lines to a chain of 500,000 segments | 0.36 s |
| `interior_point` of a comb of 40,000 teeth (160,000 elements) | 11 ms |
| `live_paint` of a comb of 20,000 teeth | 1.2 s |
| `live_paint` of 4,000 / 8,000 / 16,000 squares in one shape | 0.18 / 0.51 / 1.6 s |
| `live_paint` of 1,000 / 2,000 / 3,300 rectangles nested in one shape, each 1,000 times as tall as it is wide | 0.14 / 0.36 / 0.75 s (4,000 and 8,000: no map, in 0.95 / 1.9 s) |

## Left in VectorCraft

The anchor-and-handle path model (`PathData`), and what is built on it: the add-ons (Smart Pen, artwork
cleanup, anchor-reduction that keeps a shape, path tools), the Smooth and Simplify slider scales,
`average`, `remove_anchor`, `add_anchor_points`, `split_into_grid`, trapping, the CMYK mixing pathfinder
effects (Hard Mix and Soft Mix), Gap Detection, `cut_out` and `run_around`, and `FaceMerger`, which merges
faces of a planar map for VectorCraft's Shape Builder tool. They need VectorCraft's model or its tools.

## Differences from VectorCraft

The algorithms and tolerances are VectorCraft's. The carve was compared with `vectorcraft-pathops` on
random shapes through every operation (the pinned Pathfinder results in `tests/it/pathfinder_pinned.rs` are
bit for bit VectorCraft's), and the results are identical except where noted:

- a subpath that is open but ends where it starts is no longer given a closing segment of no length; the
  Pathfinder Outline of one has no pieces of no length, and `join` closes a path whose ends are within the
  tolerance with a straight segment (VectorCraft left a curve bent by the old end);
- a `precision` that is not a length (NaN, infinite, negative or zero) is `DEFAULT_PRECISION` in every
  operation;
- `interior_point` takes the winding of each gap between crossings from the signed crossings it already has
  (one pass) and tests the widest few against the path itself, where VectorCraft tested every gap that was
  wider than the best so far; the point is the same except for gaps thinner than the flattening tolerance;
- a subpath of more than 256 segments is stroked in overlapping windows (as above). `try_outline_stroke`
  sweeps their outlines and refits the union once. Against VectorCraft's one stroke, on wavy open paths and
  rings of 300 to 700 cubics, it covers the same area to about a millionth (1.1e-6 at most); where the
  stroker's inner joins loop, the outline lies up to 0.012 from VectorCraft's (the refit precision,
  `DEFAULT_PRECISION`, is 0.01), with up to 6 elements more on an open path and up to 58 fewer on a ring;
- an offset of a path with such a subpath builds its stroke from the same windows, each made clean and laid
  together, and leaves the union to the offset's own arrangement. Against VectorCraft's offsets of rings of
  300 to 700 cubics, the areas agree to 3e-8, the outline lies up to 0.004 from VectorCraft's (under 0.001
  on most) and has up to 20 elements fewer or 1 more;
- one face of a planar map, in one random stack in 1,800 (five shapes, one a self-crossing quadratic
  outline): `shape_builder` in VectorCraft lists it twice, once with its fill and once as a gap, and the
  carve lists it once, with its fill; `live_paint` in VectorCraft lists it once with its fill missing, and
  the carve lists it with its fill;
- found in review of the carve, each behaviour VectorCraft has too and now changed here:
  - every winding of a point against a path (the fills a planar map's faces get their sources from, the face each hole is
    in, the groups of a Merge's faces, `Region::contains`, and the checks `interior_point` and the offset's deep point
    make of their candidates) is made in `src/winding.rs`, not by `kurbo`'s `BezPath::winding`, which VectorCraft
    uses. kurbo 0.13.1 cuts a cubic where its height turns and finds where each piece crosses the height of the point
    with the cubic formula, which puts the root in the wrong place where the cubic term of the height is rounding
    noise beside the others: for an arch whose handles are level (the control points `(0, 0)`, `(100/3, h)`,
    `(200/3, h)`, `(100, 0)` at `h` = 10.05: the half arch's term is 1.8e-15, and the root 0.1732 comes out as 0.0578),
    and so for the arcs at the top and the bottom of a circle cut into an even number of arcs. About a twentieth to a
    tenth of the points over such an arch are told wrongly (596 of the 16,000 of the grid in `tests/it/winding.rs`
    at `h` = 10.05, 617 at 10.1, none at 10.0, 9.0, 7.0 and 3.3; 1,440 to 2,083 of other grids; 228 of 400,000 random
    points in kurbo's own circle of six cubics at a tolerance of 1e-3). A face then got the wrong sources: the lens
    between two arches 0.03 to 0.3 apart was listed under both shapes, under the lower alone, or under none (11 of
    11 inputs of the test, 78 of the 1,080 faces of a scan), and so was the ring between two circles cut into 6 and
    6, 10 and 6, 6 and 10, 12 or 14 arcs (12 of 12, 29 of 720); a hole was left out of the face it is in; two holes
    that touch were faces of their own in a Merge; and the point `interior_point` took for the middle of a crescent
    was outside it (5 of 7 crescents, and 125 of 504 arches and crescents of a scan). In 300 random stacks of two
    to seven shapes a seed (seeds 1, 3, 5 and 7), judged against the fills flattened to 1e-8 and the crossings
    counted on the polygon, VectorCraft's `live_paint` has the wrong sources for 350, 261, 294 and 293 of 7,556,
    7,625, 6,801 and 6,878 faces (`shape_builder`'s regions 310, 238, 289 and 264), and its Divide of a stack with an
    open path is not one shape for each face a fill covers in 36, 30, 49 and 37 stacks of about 250; the carve has none
    wrong. Here a cubic is cut where its height turns by a
    quadratic solved without subtracting nearly equal numbers, each piece is solved for the height by Newton's method
    from the straight line between its ends, held inside the range the heights seen leave, and then by bisection (at
    most 12 and 64 evaluations of the height for a piece, never the cubic formula), and the crossings are counted by
    kurbo's rules (a ray to the left, a piece's lower end in and its upper end out, a line tested as kurbo tests it),
    so nothing changes where kurbo was right (the pinned Pathfinder results are still identical) and a point farther
    from the path than the rounding of its coordinates is told right. On a release build a winding against a path of
    many segments takes 0.07 to 0.6 of kurbo's time (a circle of 8 arcs 0.18, of 1,000 arcs 0.07, a polygon of 1,000
    lines 0.09, a blob of 6 to 100 random cubics 0.57 to 0.61: the box of a segment turns most of them away, which
    kurbo does not), and against one cubic whose box holds the point more: an arch 1.25 to 1.5 times and a general cubic
    0.9 to 1.3 times kurbo's, and near a flat end of a piece (a cubic whose height is `t^3`) about 5 times, 480 ns
    against 97;
  - `boolean_n` and `try_boolean_n` check the sweep's windings against the area of each input, as
    `try_boolean` does (for up to eight inputs), and resolve the inputs first when they do not add up;
    VectorCraft did that for two operands only, and the n-ary result gained or lost a region for a
    self-crossing one. When resolving is over a limit, `try_boolean` is `TooComplex`, where it returned the
    result it knew to be wrong (VectorCraft had no limits to be over);
  - Divide and `regions` found the inputs that cover each side of each edge with a flag for every input
    (80,000 flags for 100 shapes apart, 1,280,000 for 400: the square of their number), and now list the
    covering inputs only, in the same order and with the same results; past `MASK_WORK` entries they give
    nothing, as an input too complex to sweep does;
  - `simplify_with` with `straight_lines` measures the distance of a point to the finite segment it is
    judged against, not to the line through it, so a path that runs out past an end and back is kept;
    `smooth` is documented to measure the tolerance from the smoothed path;
  - `remove_redundant_points` (and so the stroke's spine) checks a merged run against the segments it replaces,
    not only the joint it merges across: a half circle of ten thousand chords came out as its diameter at a
    tolerance of 0.001, and a circle of radius 1,000 in 1,000 cubics as 13 elements, 20 times the tolerance
    (0.01) from the circle, and in 100,000 as 4 elements, 9,970 times; the carve keeps them. Noisy points about
    a straight line within the tolerance still merge. Two cubics are merged on samples at 16 intervals of each,
    as VectorCraft does, and the cubic is taken when its error at them, how far it can be from its chords
    between them, and how far the two can be from theirs, added to what the two had used already, are within the
    tolerance (a pair 1000 across came out 0.5005 from VectorCraft's merge at a tolerance of 0.5, and is not
    merged here). Where they are not, the run is refitted from points of its own segments: at 8 to 1,024
    intervals of it, at its joints, or where it has fewer segments than that inside them, each cut in equal
    steps (up to 2,047 points). The cubic is held to the tolerance by its error at them, its distance from its
    chords, and the run's from the chords between the points (a stretch of the run no longer than its control
    polygons lies in a lens about its chord, whose width is small where the run is smooth; the lengths are taken
    as long as rounding may have made them, 4 units of rounding of each (a segment's three edges, each a rounded
    distance, add up to three at most, and the lengths of a run are added with the rounding compensated, which
    adds half a unit of the whole however many there are), so that a shallow curve with long control edges,
    whose edges add up to its chord to the last place, is not taken for a line; 8 units for each edge added up,
    as it was, were more than the tolerance for a run of 200 thousand straight cubics in eight stretches, which
    came out as 3 elements in 0.41 s and are 2 in 15 ms). That is a bound
    of the run alone, whatever the number of segments or of merges that made it (the bounds of a run's merges,
    added up, are far more than it has used: a finely cut curve would have none of the tolerance left after a
    few dozen). A piece of eight segments or more is extended by those after it in strides, doubled while the
    run still fits and then halved, never past a joint the merge of two cubics refuses (the tangents more than 8
    degrees apart), so a long smooth run costs a few refits, not one for each segment, and a corner stays. The
    points of a refit are charged to `REDUNDANT_WORK`, 24 each. A circle of radius 1,000 comes down to as few
    elements however finely it is cut: 27 elements at a tolerance of 0.01 for 100 cubics and 23 or 24 for 1,000
    and up, 7 at 0.1 and 6 at 1 for any number, within the tolerance of the circle and the circle of them (to
    0.98 of it), in 0.015 s for 100,000 cubics (e2351f5 gave 27 and 29, 15 and 17, 6 to 11, in 0.36 s). Against
    VectorCraft, on 900 random paths a seed (seeds 1, 3, 5 and 7), the carve gives the same number of elements
    in 894, 879, 887 and 894 runs, more in 6, 21, 13 and 6, and fewer in none (5,910, 5,816, 5,833 and 5,653
    elements in all, against 5,903, 5,789, 5,814 and 5,643); 141, 164, 160 and 168 runs differ from
    VectorCraft's in any way, those with the same number of elements in their control points only (the cubic
    comes from a refit, not from the samples). VectorCraft is further than 1.05 of the tolerance from the path
    in 17, 23, 18 and 23 of them, up to 1.8 times, the carve in none (0.99 at the most). Against e2351f5, 1, 7,
    4 and 1 runs have more elements and 8, 6, 11 and 5 fewer (5,919, 5,814, 5,841 and 5,656 elements in all, now
    5,910, 5,816, 5,833 and 5,653);
  - an inset (`offset_path` with a negative delta) strokes the boundary of the area the closed subpaths
    fill, not every subpath, so a rectangle inside another one wound the same way leaves no moat; and an
    open subpath is outlined by its stroke for a negative delta too, as the documentation says (it gave
    nothing);
  - Pathfinder Outline gives an open path as its own edges, with no closing edge (but for a ring, which comes back to where it started: that is closed), and cuts at the points
    where an open path crosses another (the filled areas' arrangement, where the way there and back
    cancel, saw no crossing), and not in the middle of a path that nothing meets: a vertex of the planar map that lies on one path at one place is not a cut of the Outline
    (one open arc, 1e5 across, crossed once by a line was 5 to 7 pieces and is 4; a circle of radius 100 with no close, crossed twice by a line,
    6 and is 5). VectorCraft cuts no open path where it crosses another, so it has neither the crossings nor those cuts; Merge makes a shape of each connected component, so an island inside a
    hole of the same key is a shape of its own;
  - an open path that slopes -1/32 (level in the sweep's frame, where its two passes cancelled) is swept in
    a frame sheared by another power of two, and keeps its loose end;
  - `interior_point` looks at up to 64 more heights, between the heights of the path's vertices, when the
    first nine find nothing, so it finds a part far from the others (two squares a hundred apart);
  - the `shape_builder` fallback (more than `SHAPE_BUILDER_MAX_SEGMENTS`) fills the closed subpaths only,
    as the map does: an open triangle that cuts fills nothing;
  - the tidy drops a piece as a speck when its box (width plus height) is under a billionth of the longest
    chord (at least 1e-9), as VectorCraft did, but never one over 1e-6, or over 64 ulps of the largest
    coordinate when that is more (past coordinates of 70 million): what the sweep's own tolerance could not
    have made is no speck. For a drawing whose longest chord is under 1,000 that is VectorCraft's threshold;
    past it the carve keeps the pieces between that and a billionth of the chord, so an edge as short as the
    sweep's tolerance allows is kept beside an edge a billion long (a notch of a unit in an edge of a billion
    is a notch);
  - an input curve is matched to a piece within the sweep's noise at coordinates above a thousand (it was a
    millionth of the coordinates: a whole unit at a million, so one arch's pieces were rebuilt from the arch
    beside it);
  - `simplify` leaves a run it cannot measure, or fit within the tolerance in 40 splits, as it is (VectorCraft
    sampled the first as a point, and took the second's last cubic as a fit);
  - a closed subpath of one cubic that loops round an area is filled (VectorCraft dropped every closed
    subpath of fewer than two anchors), and so is an open one of one cubic that ends where it starts, as an
    open ring of more anchors is: an offset and a Pathfinder Outline treat both as closed, and the planar map
    and its fallback past `SHAPE_BUILDER_MAX_SEGMENTS` count the open loop of one cubic as a fill (Divide of it
    and a line across gives its two faces with its key, where it gave none, and `shape_builder` gives its
    pieces as edges of an outline, not as lines; a ring of more anchors left open is a cutting edge there, as
    in VectorCraft). It is a loop when the handles of the cubic are not along one line, told by their cross
    product against the rounding of its two terms (a loop a trillion long and half a unit wide is one, of 75
    billion square units);
  - `join` of one open cubic that comes back within the tolerance of its start and loops round an area merges
    its ends into one anchor, with no closing line (VectorCraft merged them only for three anchors or more);
  - Pathfinder Outline cuts a cubic that crosses itself at both of its visits to the point (`nearest` gave one,
    so an open curl came back in two pieces, not three), and two cuts at one point with the curve's loop
    between them are two; and it finds the middle of a chain to a relative accuracy, where an accuracy of a
    millionth asked kurbo's solver for 2^64 steps or more on a curve longer than about 9e12 (a panic where
    arithmetic is checked, as in a debug build, and so no outline);
  - Pathfinder Outline of shapes with an open path takes its crossings from the planar map alone, and makes no
    filled arrangement: open paths whose implicit closing lines overlapped each other past the limit on pairs
    were refused by it and are outlined from the map; and where the map cannot be built there are no shapes,
    where the filled areas' junctions left every open path uncut where it meets another;
  - the planar map is swept in a frame in which no open path is level after the paths are snapped to the sweep's
    tolerance (it was told before snapping, so a line that a distant shape's tolerance levelled was lost),
    from ten shears and, when each of them levels a side longer than 8 tolerances, one more between the slopes
    of those sides, where snapping levels none of them (it levels a side between two anchors only where the
    side rises by under one tolerance, and a side to a handle where it rises by under three), from 1/1024 to
    1/2, and failing that from -1/2 to -1/1024; no map is built when no shear is free in either (it used the
    first, with one path lost), and `try_shape_builder` and `try_pathfinder` say so (`Degenerate`). Eleven
    lines 9e-6 long, one level in each of the ten shears and one in that of 3/8, bar every shear from 1/1024
    to 1/2 and had no map, and so no Outline of a line 100 long beside them (the shears searched are those in which
    the art stays within the coordinates the sweep takes: near the largest, the middle of the widest stretch
    the sides leave free could be past them, with a narrower one in range, and was no frame); they are mapped in a shear below
    0. Mirrored, sloping up as much as down, they bar both. Ten lines 9e-6 long, one level in each of
    the ten, have a map in the shear of 3/8 (the eleventh was looked for where they rise by four tolerances
    or more, which left none), though lines that short are lost from it, being under the 16 tolerances
    within which the map joins the ends of pieces (Known limits); ten lines 5e-5 long are kept whole. A side
    of 2 to 8 tolerances does not rule a shear out but is kept where a shear keeps it (an edge 0.009 long
    beside a shape at 1e11 was lost, and is kept in the shear of 1/4), and one of 2 or less is not looked at,
    so a tiny feature loses only itself. A shear in which the paths pass the largest coordinates the sweep
    takes is passed over (a shape near 7e13 had no map);
  - an inset takes the boundary of the area the closed subpaths fill when a piece of them the sweep cuts out has
    the fill on both sides or on neither (a small loop of a subpath inside its own fill, which the lengths could
    not tell from noise, cut a channel round itself), when a segment of them is not covered from end to end by
    the segments of the boundary that lie along it (a spur that its subpath goes out along and comes straight
    back over cancels in the sweep, which then has no piece of it at all, and the inset cut a notch round it;
    a point of it on the boundary is not enough, as a line out and back can have its middle on another edge,
    a hole's), when the sweep gives fewer contours than there are
    subpaths, or when the lengths of the two differ by more than the accuracy they are measured to and the
    sweep's tolerance at each vertex allow (it was a millionth of the length, which a unit square in one a
    million across is), and cleans the outlines of the open subpaths of the stroker's holes as a growing
    offset does (an open curve that turns inside the radius left holes in an inset only). The boundary leaves
    out the pieces of a contour that go straight out and back along one line to within 64 units in the last
    place of their own coordinates: the sweep keeps a cubic that goes out and back along one line as two
    passes a rounding apart, and an inset of a square 1e8 across with such a cubic 0.29 high on its edge,
    upright or slanted, cut a notch round it. A slit is kept however narrow, its sides being its width apart
    (left out by the area it encloses, slits up to twice the sweep's tolerance wide were lost, and a shape
    far away widens that: 2e-3 beside a square at 1e11);
  - an offset judges a contour of its arrangement spurious by a point of its face, the contour less the
    contours directly inside it (an outer contour's holes, a hole's islands), where it took a point of the
    contour alone, which can lie in one of them near the path, and dropped a face that was right with all
    inside it. A 100 x 100 square with a 20 x 20 hole from x = 50, or from 50.2 or 30.1, had an inset by 0.25
    of nothing (it is 9,480), and by 5 of nothing (7,200); so did slits through the middle, at widths from
    0.01 to 10; a 60 x 60 hole with a 30 x 2 bar across its middle was filled by an outset of 3 (11,236, where
    8,608 is right); and the outline of open subpaths an inset adds lost a hole round an open bar. The point
    is taken only where the face's own winding is odd, the widest of the first eight spans found on the face
    flattened to a hundredth of its size that is (a ring 0.1 wide between two circles a hundred across had
    an inset of nothing: the middle of a span between the flattened circles lay in the hole; VectorCraft,
    taking the outer circle alone, kept it). One random stack of the comparison changes with that check: a
    four-lobed star of quadratics, inset by 0.5, keeps a sliver of 0.147 square units that VectorCraft and the
    carve dropped, every point of which is in the fill and 0.5 or more from its edge;
  - `interior_point` drops edges that retrace each other exactly (a zero of either sign is one coordinate),
    tries heights between the vertices where the path turns (a straight retrace through other vertices than it
    went out by used up the heights; a vertex is passed straight through when it lies within the rounding of
    the coordinates across the line through its neighbours, not when the edges turn by under a billionth of a
    radian, which a bulge 0.1 wide and a billion high on a line a trillion long does, nor by the rounding of
    its largest coordinate, which that bulge a quadrillion out along its line is within), and looks for gaps
    wider than the rounding of the two crossings that make them, at every height (it was a billionth of the
    larger of the width and the height, so two unit squares ten billion apart in height were missed; and the
    first heights took any gap, a sliver between two rounded passes of a slanted line among them); a vertical
    edge crosses at its x exactly, and allows for no rounding (a unit square at x = 1e15 had no point: the
    allowance for each of its two crossings was 0.9), and the crossing of a slanted edge allows for the rounding
    of its computation (half a unit in the last place of its x, and five halves of one of the edge's width in
    x, for the product) and two more units in the last place of x, and not eight (a parallelogram a unit wide,
    its sides slanted by 0.125, at x = 1e15, where the unit in the last place is 0.125, had no point: 0.9 for
    each of two crossings is 1.8; a retrace along a slanted line at 1e15 still has none); and a gap's middle is taken only where it is strictly
    between the two crossings (between sides that are adjacent numbers it rounds onto one of them, an edge,
    which the winding can count as inside), nor on an edge of the path at that height or one of its vertices
    there (a rectangle 1 wide at x = 1e15 and 2 high at y = 1e16, where every height tried rounds onto its
    bottom or top, gave a point on its bottom edge, and VectorCraft does too);
  - the fill of a face in a planar map is tested against the subpaths of each input whose bounds hold the point,
    where it was the whole path (16,000 squares in one shape: 15.6 s, now 1.6 s); and an item the index of the
    map's outlines has no room to file in pieces is filed under its bounds, where it was read at every lookup
    (1,000 tall rectangles nested in one shape read 1.5 million candidates for 400, now 0.3 million; the same
    items are found);
  - every area is taken with each subpath's about its own first point, where kurbo's shoelace multiplied
    absolute coordinates, whose products cancel to nothing for a small shape far out: `try_area` of a square
    half a unit across was 0 from 1e8 to 1e11, and `normalize` dropped it as a sliver; a unit square's inset by
    0.25 was empty from 3e7 to 1e9 and at 3e10 and 1e11, and its outset at 3e8, 1e9 and 3e10; a square of 100
    inset by 1 was empty at 3e10; the faces the planar map finds for two small squares far out, and Divide and
    Merge of a square with a hole far out, came out wrong; and where a result was made its area read 16,384
    for 0.25. VectorCraft gives the
    same (its `try_area` of that square is 0 at 1e8, 1e9 and 1e10, the inset is empty at 1e8 and 1e9, the
    outset at 1e9). Near the origin the areas agree with VectorCraft's to their last digits: about two thirds
    of the comparison's random areas now differ from its in those, all by under a billionth, and no other
    result changes;
  - `boolean_n`, `try_boolean_n` and `try_boolean` check the windings of up to eight inputs that draw something
    (empty paths, and paths of points only, do not count), and the call is an error when a sweep or contour
    pass of the recovery panics, a contour pass of the windings check of a resolved operand or of the resolved
    operands together included (beside another operand found off too), where it went on to the first
    arrangement; and once the operands are resolved, their result is given even where its windings do not add
    up either, never the first result, which was found wrong (VectorCraft kept the first). The first result is
    still given where an operand cannot be resolved and checked on its own: the check is off by a little for
    some curves, and an error there failed a circle intersected with a rectangle and the union of a path with
    itself. One random stack a seed, in three of the four, now matches VectorCraft's result to its tolerance
    rather than exactly;
  - the coordinates are checked as given, before the fill leaves out a subpath that draws nothing: a closed
    loop of one cubic whose handles are not numbers, or overflow when multiplied, is `NonFinite` or
    `Degenerate` in `try_area`, `try_normalize`, `try_boolean`, `try_boolean_n` and `try_unite_all`, and
    `area` gives 0 for it, where it was left out and the answer was `Ok`, or the area of the rest of the path
    (VectorCraft left out every subpath of fewer than two anchors unread: its `area` and `try_area` of a
    square with such a loop are 100); and `unite_all` leaves such a path out whole, as it leaves out any it
    cannot sweep, where it kept the square (VectorCraft's union of it and a square 4 across apart is 116,
    the carve's 16). The tidy of such a union, whose index of input curves a boundary piece is rebuilt from
    when it lies on one, is made of the paths the union is made of and not of those left out, where
    VectorCraft's holds the curves of the ones it left out too, so a boundary piece near one could be rebuilt
    from a curve of a path that is not in the union;
  - `simplify_with` with `straight_lines` samples the intervals the chords need up to a limit, where a long
    curve was sampled in 256 and its chords were further from it than the tolerance, counts the sample at
    the start of a run in its limits, and holds the samples to the tolerance less how far the curve can be
    from them between two (a curve 1.000002 high between two samples 0.99999 high came out 1.000002 from its
    one chord at a tolerance of 1);
  - `simplify` holds a refit curve to the tolerance between its samples too, where only its distance at the
    samples was checked: a cubic is taken when its error at the samples, how far it can be from the chords
    between its values at their parameters (from its second differences and the widest step between the
    parameters), and how far the run can be from the polyline of its samples add up to the tolerance at most,
    and a stretch of two samples that its tangents bend too far from their chord is no fit (the run stays as
    it is). Chains of cubics each up to about five tolerances long, turning up to 1.5 rad, came out up to
    1.026 of the tolerance from their refits (3 of 3,000 random ones; none of 9,000 now). Against
    VectorCraft 7 of 3,600 random simplifications differ, all within the tolerance either way and none further
    from the path than VectorCraft's (it was up to 0.996 of the tolerance; four of the runs are now left as
    they are, three have one element more). A refit cubic that is nearly straight becomes its chord only
    where the chord is within the tolerance of the run too: the cubic's bound and how far its handles are from
    the chord add up to the tolerance at most (a bump half a unit high on a run 75 long came out as one line
    1.0026 of the tolerance from it at a tolerance of 0.5; it is a cubic now). No random simplification of the
    comparison changes with it;
  - `stroke_region` and `try_stroke_region` hold a single outline to the element limit as they do each of
    several (the boundary of its sweep counted before its contours are made), where one outline that crossed
    itself a great deal made all its contours, and hold the union of several outlines, and `unite_all` and
    `try_unite_all` the union of their paths, to it too (counted the same way, and the contours counted again
    before they are written, and the path written counted as well): outlines within the limit each that crossed each other a great deal were all
    made into contours;
  - a face of a planar map that the first search finds no point in (it flattens the face to a thousandth of its
    size, and a lens 0.03 wide between two curves 160 long is thinner than that: 15 of 1,219 random lenses over
    the precision) is searched again, on an outline flattened to a quarter of the precision or an eighth of the first,
    whatever the precision (it was so for a precision under the default only), and the face was a gap that nothing
    fills, listed again as the filled area it is by `shape_builder`; where there is still none, `try_shape_builder`
    and `try_pathfinder` say `Degenerate` for a face thicker than the precision (the others list it with no source);
  - an offset judges a contour spurious by a point deep in its face, found on the face flattened to a hundredth of its
    size; where none of the middles of the first eight spans is in the face (a ring half a unit wide round a hole a
    hundred across, whose circles the flattening moves a unit) the widest span between the crossings of the curves
    themselves with the lines is used, where the contour was kept: an outset by more than twice the radius of a circle
    cut in 8 cubics (a tolerance of 0.001) left a hole of 8,012 square units and an island of 7,854 in the disk of
    71,158, and one cut in 4 (0.01) four slivers of 0.3 to 0.6 square units;
  - the tidy takes the pieces of a run for part of an input curve only where their middles and their ends are within the
    precision of it as well as within ten matches (the first of two input curves that share their ends and lie nearer than
    ten matches took the pieces of the other, up to 0.003 off at coordinates of 500 and a precision of 1e-4; and where only
    the middles were tested, a run of two halves of the second curve, whose joint is where the two lie furthest apart, 0.0030
    off at a precision of 0.0024, 0.0026 or 0.0029, where the middles are 0.0023 off);
  - the Pathfinder Outline looks for the junctions on a curve inside its bounds, not inside the box of its control points
    (the curve is inside its bounds, and a junction within the sweep's noise of it is inside them widened by that
    and the rounding of the coordinates), and looks them up by blocks that have the segment's heights, not one by one
    among those with its x: the same junctions are found;
- the predicate of `boolean_n` and `try_boolean_n` is asked once for each distinct winding of the sweep, and the
  contour pass is given the answers by where the windings are stored, where VectorCraft asked it every time
  the pass looked at a side of a segment, and the pass scans west through every segment that is no boundary
  for every contour it starts: 2,000 squares in a column, each an input, beside a path of 2,000 tall
  rectangles to their west took it 6.5 s (1,000: 0.85 s), the cube of their number, and 0.25 s here (1,000:
  0.07 s); the flags handed to the predicate were 32 billion and are 4 million. Nested squares round a column
  of small squares, each an input (the scan meets a long winding at each step), 1,000 and 1,000: 1.25 s, and
  0.12 s here. The results are the same for a predicate that depends on its flags alone; one that counts its
  calls sees fewer of them (it is asked once for each distinct winding, before the pass: twice for a
  rectangle, outside and inside, not once for each of the eight sides of its segments);
- eight bounds are new and refuse input VectorCraft took, in whatever time it took, with `TooComplex` (or no
  shapes, for the operations that give none):
  - more than 2^20 places where segments cross, whatever the pieces on the sweep line, counted as a bound that is
    never below the number there are, by testing the pairs of bounds that overlap (curves by the directions of their
    tangents, and the control polygon for a line), a place where a curve crosses counting for two
    (206 stars of 201 points, each above and beside the last, took VectorCraft 12.6 s and 6.5 GB, and 100 of
    bowed cubics 18.6 s and 3.7 GB);
  - more than 2^30 searches of the sweep line by the lines that really cross (a star polygon of 2,001 points took
    VectorCraft 25.8 s and 3.7 GB, 1,001 points 2.9 s: the time grows with the cube of their number);
  - a Pathfinder Outline whose junction searches pass 2^30 steps (VectorCraft looked at every junction in each
    segment's range of x, and searched the curve for each one in its control box, which a grid of cutters under
    a few hundred arches made tens of millions of; in the limit the results are the same);
  - more than 512 segment ends at one point, counted at the sweep's tolerance that all the points set
    (a pie of 257 slices took VectorCraft 0.7 s, 400 slices 3.1 s, the fourth power of their number; 400
    spikes with apexes 0.004 apart beside a cubic with control points at 1e12, which makes them one point,
    4.8 s);
  - lines that cross at one point, counted as 2^24 of work, which is 256 lines (150 lines through a
    point took VectorCraft 0.2 s, 256 1.0 s, 300 1.8 s, and the time grows with the cube of their number);
  - more than 33,554,432 entries of windings over the pieces the sweep cuts the segments into (2,896 squares
    nested took VectorCraft 0.4 s; 300 nested squares crossed by 300 bars, 1.3 GB, 0.6 s; the memory grows
    with the square of the depth, and with the bars, 270 MB at 2,000 deep and about 4.5 GB at 8,000);
  - Merge, Divide, Trim or Crop whose contour passes (a batch of classes each) would walk more than
    33,554,432 segments in all (a fan of 209 shapes at a point beside 40,000 squares took VectorCraft 4.4 s to
    merge and 5.5 s to divide; 208 take 2.6 s and 3.5 s here, and the time grows with the shapes at the
    point);
  - a planar map that would wind its faces and holes against more than 67,108,864 elements of fills and
    faces (rectangles nested 3,400 deep in one shape; 8,000 took the carve 9.2 s to map before this limit,
    the time growing with the square of the depth, and are refused in 1.9 s). Here `live_paint` gives no
    shapes, and `shape_builder` and Divide with an open path give the filled areas instead, as past
    `SHAPE_BUILDER_MAX_SEGMENTS`: an answer, not a refusal, but not the arrangement asked for (the faces are
    not split by the open paths). `try_shape_builder` and `try_pathfinder` refuse it with `TooComplex`.
  A pair of a line and a curve also counts three crossings, not one, in the sweep-event limit. Within the
  limits the results are VectorCraft's (same shape counts and areas on every input above);
- the limits and counts above, `try_outline_stroke`, `try_stroke_region`, `try_shape_builder`, `try_pathfinder`
  and `try_pathfinder_with`, the bounded flattening (VectorCraft's
  `pathfinder_with(Divide, ..)` with an open path and a precision of 1e-300 panicked with "capacity overflow"
  in `kurbo::flatten`) and the lookups of input curves and points by a rectangle index (a column of shapes
  of one width was scanned whole at every point) are new.
