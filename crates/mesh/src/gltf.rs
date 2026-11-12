//! glTF 2.0, read (`.gltf` and `.glb`) and written (`.glb`), from the Khronos glTF 2.0
//! specification.
//!
//! Read: the default scene's node hierarchy (matrix or translation/rotation/scale), triangle
//! primitives (`TRIANGLES`, `TRIANGLE_STRIP`, `TRIANGLE_FAN`, indexed or not) with POSITION,
//! NORMAL, every TEXCOORD_n, COLOR_0, JOINTS_0 and WEIGHTS_0 of any component type the
//! specification or `KHR_mesh_quantization` allows, strided buffer views, sparse accessors,
//! metallic-roughness materials (factors, the five texture slots with their UV set and wrap
//! modes, alpha modes, double-sided, `KHR_materials_emissive_strength`, `KHR_materials_unlit`;
//! `KHR_materials_pbrSpecularGlossiness` as its diffuse colour and texture with roughness from
//! the glossiness), textures whose image is only in `EXT_texture_webp`, images (embedded
//! in a buffer view or a `data:` URI, or named), skins, animations of translation, rotation
//! and scale, cameras and `KHR_lights_punctual` lights. A glTF mesh becomes one [`Mesh`] whose
//! pools hold its primitives' vertices one after another (once for primitives that name the
//! same accessors), and each primitive a [`Part`].
//!
//! Not read: required extensions that change how geometry is stored (Draco, meshopt) are
//! [`Error::Unsupported`]; points, lines, morph targets, tangents and
//! other extensions are passed over (skipped primitives are counted in a warning).
//!
//! Buffers are needed to read anything, so an external buffer is asked of the resolver (by its
//! percent-decoded URI) and is [`Error::Missing`] when declined. External images are only named
//! ([`Image::uri`]) and never asked for. Both are listed in [`Scene::external`]. The names come
//! from the file: a resolver must not trust them as paths (they may be absolute or hold `..`).
//!
//! A triangle that names a vertex that is not there, or one whose position is not finite, is
//! left out and counted in a warning.
//!
//! Written ([`write_glb`]): nodes, meshes (positions, normals, every UV set, colours),
//! materials and images. Skins, animations, cameras and lights are not written.

use std::cell::Cell;

use serde_json::{Map, Value, json};

use crate::{
    AlphaMode, Animation, Camera, Channel, ChannelPath, Corner, Error, IDENTITY, Image, Interpolation, Light, LightKind, Limits, Material, Mesh,
    NONE, Node, Part, Projection, Resolver, Result, Scene, Skin, TextureRef, Trs, Wrap, count_warning, malformed, too_large,
};

const KIND: &str = "glTF";
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;
/// Most UV sets read from one primitive.
pub const MAX_UV_SETS: usize = 8;
/// Most components that accessors without a buffer view (all zeros) may have, together:
/// nothing in the file backs their counts, so they are capped rather than checked against a
/// buffer.
pub const MAX_ZERO_COMPONENTS: usize = 1 << 24;

/// Does `bytes` start like a GLB container?
pub fn is_glb(bytes: &[u8]) -> bool {
    bytes.starts_with(b"glTF")
}

/// Split a GLB container into its JSON chunk and its binary chunk.
pub fn split_glb(bytes: &[u8]) -> Result<(&[u8], Option<&[u8]>)> {
    let bad = |m: &str| malformed("GLB", m);
    let u32_at = |at: usize| bytes.get(at..at.checked_add(4)?).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    if !is_glb(bytes) {
        return Err(bad("not a GLB file"));
    }
    let version = u32_at(4).ok_or_else(|| bad("truncated header"))?;
    if version != 2 {
        return Err(Error::Unsupported(format!("GLB: version {version} is not glTF 2.0")));
    }
    let length = u32_at(8).ok_or_else(|| bad("truncated header"))? as usize;
    if length > bytes.len() {
        return Err(bad(&format!("truncated: the header says {length} bytes, the file has {}", bytes.len())));
    }
    let mut at = 12usize;
    let (mut json, mut bin) = (None, None);
    while at.saturating_add(8) <= length {
        let (Some(len), Some(kind)) = (u32_at(at), u32_at(at + 4)) else { break };
        let start = at + 8;
        let end = start.checked_add(len as usize).filter(|e| *e <= length).ok_or_else(|| bad("a chunk runs past the end of the file"))?;
        let data = bytes.get(start..end).ok_or_else(|| bad("a chunk runs past the end of the file"))?;
        match kind {
            CHUNK_JSON if json.is_none() => json = Some(data),
            CHUNK_BIN if json.is_some() && bin.is_none() => bin = Some(data),
            CHUNK_JSON | CHUNK_BIN => return Err(bad("chunks out of order")),
            // Chunks of unknown types are skipped, as the specification asks.
            _ => {}
        }
        at = end;
    }
    Ok((json.ok_or_else(|| bad("no JSON chunk"))?, bin))
}

/// A GLB container of `json` and an optional binary chunk (each padded to four bytes).
pub fn glb_container(json: &[u8], bin: &[u8]) -> Result<Vec<u8>> {
    let pad = |n: usize| (4 - n % 4) % 4;
    let json_len = json.len() + pad(json.len());
    let bin_len = bin.len() + pad(bin.len());
    let total = 12 + 8 + json_len + if bin.is_empty() { 0 } else { 8 + bin_len };
    let size = |n: usize| u32::try_from(n).map_err(|_| too_large("GLB", "more than the format's 32-bit lengths hold"));
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&size(total)?.to_le_bytes());
    out.extend_from_slice(&size(json_len)?.to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(json);
    out.extend(std::iter::repeat_n(b' ', pad(json.len())));
    if !bin.is_empty() {
        out.extend_from_slice(&size(bin_len)?.to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(bin);
        out.extend(std::iter::repeat_n(0u8, pad(bin.len())));
    }
    Ok(out)
}

pub use crate::base64_decode;

/// Percent-decode a URI. A `%` that two hex digits do not follow stays as it is.
pub fn uri_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = |k: usize| b.get(k).and_then(|c| (*c as char).to_digit(16));
        if let (b'%', Some(h), Some(l)) = (c, hex(i + 1), hex(i + 2)) {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The media type and bytes of a `data:` URI.
fn data_uri(uri: &str) -> Option<Result<(String, Vec<u8>)>> {
    let rest = uri.strip_prefix("data:")?;
    Some((|| {
        let (meta, data) = rest.split_once(',').ok_or_else(|| malformed(KIND, "a data URI without a comma"))?;
        let mime = meta.split(';').next().unwrap_or("").to_string();
        let bytes = if meta.ends_with(";base64") {
            base64_decode(data).ok_or_else(|| malformed(KIND, "bad base64 in a data URI"))?
        } else {
            uri_decode(data).into_bytes()
        };
        Ok((mime, bytes))
    })())
}

fn arr<'v>(v: &'v Value, k: &str) -> &'v [Value] {
    v.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// A non-negative integer field; one too large for `usize` counts as absent.
fn idx(v: &Value, k: &str) -> Option<usize> {
    v.get(k).and_then(Value::as_u64).and_then(|x| usize::try_from(x).ok())
}

/// The indices in an array field that are below `len`.
fn indices(v: &Value, k: &str, len: usize) -> Vec<usize> {
    arr(v, k).iter().filter_map(Value::as_u64).filter_map(|c| usize::try_from(c).ok()).filter(|c| *c < len).collect()
}

fn num(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(d)
}

fn nums<const N: usize>(v: &Value, k: &str, d: [f64; N]) -> [f64; N] {
    let mut out = d;
    if let Some(a) = v.get(k).and_then(Value::as_array) {
        for (slot, x) in out.iter_mut().zip(a) {
            *slot = x.as_f64().filter(|x| x.is_finite()).unwrap_or(*slot);
        }
    }
    out
}

fn name(v: &Value) -> String {
    v.get("name").and_then(Value::as_str).unwrap_or("").to_string()
}

/// One component of type `ct` at the start of `s`, mapped to 0..1 / −1..1 when `norm`.
fn scalar(ct: usize, s: &[u8], norm: bool) -> Option<f64> {
    let b2 = || s.get(..2).and_then(|b| <[u8; 2]>::try_from(b).ok());
    let b4 = || s.get(..4).and_then(|b| <[u8; 4]>::try_from(b).ok());
    Some(match (ct, norm) {
        (5126, _) => f64::from(f32::from_le_bytes(b4()?)),
        (5125, _) => f64::from(u32::from_le_bytes(b4()?)),
        (5123, false) => f64::from(u16::from_le_bytes(b2()?)),
        (5123, true) => f64::from(u16::from_le_bytes(b2()?)) / 65_535.0,
        (5122, false) => f64::from(i16::from_le_bytes(b2()?)),
        (5122, true) => (f64::from(i16::from_le_bytes(b2()?)) / 32_767.0).max(-1.0),
        (5121, false) => f64::from(*s.first()?),
        (5121, true) => f64::from(*s.first()?) / 255.0,
        (5120, false) => f64::from(*s.first()? as i8),
        (5120, true) => (f64::from(*s.first()? as i8) / 127.0).max(-1.0),
        _ => return None,
    })
}

struct Doc<'a> {
    j: &'a Value,
    buffers: Vec<Vec<u8>>,
    vertices_left: Cell<usize>,
    triangles_left: Cell<usize>,
    parts_left: Cell<usize>,
    zero_left: Cell<usize>,
    /// Values skins and animations may still take, and bytes images may still copy.
    data_left: Cell<usize>,
}

fn charge(cell: &Cell<usize>, n: usize, what: &str) -> Result<()> {
    cell.set(cell.get().checked_sub(n).ok_or_else(|| too_large(KIND, format!("more {what} than the limit allows")))?);
    Ok(())
}

impl Doc<'_> {
    fn accessor(&self, a: usize) -> Result<&Value> {
        arr(self.j, "accessors").get(a).ok_or_else(|| malformed(KIND, format!("no accessor {a}")))
    }

    /// Elements the accessor declares.
    fn count(&self, a: usize) -> Option<usize> {
        idx(self.accessor(a).ok()?, "count")
    }

    /// The bytes of a buffer view.
    fn view(&self, bv: usize) -> Result<&[u8]> {
        let view = arr(self.j, "bufferViews").get(bv).ok_or_else(|| malformed(KIND, format!("no bufferView {bv}")))?;
        let buf =
            self.buffers.get(idx(view, "buffer").unwrap_or(0)).ok_or_else(|| malformed(KIND, "a bufferView names a buffer that does not exist"))?;
        let start = idx(view, "byteOffset").unwrap_or(0);
        let end = match view.get("byteLength") {
            Some(_) => start.checked_add(idx(view, "byteLength").unwrap_or(usize::MAX)),
            None => Some(buf.len()),
        };
        end.and_then(|end| buf.get(start..end)).ok_or_else(|| malformed(KIND, "a bufferView runs past its buffer"))
    }

    /// The accessor's values, `n` per element, at most `max` elements: every length is checked
    /// against the bytes that are there before anything is allocated from a count.
    fn rows(&self, a: usize, max: usize, what: &str) -> Result<(Vec<f64>, usize)> {
        let acc = self.accessor(a)?;
        let n = match acc.get("type").and_then(Value::as_str).unwrap_or("") {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" | "MAT2" => 4,
            "MAT3" => 9,
            "MAT4" => 16,
            t => return Err(malformed(KIND, format!("accessor type `{}`", t.chars().take(16).collect::<String>()))),
        };
        let ct = idx(acc, "componentType").unwrap_or(5126);
        let csize = match ct {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(malformed(KIND, format!("componentType {ct}"))),
        };
        let norm = acc.get("normalized").and_then(Value::as_bool).unwrap_or(false);
        let count = match acc.get("count") {
            None => 0,
            Some(c) => c.as_u64().and_then(|c| usize::try_from(c).ok()).ok_or_else(|| malformed(KIND, "an accessor count is not a valid size"))?,
        };
        if count > max {
            return Err(too_large(KIND, format!("{count} {what}, more than the limit allows")));
        }
        let total = count.checked_mul(n).ok_or_else(|| too_large(KIND, "an accessor count is too large"))?;
        let mut out = match idx(acc, "bufferView") {
            // No buffer view: all zeros.
            None => {
                charge(&self.zero_left, total, "values without a buffer")?;
                vec![0.0; total]
            }
            Some(bv) => self.dense(acc, bv, (ct, csize, norm), n, count)?,
        };
        if let Some(sparse) = acc.get("sparse") {
            self.sparse(sparse, (ct, csize, norm), n, &mut out)?;
        }
        Ok((out, n))
    }

    /// The `count` elements of `n` components an accessor reads from buffer view `bv`.
    fn dense(&self, acc: &Value, bv: usize, (ct, csize, norm): (usize, usize, bool), n: usize, count: usize) -> Result<Vec<f64>> {
        let total = count * n;
        let stride = arr(self.j, "bufferViews").get(bv).and_then(|v| idx(v, "byteStride")).filter(|s| *s > 0).unwrap_or(csize * n);
        if stride < csize * n {
            return Err(malformed(KIND, format!("bufferView byteStride {stride} is smaller than its {}-byte elements", csize * n)));
        }
        let past = || malformed(KIND, "an accessor reads past its buffer");
        let data = self.view(bv)?.get(idx(acc, "byteOffset").unwrap_or(0)..).ok_or_else(past)?;
        if count > 0 {
            let end = (count - 1).checked_mul(stride).and_then(|o| o.checked_add(csize * n)).ok_or_else(past)?;
            if end > data.len() {
                return Err(past());
            }
        }
        // Matrices of 1- and 2-byte components pad their columns to 4 bytes; not handled (rare).
        let mut out = vec![0.0; total];
        for (i, row) in out.chunks_exact_mut(n).enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = i.checked_mul(stride).and_then(|o| data.get(o.checked_add(c * csize)?..)).and_then(|s| scalar(ct, s, norm)).ok_or_else(past)?;
            }
        }
        Ok(out)
    }

    /// Put a sparse accessor's values over the elements it names. Its indices and values are
    /// tightly packed in their buffer views; an index past the accessor is an error.
    fn sparse(&self, sparse: &Value, (ct, csize, norm): (usize, usize, bool), n: usize, out: &mut [f64]) -> Result<()> {
        let bad = || malformed(KIND, "a sparse accessor reads past its buffer or names an element that is not there");
        let count = idx(sparse, "count").filter(|k| *k <= out.len() / n).ok_or_else(bad)?;
        let part = |key: &str| -> Result<(&Value, &[u8])> {
            let v = sparse.get(key).ok_or_else(bad)?;
            Ok((v, self.view(idx(v, "bufferView").ok_or_else(bad)?)?.get(idx(v, "byteOffset").unwrap_or(0)..).ok_or_else(bad)?))
        };
        let ((indices, index_data), (_, value_data)) = (part("indices")?, part("values")?);
        let index_type = idx(indices, "componentType").unwrap_or(5125);
        let index_size = match index_type {
            5121 => 1,
            5123 => 2,
            5125 => 4,
            t => return Err(malformed(KIND, format!("sparse index componentType {t}"))),
        };
        for e in 0..count {
            let target = index_data.get(e * index_size..).and_then(|s| scalar(index_type, s, false)).ok_or_else(bad)? as usize;
            let row = target.checked_mul(n).and_then(|at| out.get_mut(at..at.checked_add(n)?)).ok_or_else(bad)?;
            for (c, v) in row.iter_mut().enumerate() {
                *v = value_data.get((e * n + c) * csize..).and_then(|s| scalar(ct, s, norm)).ok_or_else(bad)?;
            }
        }
        Ok(())
    }

    /// A vertex attribute of `nv` vertices with `n` components in `want`; `None` when the
    /// primitive has none or it does not fit the positions.
    fn attribute(&self, attrs: &Value, key: &str, nv: usize, want: &[usize]) -> Result<Option<(Vec<f64>, usize)>> {
        let Some(a) = idx(attrs, key).filter(|a| self.count(*a) == Some(nv)) else { return Ok(None) };
        let (v, n) = self.rows(a, nv, key)?;
        Ok(want.contains(&n).then_some((v, n)))
    }
}

/// What was passed over, for the warnings.
#[derive(Default)]
struct Skipped {
    triangles: usize,
    primitives: usize,
}

fn rows_of<const N: usize>(v: &[f64]) -> impl Iterator<Item = [f64; N]> + '_ {
    v.as_chunks::<N>().0.iter().copied()
}

/// Append the primitive's vertex attributes to the mesh's pools; the bases its corners count
/// from (`NONE` for a pool the primitive does not fill).
fn attributes(d: &Doc, attrs: &Value, pos: &[f64], mesh: &mut Mesh) -> Result<[u32; 3]> {
    let nv = pos.len() / 3;
    let base_of = |len: usize| {
        u32::try_from(len)
            .ok()
            .filter(|b| (*b as usize).saturating_add(nv) < NONE as usize)
            .ok_or_else(|| too_large(KIND, "more vertices in a mesh than a 32-bit index holds"))
    };
    let base = base_of(mesh.positions.len())?;
    let mut normal_base = NONE;
    if let Some((v, _)) = d.attribute(attrs, "NORMAL", nv, &[3])? {
        normal_base = base_of(mesh.normals.len())?;
        mesh.normals.extend(rows_of::<3>(&v));
    }
    let mut sets = Vec::new();
    while sets.len() < MAX_UV_SETS {
        let Some((v, _)) = d.attribute(attrs, &format!("TEXCOORD_{}", sets.len()), nv, &[2])? else { break };
        sets.push(v);
    }
    let mut uv_base = NONE;
    if !sets.is_empty() {
        let len = mesh.uv_sets.first().map_or(0, Vec::len);
        uv_base = base_of(len)?;
        // Every set of the mesh has one length: a set this primitive lacks is zeros.
        while mesh.uv_sets.len() < sets.len() {
            mesh.uv_sets.push(vec![[0.0; 2]; len]);
        }
        for (k, set) in mesh.uv_sets.iter_mut().enumerate() {
            match sets.get(k) {
                Some(v) => set.extend(rows_of::<2>(v).map(|t| t.map(|x| x as f32))),
                None => set.resize(len + nv, [0.0; 2]),
            }
        }
    }
    let total = mesh.positions.len() + nv;
    match d.attribute(attrs, "COLOR_0", nv, &[3, 4])? {
        Some((v, n)) => {
            // Vertices of earlier primitives without a colour are white.
            mesh.colors.resize(mesh.positions.len(), [1.0; 4]);
            mesh.colors.extend(v.chunks_exact(n).map(|c| [0, 1, 2, 3].map(|k| c.get(k).map_or(1.0, |x| *x as f32))));
        }
        None if !mesh.colors.is_empty() => mesh.colors.resize(total, [1.0; 4]),
        None => {}
    }
    match (d.attribute(attrs, "JOINTS_0", nv, &[4])?, d.attribute(attrs, "WEIGHTS_0", nv, &[4])?) {
        (Some((j, _)), Some((w, _))) => {
            mesh.joints.resize(mesh.positions.len(), [0; 4]);
            mesh.weights.resize(mesh.positions.len(), [0.0; 4]);
            mesh.joints.extend(rows_of::<4>(&j).map(|r| r.map(|x| x as u16)));
            mesh.weights.extend(rows_of::<4>(&w).map(|r| r.map(|x| x as f32)));
        }
        _ if !mesh.joints.is_empty() => {
            mesh.joints.resize(total, [0; 4]);
            mesh.weights.resize(total, [0.0; 4]);
        }
        _ => {}
    }
    mesh.positions.extend(rows_of::<3>(pos));
    Ok([base, normal_base, uv_base])
}

/// Where the vertices of one set of attribute accessors start in the mesh's pools (positions,
/// normals, UVs), and how many there are. Primitives of a mesh that name the same accessors
/// (one vertex buffer, several materials) share the vertices.
type Shared = std::collections::HashMap<String, ([u32; 3], usize)>;

fn primitive(d: &Doc, p: &Value, mesh: &mut Mesh, shared: &mut Shared, materials: usize, skipped: &mut Skipped) -> Result<()> {
    let mode = idx(p, "mode").unwrap_or(4);
    let position = p.get("attributes").and_then(|a| idx(a, "POSITION"));
    let (Some(attrs), Some(pa), 4..=6) = (p.get("attributes"), position, mode) else {
        skipped.primitives += 1;
        return Ok(());
    };
    charge(&d.parts_left, 1, "primitives")?;
    let key = attrs.to_string();
    let ([base, normal_base, uv_base], nv) = match shared.get(&key) {
        Some(known) => *known,
        None => {
            let (pos, n) = d.rows(pa, d.vertices_left.get(), "vertices")?;
            if n != 3 {
                return Err(malformed(KIND, "POSITION is not VEC3"));
            }
            charge(&d.vertices_left, pos.len() / 3, "vertices")?;
            let placed = (attributes(d, attrs, &pos, mesh)?, pos.len() / 3);
            shared.insert(key, placed);
            placed
        }
    };
    let left = d.triangles_left.get();
    // A strip or fan of k indices makes k − 2 triangles; a list, k / 3.
    let cap = if mode == 4 { left.saturating_mul(3).saturating_add(2) } else { left.saturating_add(2) };
    let raw: Vec<u32> = match idx(p, "indices") {
        Some(a) => {
            let (v, n) = d.rows(a, cap, "indices")?;
            if n != 1 {
                return Err(malformed(KIND, "indices are not SCALAR"));
            }
            v.iter().map(|x| *x as u32).collect()
        }
        None if nv > cap => return Err(too_large(KIND, "more triangles than the limit allows")),
        None => (0..nv as u32).collect(),
    };
    let at = |k: usize| raw.get(k).copied();
    let triangle = |k: usize| match mode {
        4 => Some([at(3 * k)?, at(3 * k + 1)?, at(3 * k + 2)?]),
        5 if k.is_multiple_of(2) => Some([at(k)?, at(k + 1)?, at(k + 2)?]),
        5 => Some([at(k + 1)?, at(k)?, at(k + 2)?]),
        _ => Some([at(0)?, at(k + 1)?, at(k + 2)?]),
    };
    let finite = |i: u32| (i as usize) < nv && mesh.positions.get(base as usize + i as usize).is_some_and(|p| p.iter().all(|x| x.is_finite()));
    let offset = |b: u32, i: u32| if b == NONE { NONE } else { b + i };
    let mut part = Part { material: idx(p, "material").filter(|m| *m < materials), ..Part::default() };
    let count = if mode == 4 { raw.len() / 3 } else { raw.len().saturating_sub(2) };
    for t in (0..count).filter_map(triangle) {
        if t.iter().all(|i| finite(*i)) {
            part.corners.extend(t.map(|i| Corner { position: base + i, normal: offset(normal_base, i), uv: offset(uv_base, i) }));
        } else {
            skipped.triangles += 1;
        }
    }
    skipped.triangles += usize::from(mode == 4 && !raw.len().is_multiple_of(3));
    charge(&d.triangles_left, part.corners.len() / 3, "triangles")?;
    mesh.parts.push(part);
    Ok(())
}

fn buffers(j: &Value, bin: Option<&[u8]>, limits: &Limits, scene: &mut Scene, resolve: Resolver) -> Result<Vec<Vec<u8>>> {
    let list = arr(j, "buffers");
    if list.len() > limits.max_extras {
        return Err(too_large(KIND, format!("more than {} buffers", limits.max_extras)));
    }
    let mut out = Vec::new();
    let mut total = 0usize;
    for (i, b) in list.iter().enumerate() {
        let data = match b.get("uri").and_then(Value::as_str) {
            Some(uri) => match data_uri(uri) {
                Some(data) => data?.1,
                None => {
                    let name = uri_decode(uri);
                    scene.external.push(name.clone());
                    resolve(&name).ok_or(Error::Missing(name))?
                }
            },
            // Without a binary chunk the buffer is empty, and whatever reads it runs past it.
            None if i == 0 => bin.map(<[u8]>::to_vec).unwrap_or_default(),
            None => vec![],
        };
        total = total.saturating_add(data.len());
        if total > limits.max_data_bytes {
            return Err(too_large(KIND, format!("buffers of more than {} bytes", limits.max_data_bytes)));
        }
        out.push(data);
    }
    Ok(out)
}

fn images(d: &Doc, limits: &Limits, scene: &mut Scene) -> Result<()> {
    let list = arr(d.j, "images");
    if list.len() > limits.max_materials {
        return Err(too_large(KIND, format!("more than {} images", limits.max_materials)));
    }
    for im in list {
        let mut image = Image { name: name(im), mime_type: im.get("mimeType").and_then(Value::as_str).map(str::to_string), ..Image::default() };
        if let Some(uri) = im.get("uri").and_then(Value::as_str) {
            match data_uri(uri) {
                // An image that cannot be decoded is left without data, not an error.
                Some(data) => {
                    if let Ok((mime, bytes)) = data {
                        image.mime_type = image.mime_type.or(Some(mime).filter(|m| !m.is_empty()));
                        image.data = Some(bytes);
                    }
                }
                None => {
                    let name = uri_decode(uri);
                    scene.external.push(name.clone());
                    image.uri = Some(name);
                }
            }
        } else if let Some(bytes) = idx(im, "bufferView").and_then(|bv| d.view(bv).ok()) {
            charge(&d.data_left, bytes.len(), "image bytes")?;
            image.data = Some(bytes.to_vec());
        }
        scene.images.push(image);
    }
    Ok(())
}

fn materials(j: &Value, limits: &Limits, scene: &mut Scene) -> Result<()> {
    let list = arr(j, "materials");
    if list.len() > limits.max_materials {
        return Err(too_large(KIND, format!("more than {} materials", limits.max_materials)));
    }
    let wrap = |s: Option<&Value>, k: &str| match s.and_then(|s| s.get(k)).and_then(Value::as_u64) {
        Some(33071) => Wrap::Clamp,
        Some(33648) => Wrap::Mirror,
        _ => Wrap::Repeat,
    };
    let images = scene.images.len();
    let tref = |v: Option<&Value>| -> Option<TextureRef> {
        let v = v?;
        let texture = arr(j, "textures").get(idx(v, "index")?)?;
        let sampler = idx(texture, "sampler").and_then(|s| arr(j, "samplers").get(s));
        // `EXT_texture_webp`: the image when the texture has no fallback `source`.
        let webp = || texture.get("extensions").and_then(|e| e.get("EXT_texture_webp")).and_then(|e| idx(e, "source"));
        let image = idx(texture, "source").or_else(webp).filter(|i| *i < images)?;
        Some(TextureRef { image, uv_set: idx(v, "texCoord").unwrap_or(0) as u32, wrap: [wrap(sampler, "wrapS"), wrap(sampler, "wrapT")] })
    };
    for m in list {
        let null = Value::Null;
        let pbr = m.get("pbrMetallicRoughness").unwrap_or(&null);
        let ext = m.get("extensions");
        let strength = ext.and_then(|e| e.get("KHR_materials_emissive_strength")).map_or(1.0, |e| num(e, "emissiveStrength", 1.0));
        scene.materials.push(Material {
            name: name(m),
            base_color: nums(pbr, "baseColorFactor", [1.0; 4]).map(|v| v as f32),
            metallic: num(pbr, "metallicFactor", 1.0) as f32,
            roughness: num(pbr, "roughnessFactor", 1.0) as f32,
            emissive: nums(m, "emissiveFactor", [0.0; 3]).map(|v| (v * strength) as f32),
            alpha_mode: match m.get("alphaMode").and_then(Value::as_str) {
                Some("MASK") => AlphaMode::Mask(num(m, "alphaCutoff", 0.5) as f32),
                Some("BLEND") => AlphaMode::Blend,
                _ => AlphaMode::Opaque,
            },
            double_sided: m.get("doubleSided").and_then(Value::as_bool).unwrap_or(false),
            unlit: ext.and_then(|e| e.get("KHR_materials_unlit")).is_some(),
            base_color_texture: tref(pbr.get("baseColorTexture")),
            metallic_roughness_texture: tref(pbr.get("metallicRoughnessTexture")),
            normal_texture: tref(m.get("normalTexture")),
            normal_scale: m.get("normalTexture").map_or(1.0, |n| num(n, "scale", 1.0)) as f32,
            occlusion_texture: tref(m.get("occlusionTexture")),
            occlusion_strength: m.get("occlusionTexture").map_or(1.0, |n| num(n, "strength", 1.0)) as f32,
            emissive_texture: tref(m.get("emissiveTexture")),
        });
        // The older specular-glossiness model, as the nearest metallic-roughness material: its
        // diffuse colour and texture, no metal, and roughness from the glossiness.
        if let (Some(sg), Some(out)) = (ext.and_then(|e| e.get("KHR_materials_pbrSpecularGlossiness")), scene.materials.last_mut()) {
            out.base_color = nums(sg, "diffuseFactor", [1.0; 4]).map(|v| v as f32);
            out.base_color_texture = tref(sg.get("diffuseTexture"));
            out.metallic = 0.0;
            out.roughness = (1.0 - num(sg, "glossinessFactor", 1.0)).clamp(0.0, 1.0) as f32;
            out.metallic_roughness_texture = None;
        }
    }
    Ok(())
}

fn nodes(j: &Value, limits: &Limits, scene: &mut Scene) -> Result<()> {
    let list = arr(j, "nodes");
    if list.len() > limits.max_nodes {
        return Err(too_large(KIND, format!("more than {} nodes", limits.max_nodes)));
    }
    let skins = arr(j, "skins").len().min(limits.max_extras);
    for n in list {
        let matrix = n.get("matrix").and_then(Value::as_array).filter(|a| a.len() == 16).map(|_| nums(n, "matrix", IDENTITY));
        let trs = Trs {
            translation: nums(n, "translation", [0.0; 3]),
            rotation: nums(n, "rotation", [0.0, 0.0, 0.0, 1.0]),
            scale: nums(n, "scale", [1.0; 3]),
        };
        scene.nodes.push(Node {
            name: name(n),
            transform: matrix.unwrap_or_else(|| trs.matrix()),
            trs: matrix.is_none().then_some(trs),
            mesh: idx(n, "mesh").filter(|m| *m < scene.meshes.len()),
            skin: idx(n, "skin").filter(|s| *s < skins),
            camera: idx(n, "camera").filter(|c| *c < scene.cameras.len()),
            light: n.get("extensions").and_then(|e| e.get("KHR_lights_punctual")).and_then(|e| idx(e, "light")).filter(|l| *l < scene.lights.len()),
            children: indices(n, "children", list.len()),
        });
    }
    scene.roots = match arr(j, "scenes").get(idx(j, "scene").unwrap_or(0)).or_else(|| arr(j, "scenes").first()) {
        Some(s) => indices(s, "nodes", list.len()),
        None => {
            // No scene: every node that is nobody's child.
            let mut child = vec![false; list.len()];
            for c in scene.nodes.iter().flat_map(|n| &n.children) {
                if let Some(slot) = child.get_mut(*c) {
                    *slot = true;
                }
            }
            child.iter().enumerate().filter(|(_, c)| !**c).map(|(i, _)| i).collect()
        }
    };
    Ok(())
}

fn skins(d: &Doc, limits: &Limits, scene: &mut Scene) -> Result<()> {
    let list = arr(d.j, "skins");
    if list.len() > limits.max_extras {
        return Err(too_large(KIND, format!("more than {} skins", limits.max_extras)));
    }
    for s in list {
        let joints = indices(s, "joints", scene.nodes.len());
        let inverse_bind = match idx(s, "inverseBindMatrices") {
            Some(a) => {
                let (v, n) = d.rows(a, joints.len(), "inverse bind matrices")?;
                charge(&d.data_left, v.len(), "skin and animation values")?;
                if n == 16 { rows_of::<16>(&v).collect() } else { vec![] }
            }
            None => vec![],
        };
        scene.skins.push(Skin { name: name(s), joints, inverse_bind });
    }
    Ok(())
}

fn animations(d: &Doc, limits: &Limits, scene: &mut Scene) -> Result<()> {
    let list = arr(d.j, "animations");
    if list.len() > limits.max_extras {
        return Err(too_large(KIND, format!("more than {} animations", limits.max_extras)));
    }
    for a in list {
        let samplers = arr(a, "samplers");
        let mut anim = Animation { name: name(a), channels: vec![] };
        if arr(a, "channels").len() > limits.max_extras {
            return Err(too_large(KIND, format!("more than {} channels in an animation", limits.max_extras)));
        }
        for c in arr(a, "channels") {
            let Some(t) = c.get("target") else { continue };
            let Some(node) = idx(t, "node").filter(|n| *n < scene.nodes.len()) else { continue };
            let path = match t.get("path").and_then(Value::as_str) {
                Some("translation") => ChannelPath::Translation,
                Some("rotation") => ChannelPath::Rotation,
                Some("scale") => ChannelPath::Scale,
                _ => continue,
            };
            let Some(s) = idx(c, "sampler").and_then(|s| samplers.get(s)) else { continue };
            let interpolation = match s.get("interpolation").and_then(Value::as_str) {
                Some("STEP") => Interpolation::Step,
                Some("CUBICSPLINE") => Interpolation::CubicSpline,
                _ => Interpolation::Linear,
            };
            let (Some(input), Some(output)) = (idx(s, "input"), idx(s, "output")) else { continue };
            let floats = |a: usize| -> Result<Vec<f32>> {
                let (v, _) = d.rows(a, d.data_left.get(), "animation keys")?;
                charge(&d.data_left, v.len(), "skin and animation values")?;
                Ok(v.iter().map(|x| *x as f32).collect())
            };
            anim.channels.push(Channel { node, path, interpolation, times: floats(input)?, values: floats(output)? });
        }
        scene.animations.push(anim);
    }
    Ok(())
}

fn cameras_and_lights(j: &Value, limits: &Limits, scene: &mut Scene) -> Result<()> {
    let lights = j.get("extensions").and_then(|e| e.get("KHR_lights_punctual")).map_or(&[][..], |e| arr(e, "lights"));
    if arr(j, "cameras").len() > limits.max_extras || lights.len() > limits.max_extras {
        return Err(too_large(KIND, format!("more than {} cameras or lights", limits.max_extras)));
    }
    let null = Value::Null;
    for c in arr(j, "cameras") {
        let projection = match (c.get("type").and_then(Value::as_str), c.get("orthographic")) {
            (Some("orthographic"), Some(o)) => Projection::Orthographic {
                xmag: num(o, "xmag", 1.0),
                ymag: num(o, "ymag", 1.0),
                znear: num(o, "znear", 0.01),
                zfar: num(o, "zfar", 1000.0),
            },
            _ => {
                let p = c.get("perspective").unwrap_or(&null);
                Projection::Perspective {
                    yfov: num(p, "yfov", 0.8).clamp(1e-3, 3.1),
                    aspect: p.get("aspectRatio").and_then(Value::as_f64).filter(|a| a.is_finite() && *a > 0.0),
                    znear: num(p, "znear", 0.01),
                    zfar: p.get("zfar").and_then(Value::as_f64).filter(|z| z.is_finite()),
                }
            }
        };
        scene.cameras.push(Camera { name: name(c), projection });
    }
    for l in lights {
        let kind = match l.get("type").and_then(Value::as_str) {
            Some("directional") => LightKind::Directional,
            Some("spot") => {
                let s = l.get("spot").unwrap_or(&null);
                let outer = num(s, "outerConeAngle", std::f64::consts::FRAC_PI_4).clamp(1e-3, std::f64::consts::FRAC_PI_2);
                LightKind::Spot { inner: num(s, "innerConeAngle", 0.0).clamp(0.0, outer), outer }
            }
            _ => LightKind::Point,
        };
        let range = l.get("range").and_then(Value::as_f64).filter(|r| r.is_finite());
        scene.lights.push(Light { name: name(l), kind, color: nums(l, "color", [1.0; 3]), intensity: num(l, "intensity", 1.0).max(0.0), range });
    }
    Ok(())
}

/// Required extensions that only change how a file looks (textures, material models) or that
/// the reader handles: the geometry reads without the others being understood.
fn check_extensions(j: &Value, scene: &mut Scene) -> Result<()> {
    const LOOKS: [&str; 4] = ["KHR_texture_transform", "KHR_texture_basisu", "MSFT_texture_dds", "KHR_lights_punctual"];
    const READ: [&str; 5] = [
        "KHR_mesh_quantization",
        "KHR_materials_unlit",
        "KHR_materials_emissive_strength",
        "KHR_materials_pbrSpecularGlossiness",
        "EXT_texture_webp",
    ];
    for e in arr(j, "extensionsRequired").iter().filter_map(Value::as_str) {
        let short: String = e.chars().take(64).collect();
        if READ.contains(&e) {
            continue;
        }
        if !LOOKS.contains(&e) && !e.starts_with("KHR_materials_") {
            let hint = if e.contains("draco") || e.contains("meshopt") { " (compressed meshes: save the file uncompressed)" } else { "" };
            return Err(Error::Unsupported(format!("glTF: required extension {short}{hint}")));
        }
        if scene.warnings.len() < 16 {
            scene.warnings.push(format!("required extension {short} is not read"));
        }
    }
    Ok(())
}

/// Read a `.gltf` (JSON) or `.glb` file. External buffers are asked of `resolve`; see the
/// module documentation.
pub fn read(bytes: &[u8], limits: &Limits, resolve: Resolver) -> Result<Scene> {
    let (json, bin) = if is_glb(bytes) { split_glb(bytes)? } else { (bytes, None) };
    let j: Value = serde_json::from_slice(json).map_err(|e| malformed(KIND, format!("JSON: {e}")))?;
    let version = j.get("asset").and_then(|a| a.get("version")).and_then(Value::as_str).unwrap_or("");
    if version.split('.').next() != Some("2") {
        return Err(Error::Unsupported(format!("glTF: version `{}` (2.x is read)", version.chars().take(16).collect::<String>())));
    }
    let mut scene = Scene::default();
    check_extensions(&j, &mut scene)?;
    let d = Doc {
        j: &j,
        buffers: buffers(&j, bin, limits, &mut scene, resolve)?,
        vertices_left: Cell::new(limits.max_vertices),
        triangles_left: Cell::new(limits.max_triangles),
        parts_left: Cell::new(limits.max_parts),
        zero_left: Cell::new(MAX_ZERO_COMPONENTS),
        data_left: Cell::new(limits.max_data_bytes),
    };
    images(&d, limits, &mut scene)?;
    materials(&j, limits, &mut scene)?;
    if arr(&j, "meshes").len() > limits.max_meshes {
        return Err(too_large(KIND, format!("more than {} meshes", limits.max_meshes)));
    }
    let mut skipped = Skipped::default();
    for m in arr(&j, "meshes") {
        let mut mesh = Mesh { name: name(m), ..Mesh::default() };
        let mut shared = Shared::new();
        for p in arr(m, "primitives") {
            primitive(&d, p, &mut mesh, &mut shared, scene.materials.len(), &mut skipped)?;
        }
        scene.meshes.push(mesh);
    }
    cameras_and_lights(&j, limits, &mut scene)?;
    nodes(&j, limits, &mut scene)?;
    skins(&d, limits, &mut scene)?;
    animations(&d, limits, &mut scene)?;
    count_warning(&mut scene.warnings, skipped.primitives, "primitive(s) are points, lines or have no positions and were left out");
    count_warning(&mut scene.warnings, skipped.triangles, "triangle(s) could not be read and were left out");
    // A hierarchy with a cycle, or one too deep to walk, is refused here rather than by every caller.
    scene.instances(limits)?;
    Ok(scene)
}

/// The binary chunk and the buffer views and accessors over it, as they are written.
#[derive(Default)]
struct Writer {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Writer {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        self.bin.resize(self.bin.len().next_multiple_of(4), 0);
        let mut view = json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len() });
        if let (Some(t), Some(v)) = (target, view.as_object_mut()) {
            v.insert("target".into(), json!(t));
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(view);
        self.views.len() - 1
    }

    /// A float accessor of `N` components per row; with the bounds glTF wants for positions.
    fn floats<const N: usize>(&mut self, rows: impl Iterator<Item = [f32; N]>, kind: &str, bounds: bool) -> usize {
        let mut bytes = Vec::new();
        let (mut lo, mut hi, mut count) = ([f32::INFINITY; N], [f32::NEG_INFINITY; N], 0usize);
        for row in rows {
            count += 1;
            for (k, x) in row.iter().enumerate() {
                bytes.extend_from_slice(&x.to_le_bytes());
                if let (Some(l), Some(h)) = (lo.get_mut(k), hi.get_mut(k)) {
                    *l = l.min(*x);
                    *h = h.max(*x);
                }
            }
        }
        let view = self.view(&bytes, Some(34962));
        let mut acc = json!({ "bufferView": view, "componentType": 5126, "count": count, "type": kind });
        if let (true, Some(a)) = (bounds && count > 0, acc.as_object_mut()) {
            a.insert("min".into(), json!(lo.to_vec()));
            a.insert("max".into(), json!(hi.to_vec()));
        }
        self.accessors.push(acc);
        self.accessors.len() - 1
    }

    fn indices(&mut self, triangles: &[[u32; 3]]) -> usize {
        let bytes: Vec<u8> = triangles.iter().flatten().flat_map(|i| i.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34963));
        self.accessors.push(json!({ "bufferView": view, "componentType": 5125, "count": triangles.len() * 3, "type": "SCALAR" }));
        self.accessors.len() - 1
    }

    /// The glTF mesh of `mesh`, or `None` when it has no triangle.
    fn mesh(&mut self, mesh: &Mesh) -> Option<Value> {
        let u = mesh.unified();
        if u.parts.iter().all(Vec::is_empty) {
            return None;
        }
        let mut attrs = Map::new();
        attrs.insert("POSITION".into(), json!(self.floats(u.positions.iter().map(|p| p.map(|x| x as f32)), "VEC3", true)));
        if !u.normals.is_empty() {
            attrs.insert("NORMAL".into(), json!(self.floats(u.normals.iter().map(|n| n.map(|x| x as f32)), "VEC3", false)));
        }
        if !u.uvs.is_empty() {
            for (k, set) in mesh.uv_sets.iter().enumerate() {
                let rows = u.corners.iter().map(|c| set.get(c.uv as usize).copied().unwrap_or_default());
                attrs.insert(format!("TEXCOORD_{k}"), json!(self.floats(rows, "VEC2", false)));
            }
        }
        if !u.colors.is_empty() {
            attrs.insert("COLOR_0".into(), json!(self.floats(u.colors.iter().copied(), "VEC4", false)));
        }
        let mut primitives = Vec::new();
        for (part, triangles) in mesh.parts.iter().zip(&u.parts).filter(|(_, t)| !t.is_empty()) {
            let mut p = json!({ "attributes": attrs, "indices": self.indices(triangles) });
            if let (Some(m), Some(o)) = (part.material, p.as_object_mut()) {
                o.insert("material".into(), json!(m));
            }
            primitives.push(p);
        }
        let mut out = json!({ "primitives": primitives });
        if let (false, Some(o)) = (mesh.name.is_empty(), out.as_object_mut()) {
            o.insert("name".into(), json!(mesh.name));
        }
        Some(out)
    }
}

/// The media type of encoded image bytes, for the formats glTF allows.
fn sniff(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8]) { "image/jpeg" } else { "image/png" }
}

/// Images, samplers and textures as written: one texture per image and wrap pair in use.
#[derive(Default)]
struct Textures {
    images: Vec<Value>,
    /// The written index of each image of the scene (`None`: no bytes and no name to write).
    image_map: Vec<Option<usize>>,
    samplers: Vec<[Wrap; 2]>,
    textures: Vec<(usize, usize)>,
}

impl Textures {
    fn slot(&mut self, t: Option<TextureRef>) -> Option<Value> {
        let t = t?;
        let image = (*self.image_map.get(t.image)?)?;
        let sampler = self.samplers.iter().position(|s| *s == t.wrap).unwrap_or_else(|| {
            self.samplers.push(t.wrap);
            self.samplers.len() - 1
        });
        let index = self.textures.iter().position(|x| *x == (image, sampler)).unwrap_or_else(|| {
            self.textures.push((image, sampler));
            self.textures.len() - 1
        });
        Some(json!({ "index": index, "texCoord": t.uv_set }))
    }
}

fn material_json(m: &Material, textures: &mut Textures, extensions: &mut Vec<&'static str>) -> Value {
    let mut pbr = json!({ "baseColorFactor": m.base_color, "metallicFactor": m.metallic, "roughnessFactor": m.roughness });
    let mut out = json!({ "name": m.name, "doubleSided": m.double_sided });
    let mut ext = Map::new();
    // glTF's emissive factor stops at 1; the rest is a strength.
    let peak = m.emissive.iter().copied().fold(1.0f32, f32::max);
    if peak > 1.0 {
        ext.insert("KHR_materials_emissive_strength".into(), json!({ "emissiveStrength": peak }));
    }
    if m.unlit {
        ext.insert("KHR_materials_unlit".into(), json!({}));
    }
    for name in ["KHR_materials_emissive_strength", "KHR_materials_unlit"] {
        if ext.contains_key(name) && !extensions.contains(&name) {
            extensions.push(name);
        }
    }
    let set = |o: &mut Value, key: &str, v: Option<Value>| {
        if let (Some(v), Some(o)) = (v, o.as_object_mut()) {
            o.insert(key.into(), v);
        }
    };
    set(&mut pbr, "baseColorTexture", textures.slot(m.base_color_texture));
    set(&mut pbr, "metallicRoughnessTexture", textures.slot(m.metallic_roughness_texture));
    let with = |slot: Option<Value>, key: &str, v: f32| {
        slot.map(|mut s| {
            if let Some(o) = s.as_object_mut() {
                o.insert(key.into(), json!(v));
            }
            s
        })
    };
    set(&mut out, "normalTexture", with(textures.slot(m.normal_texture), "scale", m.normal_scale));
    set(&mut out, "occlusionTexture", with(textures.slot(m.occlusion_texture), "strength", m.occlusion_strength));
    set(&mut out, "emissiveTexture", textures.slot(m.emissive_texture));
    set(&mut out, "emissiveFactor", Some(json!(m.emissive.map(|e| e / peak))));
    set(&mut out, "pbrMetallicRoughness", Some(pbr));
    match m.alpha_mode {
        AlphaMode::Opaque => {}
        AlphaMode::Blend => set(&mut out, "alphaMode", Some(json!("BLEND"))),
        AlphaMode::Mask(cutoff) => {
            set(&mut out, "alphaMode", Some(json!("MASK")));
            set(&mut out, "alphaCutoff", Some(json!(cutoff)));
        }
    }
    set(&mut out, "extensions", (!ext.is_empty()).then_some(Value::Object(ext)));
    out
}

/// JSON has no number for these, and a file with them is not glTF.
fn check_finite(scene: &Scene) -> Result<()> {
    let positions = scene.meshes.iter().flat_map(|m| m.positions.iter().flatten()).all(|x| (*x as f32).is_finite());
    let nodes = scene.nodes.iter().flat_map(|n| n.transform.iter()).all(|x| x.is_finite());
    let materials = scene.materials.iter().all(|m| {
        let values = [m.metallic, m.roughness, m.normal_scale, m.occlusion_strength];
        let cutoff = if let AlphaMode::Mask(c) = m.alpha_mode { c } else { 0.0 };
        m.base_color.iter().chain(&m.emissive).chain(&values).all(|x| x.is_finite()) && cutoff.is_finite()
    });
    if positions && nodes && materials { Ok(()) } else { Err(malformed(KIND, "a position, transform or material value is not finite")) }
}

/// The scene as a binary glTF file: its nodes, meshes (each part a primitive over the mesh's
/// [`Mesh::unified`] vertices: positions, normals, UV sets and colours) and materials, with
/// images that have bytes embedded and the others named by their URI. The binary chunk is
/// limited to `limits.max_data_bytes`.
pub fn write_glb(scene: &Scene, limits: &Limits) -> Result<Vec<u8>> {
    scene.validate()?;
    scene.instances(limits)?;
    check_finite(scene)?;
    let mut w = Writer::default();
    let mut meshes = Vec::new();
    let mesh_map: Vec<Option<usize>> = scene
        .meshes
        .iter()
        .map(|m| {
            w.mesh(m).map(|v| {
                meshes.push(v);
                meshes.len() - 1
            })
        })
        .collect();
    let mut textures = Textures::default();
    for image in &scene.images {
        let mut out = Map::new();
        if !image.name.is_empty() {
            out.insert("name".into(), json!(image.name));
        }
        match (&image.data, &image.uri) {
            (Some(bytes), _) => {
                out.insert("bufferView".into(), json!(w.view(bytes, None)));
                out.insert("mimeType".into(), json!(image.mime_type.clone().unwrap_or_else(|| sniff(bytes).into())));
            }
            (None, Some(uri)) => drop(out.insert("uri".into(), json!(uri))),
            (None, None) => {
                textures.image_map.push(None);
                continue;
            }
        }
        textures.image_map.push(Some(textures.images.len()));
        textures.images.push(Value::Object(out));
    }
    let mut extensions = Vec::new();
    let materials: Vec<Value> = scene.materials.iter().map(|m| material_json(m, &mut textures, &mut extensions)).collect();
    let nodes: Vec<Value> = scene
        .nodes
        .iter()
        .map(|n| {
            let mut out = Map::new();
            if !n.name.is_empty() {
                out.insert("name".into(), json!(n.name));
            }
            if n.transform != IDENTITY {
                out.insert("matrix".into(), json!(n.transform));
            }
            if let Some(m) = n.mesh.and_then(|m| mesh_map.get(m).copied().flatten()) {
                out.insert("mesh".into(), json!(m));
            }
            if !n.children.is_empty() {
                out.insert("children".into(), json!(n.children));
            }
            Value::Object(out)
        })
        .collect();
    if w.bin.len() > limits.max_data_bytes {
        return Err(too_large("GLB", format!("more than {} bytes of data", limits.max_data_bytes)));
    }
    let wrap = |w: Wrap| match w {
        Wrap::Repeat => 10497,
        Wrap::Clamp => 33071,
        Wrap::Mirror => 33648,
    };
    let mut root = Map::new();
    root.insert("asset".into(), json!({ "version": "2.0", "generator": "craft-mesh" }));
    root.insert("scene".into(), json!(0));
    root.insert("scenes".into(), json!([{ "nodes": scene.roots }]));
    // glTF does not allow empty arrays.
    let mut list = |key: &str, values: Vec<Value>| {
        if !values.is_empty() {
            root.insert(key.into(), Value::Array(values));
        }
    };
    list("nodes", nodes);
    list("meshes", meshes);
    list("materials", materials);
    list("textures", textures.textures.iter().map(|(image, sampler)| json!({ "source": image, "sampler": sampler })).collect());
    list("samplers", textures.samplers.iter().map(|s| json!({ "wrapS": wrap(s[0]), "wrapT": wrap(s[1]) })).collect());
    list("images", textures.images);
    list("accessors", w.accessors);
    list("bufferViews", w.views);
    list("extensionsUsed", extensions.iter().map(|e| json!(e)).collect());
    if !w.bin.is_empty() {
        root.insert("buffers".into(), json!([{ "byteLength": w.bin.len() }]));
    }
    let json = serde_json::to_vec(&Value::Object(root)).map_err(|e| malformed(KIND, e))?;
    glb_container(&json, &w.bin)
}
