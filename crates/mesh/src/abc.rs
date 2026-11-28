//! Alembic (`.abc`), read: the Ogawa container, `AbcGeom_Xform` hierarchy (matrix, translate,
//! rotate, scale ops; `.inherits`), `AbcGeom_PolyMesh` at its first sample (positions `P`,
//! polygons `.faceIndices` / `.faceCounts` turned counter-clockwise from Alembic's clockwise
//! winding, normals `N` and UVs `uv` at any geometry scope, indexed or not), and
//! `AbcGeom_FaceSet` children as materials.
//!
//! UVs are flipped to top-left origin (`1.0 - v`), matching the rest of this crate. HDF5-based
//! Alembic files are refused with [`Error::Unsupported`].

use std::collections::HashMap;

use crate::{Corner, Error, IDENTITY, Limits, Mat4, Material, Mesh, Node, Part, Result, Scene, malformed, mat_mul, too_large};

const KIND: &str = "Alembic";
const OGAWA_MAGIC: &[u8] = b"Ogawa";
const HDF5_MAGIC: &[u8] = b"\x89HDF\r\n\x1a\n";
const DATA_BIT: u64 = 1 << 63;
const KEY_BYTES: usize = 16;
const OBJECT_HASH_BYTES: usize = 32;
const MAX_INDEXED_META: usize = 255;

/// Read an Alembic (`.abc` Ogawa) file into a [`Scene`].
pub fn read(bytes: &[u8], limits: &Limits) -> Result<Scene> {
    if bytes.starts_with(HDF5_MAGIC) {
        return Err(Error::Unsupported("HDF5-based Alembic (re-export the file as Alembic Ogawa, the default since Alembic 1.5)".into()));
    }
    if !bytes.starts_with(OGAWA_MAGIC) || bytes.len() < 16 {
        return Err(malformed(KIND, "not an Alembic Ogawa file"));
    }
    let mut o = Ogawa { b: bytes, budget: limits.max_data_bytes, max_budget: limits.max_data_bytes, indexed_meta: vec![String::new()] };
    let root_ref = o.u64_at(8)?;
    let root = o.group(root_ref)?;
    let is_data = |i: usize| root.get(i).is_some_and(|r| r & DATA_BIT != 0);
    if root.len() < 6 || !(0..6).all(|i| is_data(i) != (i == 2)) {
        return Err(malformed(KIND, "the archive group does not have the Alembic layout"));
    }
    let block = o.data(root[5])?;
    read_indexed_meta(block, &mut o.indexed_meta)?;

    let mut walk = Walk { o, limits, scene: Scene::default(), materials: HashMap::new(), visits: 0, triangles: 0, parts: 0 };
    walk.object(root[2], None, "", 0)?;
    Ok(walk.scene)
}

struct Ogawa<'a> {
    b: &'a [u8],
    budget: usize,
    max_budget: usize,
    indexed_meta: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum PropKind {
    Compound,
    Scalar,
    Array,
}

struct Prop {
    name: String,
    meta: String,
    kind: PropKind,
    pod: u32,
    extent: usize,
    group: u64,
}

fn meta_get<'m>(meta: &'m str, key: &str) -> Option<&'m str> {
    meta.split(';').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| v)
}

struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.pos.checked_add(n).and_then(|end| self.b.get(self.pos..end)).ok_or_else(|| malformed(KIND, "header block ends early"))?;
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?.first().copied().unwrap_or(0))
    }

    fn u32(&mut self) -> Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn hinted(&mut self, hint: u32) -> Result<usize> {
        Ok(match hint {
            0 => self.u8()? as usize,
            1 => {
                let s = self.take(2)?;
                u16::from_le_bytes([s[0], s[1]]) as usize
            }
            2 => self.u32()? as usize,
            _ => return Err(malformed(KIND, format!("bad size hint {hint}"))),
        })
    }

    fn string(&mut self, n: usize) -> Result<String> {
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

fn f16_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0_f64 } else { 1.0_f64 };
    let exp = (bits >> 10) & 0x1f;
    let mant = bits & 0x03ff;
    match exp {
        0 => sign * (f64::from(mant) * 2.0_f64.powi(-24)),
        31 => {
            if mant == 0 {
                sign * f64::INFINITY
            } else {
                f64::NAN
            }
        }
        _ => sign * ((1.0 + f64::from(mant) / 1024.0) * 2.0_f64.powi(i32::from(exp) - 15)),
    }
}

impl<'a> Ogawa<'a> {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.budget = self.budget.checked_sub(bytes).ok_or_else(|| too_large(KIND, format!("decodes to more than {} bytes", self.max_budget)))?;
        Ok(())
    }

    fn u64_at(&self, pos: u64) -> Result<u64> {
        let p = usize::try_from(pos).map_err(|_| malformed(KIND, "offset out of range"))?;
        let s =
            p.checked_add(8).and_then(|e| self.b.get(p..e)).ok_or_else(|| malformed(KIND, format!("offset {pos} is past the end of the file")))?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(u64::from_le_bytes(a))
    }

    fn group(&mut self, r: u64) -> Result<Vec<u64>> {
        if r & DATA_BIT != 0 {
            return Err(malformed(KIND, "expected a group, found a data block"));
        }
        if r == 0 {
            return Ok(Vec::new());
        }
        let n = self.u64_at(r)?;
        let room = (self.b.len() as u64).saturating_sub(r.saturating_add(8)) / 8;
        if n > room {
            return Err(malformed(KIND, format!("group at {r} claims {n} children")));
        }
        self.charge(n as usize * 8)?;
        (0..n).map(|i| self.u64_at(r + 8 + 8 * i)).collect()
    }

    fn data(&self, r: u64) -> Result<&'a [u8]> {
        if r & DATA_BIT == 0 {
            return Err(malformed(KIND, "expected a data block, found a group"));
        }
        let off = r & !DATA_BIT;
        if off == 0 {
            return Ok(&[]);
        }
        let size = self.u64_at(off)?;
        let start = usize::try_from(off + 8).map_err(|_| malformed(KIND, "offset out of range"))?;
        usize::try_from(size)
            .ok()
            .and_then(|s| start.checked_add(s))
            .and_then(|end| self.b.get(start..end))
            .ok_or_else(|| malformed(KIND, format!("data block at {off} ({size} bytes) overruns the file")))
    }

    fn meta_at(&self, index: u8) -> String {
        self.indexed_meta.get(index as usize).cloned().unwrap_or_default()
    }

    fn compound(&mut self, r: u64) -> Result<Vec<Prop>> {
        let children = self.group(r)?;
        let Some(&last) = children.last().filter(|c| **c & DATA_BIT != 0) else { return Ok(Vec::new()) };
        let block = self.data(last)?;
        self.charge(block.len())?;
        let mut c = Cursor { b: block, pos: 0 };
        let mut out = Vec::new();
        while c.pos < block.len() {
            let info = c.u32()?;
            let hint = (info & 0xc) >> 2;
            let kind = match info & 0x3 {
                0 => PropKind::Compound,
                1 => PropKind::Scalar,
                _ => PropKind::Array,
            };
            let (pod, extent) = ((info & 0xf0) >> 4, ((info & 0xff000) >> 12) as usize);
            if kind != PropKind::Compound {
                let _samples = c.hinted(hint)?;
                if info & 0x200 != 0 {
                    c.hinted(hint)?;
                    c.hinted(hint)?;
                }
                if info & 0x100 != 0 {
                    c.hinted(hint)?;
                }
            }
            let name_len = c.hinted(hint)?;
            let name = c.string(name_len)?;
            let meta_index = ((info & 0xff0_0000) >> 20) as u8;
            let meta = if meta_index == 0xff {
                let n = c.hinted(hint)?;
                c.string(n)?
            } else {
                self.meta_at(meta_index)
            };
            let group = children.get(out.len()).copied().ok_or_else(|| malformed(KIND, format!("property \"{name}\" has no group")))?;
            out.push(Prop { name, meta, kind, pod, extent, group });
        }
        Ok(out)
    }

    fn values(&mut self, property: &Prop) -> Result<Vec<f64>> {
        let samples = self.group(property.group)?;
        let Some(&first) = samples.first() else { return Ok(Vec::new()) };

        let raw = self.data(first)?;
        let raw = if raw.is_empty() {
            raw
        } else {
            raw.get(KEY_BYTES..).ok_or_else(|| malformed(KIND, format!("sample of \"{}\" is shorter than its key", property.name)))?
        };

        let value_bytes = match property.pod {
            0..=2 => 1,
            3 | 4 | 9 => 2,
            5 | 6 | 10 => 4,
            7 | 8 | 11 => 8,
            other => return Err(Error::Unsupported(format!("Alembic property \"{}\" of data type {other}", property.name))),
        };
        self.charge((raw.len() / value_bytes).saturating_mul(8))?;

        Ok(match property.pod {
            0 | 1 => raw.iter().map(|v| *v as f64).collect(),
            2 => raw.iter().map(|v| *v as i8 as f64).collect(),
            3 => raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c) as f64).collect(),
            4 => raw.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c) as f64).collect(),
            5 => raw.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c) as f64).collect(),
            6 => raw.as_chunks::<4>().0.iter().map(|c| i32::from_le_bytes(*c) as f64).collect(),
            7 => raw.as_chunks::<8>().0.iter().map(|c| u64::from_le_bytes(*c) as f64).collect(),
            8 => raw.as_chunks::<8>().0.iter().map(|c| i64::from_le_bytes(*c) as f64).collect(),
            9 => raw.as_chunks::<2>().0.iter().map(|c| f16_to_f64(u16::from_le_bytes(*c))).collect(),
            10 => raw.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c) as f64).collect(),
            _ => raw.as_chunks::<8>().0.iter().map(|c| f64::from_le_bytes(*c)).collect(),
        })
    }

    fn object_headers(&mut self, children: &[u64]) -> Result<Vec<(String, String)>> {
        let Some(&last) = children.last().filter(|c| **c & DATA_BIT != 0) else { return Ok(Vec::new()) };
        let block = self.data(last)?;
        self.charge(block.len())?;
        let body = block.get(..block.len().saturating_sub(OBJECT_HASH_BYTES)).unwrap_or_default();
        let mut c = Cursor { b: body, pos: 0 };
        let mut out = Vec::new();
        while c.pos < body.len() {
            let n = c.u32()? as usize;
            let name = c.string(n)?;
            let index = c.u8()?;
            let meta = if index == 0xff {
                let n = c.u32()? as usize;
                c.string(n)?
            } else {
                self.meta_at(index)
            };
            out.push((name, meta));
        }
        Ok(out)
    }
}

fn read_indexed_meta(block: &[u8], out: &mut Vec<String>) -> Result<()> {
    let mut c = Cursor { b: block, pos: 0 };
    while c.pos < block.len() && out.len() < MAX_INDEXED_META {
        let n = c.u8()? as usize;
        out.push(c.string(n)?);
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Scope {
    Constant,
    Uniform,
    Vertex,
    FaceVarying,
}

struct Param {
    vals: Vec<f64>,
    indices: Option<Vec<f64>>,
    scope: Scope,
}

impl Param {
    fn get<const W: usize>(&self, k: usize, p: usize, f: usize) -> Option<[f64; W]> {
        let i = match self.scope {
            Scope::Constant => 0,
            Scope::Uniform => f,
            Scope::Vertex => p,
            Scope::FaceVarying => k,
        };
        let i = match &self.indices {
            Some(ix) => Some(*ix.get(i)?).filter(|v| *v >= 0.0)? as usize,
            None => i,
        };
        let start = i.checked_mul(W)?;
        self.vals.get(start..start.checked_add(W)?)?.try_into().ok()
    }
}

fn find<'p>(props: &'p [Prop], name: &str) -> Option<&'p Prop> {
    props.iter().find(|p| p.name == name)
}

fn schema_compound<'p>(props: &'p [Prop], schema: &str, name: &str) -> Option<&'p Prop> {
    let compounds = || props.iter().filter(|p| p.kind == PropKind::Compound);
    compounds().find(|p| meta_get(&p.meta, "schema") == Some(schema)).or_else(|| compounds().find(|p| p.name == name))
}

struct Walk<'a> {
    o: Ogawa<'a>,
    limits: &'a Limits,
    scene: Scene,
    materials: HashMap<String, usize>,
    visits: usize,
    triangles: usize,
    parts: usize,
}

impl Walk<'_> {
    fn warn(&mut self, msg: String) {
        if !self.scene.warnings.contains(&msg) {
            self.scene.warnings.push(msg);
        }
    }

    fn push_node(&mut self, node: Node, parent: Option<usize>) -> Result<usize> {
        if self.scene.nodes.len() >= self.limits.max_nodes {
            return Err(too_large(KIND, format!("more than {} nodes", self.limits.max_nodes)));
        }
        let idx = self.scene.nodes.len();
        self.scene.nodes.push(node);
        match parent {
            Some(p) => self.scene.nodes[p].children.push(idx),
            None => self.scene.roots.push(idx),
        }
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

    fn object(&mut self, r: u64, parent_node: Option<usize>, xform_name: &str, depth: usize) -> Result<()> {
        self.visits += 1;
        if depth > self.limits.max_depth || self.visits > self.limits.max_nodes {
            return Err(malformed(KIND, "objects are nested too deeply or shared too often"));
        }
        let children = self.o.group(r)?;
        let headers = self.o.object_headers(&children)?;
        for (i, (name, meta)) in headers.iter().enumerate() {
            let group = children
                .get(i + 1)
                .copied()
                .filter(|g| g & DATA_BIT == 0)
                .ok_or_else(|| malformed(KIND, format!("object \"{name}\" has no group")))?;
            let schema = meta_get(meta, "schema").unwrap_or_default();
            let kids = self.o.group(group)?;
            let props = match kids.first() {
                Some(&p) if p & DATA_BIT == 0 => self.o.compound(p)?,
                _ => Vec::new(),
            };

            let mut next_parent = parent_node;
            if schema.starts_with("AbcGeom_Xform") {
                if let Some(xf) = schema_compound(&props, schema, ".xform") {
                    let (local, inherits) = self.xform(xf.group)?;
                    let attach_to = if inherits { parent_node } else { None };
                    let node_idx = self.push_node(Node { name: name.clone(), transform: local, ..Node::default() }, attach_to)?;
                    next_parent = Some(node_idx);
                }
            } else if schema.starts_with("AbcGeom_PolyMesh")
                && let Some(geom) = schema_compound(&props, schema, ".geom")
            {
                let part_name = if xform_name.is_empty() { name.clone() } else { xform_name.to_string() };
                let sets = self.face_sets(&kids)?;
                let mesh = self.mesh(geom.group, &part_name, &sets)?;
                if self.scene.meshes.len() >= self.limits.max_meshes {
                    return Err(too_large(KIND, format!("more than {} meshes", self.limits.max_meshes)));
                }
                let mesh_idx = self.scene.meshes.len();
                self.scene.meshes.push(mesh);
                let node_idx = self.push_node(Node { name: part_name, mesh: Some(mesh_idx), ..Node::default() }, parent_node)?;
                next_parent = Some(node_idx);
            }

            if !schema.starts_with("AbcGeom_FaceSet") {
                let owner = if schema.starts_with("AbcGeom_Xform") { name.as_str() } else { "" };
                self.object(group, next_parent, owner, depth + 1)?;
            }
        }
        Ok(())
    }

    fn xform(&mut self, r: u64) -> Result<(Mat4, bool)> {
        let props = self.o.compound(r)?;
        let ops = match find(&props, ".ops") {
            Some(p) => self.o.values(p)?,
            None => Vec::new(),
        };
        let vals = match find(&props, ".vals") {
            Some(p) => self.o.values(p)?,
            None => Vec::new(),
        };
        let inherits = match find(&props, ".inherits") {
            Some(p) => self.o.values(p)?.first().is_none_or(|v| *v != 0.0),
            None => true,
        };
        let mut m = IDENTITY;
        let mut ch = 0usize;
        for op in ops {
            let kind = (op as u32) >> 4;
            let n = match kind {
                0 | 1 => 3,
                2 => 4,
                3 => 16,
                4..=6 => 1,
                _ => return Err(malformed(KIND, format!("unknown xform operation {kind}"))),
            };
            let Some(v) = vals.get(ch..ch + n) else { return Err(malformed(KIND, "xform has fewer values than its operations need")) };
            ch += n;
            let op_m = match kind {
                0 => [v[0], 0.0, 0.0, 0.0, 0.0, v[1], 0.0, 0.0, 0.0, 0.0, v[2], 0.0, 0.0, 0.0, 0.0, 1.0],
                1 => [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, v[0], v[1], v[2], 1.0],
                2 => axis_angle_matrix([v[0], v[1], v[2]], v[3].to_radians()),
                3 => {
                    let mut a = [0.0; 16];
                    a.copy_from_slice(v);
                    a
                }
                4 => rot_x(v[0].to_radians()),
                5 => rot_y(v[0].to_radians()),
                _ => rot_z(v[0].to_radians()),
            };
            m = mat_mul(&m, &op_m);
        }
        if !m.iter().all(|x| x.is_finite()) {
            return Err(malformed(KIND, "xform has non-finite values"));
        }
        Ok((m, inherits))
    }

    fn face_sets(&mut self, kids: &[u64]) -> Result<Vec<(String, Vec<f64>)>> {
        let headers = self.o.object_headers(kids)?;
        let mut out = Vec::new();
        for (i, (name, meta)) in headers.iter().enumerate() {
            let schema = meta_get(meta, "schema").unwrap_or_default();
            if !schema.starts_with("AbcGeom_FaceSet") {
                continue;
            }
            let Some(&g) = kids.get(i + 1).filter(|g| **g & DATA_BIT == 0) else { continue };
            let props = match self.o.group(g)?.first() {
                Some(&p) if p & DATA_BIT == 0 => self.o.compound(p)?,
                _ => continue,
            };
            let Some(fs) = schema_compound(&props, schema, ".faceset") else { continue };
            let inner = self.o.compound(fs.group)?;
            if let Some(faces) = find(&inner, ".faces") {
                out.push((name.clone(), self.o.values(faces)?));
            }
        }
        Ok(out)
    }

    fn param(&mut self, props: &[Prop], name: &str, counts: [usize; 3]) -> Result<Option<Param>> {
        let Some(p) = find(props, name) else { return Ok(None) };
        let (vals, indices) = match p.kind {
            PropKind::Array => (self.o.values(p)?, None),
            PropKind::Compound => {
                let inner = self.o.compound(p.group)?;
                let Some(v) = find(&inner, ".vals") else { return Ok(None) };
                let vals = self.o.values(v)?;
                let indices = match find(&inner, ".indices") {
                    Some(ix) => Some(self.o.values(ix)?),
                    None => None,
                };
                (vals, indices)
            }
            PropKind::Scalar => return Ok(None),
        };
        let width = p.extent.max(1);
        let items = indices.as_ref().map_or(vals.len() / width, Vec::len);
        let [points, faces, corners] = counts;
        let scope = match meta_get(&p.meta, "geoScope") {
            Some("con") => Scope::Constant,
            Some("uni") => Scope::Uniform,
            Some("vtx" | "var") => Scope::Vertex,
            Some("fvr") => Scope::FaceVarying,
            _ if items == corners => Scope::FaceVarying,
            _ if items == points => Scope::Vertex,
            _ if items == faces => Scope::Uniform,
            _ => Scope::Constant,
        };
        Ok(Some(Param { vals, indices, scope }))
    }

    fn mesh(&mut self, r: u64, name: &str, sets: &[(String, Vec<f64>)]) -> Result<Mesh> {
        let properties = self.o.compound(r)?;
        let get = |o: &mut Ogawa, prop_name: &str| -> Result<Vec<f64>> {
            match find(&properties, prop_name) {
                Some(p) => o.values(p),
                None => Err(malformed(KIND, format!("PolyMesh has no {prop_name}"))),
            }
        };

        let p = get(&mut self.o, "P")?;
        let indices = get(&mut self.o, ".faceIndices")?;
        let counts = get(&mut self.o, ".faceCounts")?;
        let num_points = p.len() / 3;
        if num_points > self.limits.max_vertices {
            return Err(too_large(KIND, format!("more than {} vertices", self.limits.max_vertices)));
        }
        let positions: Vec<[f64; 3]> = p.as_chunks::<3>().0.to_vec();
        let n = [positions.len(), counts.len(), indices.len()];
        let normals_param = self.param(&properties, "N", n)?;
        let uvs_param = self.param(&properties, "uv", n)?;

        let mut face_material = vec![None::<usize>; counts.len()];
        for (s, (_, faces)) in sets.iter().enumerate() {
            for f in faces {
                if let Some(m) = face_material.get_mut(*f as usize).filter(|m| m.is_none() && *f >= 0.0) {
                    *m = Some(s);
                }
            }
        }

        let mut mesh = Mesh { name: name.to_string(), positions, ..Mesh::default() };
        let mut norm_ids: HashMap<[u64; 3], u32> = HashMap::new();
        let mut uv_ids: HashMap<[u32; 2], u32> = HashMap::new();
        let mut uv_pool: Vec<[f32; 2]> = Vec::new();
        let mut parts_by_mat: HashMap<Option<usize>, Part> = HashMap::new();
        let mut part_order: Vec<Option<usize>> = Vec::new();

        let (mut k, mut face_corners, mut bad_uvs) = (0usize, Vec::new(), 0usize);
        for (f, &count) in counts.iter().enumerate() {
            let end = (count >= 0.0).then(|| k.checked_add(count as usize)).flatten().filter(|e| *e <= indices.len());
            let end = end.ok_or_else(|| malformed(KIND, format!("face {f} has {count} corners past the index list")))?;
            let face_len = end - k;
            if face_len > self.limits.max_face_corners {
                return Err(too_large(KIND, format!("face {f} has more than {} corners", self.limits.max_face_corners)));
            }
            if face_len >= 3 {
                self.triangles = self.triangles.saturating_add(face_len - 2);
                if self.triangles > self.limits.max_triangles {
                    return Err(too_large(KIND, format!("more than {} triangles", self.limits.max_triangles)));
                }
                face_corners.clear();
                for (corner_index, &point_index) in indices.iter().enumerate().take(end).skip(k) {
                    if point_index < 0.0 || (point_index as usize) >= mesh.positions.len() {
                        return Err(malformed(KIND, format!("point index {point_index} out of range")));
                    }
                    let pt = point_index as usize;
                    let norm_idx = match normals_param.as_ref().and_then(|l| l.get::<3>(corner_index, pt, f)) {
                        Some(norm) => intern_norm(&mut mesh.normals, &mut norm_ids, norm, self.limits.max_vertices)?,
                        None => crate::NONE,
                    };
                    let uv_idx = match &uvs_param {
                        Some(l) => {
                            let uv = match l.get::<2>(corner_index, pt, f) {
                                Some([u, v]) => [u as f32, 1.0 - v as f32],
                                None => {
                                    bad_uvs += 1;
                                    [0.0, 0.0]
                                }
                            };
                            intern_uv(&mut uv_pool, &mut uv_ids, uv, self.limits.max_vertices)?
                        }
                        None => crate::NONE,
                    };
                    face_corners.push(Corner { position: pt as u32, normal: norm_idx, uv: uv_idx });
                }
                // Alembic winds polygons clockwise; reverse to counter-clockwise.
                face_corners.reverse();

                let mat_idx = match face_material.get(f).copied().flatten().and_then(|s| sets.get(s)) {
                    Some((mat_name, _)) => Some(self.intern_material(mat_name)?),
                    None => None,
                };
                let part = match parts_by_mat.entry(mat_idx) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        self.parts += 1;
                        if self.parts > self.limits.max_parts {
                            return Err(too_large(KIND, format!("more than {} parts", self.limits.max_parts)));
                        }
                        part_order.push(mat_idx);
                        e.insert(Part { material: mat_idx, ..Part::default() })
                    }
                };
                part.corners.extend_from_slice(&face_corners);
                part.face_sizes.push(face_corners.len() as u32);
            }
            k = end;
        }

        for mat_idx in part_order {
            if let Some(mut part) = parts_by_mat.remove(&mat_idx) {
                if part.face_sizes.iter().all(|&s| s == 3) {
                    part.face_sizes.clear();
                }
                mesh.parts.push(part);
            }
        }
        if !uv_pool.is_empty() {
            mesh.uv_sets.push(uv_pool);
        }
        if bad_uvs > 0 {
            self.warn(format!("{bad_uvs} polygon corners have no UV in the Alembic uv parameter (set to 0, 0)"));
        }
        Ok(mesh)
    }
}

fn intern_norm(pool: &mut Vec<[f64; 3]>, map: &mut HashMap<[u64; 3], u32>, val: [f64; 3], limit: usize) -> Result<u32> {
    let key = val.map(f64::to_bits);
    if let Some(&id) = map.get(&key) {
        return Ok(id);
    }
    if pool.len() >= limit {
        return Err(too_large(KIND, format!("more than {limit} normals")));
    }
    let id = u32::try_from(pool.len()).map_err(|_| too_large(KIND, "too many normals"))?;
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

fn axis_angle_matrix(axis: [f64; 3], angle: f64) -> Mat4 {
    let len = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    if !len.is_finite() || len == 0.0 {
        return IDENTITY;
    }
    let [x, y, z] = axis.map(|v| v / len);
    let (s, c) = angle.sin_cos();
    let t = 1.0 - c;
    [
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
    ]
}

fn rot_x(a: f64) -> Mat4 {
    let (s, c) = a.sin_cos();
    [1.0, 0.0, 0.0, 0.0, 0.0, c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0]
}

fn rot_y(a: f64) -> Mat4 {
    let (s, c) = a.sin_cos();
    [c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0]
}

fn rot_z(a: f64) -> Mat4 {
    let (s, c) = a.sin_cos();
    [c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
}
