# craft-mesh-ops

Operations on triangle meshes, for apps that paint, bake or preview materials on 3D models. The
crate has no UI, does no file or network I/O and has no `unsafe` code. File formats stay in
[`craft-mesh`](../mesh/README.md).

- **`Mesh`.** Positions, normals, tangents, UVs, vertex colors, indices, per-material submeshes
  and polygon-diagonal flags, with `validate`, `append`, `subdivided`, `normalize_to`, wireframe
  and UV-seam edges, and UV-tile helpers. `from_parts` rejects non-finite positions and invalid
  index ranges before tangent generation.
  With a complete normal array, tangent generation repairs non-finite normal entries and retains
  finite entries.
  `normalize_to` centers and scales in `f64` before storing the resulting positions.
  Subdivision preserves equal subnormal position, UV, and color components at edge midpoints.
  Midpoint normals and tangent directions are normalized before conversion to `f32`.
- **Normals and tangents.** Smooth normals are area weighted. Tangents come from MikkTSpace, which
  glTF 2.0 recommends for meshes without tangents
  (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>): `w` holds the bitangent sign.
  A vertex whose triangles have different tangent frames (mirrored UVs) is split into one copy
  per frame. Faces with coincident corners stay in the mesh and reuse shared vertex frames when
  available. MikkTSpace omits those faces. A mesh with more than 2,048 noncollapsed triangles around one vertex, or with non-finite
  positions or UVs, or with positions or UVs above 1e6 or below 1e-6 in magnitude, gets per-vertex tangents from UV gradients instead of MikkTSpace. Tangents
  are always finite. Non-finite normal entries are recomputed; finite entries are retained.
- **UV unwrapping.** `unwrap::auto_unwrap` makes a non-overlapping layout per material, splitting
  vertices where charts meet. It checks the mesh against its size limits and its attribute data
  first and returns an error without changing the mesh when a check fails. Charts are projected
  and packed in `f64`. Overlapping projected interiors get separate charts. Padding uses the
  requested resolution, with a minimum resolution of 16. A layout that cannot fit returns an
  error without changing the mesh. `auto_unwrap` also returns an error without changing the mesh
  if stored UVs collapse a nondegenerate face or lose requested border padding.
  Padding must be finite and nonnegative. Integer resolutions remain exact during packing.
  Missing normals use area-weighted `f64` accumulation and normalization.
- **UV G-buffer.** `UvGBuffer` records, for each texel of one material's UV space, the triangle
  and barycentrics under the texel center. `surface` reads its position, normal and tangent frame.
  `surface_at` interpolates a triangle at supplied barycentric weights.
  `surface_at` retains positive interior weights. When nonnegative supplied `f32` weights sum
  to `1.0` in `f32` arithmetic, a negative opposite corner weight after widening is clamped to zero. Small offsets
  along that edge are preserved.
  `bary` stores compact weights of vertices 1 and 2 as `Vec2` for each texel. `barycentrics`
  computes all three corner weights as `[f64; 3]` on demand. Tolerated outside samples use the
  triangle boundary. `overlaps` counts each texel
  once when distinct triangles with overlapping interiors cover its center.
  `surface` retains local attribute offsets between far UV corners. Its unit tangents are
  perpendicular to the interpolated normals.
  `raster_uvs` stores vertex IDs and UV corners for triangle IDs recorded in `tri`. Other triangle IDs
  are absent. The raster mapping stays fixed when the mesh's UVs change or its triangle corners
  are reordered. `surface` returns `None` when a triangle's vertex membership changes or its
  interpolated positions or UVs are non-finite.
  `bary` keeps the corner order used during rasterization. Use `surface` after reordering corners.
- **Primitives.** Sphere, rounded box, cube, cylinder, torus and plane with UVs and tangents, and
  the built-in scenes in `primitives::BUILTINS`. `primitives::cube` and `primitives::rounded_box`
  return `Result<Mesh>`. They reject non-finite or nonpositive dimensions and dimensions whose
  doubled half extents overflow. A box bevel must be finite and between zero and 95% of its
  smallest half extent; an invalid bevel returns `GeomError::Invalid`.
  Sphere and torus seam endpoints have identical positions and retain distinct UV endpoints.
  Cylinder caps share exact rim positions with the side, and rounded-box faces share exact boundary positions.
  Lathe profile lengths and direction normalization use `f64` before the UVs and normals are stored as `f32`.
- **BVH.** `Bvh` is a binned SAH tree on all three axes with a median split when no split beats a
  leaf. It answers closest-hit, any-hit and filtered ray casts, nearest-point queries and box
  queries. Ray tests use double precision and exact signs near triangle edges. Slab intervals
  include a floating-point rounding margin.
  `Bvh::nearest` keeps projected points in `f64` through the distance comparison.
- **Camera.** `OrbitCamera` and `Ray` give an orbit view and picking rays. `frame` fits a square
  viewport; `frame_with_aspect` fits a width-to-height aspect greater than 0.0001 and returns an
  error when the aspect or framed view is invalid.

`GeomError` reports invalid data and allocation failures. `Mesh::try_append` returns these errors;
the compatibility method `Mesh::append` discards them and leaves the mesh unchanged on error.
`rayon` parallelizes the UV G-buffer, and `serde`
serializes `OrbitCamera`; both are always on.

## Origin

| Part | Source |
|---|---|
| `mesh.rs`, `bvh.rs`, `primitives.rs`, `gbuffer.rs`, `unwrap.rs`, `camera.rs` and their tests | TextureCraft `crates/geom` at [386630e](https://github.com/storytold/texturecraft/commit/386630ee85cb22679295b24013575d1aecb39ad2) |
| `Mesh::subdivided` and `subdivision_shares_edge_midpoints` | MaterialCraft `crates/geom/src/mesh.rs` at [3133062](https://github.com/storytold/materialcraft/commit/3133062968b8a1566eda12a357cb06062db16687) |

MikkTSpace comes from the `mikktspace` crate; its notice is in [ATTRIBUTION.md](ATTRIBUTION.md).

## Migration

| App | Status |
|---|---|
| TextureCraft | `texturecraft-geom` is the source. The app switches to this crate in its own pull request. |
| MaterialCraft | `materialcraft-geom` is the source of `subdivided`. The app switches in its own pull request. |
