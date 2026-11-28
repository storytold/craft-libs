# craft-mesh-ops

Operations on triangle meshes, for apps that paint, bake or preview materials on 3D models. The
crate has no UI, does no file or network I/O and has no `unsafe` code. File formats stay in
[`craft-mesh`](../mesh/README.md).

- **`Mesh`.** Positions, normals, tangents, UVs, vertex colors, indices, per-material submeshes
  and polygon-diagonal flags, with `validate`, `append`, `subdivided`, `normalize_to`, wireframe
  and UV-seam edges, and UV-tile helpers.
- **Normals and tangents.** Smooth normals are area weighted. Tangents come from MikkTSpace, which
  glTF 2.0 recommends for meshes without tangents
  (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>): `w` holds the bitangent sign.
- **UV unwrapping.** `unwrap::auto_unwrap` makes a non-overlapping layout per material, splitting
  vertices where charts meet.
- **UV G-buffer.** `UvGBuffer` records, for each texel of one material's UV space, the triangle
  and barycentrics under the texel center; `surface_at` turns them into a position, normal and
  tangent frame.
- **Primitives.** Sphere, rounded box, cube, cylinder, torus and plane with UVs and tangents, and
  the built-in scenes in `primitives::BUILTINS`.
- **BVH.** `Bvh` is a binned SAH tree on all three axes with a median split when no split beats a
  leaf. It answers closest-hit, any-hit and filtered ray casts, nearest-point queries and box
  queries.
- **Camera.** `OrbitCamera` and `Ray` give an orbit view and picking rays.

`GeomError::Invalid` is the one error type. `rayon` parallelizes the UV G-buffer, and `serde`
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
