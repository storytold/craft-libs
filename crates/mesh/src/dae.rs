//! COLLADA (`.dae`), read: `triangles`, `polylist` and `polygons` with `VERTEX` / `NORMAL` /
//! `TEXCOORD` inputs, materials bound through `instance_geometry` and `instance_controller`, the
//! visual scene's node hierarchy and transforms (`matrix`, `translate`, `rotate`, `scale`,
//! `lookat`), `instance_node`, `instance_camera`, and the asset's unit (`meter`) and `up_axis`.
//!
//! [`read_document`] returns the [`Scene`] in the file's own axes and units alongside
//! [`Document::meter`] and [`Document::up_axis`] (and [`Document::global_transform`] to convert to
//! Y-up metres). [`read`] returns the [`Scene`] directly.
//!
//! UVs are flipped to top-left origin (`1.0 - v`), matching the rest of this crate.

use std::collections::HashMap;
use std::rc::Rc;

use craft_xml::{El, Options, UnknownEntity};

use crate::{Camera, Corner, IDENTITY, Limits, Mat4, Material, Mesh, Node, Part, Projection, Result, Scene, malformed, mat_mul, too_large};

const KIND: &str = "COLLADA";
const MAX_XML_DEPTH: usize = 256;
const MAX_XML_NODES: usize = 2_000_000;

/// Up axis of a COLLADA `<asset>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpAxis {
    #[default]
    YUp,
    ZUp,
    XUp,
}

impl UpAxis {
    /// Rotation matrix that turns this axis into +Y up.
    pub fn to_y_up(self) -> Mat4 {
        match self {
            UpAxis::YUp => IDENTITY,
            UpAxis::ZUp => [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            UpAxis::XUp => [0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// A COLLADA document: the scene in the file's own axes and units, plus its `<asset>` scale and
/// up axis.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub scene: Scene,
    /// Metres per file unit (default `1.0`).
    pub meter: f64,
    pub up_axis: UpAxis,
}

impl Document {
    /// Transform from the file's axes and units to Y-up metres.
    pub fn global_transform(&self) -> Mat4 {
        let scale = [self.meter, 0.0, 0.0, 0.0, 0.0, self.meter, 0.0, 0.0, 0.0, 0.0, self.meter, 0.0, 0.0, 0.0, 0.0, 1.0];
        mat_mul(&self.up_axis.to_y_up(), &scale)
    }
}

/// A COLLADA document with all retained camera instances.
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentWithCameras {
    document: Document,
    additional_cameras: HashMap<usize, Vec<Camera>>,
}

impl DocumentWithCameras {
    /// The underlying document, including the primary camera for each node.
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Cameras attached to this node, in instance order, all sharing its world transform.
    /// Use this instead of reading the node's primary camera separately.
    pub fn cameras_for_node(&self, node: usize) -> impl Iterator<Item = &Camera> {
        let scene = &self.document.scene;
        let primary = scene.nodes.get(node).and_then(|n| n.camera).and_then(|i| scene.cameras.get(i));
        primary.into_iter().chain(self.additional_cameras.get(&node).into_iter().flatten())
    }
}

/// Read a COLLADA file into a [`Scene`] (in the file's own axes and units).
/// Only the first valid camera instance per node is retained.
pub fn read(bytes: &[u8], limits: &Limits) -> Result<Scene> {
    Ok(read_document(bytes, limits)?.scene)
}

/// Read a COLLADA file into a [`Document`], retaining only the first valid camera per node.
pub fn read_document(bytes: &[u8], limits: &Limits) -> Result<Document> {
    Ok(read_document_inner(bytes, limits, false)?.document)
}

/// Read a COLLADA document and retain additional cameras without adding scene nodes.
///
/// Primary and additional cameras share [`Limits::max_extras`]. Valid instances are
/// retained in scene traversal order until that limit is reached.
pub fn read_document_with_cameras(bytes: &[u8], limits: &Limits) -> Result<DocumentWithCameras> {
    read_document_inner(bytes, limits, true)
}

fn read_document_inner(bytes: &[u8], limits: &Limits, retain_additional_cameras: bool) -> Result<DocumentWithCameras> {
    let mut opts = Options::local();
    opts.unknown_entity = UnknownEntity::Literal;
    opts.strict_end_tags = true;
    opts.limits.max_input = limits.max_data_bytes;
    opts.limits.max_depth = MAX_XML_DEPTH;
    opts.limits.max_nodes = MAX_XML_NODES;
    let root = opts.parse(bytes).map_err(|e| if e.is_limit() { too_large(KIND, e) } else { malformed(KIND, e) })?;
    if root.name != "COLLADA" {
        return Err(malformed(KIND, "not a COLLADA document"));
    }

    let mut ids: HashMap<&str, &El> = HashMap::new();
    let mut stack = vec![&root];
    while let Some(el) = stack.pop() {
        if let Some(id) = el.attr("id") {
            ids.entry(id).or_insert(el);
        }
        stack.extend(el.elements());
    }

    let asset = root.child("asset");
    let meter = asset
        .and_then(|a| a.child("unit"))
        .and_then(|u| u.attr("meter"))
        .and_then(|m| m.trim().parse::<f64>().ok())
        .filter(|m| m.is_finite() && *m > 0.0)
        .unwrap_or(1.0);
    let up_axis = match asset.and_then(|a| a.child("up_axis")).map(|u| u.text().trim().to_owned()).as_deref() {
        Some("Z_UP") => UpAxis::ZUp,
        Some("X_UP") => UpAxis::XUp,
        _ => UpAxis::YUp,
    };

    let mut reader = Reader {
        ids,
        sources: HashMap::new(),
        limits,
        scene: Scene::default(),
        materials: HashMap::new(),
        mesh_cache: HashMap::new(),
        triangles: 0,
        retain_additional_cameras,
        additional_cameras: HashMap::new(),
        camera_slots: limits.max_extras,
    };

    let visual_scene = root
        .child("scene")
        .and_then(|s| s.child("instance_visual_scene"))
        .and_then(|i| reader.get(i.attr("url")))
        .or_else(|| root.child("library_visual_scenes").and_then(|l| l.child("visual_scene")));

    if let Some(vs) = visual_scene {
        for n in vs.children("node") {
            let idx = reader.read_node(n, 0)?;
            reader.scene.roots.push(idx);
        }
    } else {
        for geom in root.children("library_geometries").flat_map(|l| l.children("geometry")) {
            let mesh_idx = reader.get_or_build_mesh(geom, &[])?;
            let name = reader.scene.meshes[mesh_idx].name.clone();
            let node_idx = reader.push_node(Node { name, mesh: Some(mesh_idx), ..Node::default() })?;
            reader.scene.roots.push(node_idx);
        }
    }

    Ok(DocumentWithCameras { document: Document { scene: reader.scene, meter, up_axis }, additional_cameras: reader.additional_cameras })
}

struct Source {
    data: Vec<f64>,
    stride: usize,
    offset: usize,
}

impl Source {
    fn get<const W: usize>(&self, i: u32) -> Option<[f64; W]> {
        let start = (i as usize).checked_mul(self.stride)?.checked_add(self.offset)?;
        self.data.get(start..start.checked_add(W)?)?.try_into().ok()
    }
}

struct Input {
    offset: usize,
    source: Rc<Source>,
}

struct Reader<'a> {
    ids: HashMap<&'a str, &'a El>,
    sources: HashMap<&'a str, Rc<Source>>,
    limits: &'a Limits,
    scene: Scene,
    materials: HashMap<String, usize>,
    /// Keyed by `(geometry element pointer, sorted (symbol, material_name) bindings)`.
    mesh_cache: HashMap<(usize, Vec<(String, String)>), usize>,
    triangles: usize,
    retain_additional_cameras: bool,
    additional_cameras: HashMap<usize, Vec<Camera>>,
    camera_slots: usize,
}

impl<'a> Reader<'a> {
    fn get(&self, url: Option<&str>) -> Option<&'a El> {
        url.and_then(|u| u.strip_prefix('#')).and_then(|id| self.ids.get(id)).copied()
    }

    fn warn(&mut self, msg: String) {
        if !self.scene.warnings.contains(&msg) {
            self.scene.warnings.push(msg);
        }
    }

    fn push_node(&mut self, node: Node) -> Result<usize> {
        if self.scene.nodes.len() >= self.limits.max_nodes {
            return Err(too_large(KIND, format!("more than {} nodes", self.limits.max_nodes)));
        }
        let idx = self.scene.nodes.len();
        self.scene.nodes.push(node);
        Ok(idx)
    }

    fn intern_material(&mut self, name: &str) -> Result<usize> {
        if let Some(&idx) = self.materials.get(name) {
            return Ok(idx);
        }
        if self.scene.materials.len() >= self.limits.max_materials {
            return Err(too_large(KIND, format!("more than {} materials", self.limits.max_materials)));
        }
        let idx = self.scene.materials.len();
        self.scene.materials.push(Material { name: name.to_owned(), ..Material::default() });
        self.materials.insert(name.to_owned(), idx);
        Ok(idx)
    }

    fn source(&mut self, url: Option<&str>) -> Result<Rc<Source>> {
        let el = self.get(url).ok_or_else(|| malformed(KIND, format!("missing source {}", url.unwrap_or("(none)"))))?;
        let id = el.attr("id").unwrap_or_default();
        if !id.is_empty()
            && let Some(s) = self.sources.get(id)
        {
            return Ok(s.clone());
        }
        let acc = el.child("technique_common").and_then(|t| t.child("accessor"));
        let array = acc.and_then(|a| self.get(a.attr("source"))).or_else(|| el.child("float_array"));
        let array = array.ok_or_else(|| malformed(KIND, format!("source \"{id}\" has no float_array")))?;
        let num = |k: &str, default: usize| acc.and_then(|a| a.attr(k)).and_then(|v| v.parse().ok()).unwrap_or(default);
        let data = parse_floats(array)?;
        if data.len() / 3 > self.limits.max_vertices {
            return Err(too_large(KIND, format!("source has more than {} vertices", self.limits.max_vertices)));
        }
        let s = Rc::new(Source { data, stride: num("stride", 1).max(1), offset: num("offset", 0) });
        if !id.is_empty() {
            self.sources.insert(id, s.clone());
        }
        Ok(s)
    }

    fn read_node(&mut self, el: &'a El, depth: usize) -> Result<usize> {
        if depth > self.limits.max_depth {
            return Err(too_large(KIND, format!("node hierarchy is deeper than {}", self.limits.max_depth)));
        }
        let name = el.attr("name").or_else(|| el.attr("id")).unwrap_or_default().to_string();
        let transform = node_transform(el)?;
        let mut node = Node { name: name.clone(), transform, ..Node::default() };
        let mut additional_cameras = Vec::new();

        for inst in el.elements() {
            if inst.name == "instance_camera" {
                if self.camera_slots > 0
                    && (node.camera.is_none() || self.retain_additional_cameras)
                    && let Some(cam_el) = self.get(inst.attr("url"))
                    && let Some(cam) = parse_camera(cam_el, &name)
                {
                    self.camera_slots -= 1;
                    if node.camera.is_none() {
                        node.camera = Some(self.scene.cameras.len());
                        self.scene.cameras.push(cam);
                    } else {
                        additional_cameras.push(cam);
                    }
                }
                continue;
            }
            let (geom, bind_shape) = match inst.name.as_str() {
                "instance_geometry" => (self.get(inst.attr("url")), IDENTITY),
                "instance_controller" => {
                    let skin = self.get(inst.attr("url")).and_then(|c| c.child("skin"));
                    let bind = skin.and_then(|s| s.child("bind_shape_matrix")).map(parse_floats).transpose()?.and_then(|v| row_major_matrix(&v));
                    (skin.and_then(|s| self.get(s.attr("source"))), bind.unwrap_or(IDENTITY))
                }
                _ => continue,
            };
            let Some(geom) = geom else {
                self.warn("an instanced geometry is missing from the file".into());
                continue;
            };
            let bind = self.bindings(inst);
            let mesh_idx = self.get_or_build_mesh(geom, &bind)?;
            if node.mesh.is_none() && bind_shape == IDENTITY {
                node.mesh = Some(mesh_idx);
            } else {
                let child = self.push_node(Node { name: name.clone(), transform: bind_shape, mesh: Some(mesh_idx), ..Node::default() })?;
                node.children.push(child);
            }
        }

        for child_el in el.children("node") {
            let child = self.read_node(child_el, depth + 1)?;
            node.children.push(child);
        }
        for inst_node in el.children("instance_node") {
            if let Some(target) = self.get(inst_node.attr("url")) {
                let child = self.read_node(target, depth + 1)?;
                node.children.push(child);
            }
        }

        let id = self.push_node(node)?;
        if !additional_cameras.is_empty() {
            self.additional_cameras.insert(id, additional_cameras);
        }
        Ok(id)
    }

    fn bindings(&self, inst: &El) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = inst
            .child("bind_material")
            .and_then(|b| b.child("technique_common"))
            .into_iter()
            .flat_map(|t| t.children("instance_material"))
            .filter_map(|im| {
                let symbol = im.attr("symbol")?;
                let mat = self.get(im.attr("target"));
                let name = mat.and_then(|m| m.attr("name").or_else(|| m.attr("id"))).unwrap_or(symbol);
                Some((symbol.to_string(), name.to_string()))
            })
            .collect();
        out.sort();
        out
    }

    fn get_or_build_mesh(&mut self, geom: &'a El, bind: &[(String, String)]) -> Result<usize> {
        let key = ((geom as *const El) as usize, bind.to_vec());
        if let Some(&idx) = self.mesh_cache.get(&key) {
            return Ok(idx);
        }
        if self.scene.meshes.len() >= self.limits.max_meshes {
            return Err(too_large(KIND, format!("more than {} meshes", self.limits.max_meshes)));
        }
        let mesh = self.build_mesh(geom, bind)?;
        let idx = self.scene.meshes.len();
        self.scene.meshes.push(mesh);
        self.mesh_cache.insert(key, idx);
        Ok(idx)
    }

    fn build_mesh(&mut self, geom: &'a El, bind: &[(String, String)]) -> Result<Mesh> {
        let name = geom.attr("name").or_else(|| geom.attr("id")).unwrap_or_default().to_string();
        let mut mesh = Mesh { name, ..Mesh::default() };
        let Some(mesh_el) = geom.child("mesh") else { return Ok(mesh) };

        let mut pools = MeshPools::default();
        let mut part_count: usize = self.scene.meshes.iter().map(|m| m.parts.len()).sum();

        for prim in mesh_el.elements() {
            if !matches!(prim.name.as_str(), "triangles" | "polylist" | "polygons") {
                if matches!(prim.name.as_str(), "lines" | "linestrips" | "trifans" | "tristrips") {
                    self.warn(format!("skipped COLLADA <{}> (only triangles and polygons are supported)", prim.name));
                }
                continue;
            }
            let Some(inputs) = self.read_primitive_inputs(prim)? else {
                self.warn("skipped COLLADA primitives without positions".into());
                continue;
            };
            part_count += 1;
            if part_count > self.limits.max_parts {
                return Err(too_large(KIND, format!("more than {} parts", self.limits.max_parts)));
            }
            let material = match prim.attr("material") {
                Some(sym) => {
                    let mat_name = bind.binary_search_by(|(k, _)| k.as_str().cmp(sym)).ok().map_or(sym, |i| bind[i].1.as_str());
                    Some(self.intern_material(mat_name)?)
                }
                None => None,
            };
            let lists = read_primitive_lists(prim, inputs.stride)?;
            let part = self.build_part(material, &inputs, &lists, &mut mesh, &mut pools)?;
            if !part.corners.is_empty() {
                mesh.parts.push(part);
            }
        }
        if !pools.uvs.is_empty() {
            mesh.uv_sets.push(pools.uvs);
        }
        Ok(mesh)
    }

    fn read_primitive_inputs(&mut self, prim: &'a El) -> Result<Option<PrimInputs>> {
        let (mut position, mut normal, mut uv): (Option<Input>, Option<Input>, Option<(u32, Input)>) = (None, None, None);
        let mut stride = 1usize;
        for input in prim.children("input") {
            let offset: usize = input.attr("offset").and_then(|o| o.parse().ok()).unwrap_or(0);
            stride = stride.max(offset.saturating_add(1));
            let nested: Vec<&El> = match input.attr("semantic") {
                Some("VERTEX") => self.get(input.attr("source")).map(|v| v.children("input").collect()).unwrap_or_default(),
                _ => vec![input],
            };
            for i in nested {
                let set: u32 = input.attr("set").or_else(|| i.attr("set")).and_then(|s| s.parse().ok()).unwrap_or(0);
                match i.attr("semantic") {
                    Some("POSITION") => position = Some(Input { offset, source: self.source(i.attr("source"))? }),
                    Some("NORMAL") if normal.is_none() => normal = Some(Input { offset, source: self.source(i.attr("source"))? }),
                    Some("TEXCOORD") if uv.as_ref().is_none_or(|(s, _)| set < *s) => {
                        uv = Some((set, Input { offset, source: self.source(i.attr("source"))? }))
                    }
                    _ => {}
                }
            }
        }
        Ok(position.map(|position| PrimInputs { stride, position, normal, uv: uv.map(|(_, i)| i) }))
    }

    fn build_part(
        &mut self,
        material: Option<usize>,
        inputs: &PrimInputs,
        lists: &[(Vec<u32>, Vec<usize>)],
        mesh: &mut Mesh,
        pools: &mut MeshPools,
    ) -> Result<Part> {
        let mut part = Part { material, ..Part::default() };
        let mut all_triangles = true;
        for (p, sizes) in lists {
            let mut next = 0usize;
            for &size in sizes {
                if size > self.limits.max_face_corners {
                    return Err(too_large(KIND, format!("face has more than {} corners", self.limits.max_face_corners)));
                }
                if size >= 3 {
                    self.triangles = self.triangles.saturating_add(size - 2);
                    if self.triangles > self.limits.max_triangles {
                        return Err(too_large(KIND, format!("more than {} triangles", self.limits.max_triangles)));
                    }
                    if size != 3 {
                        all_triangles = false;
                    }
                    for c in next..next.saturating_add(size) {
                        let idx =
                            |input: &Input| c.checked_mul(inputs.stride).and_then(|b| b.checked_add(input.offset)).and_then(|k| p.get(k)).copied();
                        let pos = idx(&inputs.position)
                            .and_then(|i| inputs.position.source.get::<3>(i))
                            .ok_or_else(|| malformed(KIND, "polygon index out of range"))?;
                        let pos_idx = intern_f64_3(&mut mesh.positions, &mut pools.pos_ids, pos, self.limits.max_vertices, "positions")?;
                        let norm_idx = match inputs.normal.as_ref().and_then(|n| idx(n).and_then(|i| n.source.get::<3>(i))) {
                            Some(n) => intern_f64_3(&mut mesh.normals, &mut pools.norm_ids, n, self.limits.max_vertices, "normals")?,
                            None => crate::NONE,
                        };
                        let uv_idx = match inputs.uv.as_ref().and_then(|t| idx(t).and_then(|i| t.source.get::<2>(i))) {
                            Some([u, v]) => intern_uv(&mut pools.uvs, &mut pools.uv_ids, [u as f32, 1.0 - v as f32], self.limits.max_vertices)?,
                            None => crate::NONE,
                        };
                        part.corners.push(Corner { position: pos_idx, normal: norm_idx, uv: uv_idx });
                    }
                    part.face_sizes.push(size as u32);
                }
                next = next.saturating_add(size);
            }
        }
        if all_triangles {
            part.face_sizes.clear();
        }
        Ok(part)
    }
}

#[derive(Default)]
struct MeshPools {
    pos_ids: HashMap<[u64; 3], u32>,
    norm_ids: HashMap<[u64; 3], u32>,
    uv_ids: HashMap<[u32; 2], u32>,
    uvs: Vec<[f32; 2]>,
}

struct PrimInputs {
    stride: usize,
    position: Input,
    normal: Option<Input>,
    uv: Option<Input>,
}

fn read_primitive_lists(prim: &El, stride: usize) -> Result<Vec<(Vec<u32>, Vec<usize>)>> {
    match prim.name.as_str() {
        "polygons" => prim
            .children("p")
            .map(|p| {
                let p = parse_indices(p)?;
                let n = p.len() / stride;
                Ok((p, vec![n]))
            })
            .collect(),
        _ => {
            let p = prim.child("p").map(parse_indices).transpose()?.unwrap_or_default();
            let sizes = match prim.child("vcount") {
                Some(v) if prim.name == "polylist" => parse_indices(v)?.into_iter().map(|c| c as usize).collect(),
                _ => vec![3; p.len() / stride / 3],
            };
            Ok(vec![(p, sizes)])
        }
    }
}

fn intern_f64_3(pool: &mut Vec<[f64; 3]>, map: &mut HashMap<[u64; 3], u32>, val: [f64; 3], limit: usize, what: &str) -> Result<u32> {
    let key = val.map(f64::to_bits);
    if let Some(&id) = map.get(&key) {
        return Ok(id);
    }
    if pool.len() >= limit {
        return Err(too_large(KIND, format!("more than {limit} {what}")));
    }
    let id = u32::try_from(pool.len()).map_err(|_| too_large(KIND, format!("too many {what}")))?;
    pool.push(val);
    map.insert(key, id);
    Ok(id)
}

fn intern_uv(pool: &mut Vec<[f32; 2]>, map: &mut HashMap<[u32; 2], u32>, val: [f32; 2], limit: usize) -> Result<u32> {
    let key = val.map(f32::to_bits);
    if let Some(&id) = map.get(&key) {
        return Ok(id);
    }
    if pool.len() >= limit {
        return Err(too_large(KIND, format!("more than {limit} UVs")));
    }
    let id = u32::try_from(pool.len()).map_err(|_| too_large(KIND, "too many UVs"))?;
    pool.push(val);
    map.insert(key, id);
    Ok(id)
}

fn parse_floats(el: &El) -> Result<Vec<f64>> {
    el.text().split_whitespace().map(|w| w.parse::<f64>().map_err(|_| malformed(KIND, format!("bad number \"{w}\" in <{}>", el.name)))).collect()
}

fn parse_indices(el: &El) -> Result<Vec<u32>> {
    el.text().split_whitespace().map(|w| w.parse::<u32>().map_err(|_| malformed(KIND, format!("bad index \"{w}\" in <{}>", el.name)))).collect()
}

fn parse_camera(cam: &El, node_name: &str) -> Option<Camera> {
    let tc = cam.child("optics")?.child("technique_common")?;
    let num = |e: &El, k: &str| e.child(k).and_then(|v| v.text().trim().parse::<f64>().ok()).filter(|v| v.is_finite() && *v > 0.0);
    let name = if node_name.is_empty() { cam.attr("name").or_else(|| cam.attr("id")).unwrap_or_default().to_string() } else { node_name.to_string() };
    if let Some(p) = tc.child("perspective") {
        let x_to_y = |x: f64| Some(2.0 * ((x * 0.5).to_radians().tan() / num(p, "aspect_ratio")?).atan().to_degrees());
        let fov_deg = num(p, "yfov").or_else(|| num(p, "xfov").and_then(x_to_y)).or_else(|| num(p, "xfov")).unwrap_or(40.0);
        let aspect = num(p, "aspect_ratio");
        let znear = num(p, "znear").unwrap_or(0.1);
        let zfar = num(p, "zfar");
        return Some(Camera { name, projection: Projection::Perspective { yfov: fov_deg.to_radians(), aspect, znear, zfar } });
    }
    let o = tc.child("orthographic")?;
    let aspect = num(o, "aspect_ratio");
    let ymag = num(o, "ymag").or_else(|| Some(num(o, "xmag")? / aspect?)).or_else(|| num(o, "xmag")).unwrap_or(1.0);
    let xmag = num(o, "xmag").unwrap_or_else(|| ymag * aspect.unwrap_or(1.0));
    let znear = num(o, "znear").unwrap_or(0.1);
    let zfar = num(o, "zfar").unwrap_or(1000.0);
    Some(Camera { name, projection: Projection::Orthographic { xmag, ymag, znear, zfar } })
}

/// Row-major 4×4 floats to a column-major matrix.
fn row_major_matrix(v: &[f64]) -> Option<Mat4> {
    let a: [f64; 16] = v.get(..16)?.try_into().ok()?;
    Some([a[0], a[4], a[8], a[12], a[1], a[5], a[9], a[13], a[2], a[6], a[10], a[14], a[3], a[7], a[11], a[15]])
}

fn node_transform(node: &El) -> Result<Mat4> {
    let mut m = IDENTITY;
    for t in node.elements() {
        let v = match t.name.as_str() {
            "matrix" | "translate" | "rotate" | "scale" | "lookat" => parse_floats(t)?,
            _ => continue,
        };
        let step = match (t.name.as_str(), v.as_slice()) {
            ("matrix", _) => row_major_matrix(&v),
            ("translate", [x, y, z, ..]) => Some([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, *x, *y, *z, 1.0]),
            ("scale", [x, y, z, ..]) => Some([*x, 0.0, 0.0, 0.0, 0.0, *y, 0.0, 0.0, 0.0, 0.0, *z, 0.0, 0.0, 0.0, 0.0, 1.0]),
            ("rotate", [x, y, z, deg, ..]) => axis_angle_matrix([*x, *y, *z], deg.to_radians()),
            ("lookat", [ex, ey, ez, ix, iy, iz, ux, uy, uz, ..]) => lookat_matrix([*ex, *ey, *ez], [*ix, *iy, *iz], [*ux, *uy, *uz]),
            _ => None,
        };
        if let Some(s) = step {
            m = mat_mul(&m, &s);
        }
    }
    Ok(m)
}

fn normalize(v: [f64; 3]) -> Option<[f64; 3]> {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (len.is_finite() && len > 0.0).then(|| v.map(|x| x / len))
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn axis_angle_matrix(axis: [f64; 3], angle: f64) -> Option<Mat4> {
    let [x, y, z] = normalize(axis)?;
    let (s, c) = angle.sin_cos();
    let t = 1.0 - c;
    let m = [
        t * x * x + c,
        t * x * y + s * z,
        t * x * z - s * y,
        0.0,
        t * x * y - s * z,
        t * y * y + c,
        t * y * z + s * x,
        0.0,
        t * x * z + s * y,
        t * y * z - s * x,
        t * z * z + c,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ];
    m.iter().all(|v| v.is_finite()).then_some(m)
}

fn lookat_matrix(eye: [f64; 3], center: [f64; 3], up: [f64; 3]) -> Option<Mat4> {
    let f = normalize([center[0] - eye[0], center[1] - eye[1], center[2] - eye[2]])?;
    let s = normalize(cross(f, up))?;
    let u = cross(s, f);
    let m = [s[0], s[1], s[2], 0.0, u[0], u[1], u[2], 0.0, -f[0], -f[1], -f[2], 0.0, eye[0], eye[1], eye[2], 1.0];
    m.iter().all(|v| v.is_finite()).then_some(m)
}
