# craft-mesh-ops

Operations on triangle meshes, for apps that paint, bake or preview materials on 3D models. The
crate has no UI, does no file or network I/O and has no `unsafe` code. File formats stay in
[`craft-mesh`](../mesh/README.md).

- **Slice functions.** `bounds`, `smooth_normals`, `flat_normals`, `tangents`, `weld` and
  `unwrap_uvs` at the crate root, `UvGBuffer::from_slices` and `Bvh::from_slices` take plain slices: positions and normals
  as `[f32; 3]`, UVs as `[f32; 2]`, triangle indices as `u32`. They check the counts against
  `Limits`, every index and every value (finite) before any work, and return a `MeshOpsError`.
  Values are `f32`; a caller that holds `f64` converts first and rejects values outside the `f32`
  range.
- **Normals and tangents.** Smooth normals are area weighted: each triangle adds the cross
  product of its edges, twice its area times its unit normal, to its three vertices, and each sum
  is brought to unit length, the weighting described in Caltech's CS 171 notes
  (<https://courses.cms.caltech.edu/cs171/assignments/hw5/hw5-notes/notes-hw5.html>). Vertices are
  told apart by index, so the two copies of a vertex on a UV seam keep their own normals. The ids
  `weld` returns are dense, so a caller that wants one normal across the seam builds one position
  per id (the position of the first vertex with that id), passes those positions with each
  triangle index replaced by its vertex's id, and copies each id's normal back. A vertex no
  triangle uses, or whose sum is zero, gets +Y.
  `flat_normals` and `Mesh::try_flat_normals` give every triangle corner its own vertex with the
  triangle's unit normal, which glTF 2.0 asks a loader to compute when a mesh has no normals; a
  triangle without area gets +Z. `flat_normals` returns the input vertex each output vertex
  copies, so a caller can carry UVs, colors, joints and weights over; `Mesh::try_flat_normals`
  carries UVs and colors, keeps submeshes and polygon diagonals, and computes the tangents again
  when the mesh had them. `Mesh::wire_edges` matches a diagonal's twin by position and UV, so the
  diagonals stay hidden after `try_flat_normals` gives each triangle its own vertices. Both
  builders take the cross products in `f64`, from the corner opposite the longest edge, so a
  triangle with a side of 1e-15 or 1e20, or a short side next to corners 2^80 apart, keeps its
  normal. `Mesh::try_compute_normals` and `try_flat_normals`
  return `Invalid` for an index count that is not a multiple of 3, an index past the last vertex
  or a non-finite position, with the mesh unchanged; `compute_normals` skips a triangle with an
  index out of range. Tangents come from MikkTSpace, which
  glTF 2.0 recommends for meshes without tangents
  (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>): `w` holds the bitangent sign, and
  UVs have `(0, 0)` at the upper-left corner with `v` increasing downward. A mesh MikkTSpace
  cannot handle (every triangle degenerate, no UV area, more than
  `mesh::MAX_MIKKTSPACE_VALENCE` (2,048) triangles around one vertex, or so many corners at one
  position that welding them would take more than `mesh::MAX_MIKKTSPACE_WELD_WORK` (2^27)
  comparisons) gets per-vertex tangents, with `w` set as MikkTSpace sets it.
  MikkTSpace gives each triangle corner its own frame: `Mesh::compute_tangents` splits a vertex
  whose corners get different frames (UVs mirrored across it), and `tangents` returns an
  `Invalid` error for such a vertex, since it returns one tangent per input vertex. When the
  largest coordinate is above 1e6 or below 1e-6 in magnitude, normals and tangents are computed
  from the positions divided by it, and finite input gives finite output. UVs passed to
  `tangents` are divided the same way, and normals when their largest component is above 2 or
  below 0.5. Normals are unit vectors, as glTF 2.0 stores them.
- **UV unwrapping.** `unwrap_uvs` and `unwrap::auto_unwrap` make a non-overlapping layout per
  material, splitting vertices where charts meet. An initial packing resolution above 65,536
  texels per side returns `MeshOpsError::Invalid` without changing the mesh. Padding is measured
  at the requested resolution. A layout that cannot fit returns an error without changing the
  mesh. A face joins a chart only when its projection
  onto the chart's plane does not overlap the chart's other faces, and every chart is packed at
  a scale above zero. The charts are projected, measured and packed in `f64` from the positions
  as given, so a small chart far from the origin keeps its shape. A mesh whose submeshes put one
  triangle in two materials is an `Invalid` error. `weld`
  gives vertices at the same position (on a grid of 1e-6 of the bounding diagonal) the same id,
  numbered in order of first appearance.
- **UV G-buffer.** `UvGBuffer` records, for each texel of one material's UV space, the triangle
  and barycentrics under the texel center; `surface_at` turns them into a position, normal and
  tangent frame. A triangle with a corner more than 1e18 texels from the UV origin is skipped.
  A texel center counts as inside a triangle when it is no more than 1e-6 texel outside each
  edge, plus the `f64` rounding of the edge function at the triangle's coordinates, and at most
  1e-3 of the triangle's smallest height.
- **Primitives.** `sphere`, `rounded_box`, `cube`, `cylinder`, `torus` and `plane`, and the
  built-in scenes in `primitives::BUILTINS`, all with UVs and tangents. The `try_` forms return an
  error for a non-finite or non-positive size, a size so large that a coordinate overflows `f32`,
  or a segment count over `Limits::max_segments`; the other forms clamp the segment count and
  return an empty mesh for an invalid size.
- **BVH** (feature `bvh`). Binned SAH on all three axes, with a median split on the widest
  centroid axis when no split beats a leaf. Closest-hit, any-hit and filtered ray casts, the
  nearest point within a distance, and box queries. Triangles with a non-finite corner are left
  out of the tree. A ray with a non-finite origin or direction, a zero direction or a NaN
  distance, a NaN box and a negative or NaN search distance find nothing. Ray casts and the
  nearest-point search work in `f64` at any finite `f32` coordinates. The ray test is watertight
  (Woop, Benthin and Wald, 2013): a ray through an edge that two triangles share hits at least
  one of them, and the test has no slack around the edges. Box exit distances are scaled past
  their rounding error, so a ray that touches a box where it enters on one axis and leaves on
  another still reaches its triangles. A triangle whose corners lie on one line is never hit, and
  `nearest` still measures it as a segment. A hit closer than the least positive
  `f32` is reported at that distance. The nearest-point search does not
  find a point more than about 1.8e19 away, where the squared distance overflows `f32`.
- **`Mesh`.** Positions, normals, tangents, UVs, vertex colors, indices, per-material submeshes
  and polygon-diagonal flags, with `validate`, `append` and `try_append`, `subdivided` and
  `try_subdivided`, wireframe and UV-seam edges, and UV-tile helpers. `OrbitCamera` and `Ray`
  give an orbit view and picking rays; `OrbitCamera::frame` ignores a box whose center or framing
  distance is not finite, and `OrbitCamera::zoom` ignores a distance at which the eye rounds to
  the target. `OrbitCamera::ray` unprojects in `f64`.

- **Shapes.** `primitives::try_shape` builds a `ShapeKind` (cuboid, sphere, plane, torus, cone
  or cylinder) from `ShapeParams`, with each face or surface mapped to the whole UV square: a
  cuboid of separate width, height and depth, a plane in XY facing +Z, a sphere with exact poles
  and one triangle per segment at each pole, a ring torus, and a cone and cylinder whose caps
  each take the whole square. It returns `Invalid` for a size, radius, tube or height the shape
  reads that is not finite and positive, and for a torus tube that is not smaller than its
  radius; a segment count over `Limits::max_segments` is `TooLarge`. The cone's side normal is
  worked out in `f64`, so a cone of height 1e-9 and radius 1e-6 keeps it.
- **Triangulation.** `triangulate::triangulate` takes an outer contour and its holes as `[f64; 2]`
  points and returns `u32` triangles over the contours concatenated in order, counterclockwise
  with y up. Holes are joined to the outer contour by bridges and ears are clipped, following
  Eberly's *Triangulation by Ear Clipping*
  (<https://www.geometrictools.com/Documentation/TriangulationByEarClipping.pdf>); a vertex where
  the contour goes straight on becomes a triangle without area, so every contour edge belongs to
  exactly one triangle. It returns `Invalid` for a contour with fewer than three points, a
  non-finite or repeated point, a contour without area, edges that cross or touch, a hole outside
  the outer contour or inside another hole, a bounding box wider or taller than 1e150, and a
  polygon with no ear left to clip. Coordinates are used as given, and every test of which side
  of a line a point lies on takes the exact sign of the cross product, so a contour far from the
  origin keeps its shape and points far apart in scale stay apart. The sign is exact unless a
  product of two coordinate differences, or its rounding error, falls below the normal `f64`
  range (differences near 1e-154 or smaller); such a cross product can round to 0. `triangulate::signed_area`
  takes its terms from the contour's lowest point, so the area does not depend on where the
  contour starts.
- **Outlines.** `outline::group_contours` groups closed contours into `Outline`s by nesting depth
  (a hole goes to the innermost containing outer contour; overlapping contours each count as a
  container) after removing repeated points and dropping contours of area 1e-9 or less. Contours
  that together span more than 1e150 in x or y are `Invalid`. A contour that touches a larger
  one still counts as inside it; `extrude` rejects a hole that touches its outer contour, so such
  a hole has to be moved off it or merged into it first. `outline::extrude` builds a beveled solid from outlines in a y-down plane: the front cap at
  z = 0 facing -z, the back cap at the extrusion depth facing +z, walls between them and a
  `BevelStyle` (none, angular, concave or convex) at both edges. `extrude_surfaces` splits the
  result into front, bevel, side and back meshes, and the `_stacked` forms move it by
  `outline::STACK_GAP` (0.02) per level towards -z before the conversion to `f32`. Caps and walls share positions, so the solid
  is closed when welded by position. `ExtrudeParams` with a depth that is not finite and positive,
  a bevel depth that is not finite and 0 or more, or a hole bevel outside 0 to 1 is `Invalid`, as
  is a bevel that collapses or turns a cap's contour inside out, a point outside the `f32`
  range, and neighboring points that round to one `f32` position (a unit square near x = 1e8,
  where `f32` steps by 8), cap or wall contours that cross or touch once rounded to `f32`, a
  depth that rounds to 0 in `f32`, and a bevel step that rounds to no width in `f32`. A bevel
  deeper than half the extrusion is built at half the extrusion.

Conversion from `craft_mesh::Mesh` is not in the crate yet.

## Cargo features

No default features.

| Feature | Provides | Pulls in |
|---|---|---|
| `bvh` | `Bvh`, `Hit` | nothing |
| `parallel` | the UV G-buffer rasterizes bands of rows on several threads; the result is the same | `rayon` |
| `serde` | `Serialize` and `Deserialize` for `OrbitCamera` and `outline::BevelStyle` | `serde`, `glam/serde` |

Without `parallel` the crate builds for `wasm32-unknown-unknown`.

## Limits

| Field | Default | Checked by |
|---|---|---|
| `max_vertices` | 20,000,000 | every slice function, `flat_normals` and `Mesh::try_flat_normals` (input and output: one vertex per index), `try_subdivided`, the `try_` primitives (also after their tangents split vertices), `UvGBuffer::try_new`, `unwrap::auto_unwrap` (input and output), `Mesh::map_material_uvs` (the mesh's vertices and the copies of the vertices it shares with other materials) |
| `max_triangles` | 20,000,000 | the same except `auto_unwrap`, and `Bvh::from_slices`; the UV G-buffer also counts a triangle again each time another submesh of the material lists it; `Mesh::wire_edges`, `Mesh::uv_seam_edges`, `Mesh::uv_tiles` and `Mesh::map_material_uvs` count the triangles the submeshes list in the same way |
| `max_segments` | 2,048 | the `try_` primitives (segments, rings or subdivisions along one direction; a count under the primitive's minimum is `Invalid`, and the builders without `try_` raise it; a box face has 12, or 22 with a bevel) |
| `max_gbuffer_side` | 8,192 | `UvGBuffer::try_new`, `UvGBuffer::from_slices`; `UvGBuffer::new` clamps each side to `1..=8192` |
| `max_gbuffer_texels` | 2^26 (8,192 × 8,192) | the same |
| `max_gbuffer_work` | 2^32 | the same: the texels the rasterizer visits, summed over each triangle's bounding box clipped to the buffer |
| `max_gbuffer_bin_entries` | 2^26 | the same: one entry per triangle for each band of 32 rows its clipped box reaches |
| `max_unwrap_triangles` | 4,000,000 | `unwrap_uvs` and `unwrap::auto_unwrap`, for the mesh's triangles and for the triangles its submeshes list |
| `max_outline_vertices` | 1,000,000 | `triangulate` (the polygon's points), `group_contours` and `group_largest_contours` (every contour's points), `extrude` and its forms (every outline's points) |
| `max_outline_contours` | 10,000 | `triangulate` and `extrude` (contours in one polygon), `group_contours` (contours left after dropping; `group_largest_contours` keeps the largest instead) |
| `max_outline_work` | 2^30 | `triangulate`, the grouping and the extrusion: edge pairs compared for crossings, points tested for containment, and vertices scanned while clipping ears, summed over the call |

A BVH holds at most `bvh::MAX_BVH_TRIANGLES` (2^31 − 1) triangles.

`Bvh::from_slices`, `Bvh::try_new`, `UvGBuffer::try_new` and `UvGBuffer::from_slices` reserve all
their memory before using it. The other slice functions, `weld`, `Mesh::try_compute_normals`,
`Mesh::try_flat_normals`, `Mesh::try_compute_tangents`, `Mesh::try_subdivided`, the `try_` primitives, `unwrap::weld_ids`
and the unwrap functions reserve their working memory and their results as they grow. `triangulate`,
the grouping and `extrude` reserve their points, rings and triangles before filling them; `extrude`
counts its output and checks it against `max_vertices` and `max_triangles` before it builds. Each
returns `MeshOpsError::OutOfMemory` when a reservation fails. `Bvh::new` then returns an empty
tree, `UvGBuffer::new` a 1 × 1 uncovered buffer, and `compute_normals` and `compute_tangents`
leave the mesh unchanged. `UvGBuffer::new` also returns that buffer when the input is over a
default limit. The UV G-buffer checks its work and band entries in a first pass over the
triangles, before it reserves its band lists (in one piece) or its texel buffers, so a request
over those limits returns `MeshOpsError::TooLarge`. The working memory of the `mikktspace` crate
is allocated inside it and is not reserved this way; it is a fixed multiple of the input the
limits allow.

## Origin

| Part | Source |
|---|---|
| `mesh.rs`, `bvh.rs`, `primitives.rs`, `gbuffer.rs`, `unwrap.rs`, `camera.rs` and their tests | TextureCraft `crates/geom` at [386630e](https://github.com/storytold/texturecraft/commit/386630ee85cb22679295b24013575d1aecb39ad2) |
| `Mesh::subdivided` and `subdivision_shares_edge_midpoints` | MaterialCraft `crates/geom/src/mesh.rs` at [3133062](https://github.com/storytold/materialcraft/commit/3133062968b8a1566eda12a357cb06062db16687) |
| The index check in `Mesh::from_parts` and `compute_tangents`, and its two tests | TextureCraft [#386](https://github.com/storytold/texturecraft/pull/386) by simonhull, at `faa4548a` |
| `triangulate.rs`, `outline.rs`, `try_shape` and their moved tests | EffectCraft `crates/model/src/triangulate.rs`, `extrude.rs`, `prim.rs` and `tests.rs` at [fae7ad7](https://github.com/storytold/effectcraft/commit/fae7ad76412f29b104a4f070373020f70bd9b479) |
| `Limits`, `MeshOpsError::TooLarge` and `OutOfMemory`, the slice functions, the `try_` forms, the finite-value and overflow checks, the outline checks and limits, the `bvh`, `parallel` and `serde` features | written here |

MikkTSpace comes from the `mikktspace` crate; its notice is in [ATTRIBUTION.md](ATTRIBUTION.md).

The crate takes `mikktspace` with `default-features = false, features = ["glam"]`. The `glam`
and `nalgebra` features of `mikktspace` 0.3 do not build together, and Cargo merges the features
that every crate in a build asks for. An app that also depends on `mikktspace` itself must ask
for the same features; TextureCraft and MaterialCraft already do. `cargo tree -e features -i
mikktspace` in the app lists the `glam` feature and no `nalgebra` or `default` feature.

## Migration

| App | Status |
|---|---|
| TextureCraft | `texturecraft-geom` is the source. The app switches to this crate in its own pull request. |
| MaterialCraft | `materialcraft-geom` is the source of `subdivided`. The app switches in its own pull request. |
| EffectCraft | `effectcraft-model`'s `Primitive::compute_normals` and `flat_normals` give the same normals as `smooth_normals` and `flat_normals`, and its `triangulate`, `extrude` and `prim` modules the same geometry as `triangulate`, `outline` and `try_shape`, apart from the rows below. The app switches in its own pull request. |

For both apps:

| `*-geom` | craft-mesh-ops |
| --- | --- |
| `GeomError::Invalid` | `MeshOpsError::Invalid`, plus `TooLarge` and `OutOfMemory` |
| `BaseColor` | not moved; it stays in the app |
| `Bvh`, `Hit` always built | the `bvh` feature |
| `rayon` and `serde` always on | the `parallel` and `serde` features |
| `Mesh::bounds` with a NaN position | NaN positions are left out of the box |
| `Mesh::append` with a mismatched attribute | the missing normals, UVs or tangents are filled in; `try_append` returns an error on overflow or a bad index |
| `Mesh::append` with submeshes and no material list on either side | that side gets one unnamed material |
| `Mesh::uv_tiles` with a UV centroid whose tile is outside the `i32` range | `Invalid`, not a saturated tile |
| `UvGBuffer::try_new` with a trailing index, or a corner of the material's triangles without a vertex, a finite UV, a normal or a tangent, or with a non-finite position, normal or tangent | `Invalid`, not a buffer without those triangles; `UvGBuffer::new` returns the 1 × 1 uncovered buffer |
| `Mesh::append` with a trailing index, a submesh that does not start and end on a triangle, or a submesh whose material is not listed | `try_append` returns `Invalid`, and `append` leaves the mesh unchanged |
| `Mesh::validate` | also rejects a submesh that does not start and end on a triangle, and a non-finite normal, UV, tangent or color |
| `Mesh::subdivided` with an invalid mesh | returns a copy; `try_subdivided` returns the error |
| `primitives::*` with a NaN or negative size | an empty mesh; the `try_` forms return the error |
| `auto_unwrap` with a non-finite position, normal or color, a bad submesh or colors for some vertices only | an error, with the mesh unchanged |
| `Bvh::nearest` skips a point exactly `max_dist` away | the point is found |
| Rays miss a triangle whose two edge lengths multiply to under 1e-12, and a shallow ray across a triangle | the watertight test in `f64` has no cutoff, so the ray hits |
| The ray test accepts points up to 1e-5 outside each edge | it accepts no point outside the triangle; a ray through an edge two triangles share still hits one of them |
| The slab test treats a direction component whose `f32` reciprocal overflows as never reaching the box | the reciprocal is taken in `f64`, and the hit is found |
| `Bvh::nearest` rounds the projected point and the squared distance to `f32`, and picks an edge for a point inside a very thin triangle | both stay in `f64`; the region of an inside point comes from cross products |
| A tree with every triangle left out keeps the memory reserved for them | the memory is released |
| The cylinder's caps start at +X while its side starts at +Z, leaving gaps at the rim | the caps use the side's rim points, and the cylinder is closed |
| The sphere and the material ball emit a triangle without area in each cell at a pole | one triangle per pole cell |
| `try_cylinder` accepts a height whose half is 0 in `f32` | an `Invalid` error |
| `try_torus` builds a tube so thin that its cross-section rounds flat in `f32` | an `Invalid` error; `try_shape` checks its torus the same way |
| The rounded box overflows `f32` while stepping across a face with a half size above about 1.4e37 | the steps are taken in `f64`; the box builds while its coordinates fit `f32` |
| `OrbitCamera::orbit` and `pan` can move the eye onto the target far from the origin | the camera is unchanged |
| `OrbitCamera::ray` inverts the view-projection in `f32`, which loses the direction far from the origin | it inverts it in `f64` |
| `Mesh::normalize_to` subtracts the center in `f32` | the center and scale are applied in `f64` |
| `Mesh::subdivided` adds the halves of two coordinates in `f32`, which rounds subnormal coordinates away | the midpoints are taken in `f64` |
| `Mesh::from_parts` accepts normals or UVs of another length than the positions | an `Invalid` error |
| `Mesh::map_material_uvs` checks the vertex count only | it also checks the triangle count |
| Face normals take the cross product in `f32` after subtracting one corner, which loses a short edge next to a long one | the cross product uses exact arithmetic where the fast one is within its rounding bound |
| The per-vertex tangent fallback sums every corner into one frame | corners whose UVs face opposite ways get separate frames, and their vertices are split |
| The UV G-buffer scales UVs in `f32` and drops triangles whose doubled area is under a fixed cutoff | it scales in `f64` and drops only triangles with no area; a submesh that does not start and end on a triangle is `Invalid` |
| `auto_unwrap` treats an overlap smaller than a face's height as contact | only the rounding at the coordinates' magnitude counts as contact |
| `auto_unwrap` returns a layout in which a face's stored `f32` UVs have no area | an `Invalid` error, with the mesh unchanged |
| `Mesh::normalize_to` with a radius that overflows a coordinate | the mesh is unchanged |
| `Mesh::normalize_to` leaves a mesh with a radius under 1e-4 too small | the mesh gets the requested radius |
| `weld_ids` on a grid of 1e-12 when the bounding diagonal is under 1e-6 | a grid of 1e-6 of the diagonal at every size |
| `OrbitCamera::frame` with a box whose center or distance is not finite | the camera is unchanged |
| `OrbitCamera::project` in orthographic mode returns points behind the camera | it returns `None` for them |
| The orthographic projection and `pan` use the field of view as it is | they clamp it to 5° to 150°, as the perspective projection does |
| `Mesh::compute_tangents` keeps the last corner's frame at a shared vertex | the vertex is split where its corners' frames differ |
| The per-vertex tangent fallback always sets `w` to +1 | `w` is -1 where the UVs are mirrored, as MikkTSpace sets it |
| `auto_unwrap` packs a triangle that submeshes of two materials list with only the last one | an `Invalid` error, with the mesh unchanged |
| `OrbitCamera::zoom` close to a target far from the origin gives a view that is not finite | the camera is unchanged |
| `Bvh::nearest` misses points inside very thin triangles and near a triangle with a zero-length edge | the nearest point is found |
| `Bvh::nodes` with no triangles lists one node | it lists none |
| `surface_at` and the rounded box replace a vector whose squared length overflows `f32` with a fallback direction | they keep its direction |
| The cylinder's side UVs shrink for a profile shorter than 1e-6, and a profile longer than `f32::MAX` is an error | the side UVs are the same at every size |
| `auto_unwrap` joins faces whose projections overlap into one chart | such a face starts another chart |
| `auto_unwrap` accepts a layout that fits only at scale zero | the packing square grows until the charts fit at a positive scale |
| `weld_ids` returns the ids | it returns `Result`, with `OutOfMemory` when the ids or the grid cannot be reserved |
| `Mesh::compute_normals` sums the cross products in `f32`, so a vertex of a triangle with sides of 1e-15 or 1e20 gets +Y | the sums are in `f64`, and the vertex keeps its normal |
| `Mesh::compute_normals` skips a triangle with an index out of range and ignores a trailing index | `compute_normals` still does; `try_compute_normals` returns `Invalid` for those and for a non-finite position |
| TextureCraft's mesh import welds vertices at one position before summing | `smooth_normals` keeps them apart; weld with `weld` first to share a normal across a seam |

For EffectCraft:

| `effectcraft-model` `Primitive` | craft-mesh-ops |
| --- | --- |
| `compute_normals` gives +Z to a vertex no triangle uses or whose sum is zero | `smooth_normals` gives +Y; `flat_normals` gives +Z to a triangle without area, as `Primitive::flat_normals` does |
| `compute_normals` and `flat_normals` panic on an index out of range | an `Invalid` error |
| `flat_normals` panics when the UVs, joints or weights are shorter than the positions | `flat_normals` returns `source`, and the caller copies each array through it; `Mesh::try_flat_normals` returns `Invalid` for short UVs or colors |
| The cross products are taken in `f32`, so a triangle with sides of 1e-15 or 1e20 gets +Z | they are taken in `f64`, and the triangle keeps its normal |
| `flat_normals` copies each vertex's tangent to its corners | `Mesh::try_flat_normals` computes the tangents again for the new normals |

| `effectcraft-model` `triangulate`, `extrude` and `prim` | craft-mesh-ops |
| --- | --- |
| `triangulate` returns `usize` triangles | `u32` triangles, with the contour count, point count and work checked against `Limits` |
| `triangulate` clips arbitrary triangles when no ear is left (a bow-tie, a crossing or touching hole, a hole outside the outer contour) and ignores a hole shorter than three points | an `Invalid` error |
| `triangulate` can join a hole to the copy of an earlier bridge that faces away from it, leaving no ear | only edges that run upward across the hole's ray count, so the hole joins the copy that faces it |
| `signed_area` multiplies absolute coordinates, so a contour far from the origin loses its area | the terms are taken from the contour's lowest point, so the area does not depend on where the contour starts |
| `group_contours` takes `Vec<Vec<[f64; 2]>>`, drops a contour with a NaN point, and keeps the 10,000 largest contours with a warning | it takes a slice of contours, returns `Invalid` for a non-finite point or a span over 1e150 and `TooLarge` for more than `max_outline_contours`; `group_largest_contours` keeps the largest |
| `extrude` raises a depth under 1e-3, or NaN, to 1e-3, clamps the hole bevel to 0..1, and builds no bevel for a NaN bevel depth or hole bevel | an `Invalid` error for a depth that is not finite and positive, a bevel depth that is not finite and 0 or more, and a hole bevel outside 0 to 1 |
| `extrude` builds a cap from a bevel that collapses or turns the contour inside out, and from an outline whose contours cross | an `Invalid` error |
| `extrude` converts to `f32` without a check, so a point past the `f32` range becomes infinite and neighboring points can round to one position | an `Invalid` error |
| `extrude` returns an `effectcraft_model::Primitive` with material 0, and `Surface` names the parts | an `OutlineMesh` of positions, normals, UVs and indices, and `OutlineSurface` names the parts; `BevelStyle` has no labels |
| `mesh(PrimitiveKind, &PrimParams)` takes absolute values of negative sizes and clamps segments to 3..=512 and rings to 2..=512 | `try_shape(ShapeKind, &ShapeParams, &Limits)` returns `Invalid` for a size that is not finite and positive or whose half is 0 in `f32`, for a shape whose triangle corners round to one `f32` position and for fewer than 3 segments, 2 sphere rings or 3 torus tube sides, and `TooLarge` past `max_segments` |
| `PrimitiveKind::Cube` | `ShapeKind::Cuboid` |
| A torus whose tube is as wide as its radius or wider | an `Invalid` error |
| A torus whose tube is so thin that its cross-section rounds flat in `f32` | an `Invalid` error |
| `group_contours` counts a larger contour as a container when it holds a contour's first point, so the result depends on where the contour starts | a larger contour is a container only when the whole contour lies inside it or on its edges |
| `group_contours` drops a contour whose area overflows to NaN | a contour wider than 1e150 is `Invalid` before its area is taken |
| `triangulate` shifts the points by their lowest corner, which can merge points of an outline that spans very different scales | the points are used as given |
| `triangulate` treats a turn whose cross product is under 1e-12 as straight | only an exact zero is straight, and every cross product's sign is exact unless its products or their rounding errors fall below the normal `f64` range |
| `triangulate` can fail to bridge a hole whose ray reaches a ring vertex exactly | the hole bridges to that vertex |
| `extrude` triangulates the caps at `f64` precision, so a cap triangle can face the wrong way once stored | the caps are triangulated from the `f32` positions the mesh stores; profile rings that round to one position, and bevel steps without `f32` width, are `Invalid` |
| The cone's side normal divides by at least 1e-6, so a cone lower than that tilts its normal | the slope is taken from the real height in `f64` |
| `compute_tangents` gives one tangent per vertex | MikkTSpace tangents; the cone's 15 inner base vertices (at 16 segments) are split where the two side triangles get different frames |
| `model` marks the plane's material double-sided, and `PrimitiveKind::label` and `parse` name the kinds | not moved; they stay in the app |
