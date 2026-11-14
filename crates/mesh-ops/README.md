# craft-mesh-ops

Operations on triangle meshes, for apps that paint, bake or preview materials on 3D models. The
crate has no UI, does no file or network I/O and has no `unsafe` code. File formats stay in
[`craft-mesh`](../mesh/README.md).

- **Slice functions.** `bounds`, `smooth_normals`, `tangents`, `weld` and `unwrap_uvs` at the crate
  root, `UvGBuffer::from_slices` and `Bvh::from_slices` take plain slices: positions and normals
  as `[f32; 3]`, UVs as `[f32; 2]`, triangle indices as `u32`. They check the counts against
  `Limits`, every index and every value (finite) before any work, and return a `MeshOpsError`.
  Values are `f32`; a caller that holds `f64` converts first and rejects values outside the `f32`
  range.
- **Normals and tangents.** Smooth normals are area weighted. Tangents come from MikkTSpace, which
  glTF 2.0 recommends for meshes without tangents
  (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>): `w` holds the bitangent sign, and
  UVs have `(0, 0)` at the upper-left corner with `v` increasing downward. A mesh MikkTSpace
  cannot handle (every triangle degenerate, no UV area) gets per-triangle tangents. When the
  largest coordinate is above 1e6 or below 1e-6 in magnitude, normals and tangents are computed
  from the positions divided by it, and finite input gives finite output. UVs passed to
  `tangents` are divided the same way, and normals when their largest component is above 2 or
  below 0.5. Normals are unit vectors, as glTF 2.0 stores them.
- **UV unwrapping.** `unwrap_uvs` and `unwrap::auto_unwrap` make a non-overlapping layout per
  material, splitting vertices where charts meet. The layout is the same at every scale: outside
  1e-6 to 1e6 the charts are made from the positions divided by the largest coordinate. `weld`
  gives vertices at the same position (on a grid of 1e-6 of the bounding diagonal) the same id,
  numbered in order of first appearance.
- **UV G-buffer.** `UvGBuffer` records, for each texel of one material's UV space, the triangle
  and barycentrics under the texel center; `surface_at` turns them into a position, normal and
  tangent frame. A triangle with a corner more than 1e18 texels from the UV origin is skipped.
- **Primitives.** `sphere`, `rounded_box`, `cube`, `cylinder`, `torus` and `plane`, and the
  built-in scenes in `primitives::BUILTINS`, all with UVs and tangents. The `try_` forms return an
  error for a non-finite or non-positive size, a size so large that a coordinate overflows `f32`,
  or a segment count over `Limits::max_segments`; the other forms clamp the segment count and
  return an empty mesh for an invalid size.
- **BVH** (feature `bvh`). Binned SAH on all three axes, with a median split on the widest
  centroid axis when no split beats a leaf. Closest-hit, any-hit and filtered ray casts, the
  nearest point within a distance, and box queries. Triangles with a non-finite corner are left
  out of the tree. A ray with a non-finite origin or direction, a zero direction or a NaN
  distance, a NaN box and a negative or NaN search distance find nothing. Ray casts find hits on
  triangles with coordinates between about 1e-12 and 1e12; outside that range the products in
  the triangle test overflow or underflow `f32`. The nearest-point
  search does not find a point more than about 1.8e19 away, where the squared distance overflows
  `f32`.
- **`Mesh`.** Positions, normals, tangents, UVs, vertex colors, indices, per-material submeshes
  and polygon-diagonal flags, with `validate`, `append` and `try_append`, `subdivided` and
  `try_subdivided`, wireframe and UV-seam edges, and UV-tile helpers. `OrbitCamera` and `Ray`
  give an orbit view and picking rays; `OrbitCamera::frame` ignores a box whose center or framing
  distance is not finite.

Triangulation with holes, extrusion of 2D outlines and conversion from `craft_mesh::Mesh` are not
in the crate yet.

## Cargo features

No default features.

| Feature | Provides | Pulls in |
|---|---|---|
| `bvh` | `Bvh`, `Hit` | nothing |
| `parallel` | the UV G-buffer rasterizes bands of rows on several threads; the result is the same | `rayon` |
| `serde` | `Serialize` and `Deserialize` for `OrbitCamera` | `serde`, `glam/serde` |

Without `parallel` the crate builds for `wasm32-unknown-unknown`.

## Limits

| Field | Default | Checked by |
|---|---|---|
| `max_vertices` | 20,000,000 | every slice function, `try_subdivided`, the `try_` primitives, `UvGBuffer::try_new`, `unwrap::auto_unwrap` (input and output) |
| `max_triangles` | 20,000,000 | the same except `auto_unwrap`, and `Bvh::from_slices`; the UV G-buffer also counts a triangle again each time another submesh of the material lists it |
| `max_segments` | 2,048 | the `try_` primitives (segments, rings or subdivisions along one direction, after a count under the primitive's minimum is raised to it; a box face has 12, or 22 with a bevel) |
| `max_gbuffer_side` | 8,192 | `UvGBuffer::try_new`, `UvGBuffer::from_slices`; `UvGBuffer::new` clamps each side to `1..=8192` |
| `max_gbuffer_texels` | 2^26 (8,192 × 8,192) | the same |
| `max_gbuffer_work` | 2^32 | the same: the texels the rasterizer visits, summed over each triangle's bounding box clipped to the buffer |
| `max_gbuffer_bin_entries` | 2^26 | the same: one entry per triangle for each band of 32 rows its clipped box reaches |
| `max_unwrap_triangles` | 4,000,000 | `unwrap_uvs` and `unwrap::auto_unwrap`, for the mesh's triangles and for the triangles its submeshes list |

A BVH holds at most `bvh::MAX_BVH_TRIANGLES` (2^31 − 1) triangles.

`Bvh::from_slices`, `Bvh::try_new`, `UvGBuffer::try_new` and `UvGBuffer::from_slices` reserve all
their memory before using it, and the other slice functions reserve the arrays they return; each
returns `MeshOpsError::OutOfMemory` when that fails. `Bvh::new` then returns an empty tree and
`UvGBuffer::new` a 1 × 1 uncovered buffer. `UvGBuffer::new` also returns that buffer when the
input is over a default limit. The UV G-buffer checks its work and band entries in a first pass
over the triangles, before it reserves its band lists (in one piece) or its texel buffers, so a
request over those limits returns `MeshOpsError::TooLarge`. `weld`,
`unwrap::weld_ids`, the unwrap functions and `Mesh::try_subdivided` reserve the arrays and tables
they build and return `OutOfMemory` when a reservation fails. The working memory of
normals and tangents (MikkTSpace) is a fixed multiple of the input the limits allow and is not
reserved that way.

## Origin

| Part | Source |
|---|---|
| `mesh.rs`, `bvh.rs`, `primitives.rs`, `gbuffer.rs`, `unwrap.rs`, `camera.rs` and their tests | TextureCraft `crates/geom` at [386630e](https://github.com/storytold/texturecraft/commit/386630ee85cb22679295b24013575d1aecb39ad2) |
| `Mesh::subdivided` and `subdivision_shares_edge_midpoints` | MaterialCraft `crates/geom/src/mesh.rs` at [3133062](https://github.com/storytold/materialcraft/commit/3133062968b8a1566eda12a357cb06062db16687) |
| The index check in `Mesh::from_parts` and `compute_tangents`, and its two tests | TextureCraft [#386](https://github.com/storytold/texturecraft/pull/386) by simonhull, at `faa4548a` |
| `Limits`, `MeshOpsError::TooLarge` and `OutOfMemory`, the slice functions, the `try_` forms, the finite-value and overflow checks, the `bvh`, `parallel` and `serde` features | written here |

MikkTSpace comes from the `mikktspace` crate; its notice is in [ATTRIBUTION.md](ATTRIBUTION.md).

## Migration

| App | Status |
|---|---|
| TextureCraft | `texturecraft-geom` is the source. The app switches to this crate in its own pull request. |
| MaterialCraft | `materialcraft-geom` is the source of `subdivided`. The app switches in its own pull request. |

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
| `Mesh::validate` | also rejects a submesh that does not start and end on a triangle, and a non-finite normal, UV, tangent or color |
| `Mesh::subdivided` with an invalid mesh | returns a copy; `try_subdivided` returns the error |
| `primitives::*` with a NaN or negative size | an empty mesh; the `try_` forms return the error |
| `auto_unwrap` with a non-finite position, normal or color, a bad submesh or colors for some vertices only | an error, with the mesh unchanged |
| `Bvh::nearest` skips a point exactly `max_dist` away | the point is found |
| Rays miss a triangle whose two edge lengths multiply to under 1e-12 | the cutoff follows the triangle's size, so the ray hits |
| `Mesh::normalize_to` with a radius that overflows a coordinate | the mesh is unchanged |
| `Mesh::normalize_to` leaves a mesh with a radius under 1e-4 too small | the mesh gets the requested radius |
| `weld_ids` on a grid of 1e-12 when the bounding diagonal is under 1e-6 | a grid of 1e-6 of the diagonal at every size |
| `OrbitCamera::frame` with a box whose center or distance is not finite | the camera is unchanged |
