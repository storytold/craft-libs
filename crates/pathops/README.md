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
| Outline | Returns open edge paths, split at every junction; coincident edges are de-duplicated, and each keeps the front-most key. A closed subpath gives the edges of its outline, an open one the edges of the path itself (no closing edge; one of three anchors or more that ends where it starts is a ring, and counts as closed), and where an open path crosses another is a junction |

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
  nine times) times the most pieces on the line at one height are held to `MAX_SWEEP_EVENT_WORK`
  (4,294,967,296). All of them are counted in O(n log n) before the sweep;
- a boolean operation sets a flag for each input every time its predicate is asked about a face's side, about
  four times for each segment of the sweep, which is more than the sweep's own work when most of many inputs
  are empty: past `MASK_WORK` (268,435,456) such flags it is `TooComplex`. The union of many inputs, and of
  the outlines of a stroke, stops as soon as the contours made pass `MAX_ELEMENTS`, and does not make the
  rest;
- a stroke's outline is counted as it is made (a subpath of more than 256 segments is stroked in windows of
  256, so that a width that is huge against the tolerance cannot make millions of curves first) and the
  operation is `TooComplex` past `MAX_ELEMENTS` elements of outline;
- the operations without a `Result` answer an empty path, no shapes, or the input unchanged, as their
  documentation says. `simplify` stops fitting at `MAX_FIT_POINTS` samples of one run and at `FIT_WORK` in
  all, `remove_redundant_points` at `REDUNDANT_WORK` (the joints it reads in its search, and the points of a run it
  checks one by one against the line that replaces it) and `join` at `JOIN_WORK`, and leaves the rest as it is;
- lengths are measured to a relative accuracy (kurbo's takes an accuracy in the units of the path, which a cubic of
  large coordinates meets only by subdividing the most it ever does), and an inset compares the lengths of the
  closed subpaths and of the boundary of their fill for at most 4,096 curves: past that, or with a curve whose
  coordinates are past a billion, it strokes the boundary, which is right for any input;
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
`fit_points`, `fit_work`, `join_work`, `redundant_work`, `mask_work`, `sweep_event_work`) and is `#[non_exhaustive]`, so start from
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
to nine times), so a hatch of curves is refused at fewer. A boolean operation of more than a few thousand
inputs, most of them empty, is `TooComplex` past `MASK_WORK`.

The element limit refuses a drawing of more than 524,288 elements in one operation (a traced image of half a
million points), and `shape_builder` gives only the filled areas for more than 65,536 segments.

## Costs

Within the limits an operation can still take seconds and about a gigabyte, so run the heavy ones off a UI
thread and ask first with the `try_` forms. Measured on one machine, in a release build:

| Operation | Time |
|---|---|
| `try_boolean` of two polygons of 262,000 sides each (the element limit; peak memory 1.0 GB) | 1.3 s |
| `unite_all` of a column of 16,000 / 32,000 / 64,000 dots (64,000 / 128,000 / 256,000 elements) | 0.5 / 1.2 / 2.1 s |
| `unite_all` of a row of 5,500 dots on one line (the sweep-line limit is at 5,800) | 2.8 s |
| `unite_all` of a grid of 16,000 / 32,000 lenses with one more 10 million away | 0.6 / 1.4 s (as without the far one) |
| `try_area` of a rectangle with 20,000 collinear vertices on its top and bottom (40,000 pieces of no height) | 24 ms |
| `try_outline_stroke` of 3,000 wavy cubics 3 wide (the default limit is at about 3,100) | 1.0 s |
| `try_outline_stroke` of 1,000 cubics 1,000 wide (22,000 elements of outline, much of it crossing itself) | 5.4 s |
| `try_outline_stroke` of 200,000 cubics 100 million wide | refused in 0.3 s |
| `simplify` of 100,000 segments | 1.7 s (the `FIT_WORK` limit) |
| `join` of 4,096 lines to a chain of 500,000 segments | 0.36 s |
| `interior_point` of a comb of 40,000 teeth (160,000 elements) | 11 ms |
| `live_paint` of a comb of 20,000 teeth | 1.2 s |

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
  - the tidy keeps an edge as short as the sweep's tolerance (set by the coordinates) allows, beside an edge
    a billion long (it dropped any under a billionth of the longest), and matches an input curve to a piece
    within the sweep's noise at coordinates above a thousand (it was a millionth of the coordinates: a whole
    unit at a million, so one arch's pieces were rebuilt from the arch beside it);
  - `simplify` leaves a run it cannot measure, or fit within the tolerance in 40 splits, as it is (VectorCraft
    sampled the first as a point, and took the second's last cubic as a fit);
  - a closed subpath of one cubic that loops round an area is filled (VectorCraft dropped every closed
    subpath of fewer than two anchors);
- the limits and counts above, `try_outline_stroke`, `try_stroke_region`, the bounded flattening (VectorCraft's
  `pathfinder_with(Divide, ..)` with an open path and a precision of 1e-300 panicked with "capacity overflow"
  in `kurbo::flatten`) and the lookups of input curves and points by a rectangle index (a column of shapes
  of one width was scanned whole at every point) are new.
