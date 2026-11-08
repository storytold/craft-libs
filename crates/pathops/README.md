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
pub fn has_open_path(shapes: &[Shape]) -> bool;
pub fn interior_point(path: &BezPath) -> Option<Point>;
```

Unlike `regions`, which splits the filled areas of closed shapes, `live_paint` treats every path as an
edge, open paths included. Three crossing lines enclose a triangle, a line across a rectangle splits it
in two, and an area enclosed by paths becomes a face even when nothing fills it. Each face lists the inputs
whose fill covers it (empty for an area only enclosed by paths). Each edge is a piece of a path, split
wherever another path meets it, and keeps the key of the front-most input it lies on. `shape_builder` gives
the regions an editor offers for a set of shapes: with closed paths only they are the `regions` (plus the
gaps closed shapes enclose); once an open path is among them they are the planar faces, so a stroke-only
line across a shape splits it. Inputs with more than `SHAPE_BUILDER_MAX_SEGMENTS` segments fall back to
the filled areas.

The sweep takes an open path's two passes (there and back) for cancelling out where the path is level, so the
map is swept in a frame sheared (`y + k x`) so that no open path is level in it after the paths are snapped to the
sweep's tolerance, which a shape far away widens. The first of ten shears (1/32, 1/64, 1/16, ...) that leaves none
level is used. Open paths with among them a side level in each of the ten (a side of a segment's chord or control
polygon, longer than 8 tolerances of the sweep: it takes ten such sides, one level in each shear) give no map, as
an input the sweep cannot take does. A side no longer than that is at the noise of the sweep, level in every shear
and not looked at: a stub, a point repeated a hair apart or a handle a nanometre long does not empty the map, and a
feature that small may be lost from it (the sweep cannot tell it from a point).

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
- a stroke's outline is counted as it is made (a subpath of more than 256 segments is stroked in windows of
  256, so that a width that is huge against the tolerance cannot make millions of curves first) and the
  operation is `TooComplex` past `MAX_ELEMENTS` elements of outline;
- the operations without a `Result` answer an empty path, no shapes, or the input unchanged, as their
  documentation says. `simplify` stops fitting at `MAX_FIT_POINTS` samples of one run and at `FIT_WORK` in
  all, `remove_redundant_points` at `REDUNDANT_WORK` (the joints it reads in its search, and the points of a run it
  checks one by one against the line that replaces it) and `join` at `JOIN_WORK`, and leaves the rest as it is;
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
`ends_at_a_point`, `crossing_work`, `coverage_entries`) and is `#[non_exhaustive]`, so start from
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

Four more bounds refuse input a drawing can hold, none of it common: a pie of more than 256 slices (or any
more than 512 segment ends at one point; a fan of triangles has two at the point for each, so 257 of them) by `MAX_ENDS_AT_A_POINT`, 257 lines
through one point by `MAX_CROSSING_WORK`, shapes nested 2,896 deep, or 202 deep and crossed by 202 bars, by
`MAX_COVERAGE_ENTRIES`, and Merge, Divide, Trim or Crop of a fan of 209 or more shapes at a point beside 40,000
other squares by `MAX_PASS_WORK`. Each is a worst case measured in seconds or in half a gigabyte (the pie of
256 takes 0.5 to 0.9 s, 256 lines through a point 1.1 s, 2,895 nested squares 0.2 s and 550 MB), so an app
that can wait or has the memory raises it in `Limits`.

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
  way, as an out-and-back path has) before it looks at the few heights it tries; a retrace that goes back
  through other vertices than it went out by, or along part of the way only, is not dropped, and its vertices use
  up the heights: a unit square beside a long stub of that kind can be missed.
- `simplify` with `straight_lines` samples a segment in as many intervals as keep the chords within a tenth of
  the tolerance of the curve, up to 65,536 for one segment (a quarter circle of radius 1e9 at a tolerance of
  1), and leaves a run that needs more as it is; the segments it takes the default 256 intervals for (the
  chords of those are as near) are as before.
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
  - `remove_redundant_points` (and so the stroke's spine) checks a merged run against the segments it
    replaces, not only the joint it merges across: a half circle of ten thousand chords came out as its
    diameter at a tolerance of 0.001, and is now kept; noisy points about a straight line within the
    tolerance still merge. It costs a bounded number of checks for each point;
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
    open ring of more anchors is: an offset, a Pathfinder Outline and the planar map treat both as closed. It
    is a loop when the handles of the cubic are not along one line, to the rounding of their cross product
    (a loop a billion long and a unit wide is one);
  - Pathfinder Outline of shapes with an open path takes its crossings from the planar map alone, and makes no
    filled arrangement: open paths whose implicit closing lines overlapped each other past the limit on pairs
    were refused by it and are outlined from the map; and where the map cannot be built there are no shapes,
    where the filled areas' junctions left every open path uncut where it meets another;
  - the planar map is swept in a frame in which no open path is level after the paths are snapped to the sweep's
    tolerance (it was told before snapping, so a line that a distant shape's tolerance levelled was lost),
    from ten shears, and no map is built when open paths have a side level in each (it used the first, with one
    path lost); sides of no more than 8 tolerances are not looked at, so a tiny feature loses only itself;
  - an inset takes the boundary of the area the closed subpaths fill when the sweep gives fewer contours than
    there are subpaths, or when the lengths of the two differ by more than the accuracy they are measured to and
    the sweep's tolerance at each vertex allow (it was a millionth of the length, which a unit square in one a
    million across is), and cleans the outlines of the open subpaths of the stroker's holes as a growing
    offset does (an open curve that turns inside the radius left holes in an inset only);
  - `interior_point` drops edges that retrace each other exactly, looks for gaps wider than 64 ulps of the
    largest x (it was a billionth of the larger of the width and the height, so two unit squares ten
    billion apart in height were missed);
  - the fill of a face in a planar map is tested against the subpaths of each input whose bounds hold the point,
    where it was the whole path (16,000 squares in one shape: 15.6 s, now 1.6 s);
  - `boolean_n`, `try_boolean_n` and `try_boolean` check the windings of up to eight inputs that draw something
    (empty paths do not count), and the call is an error when the contour pass of the resolved operands
    fails, where it went on to the first arrangement;
  - `simplify_with` with `straight_lines` samples the intervals the chords need up to a limit, where a long
    curve was sampled in 256 and its chords were further from it than the tolerance, and counts the sample at
    the start of a run in its limits;
- the predicate of `boolean_n` and `try_boolean_n` is asked once for each distinct winding of the sweep, and the
  contour pass is given the answers by where the windings are stored, where VectorCraft asked it every time
  the pass looked at a side of a segment, and the pass scans west through every segment that is no boundary
  for every contour it starts: 2,000 squares in a column, each an input, beside a path of 2,000 tall
  rectangles to their west took it 6.5 s (1,000: 0.85 s), the cube of their number, and 0.25 s here (1,000:
  0.07 s); the flags handed to the predicate were 32 billion and are 4 million. Nested squares round a column
  of small squares, each an input (the scan meets a long winding at each step), 1,000 and 1,000: 1.25 s, and
  0.12 s here. The results are the same for a predicate that depends on its flags alone; one that counts its
  calls sees fewer of them (it is asked of each side of each segment, once, before the pass);
- four bounds are new and refuse input VectorCraft took, in whatever time it took, with `TooComplex` (or no
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
    point).
  A pair of a line and a curve also counts three crossings, not one, in the sweep-event limit. Within the
  limits the results are VectorCraft's (same shape counts and areas on every input above);
- the limits and counts above, `try_outline_stroke`, `try_stroke_region`, the bounded flattening (VectorCraft's
  `pathfinder_with(Divide, ..)` with an open path and a precision of 1e-300 panicked with "capacity overflow"
  in `kurbo::flatten`) and the lookups of input curves and points by a rectangle index (a column of shapes
  of one width was scanned whole at every point) are new.
