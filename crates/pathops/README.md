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
| Outline | Returns open edge paths, split at every junction; coincident edges are de-duplicated, and each keeps the front-most key. A closed subpath gives the edges of its outline, an open one the edges of the path itself (no closing edge; one of three anchors or more that ends where it starts is a ring, and counts as closed, and so does one cubic that loops round an area, closed or not), and where an open path crosses another is a junction. The crossings of open paths come from the planar map (below), which is built for up to `SHAPE_BUILDER_MAX_SEGMENTS` segments: past that, or when no map can be built, there are no shapes, since the filled areas, where an open path's way there and back cancel, do not see where it crosses another |

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
- segments that meet at one point cost about the fourth power of their number, which none of those counts
  (a pie of k slices has 2k ends at its centre): more than `MAX_ENDS_AT_A_POINT` (512) ends at one point is
  `TooComplex`. The ends are counted in a hash at the sweep's tolerance, which every point of every element
  sets, control points too (one cubic with control points at 1e12 makes it 0.014, and 400 spikes whose apexes
  are 0.004 apart one point to the sweep, which took 4.4 s). A pie of 256 slices takes 0.5 to 0.9 s, 257 are
  refused in a few milliseconds;
- lines that cross at one point, with no end there, cost about the cube of their number (150 take 0.2 s, 300
  1.7 s, 450 8 s, and about a thousand passed every other limit and would take a minute and a half). A line
  that crosses `m` others at one point counts `m`², so `c` lines through a point count `c` (`c` - 1)², and
  `MAX_CROSSING_WORK` (2^24) is at 256 lines through a point, which take about a second (257 are `TooComplex`
  in a few milliseconds). The crossings are computed, for the lines whose bounds overlap, only when that
  many pairs could reach the limit if they were at one point, and then once for each pair (in O(pairs log
  n), the pairs being held to `MAX_SEGMENT_PAIRS` already). Lines that end at the point are ends, above;
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
  asked for, the faces unsplit by the open paths (rectangles nested 3,300 deep pass, in 1.3 s; 3,400 do not).
  `try_shape_builder` and `try_pathfinder` give `TooComplex` there instead, so a caller can tell, and also
  where the faces of closed shapes are past `mask_work` or `pass_work` (Divide gives no shapes and
  `shape_builder` no regions of the filled areas there, as `regions` gives none), which they do not hide as
  an empty drawing or a map with the filled areas missing; they check the shapes as given as well, before
  the fill leaves out a loop of one cubic whose handle is no number (`NonFinite`, or `Degenerate` for one
  past what the sweep takes);
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
`fit_points`, `fit_work`, `join_work`, `redundant_work`, `mask_work`, `sweep_event_work`, `pass_work`,
`fill_work`, `ends_at_a_point`, `crossing_work`, `coverage_entries`) and is `#[non_exhaustive]`, so start from
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

Five more bounds refuse input a drawing can hold, none of it common: a pie of more than 256 slices (or any
more than 512 segment ends at one point; a fan of triangles has two at the point for each, so 257 of them) by `MAX_ENDS_AT_A_POINT`, 257 lines
through one point by `MAX_CROSSING_WORK`, shapes nested 2,896 deep, or 202 deep and crossed by 202 bars, by
`MAX_COVERAGE_ENTRIES`, Merge, Divide, Trim or Crop of a fan of 209 or more shapes at a point beside 40,000
other squares by `MAX_PASS_WORK`, and the planar map of thousands of contours nested in one shape (3,400
rectangles a thousand times as tall as they are wide) by `MAX_FILL_WORK`. Each is a worst case measured in
seconds or in half a gigabyte (the pie of 256 takes 0.5 to 0.9 s, 256 lines through a point 1.1 s, 2,895 nested
squares 0.2 s and 550 MB, 3,300 nested rectangles in one shape 1.3 s), so an app that can wait or has the memory
raises it in `Limits`.

The element limit refuses a drawing of more than 524,288 elements in one operation (a traced image of half a
million points), and `shape_builder` gives only the filled areas for more than 65,536 segments.

### Known limits

The limits count before the sweep and stand for a time or a size; no proof ties the sweep's time to the counts, and
each was set from the worst input found for it (by measuring and by review). What is known to pass them and still
cost more than a second or so, measured once in a release build:

- Curves that pass through one point with no end there are not counted, only lines are (and the event limit
  prices a pair of curves at nine crossings, a worst case, so it refuses a stack of curves before it is
  slow). Lenses whose two cubics each pass through one point (the cubics, not lines) take 0.17 s for 100, 0.7 s for
  200, 3.3 s for 400 and 4.1 s for 450; 500 are refused by the event limit. The cost grows with about the
  square of their number there, not the cube. Lines that pass through that point are counted, as if the curves
  were not there.
- The estimate of the entries of the windings counts one crossing for each pair of curves when an eighth of the
  one lies within the bounds of an eighth of the other, where two cubics may cross up to nine times, so curves
  that cross each other several times can hold more than the limit's half a gigabyte. It was at least the
  entries held in every arrangement of lines and of circles that the tests built, and is not a bound for
  curves that cross several times each.
- The event limit and the limit on pairs of pieces at overlapping heights are worst cases priced in
  nanoseconds, and the slowest rows of the Costs table are what they admit: 3.1 s for the largest hatch,
  2.8 s for the largest row of dots, 5.4 s for an outline stroke of cubics a thousand wide, 1.8 s and a
  gigabyte for two polygons at the element limit.
- `interior_point` drops the edges that run back over each other exactly (an edge and the same edge the other
  way, as an out-and-back path has) before it looks at the few heights it tries, and takes those heights from
  the vertices where the path turns, not those it passes straight through, so a stub out and back along a
  line, through the same vertices or others, costs it nothing. A retrace along a curve flattened at other
  points each way (a vertex at each turn of the flattening, and no edge the same) is not dropped, and its
  vertices use up the heights: a unit square beside a long curved stub of that kind can be missed.
- A planar map is built before the fills of its faces are tested, and the fill limit stops only the tests:
  rectangles nested in one shape, each a thousand times as tall as it is wide (so that the bounds of their sides
  overlap only at the corners, and the limits counted before the sweep admit over 8,000 of them), are refused
  by `MAX_FILL_WORK` past 3,300, in 1.5 s at 4,000 and 2.4 s at 8,000 (answering 8,000 took 9.2 s before the limit).
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
- The counts themselves are made before the sweep and cost time: up to 0.3 to 0.5 s on an input of 250,000
  segments (they are a few sweeps of the bounds), with 64,000 dots as separate inputs the most.

An operation within the limits that takes longer than the seconds above, or more than a gigabyte, is not
known; the inputs above were found by testing and by independent review, and review has found more after each
round, so there may be others.

## Costs

Within the limits an operation can still take seconds and about a gigabyte, so run the heavy ones off a UI
thread and ask first with the `try_` forms. Measured on one machine, in a release build:

| Operation | Time |
|---|---|
| `try_boolean` of two polygons of 262,000 sides each (the element limit; peak memory 1.0 GB) | 1.8 s |
| `unite_all` of a column of 16,000 / 32,000 / 64,000 dots (64,000 / 128,000 / 256,000 elements) | 0.5 / 1.2 / 2.1 s |
| `unite_all` of a row of 5,500 dots on one line (the sweep-line limit is at 5,800) | 2.8 s |
| `unite_all` of a grid of 16,000 / 32,000 lenses with one more 10 million away | 0.6 / 1.4 s (as without the far one) |
| `try_area` of a pie of 256 slices (512 segment ends at its centre, the limit) | 0.5 to 0.9 s |
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
| `live_paint` of 1,000 / 2,000 / 3,300 rectangles nested in one shape, each 1,000 times as tall as it is wide | 0.19 / 0.57 / 1.3 s (4,000 and 8,000: no map, in 1.5 / 2.4 s) |

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
    as long as rounding may have made them, 8 units in the last place of each, so that a shallow curve with long
    control edges, whose edges add up to its chord to the last place, is not taken for a line). That is a bound
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
    cancel, saw no crossing); Merge makes a shape of each connected component, so an island inside a
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
    allowance for each of its two crossings was 0.9); and a gap's middle is taken only where it is strictly
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
- five bounds are new and refuse input VectorCraft took, in whatever time it took, with `TooComplex` (or no
  shapes, for the operations that give none):
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
    the time growing with the square of the depth, and are refused in 2.4 s). Here `live_paint` gives no
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
