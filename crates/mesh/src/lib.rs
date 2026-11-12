//! 3D mesh file formats behind one neutral model.
//!
//! Each format is a module behind a Cargo feature of the same name: [`stl`], [`obj`] (with MTL),
//! [`ply`], [`gltf`] (glTF 2.0 and GLB), [`fbx`] and [`threemf`] (3MF). A reader turns bytes into a [`Scene`]; a
//! writer turns
//! a [`Scene`] into bytes. Mapping a [`Scene`] onto an application's own mesh, body or document
//! types is the caller's business, as are computing missing normals, tangents and UVs, welding,
//! unit scaling and turning a file's up axis.
//!
//! Conventions of the model, whatever the file:
//!
//! - Positions and normals are `f64` in the file's own axes and units (STL and PLY files are
//!   usually Z-up, glTF is Y-up in metres; nothing is turned or scaled here).
//! - UVs have their origin at the top left with v pointing down (glTF's convention). OBJ and PLY
//!   store v up; their readers and writers flip it.
//! - Material colours are linear RGB(A). MTL and STL colours are display (sRGB) colours and are
//!   converted. Vertex colours are the stored values scaled to 0..1.
//! - A face is a polygon of three or more corners. Each corner names a position, and optionally
//!   a normal and a UV, by index into the mesh's pools, so a format with separate indices (OBJ)
//!   is kept exactly; [`Mesh::unified`] gives the one-index triangle list a renderer wants.
//!
//! Input is untrusted. Every reader takes [`Limits`], checks every count, index and length
//! against the bytes that are there, and returns an [`Error`] or a warning in
//! [`Scene::warnings`] instead of panicking. No reader opens a file: a reference to another file
//! (an OBJ material library, a glTF buffer or image) is listed in [`Scene::external`] and asked
//! of the caller's [`Resolver`], which may decline.
//!
//! Depends on no other Craft crate, except that `threemf` opens its package with `craft-opc`
//! and reads its XML with `craft-xml`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
// The helpers the formats share are not all used by every subset of them.
#![cfg_attr(
    not(all(feature = "stl", feature = "obj", feature = "ply", feature = "gltf", feature = "fbx", feature = "threemf")),
    allow(dead_code, unused_imports)
)]

#[cfg(feature = "fbx")]
pub mod fbx;
#[cfg(feature = "gltf")]
pub mod gltf;
#[cfg(feature = "obj")]
pub mod obj;
#[cfg(feature = "ply")]
pub mod ply;
#[cfg(feature = "stl")]
pub mod stl;
#[cfg(feature = "threemf")]
pub mod threemf;

use std::collections::HashMap;
use std::fmt;

/// Why a file could not be read or a scene could not be written.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// The bytes are not a valid file of the format.
    Malformed(String),
    /// The file is valid but exceeds one of the [`Limits`]; names the limit.
    TooLarge(String),
    /// The file needs something this crate does not read (a compression extension, …).
    Unsupported(String),
    /// The file needs another file that the resolver did not supply; holds its name.
    Missing(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Malformed(m) | Error::TooLarge(m) | Error::Unsupported(m) => f.write_str(m),
            Error::Missing(name) => write!(f, "`{name}`, which the file needs, was not supplied"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn malformed(kind: &str, m: impl fmt::Display) -> Error {
    Error::Malformed(format!("{kind}: {m}"))
}

pub(crate) fn too_large(kind: &str, m: impl fmt::Display) -> Error {
    Error::TooLarge(format!("{kind}: {m}"))
}

/// The bounds a reader keeps. The defaults suit a desktop application; lower them for a
/// browser or a preview.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// Most entries in any one attribute pool of the file (positions, normals, UVs).
    pub max_vertices: usize,
    /// Most triangles in the file, counting a polygon of `n` corners as `n - 2`.
    pub max_triangles: usize,
    /// Most corners of one face.
    pub max_face_corners: usize,
    /// Most meshes (OBJ objects, glTF meshes).
    pub max_meshes: usize,
    /// Most parts (groups, material runs, glTF primitives) in the file.
    pub max_parts: usize,
    /// Most materials, and most images.
    pub max_materials: usize,
    /// Most nodes, and most node visits when a hierarchy is walked.
    pub max_nodes: usize,
    /// Deepest node nesting.
    pub max_depth: usize,
    /// Most bytes of external and embedded data (buffers, material libraries) taken in total.
    pub max_data_bytes: usize,
    /// Most skins, animations, cameras and lights (each), and most channels of one animation.
    pub max_extras: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_vertices: 20_000_000,
            max_triangles: 20_000_000,
            max_face_corners: 4096,
            max_meshes: 100_000,
            max_parts: 100_000,
            max_materials: 100_000,
            max_nodes: 100_000,
            max_depth: 64,
            max_data_bytes: 1 << 30,
            max_extras: 10_000,
        }
    }
}

/// Answers a file's reference to another file (a name exactly as the file writes it) with that
/// file's bytes, or `None`. The crate never opens anything itself.
pub type Resolver<'a> = &'a mut dyn FnMut(&str) -> Option<Vec<u8>>;

/// A [`Resolver`] that declines everything: for self-contained files.
pub fn no_external(_: &str) -> Option<Vec<u8>> {
    None
}

/// Marks a corner without a normal or a UV.
pub const NONE: u32 = u32::MAX;

/// One corner of a face: indices into [`Mesh::positions`], [`Mesh::normals`] and the UV sets
/// ([`NONE`] when the corner has no normal or no UV).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Corner {
    pub position: u32,
    pub normal: u32,
    pub uv: u32,
}

impl Corner {
    /// A corner with only a position.
    pub fn at(position: u32) -> Corner {
        Corner { position, normal: NONE, uv: NONE }
    }

    /// A corner whose position, normal and UV share one index.
    pub fn all(index: u32) -> Corner {
        Corner { position: index, normal: index, uv: index }
    }
}

/// Faces of a mesh that share a group name, a material and a smoothing group.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Part {
    /// The group's name (OBJ `g`); empty when the file has none.
    pub name: String,
    /// Index into [`Scene::materials`].
    pub material: Option<usize>,
    /// OBJ smoothing group; 0 is off.
    pub smoothing: u32,
    /// Corners of each face, in order. Empty means every face is a triangle.
    pub face_sizes: Vec<u32>,
    /// The corners of all faces, one face after another.
    pub corners: Vec<Corner>,
}

impl Part {
    /// The faces, each a slice of its corners. Stops at the first face the corner list is too
    /// short for.
    pub fn faces(&self) -> impl Iterator<Item = &[Corner]> {
        let mut rest = self.corners.as_slice();
        let mut sizes = self.face_sizes.iter();
        let all_triangles = self.face_sizes.is_empty();
        std::iter::from_fn(move || {
            let n = if all_triangles { 3 } else { *sizes.next()? as usize };
            let (face, tail) = rest.split_at_checked(n)?;
            rest = tail;
            (n > 0).then_some(face)
        })
    }

    /// The faces as triangles: a polygon becomes a fan from its first corner.
    pub fn triangles(&self) -> impl Iterator<Item = [Corner; 3]> + '_ {
        self.faces().flat_map(|f| {
            let first = f.first().copied();
            f.get(1..).unwrap_or_default().windows(2).filter_map(move |w| match (first, w) {
                (Some(a), [b, c]) => Some([a, *b, *c]),
                _ => None,
            })
        })
    }

    /// Triangles the faces fan into.
    pub fn triangle_count(&self) -> usize {
        if self.face_sizes.is_empty() { self.corners.len() / 3 } else { self.face_sizes.iter().map(|n| (*n as usize).saturating_sub(2)).sum() }
    }
}

/// One object: attribute pools and the parts whose corners index them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub positions: Vec<[f64; 3]>,
    pub normals: Vec<[f64; 3]>,
    /// UV sets; every set has the same length and is indexed by [`Corner::uv`].
    pub uv_sets: Vec<Vec<[f32; 2]>>,
    /// RGBA per position (same length as `positions`), or empty.
    pub colors: Vec<[f32; 4]>,
    /// Skinning (glTF): up to four joints (indices into the skin's joint list) and weights per
    /// position; both the length of `positions`, or empty.
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub parts: Vec<Part>,
}

/// A triangle list with one index per vertex, as [`Mesh::unified`] builds it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Unified {
    pub positions: Vec<[f64; 3]>,
    /// One per vertex when every corner has a normal, else empty.
    pub normals: Vec<[f64; 3]>,
    /// The first UV set, one per vertex when every corner has a UV, else empty.
    pub uvs: Vec<[f32; 2]>,
    /// One per vertex when the mesh has colours, else empty.
    pub colors: Vec<[f32; 4]>,
    /// The corner of the mesh each vertex came from: its position index finds the joints and
    /// weights, its UV index the other UV sets.
    pub corners: Vec<Corner>,
    /// Triangles per part, in the order of [`Mesh::parts`].
    pub parts: Vec<Vec<[u32; 3]>>,
}

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.parts.iter().map(Part::triangle_count).sum()
    }

    /// Checks what every reader guarantees and every writer needs: each corner's indices are in
    /// range, the face sizes add up, and the per-position arrays have the positions' length.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::Malformed(format!("mesh `{}`: {m}", self.name)));
        let uvs = self.uv_sets.first().map_or(0, Vec::len);
        if self.uv_sets.iter().any(|s| s.len() != uvs) {
            return bad("UV sets differ in length");
        }
        for (what, len) in [("colours", self.colors.len()), ("joints", self.joints.len()), ("weights", self.weights.len())] {
            if len != 0 && len != self.positions.len() {
                return bad(&format!("{what} do not match the positions"));
            }
        }
        for p in &self.parts {
            let total = if p.face_sizes.is_empty() {
                (p.corners.len() % 3 == 0).then_some(p.corners.len())
            } else {
                p.face_sizes.iter().try_fold(0usize, |n, s| if *s >= 3 { n.checked_add(*s as usize) } else { None })
            };
            if total != Some(p.corners.len()) {
                return bad("face sizes do not match the corners");
            }
            for c in &p.corners {
                let ok = (c.position as usize) < self.positions.len()
                    && (c.normal == NONE || (c.normal as usize) < self.normals.len())
                    && (c.uv == NONE || (c.uv as usize) < uvs);
                if !ok {
                    return bad("a corner index is out of range");
                }
            }
        }
        Ok(())
    }

    /// The mesh as triangles with one vertex per distinct corner. Corners whose indices are out
    /// of range are dropped with their triangle.
    pub fn unified(&self) -> Unified {
        let mut out = Unified::default();
        let all = |f: &dyn Fn(&Corner) -> bool| self.parts.iter().all(|p| p.corners.iter().all(f));
        let normals = !self.normals.is_empty() && all(&|c| c.normal != NONE);
        let uv_set = self.uv_sets.first().filter(|_| all(&|c| c.uv != NONE));
        let colors = self.colors.len() == self.positions.len() && !self.colors.is_empty();
        let mut ids: HashMap<Corner, u32> = HashMap::new();
        for part in &self.parts {
            let mut tris = Vec::with_capacity(part.triangle_count());
            'tri: for t in part.triangles() {
                let mut tri = [0u32; 3];
                for (slot, c) in tri.iter_mut().zip(t) {
                    let key = Corner {
                        position: c.position,
                        normal: if normals { c.normal } else { NONE },
                        uv: if uv_set.is_some() { c.uv } else { NONE },
                    };
                    if let Some(id) = ids.get(&key) {
                        *slot = *id;
                        continue;
                    }
                    let Some(p) = self.positions.get(c.position as usize) else { continue 'tri };
                    let n = self.normals.get(c.normal as usize).filter(|_| normals);
                    let uv = uv_set.and_then(|s| s.get(c.uv as usize));
                    if (normals && n.is_none()) || (uv_set.is_some() && uv.is_none()) {
                        continue 'tri;
                    }
                    let Ok(id) = u32::try_from(out.positions.len()) else { continue 'tri };
                    out.positions.push(*p);
                    out.normals.extend(n);
                    out.uvs.extend(uv);
                    if colors {
                        out.colors.extend(self.colors.get(c.position as usize));
                    }
                    out.corners.push(c);
                    ids.insert(key, id);
                    *slot = id;
                }
                tris.push(tri);
            }
            out.parts.push(tris);
        }
        out
    }
}

/// How a material's alpha is used (glTF `alphaMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum AlphaMode {
    #[default]
    Opaque,
    /// Visible where alpha is at least the cutoff.
    Mask(f32),
    Blend,
}

/// How a texture continues outside 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Wrap {
    #[default]
    Repeat,
    Clamp,
    Mirror,
}

/// A texture slot: an image of the scene, the UV set it reads and how it wraps along u and v.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextureRef {
    /// Index into [`Scene::images`].
    pub image: usize,
    pub uv_set: u32,
    pub wrap: [Wrap; 2],
}

/// A metallic-roughness material: the simple parameters every format here can carry.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    /// Linear RGBA.
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    /// Linear RGB.
    pub emissive: [f32; 3],
    pub alpha_mode: AlphaMode,
    pub double_sided: bool,
    /// Shown as it is, without lighting (glTF `KHR_materials_unlit`).
    pub unlit: bool,
    pub base_color_texture: Option<TextureRef>,
    pub metallic_roughness_texture: Option<TextureRef>,
    pub normal_texture: Option<TextureRef>,
    pub normal_scale: f32,
    pub occlusion_texture: Option<TextureRef>,
    pub occlusion_strength: f32,
    pub emissive_texture: Option<TextureRef>,
}

impl Default for Material {
    fn default() -> Self {
        Material {
            name: String::new(),
            base_color: [1.0; 4],
            metallic: 1.0,
            roughness: 1.0,
            emissive: [0.0; 3],
            alpha_mode: AlphaMode::Opaque,
            double_sided: false,
            unlit: false,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            normal_scale: 1.0,
            occlusion_texture: None,
            occlusion_strength: 1.0,
            emissive_texture: None,
        }
    }
}

impl Material {
    /// A plain dielectric of one colour.
    pub fn plain(name: impl Into<String>, base_color: [f32; 4]) -> Material {
        Material { name: name.into(), base_color, metallic: 0.0, roughness: 0.5, ..Material::default() }
    }
}

/// An image a material uses: a reference the caller resolves, or bytes the file embedded. The
/// bytes are the encoded image (PNG, JPEG, …); decoding is the caller's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Image {
    pub name: String,
    /// The reference as the file writes it, when the image is another file.
    pub uri: Option<String>,
    pub mime_type: Option<String>,
    /// The encoded bytes, when the file embeds them or the resolver supplied them.
    pub data: Option<Vec<u8>>,
}

/// A 4×4 transform, column-major (as glTF writes it): a point is `M · [x, y, z, 1]`.
pub type Mat4 = [f64; 16];

pub const IDENTITY: Mat4 = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// `a · b`: `b` applies first.
pub fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut m = [0.0; 16];
    for (i, v) in m.iter_mut().enumerate() {
        let (col, row) = (i / 4, i % 4);
        *v = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
    }
    m
}

/// The point `p` moved by `m`.
pub fn transform_point(m: &Mat4, p: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r])
}

/// Does `m` mirror (a negative determinant)? A mirrored mesh needs its winding reversed to
/// keep facing out; the writers of formats without a hierarchy do that.
pub fn is_mirror(m: &Mat4) -> bool {
    let l = |r: usize, c: usize| m[c * 4 + r];
    l(0, 0) * (l(1, 1) * l(2, 2) - l(1, 2) * l(2, 1)) - l(0, 1) * (l(1, 0) * l(2, 2) - l(1, 2) * l(2, 0))
        + l(0, 2) * (l(1, 0) * l(2, 1) - l(1, 1) * l(2, 0))
        < 0.0
}

/// The normal `n` under `m` (by the inverse transpose of its linear part, so scale, shear and
/// mirrors are right), normalised; `n` itself when the result has no length.
pub fn transform_normal(m: &Mat4, n: [f64; 3]) -> [f64; 3] {
    let l = |r: usize, c: usize| m[c * 4 + r];
    // Cyclic indices give each cofactor its sign; the cofactors are the inverse transpose
    // times the determinant, whose sign a mirror flips.
    let cof = |r: usize, c: usize| {
        let (r1, r2, c1, c2) = ((r + 1) % 3, (r + 2) % 3, (c + 1) % 3, (c + 2) % 3);
        l(r1, c1) * l(r2, c2) - l(r1, c2) * l(r2, c1)
    };
    let sign = if is_mirror(m) { -1.0 } else { 1.0 };
    let v = [0, 1, 2].map(|r| sign * (cof(r, 0) * n[0] + cof(r, 1) * n[1] + cof(r, 2) * n[2]));
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len.is_finite() && len > 0.0 { v.map(|x| x / len) } else { n }
}

/// A placement as translation, rotation (a unit quaternion `x, y, z, w`) and scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trs {
    pub translation: [f64; 3],
    pub rotation: [f64; 4],
    pub scale: [f64; 3],
}

impl Trs {
    /// `T · R · S`.
    pub fn matrix(&self) -> Mat4 {
        let [x, y, z, w] = self.rotation;
        let r = [
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w)],
            [2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w)],
            [2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y)],
        ];
        let mut m = IDENTITY;
        for (c, (col, s)) in r.iter().zip(self.scale).enumerate() {
            for (row, v) in col.iter().enumerate() {
                m[c * 4 + row] = v * s;
            }
        }
        m[12] = self.translation[0];
        m[13] = self.translation[1];
        m[14] = self.translation[2];
        m
    }
}

/// A node of the scene's hierarchy.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: String,
    /// Placement relative to the parent.
    pub transform: Mat4,
    /// The same placement in parts, when the file gave it so (animations move the parts).
    pub trs: Option<Trs>,
    /// Index into [`Scene::cameras`].
    pub camera: Option<usize>,
    /// Index into [`Scene::lights`].
    pub light: Option<usize>,
    /// Index into [`Scene::meshes`].
    pub mesh: Option<usize>,
    /// Index into [`Scene::skins`].
    pub skin: Option<usize>,
    /// Indices into [`Scene::nodes`].
    pub children: Vec<usize>,
}

impl Default for Node {
    fn default() -> Self {
        Node { name: String::new(), transform: IDENTITY, trs: None, camera: None, light: None, mesh: None, skin: None, children: Vec::new() }
    }
}

/// A skin (glTF): the joint nodes and their inverse bind matrices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skin {
    pub name: String,
    /// Indices into [`Scene::nodes`].
    pub joints: Vec<usize>,
    /// One per joint, or empty for identity.
    pub inverse_bind: Vec<Mat4>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// Vertical field of view in radians; the aspect ratio when the file fixes one.
    Perspective { yfov: f64, aspect: Option<f64>, znear: f64, zfar: Option<f64> },
    /// Half the view's width and height.
    Orthographic { xmag: f64, ymag: f64, znear: f64, zfar: f64 },
}

/// A camera, looking along its node's −Z with +Y up.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub name: String,
    pub projection: Projection,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightKind {
    Directional,
    Point,
    /// Cone angles in radians from the axis.
    Spot {
        inner: f64,
        outer: f64,
    },
}

/// A punctual light, shining along its node's −Z.
#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub name: String,
    pub kind: LightKind,
    /// Linear RGB.
    pub color: [f64; 3],
    /// Candela for point and spot lights, lux for directional ones.
    pub intensity: f64,
    pub range: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelPath {
    Translation,
    Rotation,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Step,
    Linear,
    /// Three values per key: in-tangent, value, out-tangent.
    CubicSpline,
}

/// The keys of one part of one node's placement. Sampling them is the caller's.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    /// Index into [`Scene::nodes`].
    pub node: usize,
    pub path: ChannelPath,
    pub interpolation: Interpolation,
    /// Key times in seconds.
    pub times: Vec<f32>,
    /// The keys' values, flat: 3 per key for translation and scale, 4 for rotation (times 3
    /// for a cubic spline).
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animation {
    pub name: String,
    pub channels: Vec<Channel>,
}

/// What a file holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub images: Vec<Image>,
    /// Every format fills this: a format without a hierarchy gets one root node per mesh.
    pub nodes: Vec<Node>,
    /// The top-level nodes.
    pub roots: Vec<usize>,
    pub skins: Vec<Skin>,
    pub animations: Vec<Animation>,
    pub cameras: Vec<Camera>,
    pub lights: Vec<Light>,
    /// Every other file this one names, as written, whether or not the resolver supplied it.
    pub external: Vec<String>,
    /// What was left out or repaired while reading.
    pub warnings: Vec<String>,
}

impl Scene {
    /// A scene of meshes without a hierarchy: one root node per mesh.
    pub fn from_meshes(meshes: Vec<Mesh>, materials: Vec<Material>) -> Scene {
        let nodes: Vec<Node> = meshes.iter().enumerate().map(|(i, m)| Node { name: m.name.clone(), mesh: Some(i), ..Node::default() }).collect();
        Scene { roots: (0..nodes.len()).collect(), nodes, meshes, materials, ..Scene::default() }
    }

    /// Every placed mesh: the mesh's index and its world transform, walking the hierarchy from
    /// the roots. A hierarchy that is deeper than `limits.max_depth`, takes more than
    /// `limits.max_nodes` visits (a cycle, or a shared subtree expanded too often) or names a
    /// node that is not there is an error.
    pub fn instances(&self, limits: &Limits) -> Result<Vec<(usize, Mat4)>> {
        let mut out = Vec::new();
        let mut stack: Vec<(usize, Mat4, usize)> = self.roots.iter().rev().map(|r| (*r, IDENTITY, 0)).collect();
        let mut visits = 0usize;
        while let Some((index, parent, depth)) = stack.pop() {
            visits += 1;
            if visits > limits.max_nodes {
                return Err(Error::TooLarge(format!("the node hierarchy takes more than {} visits", limits.max_nodes)));
            }
            if depth > limits.max_depth {
                return Err(Error::TooLarge(format!("the node hierarchy is deeper than {}", limits.max_depth)));
            }
            let node = self.nodes.get(index).ok_or_else(|| Error::Malformed(format!("node {index} does not exist")))?;
            let world = mat_mul(&parent, &node.transform);
            if let Some(m) = node.mesh.filter(|m| *m < self.meshes.len()) {
                out.push((m, world));
            }
            stack.extend(node.children.iter().rev().map(|c| (*c, world, depth + 1)));
        }
        Ok(out)
    }

    /// [`Mesh::validate`] for every mesh, and every material, image, mesh, skin, camera, light
    /// and node index in range.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: String| Err(Error::Malformed(m));
        for m in &self.meshes {
            m.validate()?;
            if m.parts.iter().any(|p| p.material.is_some_and(|i| i >= self.materials.len())) {
                return bad(format!("mesh `{}` names a material that does not exist", m.name));
            }
        }
        for m in &self.materials {
            let slots = [m.base_color_texture, m.metallic_roughness_texture, m.normal_texture, m.occlusion_texture, m.emissive_texture];
            if slots.iter().flatten().any(|t| t.image >= self.images.len()) {
                return bad(format!("material `{}` names an image that does not exist", m.name));
            }
        }
        let n = self.nodes.len();
        for node in &self.nodes {
            let refs =
                [(node.mesh, self.meshes.len()), (node.skin, self.skins.len()), (node.camera, self.cameras.len()), (node.light, self.lights.len())];
            if refs.iter().any(|(i, len)| i.is_some_and(|i| i >= *len)) || node.children.iter().any(|c| *c >= n) {
                return bad(format!("node `{}` names something that does not exist", node.name));
            }
        }
        let channels = self.animations.iter().flat_map(|a| &a.channels).any(|c| c.node >= n);
        if self.roots.iter().any(|r| *r >= n) || self.skins.iter().any(|s| s.joints.iter().any(|j| *j >= n)) || channels {
            return bad("a root, joint or animation channel names a node that does not exist".into());
        }
        Ok(())
    }
}

/// sRGB-encoded component to linear.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Linear component to sRGB-encoded.
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// Decode standard or URL-safe base64 (with or without padding; whitespace ignored).
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3 + 3);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Warnings that would repeat are counted and reported once.
pub(crate) fn count_warning(warnings: &mut Vec<String>, count: usize, what: &str) {
    if count > 0 {
        warnings.push(format!("{count} {what}"));
    }
}

/// The unit normal of triangle `v` from its counter-clockwise winding; `None` when it has no
/// area.
pub fn face_normal(v: [[f64; 3]; 3]) -> Option<[f64; 3]> {
    let (a, b) = ([0, 1, 2].map(|k| v[1][k] - v[0][k]), [0, 1, 2].map(|k| v[2][k] - v[0][k]));
    let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    (len.is_finite() && len > 0.0).then(|| n.map(|x| x / len))
}

#[cfg(test)]
mod tests;
