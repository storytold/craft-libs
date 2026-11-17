//! A craft-mesh [`Scene`] as a [`MeshScene`] (the `mesh` feature).
//!
//! craft-mesh keeps a file's own axes, units, linear colours and top-left UVs; a `MeshScene` wants
//! display colours, bottom-left UVs, a unit and a forest. What PRC cannot carry is counted in
//! [`FromMesh::warnings`].

use std::collections::HashMap;

use craft_mesh::{AlphaMode, Corner, IDENTITY, Mat4, Material, Mesh, NONE, Part, Scene, TextureRef, Wrap};

use super::textures::picture_info;
use super::{MAX_NODE_DEPTH, MeshScene, SceneMaterial, SceneMesh, SceneNode, ScenePrimitive, SceneTexture, TextureWrap};

/// The source file's up axis. PRC viewers show +Z up. glTF is always +Y up; other formats vary by
/// producer (FBX states it in its header).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpAxis {
    /// +Z is up (usual for STL, PLY and CAD files): nothing is turned.
    #[default]
    Z,
    /// +Y is up (glTF, and usually OBJ): each root is turned so +Y becomes +Z.
    Y,
}

/// A converted scene and what was left out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FromMesh {
    /// The scene for [`write_prc`](super::write_prc), which still refuses one that draws nothing,
    /// holds a non-finite number, or has a node matrix whose bottom row is not 0 0 0 1.
    pub scene: MeshScene,
    /// One line per kind of loss, with its count.
    pub warnings: Vec<String>,
}

/// Node copies a scene may expand to: craft-mesh's own default node limit.
pub const MAX_NODE_COPIES: usize = 100_000;

/// `scene` as a [`MeshScene`] in `units_in_mm` millimetres per unit (1000 for glTF's metres),
/// turned from `up` to +Z up.
///
/// The hierarchy is copied from the roots down, so the result is always a forest: a node with two
/// parents is copied under each, and copying stops at [`MAX_NODE_DEPTH`] levels or
/// [`MAX_NODE_COPIES`] nodes (a cycle reaches one of them). Indices out of range are dropped.
/// Node warnings count each copy.
/// Colours go from linear to display (sRGB) values; an alpha mask is drawn wholly opaque or
/// wholly invisible by its base alpha.
///
/// # Example
///
/// ```
/// use craft_mesh::{Corner, Material, Mesh, Part, Scene};
/// use craft_prc::{UpAxis, mesh_scene_from, write_prc};
///
/// let mesh = Mesh {
///     positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
///     parts: vec![Part { material: Some(0), corners: (0..3).map(Corner::at).collect(), ..Part::default() }],
///     ..Mesh::default()
/// };
/// let scene = Scene::from_meshes(vec![mesh], vec![Material::plain("red", [1.0, 0.0, 0.0, 1.0])]);
/// let converted = mesh_scene_from(&scene, 1000.0, UpAxis::Y);
/// assert!(converted.warnings.is_empty());
/// assert_eq!(write_prc(&converted.scene)?.triangles, 1);
/// # Ok::<(), craft_prc::PrcWriteError>(())
/// ```
pub fn mesh_scene_from(scene: &Scene, units_in_mm: f64, up: UpAxis) -> FromMesh {
    let mut counts = Counts::default();
    let mut out = MeshScene { units_in_mm, ..MeshScene::default() };
    let slots = materials(scene, &mut out, &mut counts);
    out.meshes = scene.meshes.iter().map(|m| mesh(m, &slots, &mut counts)).collect();
    nodes(scene, up, &mut out, &mut counts);
    FromMesh { scene: out, warnings: counts.warnings() }
}

/// Losses, each counted once and reported in one line.
#[derive(Default)]
struct Counts {
    no_texture: u64,
    masks: u64,
    other_maps: u64,
    no_uvs: u64,
    vertex_colours: u64,
    bad_triangles: u64,
    extras: u64,
    bad_meshes: u64,
    bad_materials: u64,
    bad_nodes: u64,
    cut: u64,
    animations: u64,
}

impl Counts {
    fn warnings(&self) -> Vec<String> {
        [
            (self.no_texture, "base-colour texture(s) are not embedded PNG or JPEG images and were left out"),
            (self.masks, "alpha-mask material(s) were drawn wholly opaque or invisible, not cut out by their texture"),
            (self.other_maps, "material(s) have normal, occlusion, metal-roughness or emissive maps, which were left out"),
            (self.no_uvs, "textured primitive(s) lack coordinates in their UV set and were drawn in the base colour"),
            (self.vertex_colours, "mesh(es) have vertex colours, which were left out"),
            (self.bad_triangles, "triangle(s) name a vertex that is not there and were left out"),
            (self.extras, "node(s) carry a skin, camera or light, which were left out"),
            (self.bad_materials, "part(s) name a material that is not there and were drawn in the default colour"),
            (self.bad_meshes, "node(s) name a mesh that is not there; the mesh was left out"),
            (self.bad_nodes, "root or child reference(s) name a node that is not there"),
            (self.cut, "subtree(s) were left out past the copy or depth limit (a cycle, or a subtree shared too often)"),
            (self.animations, "animation(s) were left out"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, what)| format!("{n} {what}"))
        .collect()
    }
}

/// Fills `out.materials` and `out.textures`; returns each material's kept texture slot.
fn materials(scene: &Scene, out: &mut MeshScene, counts: &mut Counts) -> Vec<Option<TextureRef>> {
    let mut seen: HashMap<(usize, [Wrap; 2], bool), usize> = HashMap::new();
    // Each image is probed once however many materials name it: a JPEG probe walks its fill bytes.
    let mut probed: HashMap<usize, Option<&[u8]>> = HashMap::new();
    let mut slots = Vec::with_capacity(scene.materials.len());
    for m in &scene.materials {
        let [r, g, b, a] = m.base_color.map(f64::from);
        let alpha = match m.alpha_mode {
            AlphaMode::Opaque => 1.0,
            AlphaMode::Blend => a,
            AlphaMode::Mask(_) if !a.is_finite() => a,
            AlphaMode::Mask(cutoff) => f64::from(u8::from(a >= f64::from(cutoff))),
        };
        let others = [m.metallic_roughness_texture, m.normal_texture, m.occlusion_texture, m.emissive_texture];
        counts.other_maps = counts.other_maps.saturating_add(u64::from(others.iter().any(Option::is_some)));
        let image = m.base_color_texture.and_then(|t| *probed.entry(t.image).or_insert_with(|| embedded(scene, t.image)));
        let slot = m.base_color_texture.filter(|_| image.is_some());
        counts.no_texture = counts.no_texture.saturating_add(u64::from(m.base_color_texture.is_some() && slot.is_none()));
        counts.masks = counts.masks.saturating_add(u64::from(slot.is_some() && matches!(m.alpha_mode, AlphaMode::Mask(_))));
        let texture = slot.zip(image).map(|(t, image)| texture(image, &t, m, &mut seen, out));
        out.materials.push(SceneMaterial {
            name: name(&m.name),
            rgba: [srgb(r), srgb(g), srgb(b), alpha],
            texture,
            emissive: m.emissive.map(|c| srgb(f64::from(c))),
        });
        slots.push(slot);
    }
    slots
}

/// The index of `slot`'s texture in `out.textures`, adding it the first time.
fn texture(image: &[u8], slot: &TextureRef, m: &Material, seen: &mut HashMap<(usize, [Wrap; 2], bool), usize>, out: &mut MeshScene) -> usize {
    let alpha = m.alpha_mode == AlphaMode::Blend;
    let key = (slot.image, slot.wrap, alpha);
    if let Some(i) = seen.get(&key) {
        return *i;
    }
    let i = out.textures.len();
    out.textures.push(SceneTexture { image: image.to_vec(), wrap: slot.wrap.map(wrap), alpha });
    seen.insert(key, i);
    i
}

/// Image `i`'s bytes when the writer can embed them: a PNG or JPEG file with a size.
fn embedded(scene: &Scene, i: usize) -> Option<&[u8]> {
    let data = scene.images.get(i)?.data.as_deref()?;
    picture_info(data).map(|_| data)
}

fn wrap(w: Wrap) -> TextureWrap {
    match w {
        Wrap::Repeat => TextureWrap::Repeat,
        Wrap::Clamp => TextureWrap::Clamp,
        Wrap::Mirror => TextureWrap::MirroredRepeat,
    }
}

fn mesh(m: &Mesh, slots: &[Option<TextureRef>], counts: &mut Counts) -> SceneMesh {
    counts.vertex_colours = counts.vertex_colours.saturating_add(u64::from(!m.colors.is_empty()));
    let primitives = m.parts.iter().map(|p| primitive(m, p, slots, counts)).collect();
    SceneMesh { name: name(&m.name), primitives }
}

/// One part as an indexed triangle list with one vertex per distinct corner. Normals are kept
/// when every corner has one, and the slot's UVs (transformed, then flipped to a bottom-left
/// origin) when every corner has one in its set.
fn primitive(m: &Mesh, part: &Part, slots: &[Option<TextureRef>], counts: &mut Counts) -> ScenePrimitive {
    let material = part.material.filter(|i| *i < slots.len());
    counts.bad_materials = counts.bad_materials.saturating_add(u64::from(part.material.is_some() && material.is_none()));
    let slot = material.and_then(|i| slots.get(i).copied().flatten());
    let normals = !m.normals.is_empty() && part.corners.iter().all(|c| (c.normal as usize) < m.normals.len());
    let set = slot.and_then(|t| m.uv_sets.get(t.uv_set as usize)).filter(|s| part.corners.iter().all(|c| (c.uv as usize) < s.len()));
    counts.no_uvs = counts.no_uvs.saturating_add(u64::from(slot.is_some() && set.is_none() && !part.corners.is_empty()));
    let transform = slot.and_then(|t| t.transform);
    let mut out = ScenePrimitive { material, ..ScenePrimitive::default() };
    let mut ids: HashMap<Corner, u32> = HashMap::new();
    'tri: for t in part.triangles() {
        // Checked first, so a dropped triangle leaves no vertex behind.
        if t.iter().any(|c| c.position as usize >= m.positions.len()) {
            counts.bad_triangles = counts.bad_triangles.saturating_add(1);
            continue;
        }
        let mut tri = [0u32; 3];
        for (id, c) in tri.iter_mut().zip(t) {
            let key = Corner { position: c.position, normal: if normals { c.normal } else { NONE }, uv: if set.is_some() { c.uv } else { NONE } };
            if let Some(known) = ids.get(&key) {
                *id = *known;
                continue;
            }
            let (Some(p), Ok(next)) = (m.positions.get(c.position as usize), u32::try_from(out.positions.len())) else {
                counts.bad_triangles = counts.bad_triangles.saturating_add(1);
                continue 'tri;
            };
            out.positions.push(*p);
            out.normals.extend(m.normals.get(c.normal as usize).filter(|_| normals));
            if let Some(&uv) = set.and_then(|s| s.get(c.uv as usize)) {
                let [u, v] = transform.map_or(uv, |x| x.apply(uv));
                out.uvs.push([f64::from(u), 1.0 - f64::from(v)]);
            }
            ids.insert(key, next);
            *id = next;
        }
        out.triangles.push(tri);
    }
    out
}

/// Copies the hierarchy from the roots down into `out.nodes` and `out.roots`.
fn nodes(scene: &Scene, up: UpAxis, out: &mut MeshScene, counts: &mut Counts) {
    counts.animations = wide(scene.animations.len());
    // (source node, parent copy, depth counting a root as 1)
    let mut stack: Vec<(usize, Option<usize>, usize)> = Vec::new();
    let kids = Children::of(scene);
    for (i, root) in scene.roots.iter().enumerate() {
        if *root >= scene.nodes.len() {
            counts.bad_nodes = counts.bad_nodes.saturating_add(1);
            continue;
        }
        if out.nodes.len() >= MAX_NODE_COPIES {
            counts.cut = counts.cut.saturating_add(wide(scene.roots.len() - i));
            return;
        }
        stack.push((*root, None, 1));
        copy_subtree(scene, up, &kids, &mut stack, out, counts);
    }
}

/// Each node's children split once into those that name a node and a count of those that do
/// not, so a copy's cost is what it queues, however many references its node lists.
struct Children {
    valid: Vec<Vec<usize>>,
    invalid: Vec<u64>,
}

impl Children {
    fn of(scene: &Scene) -> Self {
        let valid = scene.nodes.iter().map(|n| n.children.iter().copied().filter(|c| *c < scene.nodes.len()).collect::<Vec<_>>());
        let valid: Vec<Vec<usize>> = valid.collect();
        let invalid = scene.nodes.iter().zip(&valid).map(|(n, v)| wide(n.children.len() - v.len())).collect();
        Self { valid, invalid }
    }
}

/// Copies what `stack` holds, depth first. Only children that name a node are queued, and only
/// within the depth and copy budgets, so the work stays bounded by them and by the references
/// the scene lists, even when a node lists itself or nothing many times.
fn copy_subtree(
    scene: &Scene,
    up: UpAxis,
    kids: &Children,
    stack: &mut Vec<(usize, Option<usize>, usize)>,
    out: &mut MeshScene,
    counts: &mut Counts,
) {
    while let Some((source, parent, depth)) = stack.pop() {
        let Some(n) = scene.nodes.get(source) else {
            counts.bad_nodes = counts.bad_nodes.saturating_add(1);
            continue;
        };
        counts.extras = counts.extras.saturating_add(u64::from(n.skin.is_some() || n.camera.is_some() || n.light.is_some()));
        let mesh = n.mesh.filter(|m| *m < scene.meshes.len());
        counts.bad_meshes = counts.bad_meshes.saturating_add(u64::from(n.mesh.is_some() && mesh.is_none()));
        let transform = if parent.is_none() && up == UpAxis::Y { craft_mesh::mat_mul(&Y_TO_Z, &n.transform) } else { n.transform };
        let copy = out.nodes.len();
        out.nodes.push(SceneNode { name: name(&n.name), matrix: (transform != IDENTITY).then_some(transform), mesh, children: Vec::new() });
        match parent.and_then(|p| out.nodes.get_mut(p)) {
            Some(p) => p.children.push(copy),
            None => out.roots.push(copy),
        }
        let room = if depth < MAX_NODE_DEPTH { MAX_NODE_COPIES.saturating_sub(out.nodes.len().saturating_add(stack.len())) } else { 0 };
        let valid = kids.valid.get(source).map_or(&[][..], Vec::as_slice);
        counts.bad_nodes = counts.bad_nodes.saturating_add(kids.invalid.get(source).copied().unwrap_or(0));
        let queued = valid.len().min(room);
        counts.cut = counts.cut.saturating_add(wide(valid.len() - queued));
        stack.extend(valid.iter().take(queued).rev().map(|c| (*c, Some(copy), depth + 1)));
    }
}

/// Turns +Y up into +Z up: (x, y, z) to (x, −z, y), column-major.
const Y_TO_Z: Mat4 = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// A count as `u64`, which holds any `usize` on the targets Rust supports.
fn wide(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn name(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}

/// A linear colour component as a display value; a non-finite one is passed on for `write_prc` to refuse.
fn srgb(c: f64) -> f64 {
    if !c.is_finite() {
        return c;
    }
    f64::from(craft_mesh::linear_to_srgb(c.clamp(0.0, 1.0) as f32))
}
