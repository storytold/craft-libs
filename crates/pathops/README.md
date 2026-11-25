# craft-pathops

Path operations for the Craft suite's 2D vector kernel (RFC-0030): booleans, Pathfinder operations, Shape Builder regions, Live Paint planar maps, offset path, outline stroke and simplify. Everything takes and returns `craft_geom::PathData`.

Carved from VectorCraft's `crates/pathops` and re-synced from storytold/vectorcraft@9de706507c30eda3b956862875a0a1655170063f, with its tests. VectorCraft's copy is retired by its RFC-0030 adoption PR.

## Booleans (curve-preserving)

```rust
pub enum BoolOp { Union, Intersect, Difference, Xor }
pub fn boolean(a: &PathData, a_rule: FillRule, b: &PathData, b_rule: FillRule, op: BoolOp) -> PathData;
pub fn try_boolean(.., precision: f64) -> Result<PathData, PathOpsError>; // errors on NaN/inf
pub fn boolean_n(paths: &[(&PathData, FillRule)], pred: impl Fn(&[bool]) -> bool) -> PathData;
pub fn unite_all(paths: &[(&PathData, FillRule)]) -> PathData;   // fast many-shape union
pub fn normalize(path: &PathData, rule: FillRule) -> PathData;   // resolve self-overlaps
pub fn area(path: &PathData, rule: FillRule) -> f64;             // exact filled area
```

The engine is `linesweeper` (a robust sweep line that works directly on cubic Béziers, so there's no polygon flattening). The sweep splits curves at intersections and y-extrema. Afterwards:

- pieces that lie on one input cubic are rebuilt exactly as a single sub-curve of that cubic;
- other smooth joints (for example, in stroker output) are refitted by least squares within `DEFAULT_PRECISION` (0.01 pt);
- collinear line pieces are merged;
- the inputs' own anchors are always kept;
- slivers narrower than the precision are dropped.

Output contours are simple and consistently oriented: outer contours and holes wind in opposite directions, so the result fills the same under either fill rule. Open subpaths are closed implicitly because they're treated as filled.

## Pathfinder / Shape Builder

```rust
pub struct Shape { pub path: PathData, pub rule: FillRule, pub key: u64 } // key = paint identity
pub enum PathfinderOp { Unite, MinusFront, Intersect, Exclude, Divide, Trim, Merge, Crop, Outline, MinusBack }
pub fn pathfinder(op: PathfinderOp, shapes: &[Shape] /* back → front */) -> Vec<Shape>;

pub struct Region { pub path: PathData, pub sources: Vec<usize> }
pub fn regions(shapes: &[Shape]) -> Vec<Region>;            // all faces of the arrangement
pub fn region_at(shapes: &[Shape], p: Point) -> Option<Region>;
pub fn merge_regions(regions: &[&Region]) -> PathData;      // Shape Builder merge = union
pub fn shape_builder(shapes: &[Shape]) -> (Vec<Region>, Vec<Shape>); // (regions, open-path pieces)
```

Each result takes its key as follows:

| Operation | Result |
|---|---|
| Unite, Intersect, Exclude, Minus Back | Takes the front-most key |
| Minus Front | Takes the back-most key |
| Divide | One shape per face, keyed by the front-most shape that covers the face |
| Trim | One shape for each object that is still visible |
| Merge | Joins touching visible parts that share a key; each connected component becomes one shape |
| Crop | Keeps the visible parts of the lower objects inside the front-most object |
| Outline | Returns open edge paths, split at every junction; coincident edges are de-duplicated, and each keeps the front-most key |

## Live Paint

```rust
pub fn live_paint(shapes: &[Shape] /* back → front */) -> (Vec<Region>, Vec<Shape>); // (faces, edges)
pub fn interior_point(path: &PathData) -> Option<Point>;
```

Unlike `regions`, which splits the filled areas of closed shapes, `live_paint` treats every path as an edge, open paths included. Three crossing lines enclose a triangle, a line across a rectangle splits it in two, and an area enclosed by paths becomes a face even when nothing fills it. Each face lists the inputs whose fill covers it; the list is empty for an area that is only enclosed by paths. Each edge is a piece of a path, split wherever another path meets it, and keeps the key of the front-most input it lies on.

`shape_builder` gives the Shape Builder's regions. With closed paths only, they are the `regions`. Once an open path is among them, they are the `live_paint` faces, so a stroke-only line across a shape splits it and lines can enclose a region of their own. The second list holds the pieces of each path that has no closed subpath, keyed by its index, so the Shape Builder can delete them one by one. The caller closes any open path whose fill should count. Inputs with more than `SHAPE_BUILDER_MAX_SEGMENTS` segments fall back to the filled areas.

```rust
pub struct Adjacency { pub faces: Vec<MapFace>, pub edges: Vec<MapEdge>, pub slivers: Vec<PathData>, pub outlines: Vec<MapOutline> }
pub struct MapEdge { pub path: PathData, pub sources: Vec<usize>, pub left: Option<usize>, pub right: Option<usize>, pub piece: usize }
pub fn adjacency(shapes: &[Shape], precision: f64) -> Option<Adjacency>;
```

`adjacency` maps the paths as `live_paint` does and also reports, for each edge, every input it lies on and the faces on its left and right as it runs along its path (document coordinates, y down). A line drawn on top of another keeps both inputs on the edge, whatever the input order. A piece of line that ends inside a face has that face on both sides. Faces narrower than `precision` are listed as slivers, and `outlines` holds the outer boundary of each connected piece of the paths. It returns `None` for more than `SHAPE_BUILDER_MAX_SEGMENTS` segments, non-finite input or a `precision` that is not a positive number.

## Offset / stroke

```rust
pub use kurbo::{Cap, Join};
pub fn offset_path(path: &PathData, delta: f64, join: Join, miter_limit: f64) -> PathData; // delta < 0 insets
pub fn try_offset_path(path: &PathData, delta: f64, join: Join, miter_limit: f64) -> Result<PathData, PathOpsError>;
pub fn outline_stroke(path: &PathData, width: f64, cap: Cap, join: Join, miter_limit: f64) -> PathData;
```

`outline_stroke` passes the path to `kurbo::stroke` and then normalizes the result, which removes the overlaps at joins. `offset_path` computes `fill ∪ stroke(2|d|)` for a positive offset and `fill − stroke(2|d|)` for a negative one.

## Path editing

```rust
pub fn simplify(path: &PathData, tolerance: f64) -> PathData;
pub fn simplify_with(path: &PathData, opts: &SimplifyOptions) -> PathData; // tolerance, corner_angle_deg, straight_lines
pub fn smooth(path: &PathData, amount: f64) -> PathData;
pub fn remove_redundant_points(path: &PathData, tolerance: f64) -> PathData;
pub fn add_anchor_points(path: &PathData) -> PathData;
pub fn average(path: &PathData, selection: &[(usize, usize)], axis: AverageAxis) -> PathData;
pub fn join(paths: &[PathData], tolerance: f64) -> PathData;
pub fn split_into_grid(rect: Rect, rows: usize, cols: usize, gutter: f64) -> Vec<PathData>;
```

`simplify` fits least-squares cubics with fixed end tangents, then reparameterizes with Newton steps and splits at the point of worst error. Any anchor that turns more than the corner threshold (30° by default) stays a corner.

## Limits

Every boolean, Pathfinder operation, region map and normalization goes through one sweep. The `try_` functions
(`try_boolean`, `try_normalize`, `try_unite_all`, …) refuse input it can't sweep with a [`PathOpsError`](src/lib.rs)
rather than answering with part of the art missing. The others have no error to return: `boolean`, `normalize` and
`area` answer an empty path or 0, and `unite_all` leaves a refused input out of the union, or answers an empty path
when the inputs together are refused.

- `NonFinite`: a coordinate is NaN or infinite.
- `Degenerate`: the coordinates are so large that the sweep's tolerance would pass one unit (`MAX_SWEEP_EPS`), so
  features thinner than it would collapse. A panic inside the sweep on rare near-degenerate input is caught and
  reported as `Degenerate` too (on native builds; on the web a panic aborts).
- `OpenPath`: the sweep could not form closed contours from what it was given (open subpaths are closed before it).
- `TooComplex`: more than [`MAX_SEGMENT_PAIRS`](src/budget.rs) pairs of segments have overlapping bounds. They
  are counted exactly before the sweep, in O(n log n). This bounds what the sweep is given, not how many times the
  segments cross: a compact path that crosses itself many times can stay under it and still take seconds.

Results are refitted within the precision (`DEFAULT_PRECISION`, unless `try_boolean`, `unite_all_with`,
`try_unite_all_with` or `pathfinder_with` is given one) and contours thinner than it on average are dropped as slivers
(see Booleans above).

## Caveats

- linesweeper is still in beta. When two identical curves overlap, the sweep can leave a tiny lens between them; the sliver filter removes it.
- The Outline op finds split points by snapping to arrangement junctions with a small tolerance.
- The quality of offsets depends on kurbo's parallel-curve stroker. Its output is refitted, so offset results have more anchors than boolean results.
