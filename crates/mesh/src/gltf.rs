//! glTF 2.0, read (`.gltf` and `.glb`) and written (`.glb`), from the Khronos glTF 2.0
//! specification.
//!
//! Read: the default scene's node hierarchy (matrix or translation/rotation/scale), triangle
//! primitives (`TRIANGLES`, `TRIANGLE_STRIP`, `TRIANGLE_FAN`, indexed or not) with POSITION,
//! NORMAL, TANGENT, every TEXCOORD_n, COLOR_0, JOINTS_0 and WEIGHTS_0 of any component type the
//! specification or `KHR_mesh_quantization` allows, strided buffer views, sparse accessors,
//! metallic-roughness materials (factors, the five texture slots with their UV set, wrap
//! modes and `KHR_texture_transform`, alpha modes, double-sided, `KHR_materials_emissive_strength`, `KHR_materials_unlit`;
//! `KHR_materials_pbrSpecularGlossiness` as its diffuse colour and texture with roughness from
//! the glossiness), textures whose image is only in `EXT_texture_webp`, images (embedded
//! in a buffer view or a `data:` URI, or named), skins, animations of translation, rotation
//! and scale, cameras and `KHR_lights_punctual` lights. A glTF mesh becomes one [`Mesh`] whose
//! pools hold its primitives' vertices one after another (once for primitives that name the
//! same accessors and morph targets), and each primitive a [`Part`].
//!
//! Morph targets: each target's POSITION, NORMAL and TANGENT offsets (dense or sparse
//! accessors, any component type) become one [`MorphTarget`] of the mesh, with the mesh's
//! default `weights`, names from `extras.targetNames`, and a node's own `weights` in
//! [`Node::morph_weights`]. A mesh with more targets than [`Limits::max_morph_targets`] is read
//! in its base shape, and its targets are named in a warning. The offsets count against
//! [`Limits::max_data_bytes`] at 8 bytes per value, the zero offsets of vertices a target does
//! not move included, and so does each [`MorphTarget`] itself. The buffers count against the
//! same limit, and the offsets, images, skins and animations get what the buffers leave of it.
//!
//! The `extensions` and `extras` of the file, its nodes, meshes and materials are kept as
//! written ([`Scene::extensions`], [`Node::extensions`], …), for a caller that reads an
//! extension this module does not. They are kept whole: an extension that is also read into
//! the model's own fields (lights, the material extensions) stays in them as written.
//!
//! Not read: required extensions that change how geometry is stored (Draco, meshopt) are
//! [`Error::Unsupported`]; points, lines, scenes other than the default and animations of
//! morph target weights are passed over (skipped primitives and the other scenes are named in
//! warnings).
//!
//! Buffers are needed to read anything, so an external buffer is asked of the resolver (by its
//! percent-decoded URI) and is [`Error::Missing`] when declined. External images are only named
//! ([`Image::uri`]) and never asked for. Both are listed in [`Scene::external`]. The names come
//! from the file: a resolver must not trust them as paths (they may be absolute or hold `..`).
//!
//! A triangle that names a vertex that is not there, or one whose position is not finite, is
//! left out and counted in a warning.
//!
//! Written ([`write_glb`]): nodes, meshes (positions, normals, every UV set, colours, joints
//! and weights), skins, morph targets (POSITION and NORMAL offsets, the mesh's and each node's
//! weights, names in `extras.targetNames`), materials and images. A target that moves fewer
//! than three quarters of the vertices is written as a sparse accessor. The kept `extensions`
//! and `extras` of the file, nodes, meshes and materials are written as kept, except the
//! extensions the reader reads into the model's own fields: those are written from the fields
//! (the material extensions) or not at all (lights). `extensionsUsed` lists every extension the
//! written file names. The reader does not keep `extensionsRequired`; the writer requires only
//! `EXT_texture_webp`, which it uses for every texture of a WebP image (core glTF has no WebP). The file's textures ([`Scene::texture_variants`]) are written first and
//! in their order, so indices into nodes, materials and skins in kept extension data still name
//! the same things. Indices into textures, meshes and images do too, unless the reader left out
//! a texture without an image (named in a warning) or the writer leaves out a mesh without
//! triangles or an image without bytes or a URI: the later ones then move up. Indices into
//! accessors, buffer views and buffers are not rewritten. Animations, cameras, lights and
//! tangents (and so the tangent offsets of morph targets) are not written.

use std::cell::Cell;
use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::{
    AlphaMode, Animation, Camera, Channel, ChannelPath, Corner, Error, IDENTITY, Image, Interpolation, Light, LightKind, Limits, Material, Mesh,
    MorphTarget, NONE, Node, Part, Projection, Resolver, Result, Scene, Skin, TextureRef, Trs, Unified, UvTransform, Wrap, count_warning, malformed,
    too_large,
};

const KIND: &str = "glTF";
const TEXTURE_TRANSFORM: &str = "KHR_texture_transform";
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
    String::from_utf8_lossy(&percent_decode(s)).into_owned()
}

/// Percent-decode a URI, rejecting decoded bytes that are not UTF-8.
///
/// As with [`uri_decode`], incomplete or non-hex escapes stay literal and `+` stays
/// `+`. This does not validate or confine a filesystem path; callers must retain
/// their own reference policy. Decoding is linear and output is no larger than input.
/// Use this instead of the lossy decoder when a resolver requires exact UTF-8 names.
pub fn uri_decode_strict(s: &str) -> Option<String> {
    String::from_utf8(percent_decode(s)).ok()
}

/// [`uri_decode`] as bytes, so escaped bytes that are not UTF-8 survive.
fn percent_decode(s: &str) -> Vec<u8> {
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
    out
}

/// The media type and bytes of a `data:` URI.
fn data_uri(uri: &str) -> Option<Result<(String, Vec<u8>)>> {
    let rest = uri.strip_prefix("data:")?;
    Some((|| {
        let (meta, data) = rest.split_once(',').ok_or_else(|| malformed(KIND, "a data URI without a comma"))?;
        let mime = meta.split(';').next().unwrap_or("").to_string();
        // Any character may be escaped (RFC 3986 2.1), base64's `+`, `/` and `=` included.
        let bytes = percent_decode(data);
        let bytes = if meta.ends_with(";base64") {
            std::str::from_utf8(&bytes).ok().and_then(base64_decode).ok_or_else(|| malformed(KIND, "bad base64 in a data URI"))?
        } else {
            bytes
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
        self.rows_with(a, max, what, &self.zero_left)
    }

    /// [`Doc::rows`], taking the zeros of an accessor without a buffer view from `zeros`.
    fn rows_with(&self, a: usize, max: usize, what: &str, zeros: &Cell<usize>) -> Result<(Vec<f64>, usize)> {
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
                charge(zeros, total, "values without a buffer")?;
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
    /// Primitives of meshes with more morph targets than the limit allows.
    morphs: usize,
    /// Primitives whose number of morph targets differs from their mesh's.
    uneven: usize,
    /// Morph target attributes that do not fit their primitive's vertices.
    unfit: usize,
}

fn rows_of<const N: usize>(v: &[f64]) -> impl Iterator<Item = [f64; N]> + '_ {
    v.as_chunks::<N>().0.iter().copied()
}

/// Append the primitive's vertex attributes to the mesh's pools; the bases its corners count
/// from (`NONE` for a pool the primitive does not fill).
fn attributes(d: &Doc, attrs: &Value, pos: &[f64], mesh: &mut Mesh) -> Result<[u32; 4]> {
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
    let mut tangent_base = NONE;
    match d.attribute(attrs, "TANGENT", nv, &[4])? {
        Some((v, _)) => {
            if mesh.tangents.is_empty() {
                mesh.tangents.resize(mesh.positions.len(), [0.0; 4]);
            }
            tangent_base = base_of(mesh.tangents.len())?;
            mesh.tangents.extend(rows_of::<4>(&v));
        }
        None if !mesh.tangents.is_empty() => mesh.tangents.resize(mesh.positions.len() + nv, [0.0; 4]),
        None => {}
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
    Ok([base, normal_base, uv_base, tangent_base])
}

/// Append the offsets of the primitive's morph targets (`list`) to the mesh's targets, from
/// where the primitive's vertices start in each pool. An attribute that does not have one
/// `VEC3` per vertex, or that moves a pool the primitive does not fill, is left out.
fn morph_targets(
    d: &Doc,
    list: &[Value],
    mesh: &mut Mesh,
    [base, normal_base, _, tangent_base]: [u32; 4],
    nv: usize,
    skipped: &mut Skipped,
) -> Result<()> {
    // Every value is counted against the data limit before it is read, so the zeros of an
    // accessor without a buffer view are not counted again.
    let unbounded = Cell::new(usize::MAX);
    for (target, t) in mesh.morph_targets.iter_mut().zip(list) {
        for (start, key, offsets) in
            [(base, "POSITION", &mut target.positions), (normal_base, "NORMAL", &mut target.normals), (tangent_base, "TANGENT", &mut target.tangents)]
        {
            let Some(a) = idx(t, key) else { continue };
            // An offset of a pool the primitive does not fill is left out before its accessor
            // is looked up.
            if start == NONE {
                skipped.unfit += 1;
                continue;
            }
            let vec3 = d.accessor(a)?.get("type").and_then(Value::as_str) == Some("VEC3");
            if !vec3 || d.count(a) != Some(nv) {
                skipped.unfit += 1;
                continue;
            }
            charge(&d.data_left, nv.saturating_mul(size_of::<[f64; 3]>()), "morph target bytes")?;
            let (v, _) = d.rows_with(a, nv, "morph target offsets", &unbounded)?;
            // Vertices of earlier primitives that this target does not move get no offset.
            pad(&d.data_left, offsets, start as usize)?;
            offsets.extend(rows_of::<3>(&v));
        }
    }
    Ok(())
}

/// Give `offsets` `len` entries, the new ones zero, counting their bytes against the data
/// limit.
fn pad(data_left: &Cell<usize>, offsets: &mut Vec<[f64; 3]>, len: usize) -> Result<()> {
    let added = len.saturating_sub(offsets.len());
    charge(data_left, added.saturating_mul(size_of::<[f64; 3]>()), "morph target bytes")?;
    offsets.resize(len.max(offsets.len()), [0.0; 3]);
    Ok(())
}

/// Name the mesh's morph targets (`extras.targetNames`), set their default weights, and give
/// every list of offsets the length of the pool it moves.
fn finish_morph_targets(d: &Doc, m: &Value, mesh: &mut Mesh) -> Result<()> {
    if mesh.morph_targets.is_empty() {
        return Ok(());
    }
    let names = m.get("extras").map_or(&[][..], |e| arr(e, "targetNames"));
    let pools = [mesh.positions.len(), mesh.normals.len(), mesh.tangents.len()];
    for (k, t) in mesh.morph_targets.iter_mut().enumerate() {
        t.name = names.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        for (offsets, len) in [&mut t.positions, &mut t.normals, &mut t.tangents].into_iter().zip(pools) {
            if !offsets.is_empty() {
                pad(&d.data_left, offsets, len)?;
            }
        }
    }
    let weights = arr(m, "weights");
    mesh.morph_weights = (0..mesh.morph_targets.len()).map(|k| weights.get(k).map_or(0.0, finite_f32)).collect();
    Ok(())
}

/// A JSON number as `f32`; 0 when it is not a finite number.
fn finite_f32(v: &Value) -> f32 {
    v.as_f64().map(|x| x as f32).filter(|x| x.is_finite()).unwrap_or(0.0)
}

/// The object's `extensions` and `extras`, as written. Each is a part of the JSON chunk that is
/// kept once, so together they are never larger than it.
fn extension_data(v: &Value) -> (Map<String, Value>, Option<Value>) {
    (v.get("extensions").and_then(Value::as_object).cloned().unwrap_or_default(), v.get("extras").cloned())
}

/// Where the vertices of one set of attribute accessors start in the mesh's pools (positions,
/// normals, UVs), and how many there are. Primitives of a mesh that name the same accessors
/// (one vertex buffer, several materials) share the vertices.
type Shared = std::collections::HashMap<String, ([u32; 4], usize)>;

/// `targets` is the number of morph targets read for the mesh: 0 when it has none, or more
/// than the limit allows.
fn primitive(d: &Doc, p: &Value, mesh: &mut Mesh, shared: &mut Shared, materials: usize, targets: usize, skipped: &mut Skipped) -> Result<()> {
    let mode = idx(p, "mode").unwrap_or(4);
    let position = p.get("attributes").and_then(|a| idx(a, "POSITION"));
    let (Some(attrs), Some(pa), 4..=6) = (p.get("attributes"), position, mode) else {
        skipped.primitives += 1;
        return Ok(());
    };
    charge(&d.parts_left, 1, "primitives")?;
    let own = arr(p, "targets");
    if targets == 0 {
        skipped.morphs += usize::from(!own.is_empty());
    } else {
        skipped.uneven += usize::from(own.len() != targets);
    }
    // Primitives that name the same attributes but other morph targets move differently, so
    // they do not share vertices.
    let key = if targets == 0 { attrs.to_string() } else { json!([attrs, p.get("targets")]).to_string() };
    let ([base, normal_base, uv_base, tangent_base], nv) = match shared.get(&key) {
        Some(known) => *known,
        None => {
            let (pos, n) = d.rows(pa, d.vertices_left.get(), "vertices")?;
            if n != 3 {
                return Err(malformed(KIND, "POSITION is not VEC3"));
            }
            charge(&d.vertices_left, pos.len() / 3, "vertices")?;
            let bases = attributes(d, attrs, &pos, mesh)?;
            if targets > 0 {
                morph_targets(d, own, mesh, bases, pos.len() / 3, skipped)?;
            }
            let placed = (bases, pos.len() / 3);
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
    let mut part = Part { material: idx(p, "material").filter(|m| *m < materials), has_tangents: tangent_base != NONE, ..Part::default() };
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
                        charge(&d.data_left, bytes.len(), "image bytes")?;
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
    let texture_list = arr(j, "textures");
    if texture_list.len() > limits.max_materials {
        return Err(too_large(KIND, format!("more than {} textures", limits.max_materials)));
    }
    scene.texture_variants = texture_list
        .iter()
        .filter_map(|texture| {
            let sampler = idx(texture, "sampler").and_then(|s| arr(j, "samplers").get(s));
            let webp = || texture.get("extensions").and_then(|e| e.get("EXT_texture_webp")).and_then(|e| idx(e, "source"));
            let image = idx(texture, "source").or_else(webp).filter(|i| *i < images)?;
            Some(TextureRef { image, wrap: [wrap(sampler, "wrapS"), wrap(sampler, "wrapT")], ..TextureRef::default() })
        })
        .collect();
    // The textures after one that is left out move up, so `extensions` data that names them by
    // index no longer matches `texture_variants`.
    count_warning(
        &mut scene.warnings,
        texture_list.len() - scene.texture_variants.len(),
        "texture(s) have no image that is read and were left out; the textures after them are numbered from one less",
    );
    let tref = |v: Option<&Value>| -> Option<TextureRef> {
        let v = v?;
        let texture = arr(j, "textures").get(idx(v, "index")?)?;
        let sampler = idx(texture, "sampler").and_then(|s| arr(j, "samplers").get(s));
        // `EXT_texture_webp`: the image when the texture has no fallback `source`.
        let webp = || texture.get("extensions").and_then(|e| e.get("EXT_texture_webp")).and_then(|e| idx(e, "source"));
        let image = idx(texture, "source").or_else(webp).filter(|i| *i < images)?;
        let tt = v.get("extensions").and_then(|e| e.get("KHR_texture_transform"));
        // The extension's own `texCoord` overrides the texture info's.
        let uv_set = tt.and_then(|t| idx(t, "texCoord")).or_else(|| idx(v, "texCoord")).unwrap_or(0);
        Some(TextureRef {
            image,
            uv_set: u32::try_from(uv_set).unwrap_or(u32::MAX),
            wrap: [wrap(sampler, "wrapS"), wrap(sampler, "wrapT")],
            transform: tt.map(uv_transform),
        })
    };
    for m in list {
        let null = Value::Null;
        let pbr = m.get("pbrMetallicRoughness").unwrap_or(&null);
        let ext = m.get("extensions");
        let strength = ext.and_then(|e| e.get("KHR_materials_emissive_strength")).map_or(1.0, |e| num(e, "emissiveStrength", 1.0));
        let clearcoat = ext.and_then(|e| e.get("KHR_materials_clearcoat"));
        let sheen = ext.and_then(|e| e.get("KHR_materials_sheen"));
        let (extensions, extras) = extension_data(m);
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
            clearcoat: clearcoat.map_or(0.0, |c| num(c, "clearcoatFactor", 0.0)) as f32,
            clearcoat_roughness: clearcoat.map_or(0.0, |c| num(c, "clearcoatRoughnessFactor", 0.0)) as f32,
            clearcoat_texture: clearcoat.and_then(|c| tref(c.get("clearcoatTexture"))),
            clearcoat_roughness_texture: clearcoat.and_then(|c| tref(c.get("clearcoatRoughnessTexture"))),
            sheen_color: sheen.map_or([0.0; 3], |s| nums(s, "sheenColorFactor", [0.0; 3]).map(|v| v as f32)),
            sheen_roughness: sheen.map_or(0.0, |s| num(s, "sheenRoughnessFactor", 0.0)) as f32,
            sheen_color_texture: sheen.and_then(|s| tref(s.get("sheenColorTexture"))),
            sheen_roughness_texture: sheen.and_then(|s| tref(s.get("sheenRoughnessTexture"))),
            extensions,
            extras,
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
    let targets: Vec<usize> = scene.meshes.iter().map(|m| m.morph_targets.len()).collect();
    let targets_of = |mesh: Option<usize>| mesh.and_then(|m| targets.get(m)).copied().unwrap_or(0);
    for n in list {
        let matrix = n.get("matrix").and_then(Value::as_array).filter(|a| a.len() == 16).map(|_| nums(n, "matrix", IDENTITY));
        let trs = Trs {
            translation: nums(n, "translation", [0.0; 3]),
            rotation: nums(n, "rotation", [0.0, 0.0, 0.0, 1.0]),
            scale: nums(n, "scale", [1.0; 3]),
        };
        let (extensions, extras) = extension_data(n);
        scene.nodes.push(Node {
            name: name(n),
            transform: matrix.unwrap_or_else(|| trs.matrix()),
            trs: matrix.is_none().then_some(trs),
            mesh: idx(n, "mesh").filter(|m| *m < scene.meshes.len()),
            skin: idx(n, "skin").filter(|s| *s < skins),
            camera: idx(n, "camera").filter(|c| *c < scene.cameras.len()),
            light: n.get("extensions").and_then(|e| e.get("KHR_lights_punctual")).and_then(|e| idx(e, "light")).filter(|l| *l < scene.lights.len()),
            children: indices(n, "children", list.len()),
            // Only as many as the node's mesh has targets read: none when it has none.
            morph_weights: arr(n, "weights").iter().take(targets_of(idx(n, "mesh"))).map(finite_f32).collect(),
            extensions,
            extras,
        });
    }
    let scenes = arr(j, "scenes");
    let chosen = idx(j, "scene").filter(|s| *s < scenes.len()).unwrap_or(0);
    if scenes.len() > 1 {
        scene.warnings.push(format!("scene {chosen} of {} was read; the others were left out", scenes.len()));
    }
    scene.roots = match scenes.get(chosen) {
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

/// A `KHR_texture_transform` object; a missing or unusable member keeps its default.
fn uv_transform(t: &Value) -> UvTransform {
    let finite = |v: f64, default: f32| Some(v as f32).filter(|v| v.is_finite()).unwrap_or(default);
    let d = UvTransform::default();
    let [ou, ov] = nums(t, "offset", [0.0; 2]);
    let [su, sv] = nums(t, "scale", [1.0; 2]);
    UvTransform {
        offset: [finite(ou, d.offset[0]), finite(ov, d.offset[1])],
        rotation: finite(num(t, "rotation", 0.0), d.rotation),
        scale: [finite(su, d.scale[0]), finite(sv, d.scale[1])],
    }
}

/// Required extensions that only change how a file looks (textures, material models) or that
/// the reader handles: the geometry reads without the others being understood.
fn check_extensions(j: &Value, scene: &mut Scene) -> Result<()> {
    const LOOKS: [&str; 3] = ["KHR_texture_basisu", "MSFT_texture_dds", "KHR_lights_punctual"];
    const READ: [&str; 8] = [
        TEXTURE_TRANSFORM,
        "KHR_mesh_quantization",
        "KHR_materials_unlit",
        "KHR_materials_emissive_strength",
        "KHR_materials_pbrSpecularGlossiness",
        "KHR_materials_clearcoat",
        "KHR_materials_sheen",
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
    let (extensions, extras) = extension_data(&j);
    let mut scene = Scene { extensions, extras, ..Scene::default() };
    check_extensions(&j, &mut scene)?;
    let buffers = buffers(&j, bin, limits, &mut scene, resolve)?;
    // What is read out of the buffers shares one limit with the buffers themselves.
    let data_left = limits.max_data_bytes.saturating_sub(buffers.iter().map(Vec::len).sum());
    let d = Doc {
        j: &j,
        buffers,
        vertices_left: Cell::new(limits.max_vertices),
        triangles_left: Cell::new(limits.max_triangles),
        parts_left: Cell::new(limits.max_parts),
        zero_left: Cell::new(MAX_ZERO_COMPONENTS),
        data_left: Cell::new(data_left),
    };
    images(&d, limits, &mut scene)?;
    materials(&j, limits, &mut scene)?;
    if arr(&j, "meshes").len() > limits.max_meshes {
        return Err(too_large(KIND, format!("more than {} meshes", limits.max_meshes)));
    }
    let mut skipped = Skipped::default();
    for m in arr(&j, "meshes") {
        // glTF gives every primitive of a mesh the same number of morph targets; a primitive
        // with fewer is not moved by the targets it lacks.
        // Primitives that are left out (points, lines) do not count.
        let read_primitive =
            |p: &&Value| matches!(idx(p, "mode").unwrap_or(4), 4..=6) && p.get("attributes").and_then(|a| idx(a, "POSITION")).is_some();
        let targets = arr(m, "primitives").iter().filter(read_primitive).map(|p| arr(p, "targets").len()).max().unwrap_or(0);
        let targets = if targets > limits.max_morph_targets { 0 } else { targets };
        // Targets without offsets still take memory, and across many meshes it adds up.
        charge(&d.data_left, targets.saturating_mul(size_of::<MorphTarget>()), "morph target bytes")?;
        let (extensions, extras) = extension_data(m);
        let mut mesh = Mesh { name: name(m), morph_targets: vec![MorphTarget::default(); targets], extensions, extras, ..Mesh::default() };
        let mut shared = Shared::new();
        for p in arr(m, "primitives") {
            primitive(&d, p, &mut mesh, &mut shared, scene.materials.len(), targets, &mut skipped)?;
        }
        finish_morph_targets(&d, m, &mut mesh)?;
        scene.meshes.push(mesh);
    }
    cameras_and_lights(&j, limits, &mut scene)?;
    nodes(&j, limits, &mut scene)?;
    skins(&d, limits, &mut scene)?;
    animations(&d, limits, &mut scene)?;
    count_warning(&mut scene.warnings, skipped.primitives, "primitive(s) are points, lines or have no positions and were left out");
    count_warning(&mut scene.warnings, skipped.triangles, "triangle(s) could not be read and were left out");
    count_warning(
        &mut scene.warnings,
        skipped.morphs,
        "primitive(s) have more morph targets than the limit allows, which were left out (drawn in the base shape)",
    );
    count_warning(&mut scene.warnings, skipped.uneven, "primitive(s) have fewer morph targets than their mesh; the others do not move them");
    count_warning(&mut scene.warnings, skipped.unfit, "morph target attribute(s) do not fit their primitive's vertices and were left out");
    // A hierarchy with a cycle, or one too deep to walk, is refused here rather than by every caller.
    scene.instances(limits)?;
    Ok(scene)
}

/// The binary chunk and the buffer views and accessors over it, as they are written.
pub(crate) struct Writer {
    pub(crate) bin: Vec<u8>,
    views: Vec<Value>,
    pub(crate) accessors: Vec<Value>,
    /// Most bytes the binary chunk may hold ([`Limits::max_data_bytes`]).
    max_bytes: usize,
    /// Most morph targets of one mesh ([`Limits::max_morph_targets`]).
    max_targets: usize,
}

impl Writer {
    pub(crate) fn new(limits: &Limits) -> Writer {
        Writer { bin: Vec::new(), views: Vec::new(), accessors: Vec::new(), max_bytes: limits.max_data_bytes, max_targets: limits.max_morph_targets }
    }

    /// [`Error::TooLarge`] when `bytes` more would take the binary chunk past its limit.
    fn room(&self, bytes: usize) -> Result<()> {
        match self.bin.len().next_multiple_of(4).checked_add(bytes) {
            Some(total) if total <= self.max_bytes => Ok(()),
            _ => Err(too_large("GLB", format!("more than {} bytes of data", self.max_bytes))),
        }
    }

    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> Result<usize> {
        self.room(bytes.len())?;
        self.bin.resize(self.bin.len().next_multiple_of(4), 0);
        let mut view = json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len() });
        if let (Some(t), Some(v)) = (target, view.as_object_mut()) {
            v.insert("target".into(), json!(t));
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(view);
        Ok(self.views.len() - 1)
    }

    /// A float accessor of `N` components per row of vertex data; with the bounds glTF wants
    /// for positions.
    fn floats<const N: usize>(&mut self, rows: impl Iterator<Item = [f32; N]>, kind: &str, bounds: bool) -> Result<usize> {
        self.floats_in(rows, kind, bounds, Some(34962))
    }

    /// [`Writer::floats`] in a buffer view with the given `target` (`None` for data that is not
    /// per vertex).
    fn floats_in<const N: usize>(&mut self, rows: impl Iterator<Item = [f32; N]>, kind: &str, bounds: bool, target: Option<u32>) -> Result<usize> {
        let mut bytes = Vec::new();
        let (mut lo, mut hi, mut count) = ([f32::INFINITY; N], [f32::NEG_INFINITY; N], 0usize);
        for row in rows {
            count += 1;
            self.room(bytes.len() + 4 * N)?;
            for (k, x) in row.iter().enumerate() {
                bytes.extend_from_slice(&x.to_le_bytes());
                if let (Some(l), Some(h)) = (lo.get_mut(k), hi.get_mut(k)) {
                    *l = l.min(*x);
                    *h = h.max(*x);
                }
            }
        }
        let view = self.view(&bytes, target)?;
        let mut acc = json!({ "bufferView": view, "componentType": 5126, "count": count, "type": kind });
        if let (true, Some(a)) = (bounds && count > 0, acc.as_object_mut()) {
            a.insert("min".into(), json!(lo.to_vec()));
            a.insert("max".into(), json!(hi.to_vec()));
        }
        self.accessors.push(acc);
        Ok(self.accessors.len() - 1)
    }

    fn indices(&mut self, triangles: &[[u32; 3]]) -> Result<usize> {
        self.room(triangles.len().saturating_mul(12))?;
        let bytes: Vec<u8> = triangles.iter().flatten().flat_map(|i| i.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34963))?;
        self.accessors.push(json!({ "bufferView": view, "componentType": 5125, "count": triangles.len() * 3, "type": "SCALAR" }));
        Ok(self.accessors.len() - 1)
    }

    /// The glTF mesh of `mesh` and the highest joint its written vertices name, or `None` when
    /// it has no triangle.
    pub(crate) fn mesh(&mut self, mesh: &Mesh) -> Result<Option<(Value, Option<u16>)>> {
        // Parts whose corners all have normals (or UVs) and parts whose corners do not are
        // written over separate vertices, so the first keep their normals, UVs and the morph
        // targets' normal offsets.
        let layout = |p: &Part| {
            let all = |f: &dyn Fn(&Corner) -> bool| p.corners.iter().all(f);
            (!mesh.normals.is_empty() && all(&|c| c.normal != NONE), !mesh.uv_sets.is_empty() && all(&|c| c.uv != NONE))
        };
        let keys: Vec<Option<(bool, bool)>> = mesh.parts.iter().map(|p| (!p.corners.is_empty()).then(|| layout(p))).collect();
        let mut layouts = Vec::new();
        for key in keys.iter().flatten() {
            if !layouts.contains(key) {
                layouts.push(*key);
            }
        }
        let groups: Vec<Unified> = if layouts.len() <= 1 {
            vec![mesh.unified()]
        } else {
            layouts.iter().map(|l| mesh.unified_parts(|k| keys.get(k).copied().flatten() == Some(*l))).collect()
        };
        if groups.iter().all(|u| u.parts.iter().all(Vec::is_empty)) {
            return Ok(None);
        }
        let mut top = None;
        let mut streams = Vec::with_capacity(groups.len());
        for u in &groups {
            if u.parts.iter().all(Vec::is_empty) {
                streams.push(None);
                continue;
            }
            let (attrs, highest) = self.vertices(mesh, u)?;
            top = top.max(highest);
            streams.push(Some((attrs, self.targets(mesh, u)?)));
        }
        let mut primitives = Vec::new();
        for (k, part) in mesh.parts.iter().enumerate() {
            for (u, stream) in groups.iter().zip(&streams) {
                let (Some(triangles), Some((attrs, targets))) = (u.parts.get(k).filter(|t| !t.is_empty()), stream) else { continue };
                let mut p =
                    Map::from_iter([("attributes".to_string(), Value::Object(attrs.clone())), ("indices".into(), json!(self.indices(triangles)?))]);
                if let Some(m) = part.material {
                    p.insert("material".into(), json!(m));
                }
                // Every primitive of a mesh has the mesh's targets.
                if !targets.is_empty() {
                    p.insert("targets".into(), Value::Array(targets.clone()));
                }
                primitives.push(Value::Object(p));
            }
        }
        let mut out = Map::from_iter([("primitives".to_string(), Value::Array(primitives))]);
        if !mesh.name.is_empty() {
            out.insert("name".into(), json!(mesh.name));
        }
        if !mesh.morph_targets.is_empty() && !mesh.morph_weights.is_empty() {
            out.insert("weights".into(), json!(mesh.morph_weights));
        }
        if !mesh.extensions.is_empty() {
            out.insert("extensions".into(), Value::Object(mesh.extensions.clone()));
        }
        if let Some(extras) = mesh_extras(mesh) {
            out.insert("extras".into(), extras);
        }
        Ok(Some((Value::Object(out), top)))
    }

    /// The vertex attributes of `u`, the mesh's vertices for some of its parts, and the highest
    /// joint they name.
    fn vertices(&mut self, mesh: &Mesh, u: &Unified) -> Result<(Map<String, Value>, Option<u16>)> {
        let mut attrs = Map::new();
        attrs.insert("POSITION".into(), json!(self.floats(u.positions.iter().map(|p| p.map(|x| x as f32)), "VEC3", true)?));
        if !u.normals.is_empty() {
            attrs.insert("NORMAL".into(), json!(self.floats(u.normals.iter().map(|n| n.map(|x| x as f32)), "VEC3", false)?));
        }
        if !u.uvs.is_empty() {
            for (k, set) in mesh.uv_sets.iter().enumerate() {
                let rows = u.corners.iter().map(|c| set.get(c.uv as usize).copied().unwrap_or_default());
                attrs.insert(format!("TEXCOORD_{k}"), json!(self.floats(rows, "VEC2", false)?));
            }
        }
        if !u.colors.is_empty() {
            attrs.insert("COLOR_0".into(), json!(self.floats(u.colors.iter().copied(), "VEC4", false)?));
        }
        let mut top = None;
        if !mesh.joints.is_empty() && !mesh.weights.is_empty() {
            let joints: Vec<[u16; 4]> = u.corners.iter().map(|c| mesh.joints.get(c.position as usize).copied().unwrap_or_default()).collect();
            top = joints.iter().flatten().max().copied();
            attrs.insert("JOINTS_0".into(), json!(self.joints(joints.into_iter())?));
            let weights = u.corners.iter().map(|c| mesh.weights.get(c.position as usize).copied().unwrap_or_default());
            attrs.insert("WEIGHTS_0".into(), json!(self.floats(weights, "VEC4", false)?));
        }
        Ok((attrs, top))
    }

    /// `JOINTS_0`: four joint indices per vertex, as unsigned shorts.
    fn joints(&mut self, rows: impl Iterator<Item = [u16; 4]>) -> Result<usize> {
        let mut bytes = Vec::new();
        let mut count = 0usize;
        for row in rows {
            count += 1;
            self.room(bytes.len() + 8)?;
            bytes.extend(row.iter().flat_map(|j| j.to_le_bytes()));
        }
        let view = self.view(&bytes, Some(34962))?;
        self.accessors.push(json!({ "bufferView": view, "componentType": 5123, "count": count, "type": "VEC4" }));
        Ok(self.accessors.len() - 1)
    }

    /// The mesh's morph targets over the vertices of `u`: a POSITION for each, and a NORMAL
    /// where the target moves normals and the normals are written. glTF wants at least one
    /// attribute in a target, so a target that moves nothing gets a POSITION of zeros. A mesh
    /// with more targets than [`Limits::max_morph_targets`] is [`Error::TooLarge`].
    fn targets(&mut self, mesh: &Mesh, u: &Unified) -> Result<Vec<Value>> {
        if mesh.morph_targets.len() > self.max_targets {
            return Err(too_large("GLB", format!("mesh `{}` has more than {} morph targets", mesh.name, self.max_targets)));
        }
        let pick = |offsets: &[[f64; 3]], i: u32| offsets.get(i as usize).map_or([0.0; 3], |o| o.map(|x| x as f32));
        let mut out = Vec::with_capacity(mesh.morph_targets.len());
        for t in &mesh.morph_targets {
            let rows: Vec<[f32; 3]> = u.corners.iter().map(|c| pick(&t.positions, c.position)).collect();
            let mut target = Map::from_iter([("POSITION".to_string(), json!(self.offsets(&rows)?))]);
            if !t.normals.is_empty() && !u.normals.is_empty() {
                let rows: Vec<[f32; 3]> = u.corners.iter().map(|c| pick(&t.normals, c.normal)).collect();
                target.insert("NORMAL".into(), json!(self.offsets(&rows)?));
            }
            out.push(Value::Object(target));
        }
        Ok(out)
    }

    /// The `VEC3` accessor of one attribute of a morph target, with the bounds glTF wants for a
    /// target's POSITION. When fewer than three quarters of the rows move, only those are
    /// written, as a sparse accessor over zeros (no buffer view), which takes fewer bytes.
    fn offsets(&mut self, rows: &[[f32; 3]]) -> Result<usize> {
        let moved: Vec<u32> = (0u32..).zip(rows).filter(|(_, r)| **r != [0.0; 3]).map(|(i, _)| i).collect();
        // A sparse row is a 4-byte index and 12 bytes of values; a dense one, 12 bytes.
        if moved.len().saturating_mul(4) >= rows.len().saturating_mul(3) {
            return self.floats(rows.iter().copied(), "VEC3", true);
        }
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for row in rows {
            for ((l, h), x) in lo.iter_mut().zip(hi.iter_mut()).zip(row) {
                *l = l.min(*x);
                *h = h.max(*x);
            }
        }
        let mut acc = Map::from_iter([
            ("componentType".to_string(), json!(5126)),
            ("count".into(), json!(rows.len())),
            ("type".into(), json!("VEC3")),
            ("min".into(), json!(lo)),
            ("max".into(), json!(hi)),
        ]);
        if !moved.is_empty() {
            // The index and value views, 16 bytes a row. The index view ends on a 4-byte
            // boundary, so no padding goes between them.
            self.room(moved.len().saturating_mul(16))?;
            let indices: Vec<u8> = moved.iter().flat_map(|i| i.to_le_bytes()).collect();
            let values: Vec<u8> = moved.iter().filter_map(|i| rows.get(*i as usize)).flatten().flat_map(|x| x.to_le_bytes()).collect();
            let (indices, values) = (self.view(&indices, None)?, self.view(&values, None)?);
            let sparse =
                json!({ "count": moved.len(), "indices": { "bufferView": indices, "componentType": 5125 }, "values": { "bufferView": values } });
            acc.insert("sparse".into(), sparse);
        }
        self.accessors.push(Value::Object(acc));
        Ok(self.accessors.len() - 1)
    }

    /// A skin: its joints, and its inverse bind matrices when it has them (none means identity).
    fn skin(&mut self, s: &Skin) -> Result<Value> {
        let mut out = Map::from_iter([("joints".to_string(), json!(s.joints))]);
        if !s.name.is_empty() {
            out.insert("name".into(), json!(s.name));
        }
        if !s.inverse_bind.is_empty() {
            let rows = s.inverse_bind.iter().map(|m| m.map(|x| x as f32));
            out.insert("inverseBindMatrices".into(), json!(self.floats_in(rows, "MAT4", false, None)?));
        }
        Ok(Value::Object(out))
    }
}

/// A node as written. `mesh` is its mesh's written index and the mesh, when the mesh is
/// written; glTF gives a node a skin and weights only with a mesh.
fn node_json(n: &Node, mesh: Option<(usize, &Mesh)>) -> Value {
    let mut out = Map::new();
    if !n.name.is_empty() {
        out.insert("name".into(), json!(n.name));
    }
    if n.transform != IDENTITY {
        out.insert("matrix".into(), json!(n.transform));
    }
    if let Some((index, m)) = mesh {
        out.insert("mesh".into(), json!(index));
        if let Some(skin) = n.skin {
            out.insert("skin".into(), json!(skin));
        }
        // One weight per target of the mesh: the mesh's own weights stand in for any the node
        // leaves out.
        if !n.morph_weights.is_empty() && !m.morph_targets.is_empty() {
            let weight = |k: usize| n.morph_weights.get(k).or_else(|| m.morph_weights.get(k)).copied().unwrap_or(0.0);
            out.insert("weights".into(), json!((0..m.morph_targets.len()).map(weight).collect::<Vec<_>>()));
        }
    }
    if !n.children.is_empty() {
        out.insert("children".into(), json!(n.children));
    }
    let extensions = kept(&n.extensions, &[LIGHTS]);
    if !extensions.is_empty() {
        out.insert("extensions".into(), Value::Object(extensions));
    }
    if let Some(extras) = &n.extras {
        out.insert("extras".into(), extras.clone());
    }
    Value::Object(out)
}

/// The mesh's `extras` as written: as kept, with `targetNames` set to the names of its morph
/// targets when one has a name. `extras` that are not an object are written as they are, and
/// the names are then not written.
fn mesh_extras(mesh: &Mesh) -> Option<Value> {
    let named = mesh.morph_targets.iter().any(|t| !t.name.is_empty());
    let names = || json!(mesh.morph_targets.iter().map(|t| t.name.as_str()).collect::<Vec<_>>());
    match mesh.extras.clone() {
        Some(Value::Object(mut o)) => {
            if named {
                o.insert("targetNames".into(), names());
            }
            Some(Value::Object(o))
        }
        None if named => Some(json!({ "targetNames": names() })),
        other => other,
    }
}

/// The media type of encoded image bytes, for the formats glTF allows.
fn sniff(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8]) {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        "image/png"
    }
}

/// `EXT_texture_webp`: core glTF has no WebP, so a texture of a WebP image names it only through
/// the extension, which a file without another source then requires.
const WEBP: &str = "EXT_texture_webp";

/// Images, samplers and textures as written: the scene's [`Scene::texture_variants`] first, in
/// their order, then one texture per other image and wrap pair in use.
#[derive(Default)]
struct Textures {
    images: Vec<Value>,
    /// The written index of each image of the scene (`None`: no bytes and no name to write).
    image_map: Vec<Option<usize>>,
    samplers: Vec<[Wrap; 2]>,
    /// The written image (`None`: a texture of the file whose image is not written) and sampler.
    textures: Vec<(Option<usize>, usize)>,
    /// The index of the first of `textures` with each image and sampler.
    first: std::collections::HashMap<(Option<usize>, usize), usize>,
    /// Whether each written image is WebP.
    webp: Vec<bool>,
}

impl Textures {
    /// The `textures` list, and whether a texture names its image only through `EXT_texture_webp`.
    fn json(&self) -> (Vec<Value>, bool) {
        let mut webp_only = false;
        let list = self
            .textures
            .iter()
            .map(|(image, sampler)| match image {
                Some(image) if self.webp.get(*image).copied().unwrap_or(false) => {
                    webp_only = true;
                    json!({ "sampler": sampler, "extensions": { WEBP: { "source": image } } })
                }
                Some(image) => json!({ "source": image, "sampler": sampler }),
                // glTF allows a texture without a source; it keeps the index of the next ones.
                None => json!({ "sampler": sampler }),
            })
            .collect();
        (list, webp_only)
    }

    fn sampler(&mut self, wrap: [Wrap; 2]) -> usize {
        self.samplers.iter().position(|s| *s == wrap).unwrap_or_else(|| {
            self.samplers.push(wrap);
            self.samplers.len() - 1
        })
    }

    /// Keep the textures of the file at their indices, which `extensions` the writer keeps may
    /// name.
    fn variants(&mut self, variants: &[TextureRef]) {
        for t in variants {
            let image = self.image_map.get(t.image).copied().flatten();
            let sampler = self.sampler(t.wrap);
            self.push((image, sampler));
        }
    }

    /// Add a texture, and the index of the first texture with its image and sampler.
    fn push(&mut self, texture: (Option<usize>, usize)) -> usize {
        self.textures.push(texture);
        *self.first.entry(texture).or_insert(self.textures.len() - 1)
    }

    fn slot(&mut self, t: Option<TextureRef>) -> Option<Value> {
        let t = t?;
        let image = (*self.image_map.get(t.image)?)?;
        let sampler = self.sampler(t.wrap);
        let key = (Some(image), sampler);
        let index = match self.first.get(&key) {
            Some(index) => *index,
            None => self.push(key),
        };
        let mut out = json!({ "index": index, "texCoord": t.uv_set });
        if let (Some(tt), Some(o)) = (t.transform, out.as_object_mut()) {
            let ext = json!({ "offset": tt.offset, "rotation": tt.rotation, "scale": tt.scale });
            o.insert("extensions".into(), json!({ TEXTURE_TRANSFORM: ext }));
        }
        Some(out)
    }
}

/// Material extensions the reader reads into [`Material`]'s fields. The writer writes them from
/// the fields, or not at all, and never as kept.
const MATERIAL_FIELDS: [&str; 5] = [
    "KHR_materials_emissive_strength",
    "KHR_materials_unlit",
    "KHR_materials_clearcoat",
    "KHR_materials_sheen",
    "KHR_materials_pbrSpecularGlossiness",
];

/// Extensions of the file and its nodes that the reader reads into [`Scene::lights`] and
/// [`Node::light`], which the writer does not write.
const LIGHTS: &str = "KHR_lights_punctual";

/// The kept `extensions` of an object, without `skip`.
fn kept(extensions: &Map<String, Value>, skip: &[&str]) -> Map<String, Value> {
    extensions.iter().filter(|(k, _)| !skip.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn material_json(m: &Material, textures: &mut Textures) -> Value {
    let mut pbr = json!({ "baseColorFactor": m.base_color, "metallicFactor": m.metallic, "roughnessFactor": m.roughness });
    let mut out = json!({ "name": m.name, "doubleSided": m.double_sided });
    let mut ext = kept(&m.extensions, &MATERIAL_FIELDS);
    // glTF's emissive factor stops at 1; the rest is a strength.
    let peak = m.emissive.iter().copied().fold(1.0f32, f32::max);
    if peak > 1.0 {
        ext.insert("KHR_materials_emissive_strength".into(), json!({ "emissiveStrength": peak }));
    }
    if m.unlit {
        ext.insert("KHR_materials_unlit".into(), json!({}));
    }
    if m.clearcoat > 0.0 || m.clearcoat_texture.is_some() || m.clearcoat_roughness_texture.is_some() {
        let mut coat = Map::new();
        coat.insert("clearcoatFactor".into(), json!(m.clearcoat));
        if let Some(t) = textures.slot(m.clearcoat_texture) {
            coat.insert("clearcoatTexture".into(), t);
        }
        coat.insert("clearcoatRoughnessFactor".into(), json!(m.clearcoat_roughness));
        if let Some(t) = textures.slot(m.clearcoat_roughness_texture) {
            coat.insert("clearcoatRoughnessTexture".into(), t);
        }
        ext.insert("KHR_materials_clearcoat".into(), Value::Object(coat));
    }
    if m.sheen_color != [0.0; 3] || m.sheen_color_texture.is_some() || m.sheen_roughness_texture.is_some() {
        let mut sheen = Map::new();
        sheen.insert("sheenColorFactor".into(), json!(m.sheen_color));
        if let Some(t) = textures.slot(m.sheen_color_texture) {
            sheen.insert("sheenColorTexture".into(), t);
        }
        sheen.insert("sheenRoughnessFactor".into(), json!(m.sheen_roughness));
        if let Some(t) = textures.slot(m.sheen_roughness_texture) {
            sheen.insert("sheenRoughnessTexture".into(), t);
        }
        ext.insert("KHR_materials_sheen".into(), Value::Object(sheen));
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
    set(&mut out, "extras", m.extras.clone());
    out
}

/// The names of the extensions the document uses (its `extensionsUsed`): the keys of every
/// `extensions` object in it, at any depth, sorted. The application's `extras` are not
/// searched.
fn extensions_used(root: &Map<String, Value>) -> Vec<Value> {
    extension_names(root, |name| name).into_iter().map(|name| json!(name)).collect()
}

/// The keys of every `extensions` object in `root`, at any depth, each made into a `K` by `key`
/// and kept once. Each name takes a number of comparisons logarithmic in the names found so far.
pub(crate) fn extension_names<'a, K: Ord>(root: &'a Map<String, Value>, key: impl Fn(&'a str) -> K) -> BTreeSet<K> {
    let mut used = BTreeSet::new();
    let mut stack: Vec<(&str, &Value)> = root.iter().map(|(k, v)| (k.as_str(), v)).collect();
    while let Some((name, v)) = stack.pop() {
        match (name, v) {
            ("extras", _) => {}
            ("extensions", Value::Object(o)) => {
                for (name, inner) in o {
                    used.insert(key(name));
                    stack.push((name, inner));
                }
            }
            (_, Value::Object(o)) => stack.extend(o.iter().map(|(k, v)| (k.as_str(), v))),
            // An array's items are not under any key.
            (_, Value::Array(a)) => stack.extend(a.iter().map(|item| ("", item))),
            _ => {}
        }
    }
    used
}

/// JSON has no number for these, and a file with them is not glTF.
fn check_finite(scene: &Scene) -> Result<()> {
    let positions = scene.meshes.iter().flat_map(|m| m.positions.iter().flatten()).all(|x| (*x as f32).is_finite());
    let nodes = scene.nodes.iter().flat_map(|n| n.transform.iter()).all(|x| x.is_finite());
    let materials = scene.materials.iter().all(|m| {
        let values = [m.metallic, m.roughness, m.normal_scale, m.occlusion_strength, m.clearcoat, m.clearcoat_roughness, m.sheen_roughness];
        let cutoff = if let AlphaMode::Mask(c) = m.alpha_mode { c } else { 0.0 };
        let slots = [
            &m.base_color_texture,
            &m.metallic_roughness_texture,
            &m.normal_texture,
            &m.occlusion_texture,
            &m.emissive_texture,
            &m.clearcoat_texture,
            &m.clearcoat_roughness_texture,
            &m.sheen_color_texture,
            &m.sheen_roughness_texture,
        ];
        let transforms = slots
            .into_iter()
            .flatten()
            .filter_map(|t| t.transform)
            .all(|t| t.offset.iter().chain(&t.scale).chain([&t.rotation]).all(|x| x.is_finite()));
        m.base_color.iter().chain(&m.emissive).chain(&m.sheen_color).chain(&values).all(|x| x.is_finite()) && cutoff.is_finite() && transforms
    });
    if positions && nodes && materials { Ok(()) } else { Err(malformed(KIND, "a position, transform or material value is not finite")) }
}

/// Every joint a written vertex names is in the skin of each node that places the mesh, and a
/// node with a skin places only written meshes with joints and weights. `top` is the highest
/// joint of each mesh's written vertices, and `written` the written index of each mesh.
fn check_joints(scene: &Scene, top: &[Option<u16>], written: &[Option<usize>]) -> Result<()> {
    for n in &scene.nodes {
        let (Some(m), Some(s)) = (n.mesh, n.skin.and_then(|s| scene.skins.get(s))) else { continue };
        if written.get(m).copied().flatten().is_none() {
            continue;
        }
        let Some(Some(j)) = top.get(m) else {
            return Err(malformed(KIND, format!("node `{}` has skin `{}` and a mesh without joints and weights", n.name, s.name)));
        };
        if usize::from(*j) >= s.joints.len() {
            return Err(malformed(KIND, format!("node `{}` names joint {j} of skin `{}`, which has {}", n.name, s.name, s.joints.len())));
        }
    }
    Ok(())
}

/// Skins as glTF has them: a skin with joints, each node once, and with one inverse bind matrix
/// per joint when it has any, each with a last row of 0, 0, 0, 1; and a mesh with both joints
/// and weights or neither, whose vertices name each joint with a weight other than 0 once.
fn check_skins(scene: &Scene) -> Result<()> {
    if let Some(m) = scene.meshes.iter().find(|m| m.joints.is_empty() != m.weights.is_empty()) {
        return Err(malformed(KIND, format!("mesh `{}` has joints without weights or weights without joints", m.name)));
    }
    let repeated = |j: &[u16; 4], w: &[f32; 4]| (0..4).any(|a| (a + 1..4).any(|b| w[a] != 0.0 && w[b] != 0.0 && j[a] == j[b]));
    if let Some(m) = scene.meshes.iter().find(|m| m.joints.iter().zip(&m.weights).any(|(j, w)| repeated(j, w))) {
        return Err(malformed(KIND, format!("mesh `{}` has a vertex that names a joint with a weight twice", m.name)));
    }
    for s in &scene.skins {
        if s.joints.is_empty() {
            return Err(malformed(KIND, format!("skin `{}` has no joints", s.name)));
        }
        let mut seen = std::collections::HashSet::with_capacity(s.joints.len());
        if !s.joints.iter().all(|j| seen.insert(*j)) {
            return Err(malformed(KIND, format!("skin `{}` names a joint node twice", s.name)));
        }
        // Column-major: the last row is entries 3, 7, 11 and 15, as written in `f32`.
        if s.inverse_bind.iter().any(|m| [m[3], m[7], m[11], m[15]].map(|x| x as f32) != [0.0, 0.0, 0.0, 1.0]) {
            return Err(malformed(KIND, format!("skin `{}` has an inverse bind matrix whose last row is not 0, 0, 0, 1", s.name)));
        }
        if !s.inverse_bind.is_empty() && s.inverse_bind.len() != s.joints.len() {
            return Err(malformed(
                KIND,
                format!("skin `{}` has {} inverse bind matrices for {} joints", s.name, s.inverse_bind.len(), s.joints.len()),
            ));
        }
    }
    Ok(())
}

/// Skins ([`check_skins`]) and morph targets as glTF has them: every value finite, and no node
/// with more weights than its mesh has targets.
fn check_rigging(scene: &Scene) -> Result<()> {
    check_skins(scene)?;
    let targets = |n: &Node| n.mesh.and_then(|m| scene.meshes.get(m)).map_or(0, |m| m.morph_targets.len());
    if let Some(n) = scene.nodes.iter().find(|n| n.morph_weights.len() > targets(n)) {
        return Err(malformed(KIND, format!("node `{}` has {} morph weights for {} targets", n.name, n.morph_weights.len(), targets(n))));
    }
    // Morph target weights may be negative; skinning weights may not.
    if let Some(m) = scene.meshes.iter().find(|m| m.weights.iter().flatten().any(|w| *w < 0.0)) {
        return Err(malformed(KIND, format!("mesh `{}` has a negative skinning weight", m.name)));
    }
    let binds = scene.skins.iter().flat_map(|s| s.inverse_bind.iter().flatten()).all(|x| (*x as f32).is_finite());
    let offsets = scene
        .meshes
        .iter()
        .flat_map(|m| &m.morph_targets)
        .flat_map(|t| t.positions.iter().chain(&t.normals).flatten())
        .all(|x| (*x as f32).is_finite());
    let weights = scene.meshes.iter().flat_map(|m| m.morph_weights.iter().chain(m.weights.iter().flatten()));
    let node_weights = scene.nodes.iter().flat_map(|n| &n.morph_weights);
    if binds && offsets && weights.chain(node_weights).all(|x| x.is_finite()) {
        Ok(())
    } else {
        Err(malformed(KIND, "a skin or morph target value is not finite"))
    }
}

/// Texture magnification filtering for a GLB sampler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum MagFilter {
    Nearest = 9728,
    Linear = 9729,
}

/// Texture minification filtering for a GLB sampler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum MinFilter {
    Nearest = 9728,
    Linear = 9729,
    NearestMipmapNearest = 9984,
    LinearMipmapNearest = 9985,
    NearestMipmapLinear = 9986,
    LinearMipmapLinear = 9987,
}

/// Options applied to each sampler written by the GLB exporter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlbWriteOptions {
    /// Magnification filter. None leaves the choice to the viewer.
    pub mag_filter: Option<MagFilter>,
    /// Minification filter. None leaves the choice to the viewer.
    pub min_filter: Option<MinFilter>,
}

/// The scene as a binary glTF file: its nodes, meshes (each part a primitive over the mesh's
/// [`Mesh::unified`] vertices: positions, normals, UV sets, colours, joints and weights, with
/// the mesh's morph targets), skins and materials, with images that have bytes embedded and
/// the others named by their URI, and the kept `extensions` and `extras` (see the module
/// documentation). The binary chunk is limited to `limits.max_data_bytes`, and each mesh to
/// `limits.max_morph_targets` morph targets; past either, the writer stops with
/// [`Error::TooLarge`] before it writes more. A value that is not
/// finite, a skin without joints, with a joint node named twice, or whose inverse bind matrices
/// do not match its joints or have a last row other than 0, 0, 0, 1, a mesh with joints but no
/// weights (or weights but no joints), a negative skinning weight, a vertex that names a joint
/// with a weight other than 0 twice, a joint of a written vertex outside the skin of a node that
/// places the mesh, a node with a skin that places a written mesh without joints and weights,
/// and a node with more morph weights than its mesh has targets are [`Error::Malformed`].
pub fn write_glb(scene: &Scene, limits: &Limits) -> Result<Vec<u8>> {
    write_glb_with_options(scene, limits, &GlbWriteOptions::default())
}

/// Write a binary glTF scene with explicit sampler filtering options.
pub fn write_glb_with_options(scene: &Scene, limits: &Limits, options: &GlbWriteOptions) -> Result<Vec<u8>> {
    scene.validate()?;
    scene.instances(limits)?;
    check_finite(scene)?;
    check_rigging(scene)?;
    let mut w = Writer::new(limits);
    let mut meshes = Vec::new();
    let mut top = Vec::with_capacity(scene.meshes.len());
    let mesh_map: Vec<Option<usize>> = scene
        .meshes
        .iter()
        .map(|m| {
            let written = w.mesh(m)?;
            top.push(written.as_ref().and_then(|(_, j)| *j));
            Ok(written.map(|(v, _)| {
                meshes.push(v);
                meshes.len() - 1
            }))
        })
        .collect::<Result<_>>()?;
    check_joints(scene, &top, &mesh_map)?;
    let mut textures = Textures::default();
    for image in &scene.images {
        let mut out = Map::new();
        if !image.name.is_empty() {
            out.insert("name".into(), json!(image.name));
        }
        let mime = match (&image.data, &image.uri) {
            (Some(bytes), _) => {
                let mime = image.mime_type.clone().unwrap_or_else(|| sniff(bytes).into());
                out.insert("bufferView".into(), json!(w.view(bytes, None)?));
                out.insert("mimeType".into(), json!(mime));
                mime
            }
            (None, Some(uri)) => {
                out.insert("uri".into(), json!(uri));
                // The path's extension, without a query or fragment.
                let path = uri.split(['?', '#']).next().unwrap_or("");
                let webp = path.rsplit_once('.').is_some_and(|(_, e)| e.eq_ignore_ascii_case("webp"));
                match image.mime_type.clone().filter(|m| !m.is_empty()) {
                    // A URI without the format's extension keeps its media type for the next reader.
                    Some(mime) => {
                        out.insert("mimeType".into(), json!(mime));
                        mime
                    }
                    None if webp => "image/webp".into(),
                    None => String::new(),
                }
            }
            (None, None) => {
                textures.image_map.push(None);
                continue;
            }
        };
        textures.webp.push(mime == "image/webp");
        textures.image_map.push(Some(textures.images.len()));
        textures.images.push(Value::Object(out));
    }
    textures.variants(&scene.texture_variants);
    let materials: Vec<Value> = scene.materials.iter().map(|m| material_json(m, &mut textures)).collect();
    let nodes: Vec<Value> = scene
        .nodes
        .iter()
        .map(|n| {
            let mesh = n.mesh.and_then(|m| Some((mesh_map.get(m).copied().flatten()?, scene.meshes.get(m)?)));
            node_json(n, mesh)
        })
        .collect();
    let skins: Vec<Value> = scene.skins.iter().map(|s| w.skin(s)).collect::<Result<_>>()?;
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
    list("skins", skins);
    list("materials", materials);
    let (texture_list, webp_only) = textures.json();
    list("textures", texture_list);
    list(
        "samplers",
        textures
            .samplers
            .iter()
            .map(|s| {
                let mut sampler = Map::from_iter([("wrapS".into(), json!(wrap(s[0]))), ("wrapT".into(), json!(wrap(s[1])))]);
                if let Some(filter) = options.mag_filter {
                    sampler.insert("magFilter".into(), json!(filter as u16));
                }
                if let Some(filter) = options.min_filter {
                    sampler.insert("minFilter".into(), json!(filter as u16));
                }
                Value::Object(sampler)
            })
            .collect(),
    );
    list("images", textures.images);
    list("accessors", w.accessors);
    list("bufferViews", w.views);
    let extensions = kept(&scene.extensions, &[LIGHTS]);
    if !extensions.is_empty() {
        root.insert("extensions".into(), Value::Object(extensions));
    }
    if let Some(extras) = &scene.extras {
        root.insert("extras".into(), extras.clone());
    }
    let used = extensions_used(&root);
    if !used.is_empty() {
        root.insert("extensionsUsed".into(), Value::Array(used));
    }
    if webp_only {
        root.insert("extensionsRequired".into(), json!([WEBP]));
    }
    if !w.bin.is_empty() {
        root.insert("buffers".into(), json!([{ "byteLength": w.bin.len() }]));
    }
    let json = serde_json::to_vec(&Value::Object(root)).map_err(|e| malformed(KIND, e))?;
    glb_container(&json, &w.bin)
}
