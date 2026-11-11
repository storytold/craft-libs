# craft-mesh

3D mesh file formats behind one neutral model. A reader turns bytes into a `Scene`; a writer turns
a `Scene` into bytes. One Cargo feature per format, all on by default.

| Feature | Format | Read | Write | Dependency |
|---|---|---|---|---|
| `stl` | STL, binary and ASCII | yes | binary and ASCII | none |
| `obj` | Wavefront OBJ with MTL | yes | OBJ and MTL | none |
| `ply` | PLY, ASCII and both binary byte orders | yes | no (no app writes PLY) | none |
| `gltf` | glTF 2.0 (`.gltf`) and GLB | yes | GLB | `serde_json` |
| `threemf` | 3MF, core and materials extension | yes | no | `craft-opc`, `craft-xml` |
| `fbx` | FBX 6.x and 7.x, binary and ASCII | yes | no | `miniz_oxide` |

USD, Alembic and COLLADA are not here yet.

## The neutral model

- `Scene`: `meshes`, `materials`, `images`, `nodes` with `roots`, `skins`, `animations`, `cameras`,
  `lights`, plus `external` (every other file the file names) and `warnings` (what was left out or
  repaired).
- `Mesh`: a name, attribute pools (`positions` and `normals` as `f64`, `uv_sets`, per-position
  `colors`, `joints` and `weights`) and `parts`.
- `Part`: the faces of a mesh that share a group name, a material and a smoothing group. A face is
  a polygon of three or more `Corner`s; each corner indexes a position and optionally a normal and
  a UV. Formats with separate indices (OBJ) are kept exactly, and polygons stay whole.
  `Part::triangles()` fans them; `Mesh::unified()` gives one vertex per distinct corner as a
  triangle list, which is what a renderer wants.
- `Material`: metallic-roughness factors, alpha mode, double-sided, unlit, and five texture slots.
  A `TextureRef` names an `Image`, a UV set and wrap modes. An `Image` is a URI the caller
  resolves or the encoded bytes the file embedded; nothing here decodes images.
- `Node`: a name, a column-major 4×4 `transform` (and the same as `Trs` when the file gave it in
  parts), optional mesh, skin, camera and light, and children. Every reader fills `nodes`: a
  format without a hierarchy gets one root node per mesh. `Scene::instances()` walks the hierarchy
  and returns every placed mesh with its world transform.

Conventions, whatever the file:

- Positions are in the file's own axes and units. Nothing is turned or scaled (STL and PLY files
  are usually Z-up, glTF is Y-up in metres).
- UVs have their origin at the top left, v down (glTF). The OBJ and PLY readers and the OBJ writer
  flip v.
- Material colours are linear. MTL and STL colours are display colours and are converted. Vertex
  colours are the stored values scaled to 0..1.
- Missing normals stay missing. STL facets get the normal of their winding.

## Coverage

**STL.** Binary and ASCII, told apart by the exact binary size first, then by `solid` with a
`facet` or `endsolid`. Binary headers that begin with `solid`, trailing bytes, a facet count left
at 0, several ASCII solids, ASCII loops of more than three vertices, the Magics (`COLOR=` header)
and VisCAM/SolidView per-facet colours (a material and a part per colour). ASCII coordinates keep
`f64` precision. Facets with a non-finite coordinate are left out with a warning. Writers apply
the node transforms and write one solid; colours are not written.

**OBJ and MTL.** `v` with optional `r g b`, `vt`, `vn`, `f` with all four reference forms and
negative references, polygons of any size, `o` (one mesh each, holding only its own vertices in
file order), `g`, `usemtl`, `s`, `mtllib`, `\` continuations, `#` comments. A vertex or face line
that cannot be read is left out and counted in one warning; later vertices keep their numbers.
MTL: `Kd`, `d`, `Tr`, `Ke`, `Ns`, `Pr`, `Pm`, `map_Kd`, `map_Ke`, `map_Bump`/`bump`/`norm`. The
writer writes objects, vertex colours, the first UV set, normals, groups, materials, smoothing
groups and whole polygons, and an MTL library with unique material names. Lines, points and
free-form geometry are passed over.

**PLY.** ASCII, little- and big-endian binary; `\n`, `\r\n` and `\r` header lines; elements in
any order; scalar and list properties of every type. From the `vertex` element: `x y z`,
`nx ny nz`, `s t` / `u v` / `texture_u texture_v`, `red green blue alpha` (also `r g b a` and
`diffuse_*`), integer colours scaled by their type. From the `face` element: `vertex_indices` or
`vertex_index`, polygons kept whole, and a `texcoord` list of a `u v` per corner, which then is
the UV set instead of the vertices' own. A `comment TextureFile <name>` line gives the mesh a
material whose base colour texture is that file (named, never opened). A file without faces is a
point cloud (positions, no parts).
`ply::parse_header` and `ply::Rows` are public for callers that need every property as a column,
`edge` elements or thinning.

**glTF and GLB.** The default scene's node tree (matrix or TRS); `TRIANGLES`, `TRIANGLE_STRIP` and
`TRIANGLE_FAN`, indexed or not; POSITION, NORMAL, every TEXCOORD_n (up to 8), COLOR_0, JOINTS_0,
WEIGHTS_0 in any component type the specification or `KHR_mesh_quantization` allows; strided
buffer views; accessors without a buffer view (zeros); sparse accessors; materials with their
texture slots, UV set and sampler wrap modes, `KHR_materials_emissive_strength`,
`KHR_materials_unlit`, `KHR_materials_pbrSpecularGlossiness` (diffuse colour and texture, no
metal, roughness from the glossiness); textures whose image is in `EXT_texture_webp`; images embedded
in a buffer view or a `data:` URI, or named; skins; translation, rotation and scale animations;
cameras; `KHR_lights_punctual`. Primitives of a mesh that name the same accessors share their
vertices. `write_glb` writes nodes, meshes (positions, normals, UV sets, colours), materials and
images.

**3MF.** The package is opened with `craft-opc` and the model part found through its `3dmodel`
relationship; `threemf::read_model_part` reads a model part's XML on its own. Mesh objects,
component objects and build items with their transforms (every build item a root node, every
component a child node, an object placed several times one mesh), the unit (`Model::unit`; the
scene is not scaled), base materials, colour groups (a position per vertex and colour, since
colours are per position here), texture coordinate groups (a UV pool indexed per corner) and
their textures (the package part's bytes as `Image::data`, tile styles as wrap modes). Not read:
components in another model part (the production extension), composite materials,
multi-properties, slices and beam lattices; the first three are named in a warning.

Not read: sparse accessors and required geometry extensions (Draco, meshopt) are
**FBX.** Binary files (32-bit record offsets before 7.5, 64-bit from it; every property type;
arrays plain or zlib-compressed) and ASCII files give the same record tree, which
`fbx::read_document` returns as it is. From it: `Geometry` meshes with whole polygons and the
first normal, UV and material layer elements in every mapping and reference mode (kept as
indexed pools); `Model` nodes with the full transform (offsets, pivots, pre- and post-rotation,
every rotation order) and geometric transforms; Lambert and Phong `Material`s mapped onto
metallic-roughness; `Texture`s with embedded `Video` content or file names, for the diffuse,
normal and emissive slots. Models that instance one geometry with the same materials share a
mesh, and textures that end at one image share it. `Document::axes_to_y_up` and
`Document::unit_scale` give the file's axis system and unit, which the scene does not apply.
FBX 6.x files (a `Model` holds its mesh, `Properties60`, objects connected by name) give
geometry, transforms, the hierarchy and materials. Not read: FBX 5 and older, 6.x textures,
animation, skin deformers, blend shapes, cameras, lights, NURBS.

Not read (glTF): required geometry extensions (Draco, meshopt) are
Not read (glTF): sparse accessors and required geometry extensions (Draco, meshopt) are
`Error::Unsupported`; points, lines, morph targets, tangents and other extensions are passed over.
Not written: skins, animations, cameras, lights, and `.gltf` with separate files.

## Hostile input

Every reader takes `Limits` and returns `Error::TooLarge` past one. A count is checked against the
bytes that are there before anything is allocated from it. Nothing indexes with a value from the
file unchecked. Each limit has a test, and every reader is run over all truncations and two
mutations of every byte of good files.

| Limit | Default | Applies to |
|---|---|---|
| `max_vertices` | 20 000 000 | entries per attribute pool (OBJ: each of `v`, `vt`, `vn`, counting unused ones; glTF: vertices over all meshes; 3MF: vertices, colours and texture coordinates of the file together) |
| `max_triangles` | 20 000 000 | triangles after fanning polygons, strips and fans |
| `max_face_corners` | 4096 | corners of one face; PLY list lengths |
| `max_meshes` | 100 000 | OBJ objects, glTF meshes, 3MF objects |
| `max_parts` | 100 000 | groups, material runs, glTF primitives |
| `max_materials` | 100 000 | materials; images; OBJ material libraries; 3MF base materials and textures together |
| `max_nodes` | 100 000 | nodes, and node visits when the hierarchy is walked (ends cycles); 3MF placed objects, build items and components |
| `max_depth` | 64 | node nesting; 3MF component nesting (ends cycles) |
| `max_data_bytes` | 1 GiB | buffers and material libraries taken from the resolver; embedded image bytes; skin and animation values; the binary chunk `write_glb` builds; what a 3MF package inflates to, and its textures |
| `max_extras` | 10 000 | skins, animations, cameras, lights, buffers (each); channels of one animation |

Fixed bounds: a PLY header of 1 MiB, 64 elements and 256 properties per element; 2^24 components
for glTF accessors without a buffer view, over the whole file; 10 000 parts in a 3MF package.
| `max_meshes` | 100 000 | OBJ objects, glTF meshes, FBX meshes |
| `max_parts` | 100 000 | groups, material runs, glTF primitives, material slots of an FBX mesh |
| `max_materials` | 100 000 | materials; images; OBJ material libraries |
| `max_nodes` | 100 000 | nodes, and node visits when the hierarchy is walked (ends cycles) |
| `max_depth` | 64 | node nesting |
| `max_data_bytes` | 1 GiB | buffers and material libraries taken from the resolver; embedded image bytes; skin and animation values; the binary chunk `write_glb` builds; everything an FBX file keeps (parsed values at 8 bytes per array element, meshes, image bytes) |
| `max_extras` | 10 000 | skins, animations, cameras, lights, buffers (each); channels of one animation |

Fixed bounds: a PLY header of 1 MiB, 64 elements and 256 properties per element; 2^24 components
for glTF accessors without a buffer view, over the whole file; FBX records nested 64 deep and
arrays of 1 GiB.

**External files.** No reader opens a file. A reader that needs another file to parse (an OBJ
material library, a glTF buffer) asks the caller's `Resolver` for it by the name the file writes;
the resolver may decline (`None`). Images are named in `Image::uri` and not asked for, except by
the FBX reader, whose textures carry two file names that only it can try in turn.
Every name is listed in `Scene::external`. Names come from the file, so a resolver must not trust
them as paths: they may be absolute or contain `..`.

## What stays in the apps

Mapping a `Scene` onto the app's mesh, body or document types; computing normals, tangents and
UVs; welding; unit scaling and up-axis turns; flattening the hierarchy into world space (use
`Scene::instances()`); decoding images; choosing where external files come from; sampling
animations and skinning; file-size limits before reading.

## Origin

All code is from Craft apps (MIT OR Apache-2.0), written from the public format descriptions. No
sample files are shipped: every test generates its input.

| Module | Base | Ported in |
|---|---|---|
| `stl` reader | EffectCraft `crates/model/src/stl.rs` (detection, colour extensions, degenerate facets) | SolidCraft `read_stl`: a facet count left at 0, non-UTF-8 ASCII names. TextureCraft `mesh_import/stl.rs`: same detection, nothing extra. The Z-up to Y-up turn EffectCraft did stays in EffectCraft. |
| `stl` writers | SolidCraft `stl_binary`, `stl_ascii` | Node transforms, a header that never starts with `solid` |
| `obj` reader | EffectCraft `crates/model/src/obj.rs` (own parser, MTL, continuations, library name candidates) | SolidCraft `obj.rs`: checked references, objects holding only their vertices, bad lines counted not fatal, smoothing groups kept, pool caps. TextureCraft `mesh_import/obj.rs`: vertex colours, whole polygons, group names. MaterialCraft `mesh.rs`: bounds on unused `v`/`vt`/`vn` definitions. |
| `obj` writer | TextureCraft `mesh_export.rs` (`obj_text`, unique material names) | SolidCraft `obj_files`: several objects with running indices, control characters in names |
| `ply` | SolidCraft `ply.rs` (header bounds, three line endings, size checks before reading) | TextureCraft `mesh_import/ply.rs`: normals, UVs, alpha, whole polygons, faces before vertices |
| `gltf` reader | EffectCraft `crates/model/src/gltf.rs` (own reader over `serde_json`; skins, animations, cameras, lights) | SolidCraft `gltf_in.rs`: stricter GLB chunk checks, the required-extension policy, hierarchy caps. TextureCraft `mesh_import/gltf.rs`: COLOR_0, sparse accessors refused instead of read wrong. MaterialCraft `mesh.rs`: a bad index drops its triangle and later ones keep their vertices. |
| `gltf` writer | SolidCraft `glb.rs` (`glb_container`, accessor layout) | TextureCraft `glb_bytes`: UV sets, embedded images and texture slots |

TextureCraft, MaterialCraft and SolidCraft read OBJ and glTF through the `tobj` and `gltf` crates;
this crate does not depend on either.

## Migration

No app uses this crate yet.

| App | Replace | With | Keep in the app |
|---|---|---|---|
| EffectCraft | `crates/model/src/{stl,obj,gltf}.rs` | `stl::read`, `obj::read`, `gltf::read` | `Scene` to `Model`; the STL Y-up turn; computed normals per smoothing group and tangents; image decoding and the sibling-name lookup for textures; animation sampling. FBX stays. |
| TextureCraft | `mesh_import/{stl,obj,ply,gltf}.rs`, `obj_text`, the geometry part of `glb_bytes` | readers, `obj::write`, `gltf::write_glb` | `Scene` to `RawMesh` (world space through `instances()`), unwrap, tangents, cameras to orbit cameras, texture baking. GLB clearcoat and sheen extensions are not in the model yet, so `glb_bytes` keeps them until then. Drops `tobj` and `gltf`. |
| MaterialCraft | `crates/formats/src/mesh.rs` `obj` and `gltf` | `obj::read`, `gltf::read` with its 4M-triangle `Limits` | Non-finite normal and UV rejection, box UVs, normalising to the unit sphere. Drops `tobj` and `gltf`. |
| SolidCraft | `crates/io/src/{obj,ply}.rs`, `read_stl`, `stl_binary`, `stl_ascii`, `gltf_in.rs` parsing, `glb.rs` container | readers, `ply::parse_header`/`Rows` under `ply_complex.rs`, STL writers, `gltf::write_glb` | Millimetre scaling, crease normals, mesh bodies, component plans from the node tree, point-cloud thinning and columns, looks. An OBJ without `o` now gives one mesh with a part per `g`; SolidCraft splits by part to keep one body per group. Drops `gltf`. |
| DeckCraft | nothing merged; open PRs #1380 and #743 each add a `crates/mesh` | this crate | Rasterising |
