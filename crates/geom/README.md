# craft-geom

The Craft suite's 2D vector drawing kernel (RFC-0030): the anchor-based editable path model and the geometry every drawing app needs around it. Carved from VectorCraft's `crates/geom` and re-synced from storytold/vectorcraft@9de706507c30eda3b956862875a0a1655170063f, with its tests. VectorCraft's copy is retired by its RFC-0030 adoption PR.

## The path model

A [`PathData`] is a list of `SubPath`s, each a list of `Anchor`s with absolute in/out handle positions, the way every Illustrator-style tool thinks about paths. Rendering and algorithms convert to `kurbo::BezPath`. Coordinates are document points (1/72 in), y pointing down.

```rust
pub struct PathData { pub subpaths: Vec<SubPath> }
pub struct SubPath { pub anchors: Anchors, pub closed: bool }
pub struct Anchor { pub p: Point, pub h_in: Point, pub h_out: Point, pub kind: AnchorKind }
```

### The serde form is a published format

`Anchor` serialises through `AnchorRepr`: `p` as `[x, y]`, `in` and `out` omitted when they equal `p`, `kind` omitted at its default. `.vectorcraft` documents store anchors in this form (its v2-and-later anchor form), and VectorCraft's command API parses it. RFC-0030's compatibility section freezes it through the cutover: a change to it is a file format version with a migration, never a side effect of an edit here.

### Hostile input is bounded

Serde reads of paths go through an anchor budget (`with_anchor_budget`, `anchors_within`, `MAX_READ_ANCHORS`: 32M anchors native, 4M on wasm32), so a small compressed file of repeated anchors cannot expand past what the web build can address. `flatten` bounds the segments per curve (`MAX_FLATTEN_PER_CURVE`) and draws absurd-coordinate curves as lines instead of letting kurbo size unbounded buffers. House rule 1 (never crash, treat input as hostile) requires both.

## Modules

- `path`: `PathData`, `SubPath`, `Anchor`, `AnchorKind`, `FillRule`, the serde form and the read budget.
- `anchors`: the chunked copy-on-write `Anchors` storage behind `SubPath`.
- `shapes`: shape generators (rectangle, ellipse, polygon, star, line and friends).
- `hit`: hit testing (`fill_contains`, `stroke_contains`, `distance_to_outline`).
- `corners`: live corners (fillet and chamfer on anchors).
- `recognize`: shape recognition (does this path draw a rectangle, an ellipse, a line).
- `arc`: arc-length parameterisation (`ArcPath`), for text on a path and even spacing.
- `bez`: Bézier utilities shared by stroke geometry and hit testing.
- `snap`: grid and pixel snapping math (geometry only; interactive smart guides stay app work until RFC-0030 step 5).
- `projective`: homographies (`Homography`), for perspective mapping.

The crate root carries `flatten`, angle constraints (`constrain_angle`, `constrain_axes`, `rotate_deg`) and small rect helpers, and re-exports `kurbo`.

## Dependencies

`kurbo`, `serde` and `thiserror` only: no UI, no I/O, no app types. Tests take `craft-testkit` with its `geom` feature from `tests/` only.
