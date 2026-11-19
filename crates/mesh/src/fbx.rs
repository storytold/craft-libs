//! FBX 6.x and 7.x scenes, binary and ASCII.
//!
//! Written from public descriptions of the format (the binary node-record layout, the ASCII
//! syntax, and the scene objects of the Autodesk FBX SDK documentation): no importer source was
//! read.
//!
//! - **Binary**: the `Kaydara FBX Binary` header and version, node records with 32-bit (before
//!   7.5) or 64-bit offsets, every property type (`Y C I F D L S R` and the `f d l i b` arrays,
//!   zlib-compressed or not). Object names `Name\0\x01Class` become `Class::Name` as in ASCII.
//! - **ASCII**: `Key: value, value { children }` with `;` comments and `*N { a: … }` arrays.
//! - **Scene**: `Geometry` meshes (`Vertices`, `PolygonVertexIndex` polygons kept whole, the
//!   first `LayerElementNormal`, `LayerElementUV` and `LayerElementMaterial` with every mapping
//!   and reference mode), `Model` nodes with the SDK transform (translation, rotation offset and
//!   pivot, pre/post rotation, Euler rotation in its order, scaling offset and pivot, scaling)
//!   and geometric transforms (a child node named `<model> geometry`), `Material` (Lambert/Phong
//!   mapped onto metallic-roughness), `Texture` and `Video` (embedded or named image files),
//!   joined by `Connections`.
//! - **6.x**: a `Model` holds its own mesh, properties are `Properties60`, and `Connect` lines
//!   name objects by `Class::Name` instead of by id. Geometry, transforms, the hierarchy and
//!   materials are read; 6.x textures (which connect to Models, not Materials) are not.
//!
//! [`read`] gives the [`Scene`]. [`read_document`] gives the record tree itself
//! ([`Document`]), for callers that need what the scene does not carry;
//! [`Document::axes_to_y_up`] and [`Document::unit_scale`] are the file's `GlobalSettings`, which
//! the scene leaves alone like every reader here: positions stay in the file's axes and units.
//!
//! A layer element is kept as a pool: each corner indexes the `Normals` and `UV` arrays as the
//! element's mapping and reference modes say, and a corner whose index is outside the array has
//! no normal or UV. UVs are flipped to v down. A `Model` is a node; Models that instance one
//! geometry with the same materials share one mesh. A material slot of a geometry is a
//! [`Part`]. A polygon of fewer than three corners, or one that names a vertex that is not
//! there, is left out and counted in a warning.
//!
//! **Images.** A `Texture` whose `Video` embeds its content gives an [`Image`] with `data`.
//! Otherwise the resolver is asked for `RelativeFilename`, then `FileName` (each with `\` as
//! `/`, as written and then by its last component), because only the file knows both names; what
//! it supplies is the image's `data`, and a texture it declines is an image with only its `uri`.
//! Textures that end at one embedded `Video` or one supplied file share one image. Every file
//! name is listed in [`Scene::external`]. The names come from the file: a resolver must not trust
//! them as paths.
//!
//! Not read: FBX 5 and older, animation stacks, skin deformers, blend shapes, cameras and
//! lights, NURBS and patches.
//!
//! [`Limits::max_data_bytes`] bounds what one file may keep in memory: its parsed values (an
//! array element counts 8 bytes whatever its type in the file), the meshes built from them and
//! the image bytes.

use std::cell::Cell;
use std::collections::HashMap;

use crate::{
    AlphaMode, Corner, IDENTITY, Image, Limits, Mat4, Material, Mesh, NONE, Node, Part, Resolver, Result, Scene, TextureRef, base64_decode,
    count_warning, malformed, mat_mul, srgb_to_linear, too_large,
};

const KIND: &str = "FBX";

/// The binary file's magic (`Kaydara FBX Binary`, two spaces, NUL, 0x1A, 0x00).
pub const BINARY_MAGIC: &[u8] = b"Kaydara FBX Binary  \0\x1a\0";
/// Deepest record nesting accepted (real files nest well under 10 levels).
pub const MAX_DEPTH: usize = 64;
/// Largest decoded array accepted, in bytes as the file stores it.
pub const MAX_ARRAY_BYTES: usize = 1 << 30;

/// What is left of the memory one file may keep.
struct Budget {
    left: Cell<usize>,
    limit: usize,
}

impl Budget {
    fn new(bytes: usize) -> Budget {
        Budget { left: Cell::new(bytes), limit: bytes }
    }

    /// Take `bytes` (None: a size that overflowed) from the budget, or fail before anything that
    /// large is allocated.
    fn take(&self, bytes: Option<usize>, what: &str) -> Result<()> {
        let left = self.left.get();
        match bytes.filter(|b| *b <= left) {
            Some(b) => {
                self.left.set(left - b);
                Ok(())
            }
            None => Err(too_large(KIND, format!("{what} would take the file past its memory limit of {} bytes", self.limit))),
        }
    }
}

/// Bytes the values under `records` keep. Used for ASCII files, which hold array elements as one
/// [`Prop`] each and are parsed before they can be counted (the text itself bounds them).
pub(crate) fn value_bytes(records: &[Record]) -> Option<usize> {
    records.iter().try_fold(0usize, |acc, n| {
        let own = n.props.iter().try_fold(0usize, |a, p| {
            let extra = match p {
                Prop::Ints(v) => v.len().checked_mul(8)?,
                Prop::Floats(v) => v.len().checked_mul(8)?,
                Prop::Str(v) => v.len(),
                Prop::Bytes(v) => v.len(),
                Prop::Int(_) | Prop::Float(_) => 0,
            };
            a.checked_add(std::mem::size_of::<Prop>())?.checked_add(extra)
        })?;
        acc.checked_add(own)?.checked_add(value_bytes(&n.children)?)
    })
}

/// Bytes a mesh keeps.
pub(crate) fn mesh_bytes(m: &Mesh) -> Option<usize> {
    let uvs = m.uv_sets.iter().try_fold(0usize, |a, s| a.checked_add(s.len().checked_mul(8)?))?;
    let pools = m.positions.len().checked_mul(24)?.checked_add(m.normals.len().checked_mul(24)?)?.checked_add(uvs)?;
    m.parts.iter().try_fold(pools, |a, p| a.checked_add(p.corners.len().checked_mul(12)?)?.checked_add(p.face_sizes.len().checked_mul(4)?))
}

/// One property value of a record.
#[derive(Clone, Debug, PartialEq)]
pub enum Prop {
    Int(i64),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    Ints(Vec<i64>),
    Floats(Vec<f64>),
}

impl Prop {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Prop::Int(i) => Some(*i as f64),
            Prop::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Prop::Int(i) => Some(*i),
            Prop::Float(f) if f.is_finite() => Some(*f as i64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Prop::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// A record of the FBX document tree (the same for binary and ASCII files).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    pub name: String,
    pub props: Vec<Prop>,
    pub children: Vec<Record>,
}

impl Record {
    pub fn child(&self, name: &str) -> Option<&Record> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Record> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    fn str_at(&self, i: usize) -> Option<&str> {
        self.props.get(i).and_then(Prop::as_str)
    }

    /// The first string property: an object's `Class::Name`.
    fn first_str(&self) -> Option<&str> {
        self.props.iter().find_map(Prop::as_str)
    }

    /// The record's array as floats: a binary array property, an ASCII `a:` child, or the plain
    /// value list.
    pub fn floats(&self) -> Vec<f64> {
        if let Some(a) = self.child("a") {
            return a.floats();
        }
        let mut out = vec![];
        for p in &self.props {
            match p {
                Prop::Floats(v) => out.extend_from_slice(v),
                Prop::Ints(v) => out.extend(v.iter().map(|i| *i as f64)),
                Prop::Int(i) => out.push(*i as f64),
                Prop::Float(f) => out.push(*f),
                _ => {}
            }
        }
        out
    }

    /// The record's array as integers (see [`Record::floats`]).
    pub fn ints(&self) -> Vec<i64> {
        if let Some(a) = self.child("a") {
            return a.ints();
        }
        let mut out = vec![];
        for p in &self.props {
            match p {
                Prop::Ints(v) => out.extend_from_slice(v),
                Prop::Floats(v) => out.extend(v.iter().filter(|f| f.is_finite()).map(|f| *f as i64)),
                p => out.extend(p.as_i64()),
            }
        }
        out
    }
}

/// An FBX file as its tree of records.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    /// The file version (7400 is FBX 7.4, 6100 is FBX 6.1); 0 when an ASCII file does not say.
    pub version: u32,
    /// The top-level records (`FBXHeaderExtension`, `GlobalSettings`, `Objects`, `Connections`, …).
    pub records: Vec<Record>,
    /// What the records left of `Limits::max_data_bytes`.
    left: usize,
}

/// Is this a binary FBX file? Anything else is read as ASCII.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.starts_with(BINARY_MAGIC)
}

/// Read an FBX file (binary or ASCII) into its records.
pub fn read_document(bytes: &[u8], limits: &Limits) -> Result<Document> {
    let budget = Budget::new(limits.max_data_bytes);
    let (version, records) = if is_binary(bytes) {
        read_binary(bytes, &budget)?
    } else {
        let (version, records) = read_ascii(&String::from_utf8_lossy(bytes))?;
        budget.take(value_bytes(&records), "its values")?;
        (version, records)
    };
    Ok(Document { version, records, left: budget.left.get() })
}

/// Read an FBX file into a scene. `resolve` is asked for texture image files (see the module
/// documentation).
pub fn read(bytes: &[u8], limits: &Limits, resolve: Resolver) -> Result<Scene> {
    read_document(bytes, limits)?.scene(limits, resolve)
}

// ---------------------------------------------------------------------------------------------
// Binary

struct Bin<'a> {
    d: &'a [u8],
    /// 64-bit record offsets (version 7500 and later).
    wide: bool,
    budget: &'a Budget,
}

impl Bin<'_> {
    fn take(&self, pos: &mut usize, n: usize) -> Result<&[u8]> {
        let end = pos.checked_add(n).ok_or_else(|| malformed(KIND, "offset overflow"))?;
        let s = self.d.get(*pos..end).ok_or_else(|| malformed(KIND, format!("truncated at byte {pos}")))?;
        *pos = end;
        Ok(s)
    }

    fn arr<const N: usize>(&self, pos: &mut usize) -> Result<[u8; N]> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(pos, N)?);
        Ok(a)
    }

    fn u32(&self, pos: &mut usize) -> Result<u32> {
        Ok(u32::from_le_bytes(self.arr(pos)?))
    }

    fn offset(&self, pos: &mut usize) -> Result<u64> {
        if self.wide { Ok(u64::from_le_bytes(self.arr(pos)?)) } else { Ok(u64::from(self.u32(pos)?)) }
    }

    fn to_usize(v: u64) -> Result<usize> {
        usize::try_from(v).map_err(|_| malformed(KIND, "offset out of range"))
    }

    /// Records until `end` or a null record.
    fn list(&self, pos: &mut usize, end: usize, depth: usize) -> Result<Vec<Record>> {
        if depth > MAX_DEPTH {
            return Err(too_large(KIND, "records nested too deeply"));
        }
        let mut out = vec![];
        while *pos < end {
            match self.record(pos, end, depth)? {
                Some(n) => out.push(n),
                None => break,
            }
        }
        Ok(out)
    }

    /// One record ending within `limit` (the enclosing record's end); `None` for the null record
    /// that closes a list.
    fn record(&self, pos: &mut usize, limit: usize, depth: usize) -> Result<Option<Record>> {
        let start = *pos;
        let end = Self::to_usize(self.offset(pos)?)?;
        let nprops = self.offset(pos)?;
        let plen = Self::to_usize(self.offset(pos)?)?;
        let nlen = usize::from(self.arr::<1>(pos)?[0]);
        if end == 0 && nprops == 0 && plen == 0 && nlen == 0 {
            return Ok(None);
        }
        // A child reaching past its parent would be parsed again once the parent's end is
        // restored.
        if end <= start || end > limit.min(self.d.len()) {
            return Err(malformed(KIND, format!("bad record end {end} at byte {start}")));
        }
        let name = String::from_utf8_lossy(self.take(pos, nlen)?).into_owned();
        let pstart = *pos;
        let pend =
            pstart.checked_add(plen).filter(|e| *e <= end).ok_or_else(|| malformed(KIND, format!("`{name}`: property list past the record")))?;
        // Every property is at least one byte, so a count above the list length is corrupt.
        if nprops > plen as u64 {
            return Err(malformed(KIND, format!("`{name}`: {nprops} properties in {plen} bytes")));
        }
        let mut props = Vec::with_capacity(nprops as usize);
        for _ in 0..nprops {
            props.push(self.prop(pos, pend)?);
        }
        *pos = pend;
        let children = if *pos < end { self.list(pos, end, depth + 1)? } else { vec![] };
        *pos = end;
        Ok(Some(Record { name, props, children }))
    }

    fn prop(&self, pos: &mut usize, pend: usize) -> Result<Prop> {
        let sub = Bin { d: self.d.get(..pend).ok_or_else(|| malformed(KIND, "truncated property"))?, wide: self.wide, budget: self.budget };
        let t = sub.arr::<1>(pos)?[0];
        Ok(match t {
            b'Y' => Prop::Int(i64::from(i16::from_le_bytes(sub.arr(pos)?))),
            b'C' => Prop::Int(i64::from(sub.arr::<1>(pos)?[0] != 0)),
            b'I' => Prop::Int(i64::from(i32::from_le_bytes(sub.arr(pos)?))),
            b'L' => Prop::Int(i64::from_le_bytes(sub.arr(pos)?)),
            b'F' => Prop::Float(f64::from(f32::from_le_bytes(sub.arr(pos)?))),
            b'D' => Prop::Float(f64::from_le_bytes(sub.arr(pos)?)),
            b'S' => {
                let n = sub.u32(pos)? as usize;
                Prop::Str(object_name(sub.take(pos, n)?))
            }
            b'R' => {
                let n = sub.u32(pos)? as usize;
                Prop::Bytes(sub.take(pos, n)?.to_vec())
            }
            b'f' | b'd' | b'l' | b'i' | b'b' => {
                let len = sub.u32(pos)? as usize;
                let enc = sub.u32(pos)?;
                let clen = sub.u32(pos)? as usize;
                let size = match t {
                    b'd' | b'l' => 8,
                    b'f' | b'i' => 4,
                    _ => 1,
                };
                let want = len.checked_mul(size).filter(|n| *n <= MAX_ARRAY_BYTES).ok_or_else(|| too_large(KIND, "array too large"))?;
                // Elements are kept as f64/i64 (booleans and 32-bit values widen), so the budget
                // is charged for what is kept, not for the bytes in the file.
                sub.budget.take(len.checked_mul(8), "an array")?;
                let data = sub.take(pos, clen)?;
                let inflated;
                let raw = match enc {
                    0 => data,
                    1 => {
                        // The status only: the error's Debug form prints what was decompressed.
                        inflated = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, want)
                            .map_err(|e| malformed(KIND, format!("array zlib: {:?}", e.status)))?;
                        inflated.as_slice()
                    }
                    e => return Err(malformed(KIND, format!("unknown array encoding {e}"))),
                };
                let raw = raw.get(..want).ok_or_else(|| malformed(KIND, "array shorter than its length"))?;
                let (c4, c8) = (raw.as_chunks::<4>().0, raw.as_chunks::<8>().0);
                match t {
                    b'f' => Prop::Floats(c4.iter().map(|c| f64::from(f32::from_le_bytes(*c))).collect()),
                    b'd' => Prop::Floats(c8.iter().map(|c| f64::from_le_bytes(*c)).collect()),
                    b'l' => Prop::Ints(c8.iter().map(|c| i64::from_le_bytes(*c)).collect()),
                    b'i' => Prop::Ints(c4.iter().map(|c| i64::from(i32::from_le_bytes(*c))).collect()),
                    _ => Prop::Ints(raw.iter().map(|b| i64::from(*b != 0)).collect()),
                }
            }
            t => return Err(malformed(KIND, format!("unknown property type 0x{t:02x}"))),
        })
    }
}

/// Binary object names are `Name\0\x01Class`; ASCII writes `Class::Name`.
fn object_name(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(b);
    match s.split_once("\0\x01") {
        Some((name, class)) => format!("{class}::{name}"),
        None => s.into_owned(),
    }
}

fn read_binary(bytes: &[u8], budget: &Budget) -> Result<(u32, Vec<Record>)> {
    let mut pos = BINARY_MAGIC.len();
    let version = Bin { d: bytes, wide: false, budget }.u32(&mut pos)?;
    let bin = Bin { d: bytes, wide: version >= 7500, budget };
    let mut records = vec![];
    while pos < bytes.len() {
        match bin.record(&mut pos, bytes.len(), 0) {
            Ok(Some(n)) => records.push(n),
            Ok(None) => break,
            // A damaged tail (footer, padding) after complete sections is ignored.
            Err(crate::Error::Malformed(_)) if records.iter().any(|n| n.name == "Objects") => break,
            Err(e) => return Err(e),
        }
    }
    Ok((version, records))
}

// ---------------------------------------------------------------------------------------------
// ASCII

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Key(String),
    Val(Prop),
    /// `*N` array length (the values follow in an `a:` child).
    Count,
    Comma,
    Open,
    Close,
}

fn tokenize(text: &str) -> Vec<Tok> {
    let mut out = vec![];
    let mut it = text.char_indices().peekable();
    while let Some(&(i, c)) = it.peek() {
        match c {
            c if c.is_whitespace() => {
                it.next();
            }
            ';' => {
                for (_, c) in it.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            ',' => {
                it.next();
                out.push(Tok::Comma);
            }
            '{' => {
                it.next();
                out.push(Tok::Open);
            }
            '}' => {
                it.next();
                out.push(Tok::Close);
            }
            '"' => {
                it.next();
                let start = i + 1;
                let mut end = text.len();
                for (j, c) in it.by_ref() {
                    if c == '"' {
                        end = j;
                        break;
                    }
                }
                out.push(Tok::Val(Prop::Str(text.get(start..end).unwrap_or("").replace("&quot;", "\""))));
            }
            _ => {
                let start = i;
                let mut end = text.len();
                while let Some(&(j, c)) = it.peek() {
                    if c.is_whitespace() || matches!(c, ',' | '{' | '}' | '"' | ';' | ':') {
                        end = j;
                        break;
                    }
                    it.next();
                }
                let word = text.get(start..end).unwrap_or("");
                if it.peek().is_some_and(|(_, c)| *c == ':') {
                    it.next();
                    out.push(Tok::Key(word.to_string()));
                } else if word.starts_with('*') {
                    out.push(Tok::Count);
                } else if let Ok(i) = word.parse::<i64>() {
                    out.push(Tok::Val(Prop::Int(i)));
                } else if let Ok(f) = word.parse::<f64>() {
                    out.push(Tok::Val(Prop::Float(f)));
                } else if !word.is_empty() {
                    out.push(Tok::Val(Prop::Str(word.to_string())));
                } else {
                    // A stray `:` with no key.
                    it.next();
                }
            }
        }
    }
    out
}

fn ascii_list(toks: &[Tok], i: &mut usize, depth: usize) -> Result<Vec<Record>> {
    if depth > MAX_DEPTH {
        return Err(too_large(KIND, "records nested too deeply"));
    }
    let mut out = vec![];
    while let Some(t) = toks.get(*i) {
        *i += 1;
        match t {
            Tok::Close if depth > 0 => return Ok(out),
            Tok::Key(name) => {
                let mut record = Record { name: name.clone(), ..Record::default() };
                loop {
                    match toks.get(*i) {
                        Some(Tok::Val(v)) => record.props.push(v.clone()),
                        Some(Tok::Count | Tok::Comma) => {}
                        _ => break,
                    }
                    *i += 1;
                }
                if toks.get(*i) == Some(&Tok::Open) {
                    *i += 1;
                    record.children = ascii_list(toks, i, depth + 1)?;
                }
                out.push(record);
            }
            _ => {}
        }
    }
    Ok(out)
}

fn read_ascii(text: &str) -> Result<(u32, Vec<Record>)> {
    let toks = tokenize(text);
    let mut i = 0;
    let records = ascii_list(&toks, &mut i, 0)?;
    let version = records
        .iter()
        .find(|n| n.name == "FBXHeaderExtension")
        .and_then(|h| h.child("FBXVersion"))
        .and_then(|v| v.props.first())
        .and_then(Prop::as_i64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);
    Ok((version, records))
}

// ---------------------------------------------------------------------------------------------
// Scene

/// An object's properties by name (`Properties70`: `P: name, type, label, flags, values…`; 6.x
/// `Properties60`: `Property: name, type, flags, values…`).
struct Props<'a>(HashMap<&'a str, &'a Record>);

impl<'a> Props<'a> {
    fn of(record: &'a Record) -> Props<'a> {
        let mut m = HashMap::new();
        for (block, item) in [("Properties70", "P"), ("Properties60", "Property")] {
            for e in record.children_named(block).flat_map(|p| p.children_named(item)) {
                if let Some(name) = e.str_at(0) {
                    m.entry(name).or_insert(e);
                }
            }
        }
        Props(m)
    }

    /// The numbers after the name: the strings between (type, label, flags) are not numbers.
    fn values(&self, name: &str) -> Vec<f64> {
        self.0.get(name).map(|e| e.props.iter().skip(1).filter_map(Prop::as_f64).collect()).unwrap_or_default()
    }

    fn f(&self, name: &str) -> Option<f64> {
        self.values(name).first().copied().filter(|v| v.is_finite())
    }

    fn v3(&self, name: &str, default: [f64; 3]) -> [f64; 3] {
        match self.values(name).as_slice() {
            [x, y, z, ..] if x.is_finite() && y.is_finite() && z.is_finite() => [*x, *y, *z],
            _ => default,
        }
    }
}

fn translate(t: [f64; 3]) -> Mat4 {
    let mut m = IDENTITY;
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m
}

fn scale(s: [f64; 3]) -> Mat4 {
    let mut m = IDENTITY;
    m[0] = s[0];
    m[5] = s[1];
    m[10] = s[2];
    m
}

/// A right-handed rotation of `deg` degrees about axis `k` (0 = X, 1 = Y, 2 = Z).
fn rotate(k: usize, deg: f64) -> Mat4 {
    let (s, c) = deg.to_radians().sin_cos();
    let (a, b) = ((k + 1) % 3, (k + 2) % 3);
    let mut m = IDENTITY;
    m[a * 4 + a] = c;
    m[b * 4 + a] = -s;
    m[a * 4 + b] = s;
    m[b * 4 + b] = c;
    m
}

fn transpose(m: &Mat4) -> Mat4 {
    let mut t = [0.0; 16];
    for (i, v) in t.iter_mut().enumerate() {
        *v = m[(i % 4) * 4 + i / 4];
    }
    t
}

/// The product of `ms`: the last applies first.
fn chain(ms: &[Mat4]) -> Mat4 {
    ms.iter().fold(IDENTITY, |a, b| mat_mul(&a, b))
}

/// An Euler rotation (degrees) in FBX rotation order (`eEulerXYZ` = 0 rotates about X first).
fn euler(d: [f64; 3], order: i64) -> Mat4 {
    let (x, y, z) = (rotate(0, d[0]), rotate(1, d[1]), rotate(2, d[2]));
    match order {
        1 => chain(&[y, z, x]),
        2 => chain(&[x, z, y]),
        3 => chain(&[z, x, y]),
        4 => chain(&[y, x, z]),
        5 => chain(&[x, y, z]),
        _ => chain(&[z, y, x]),
    }
}

/// A Model's local transform: `T · Roff · Rp · Rpre · R · Rpost⁻¹ · Rp⁻¹ · Soff · Sp · S · Sp⁻¹`.
fn model_local(p: &Props) -> Mat4 {
    let order = p.f("RotationOrder").map_or(0, |o| o as i64);
    let active = p.f("RotationActive").is_some_and(|a| a != 0.0);
    let (pre, post) =
        if active { (euler(p.v3("PreRotation", [0.0; 3]), 0), transpose(&euler(p.v3("PostRotation", [0.0; 3]), 0))) } else { (IDENTITY, IDENTITY) };
    let rp = p.v3("RotationPivot", [0.0; 3]);
    let sp = p.v3("ScalingPivot", [0.0; 3]);
    chain(&[
        translate(p.v3("Lcl Translation", [0.0; 3])),
        translate(p.v3("RotationOffset", [0.0; 3])),
        translate(rp),
        pre,
        euler(p.v3("Lcl Rotation", [0.0; 3]), order),
        post,
        translate(rp.map(|v| -v)),
        translate(p.v3("ScalingOffset", [0.0; 3])),
        translate(sp),
        scale(p.v3("Lcl Scaling", [1.0; 3])),
        translate(sp.map(|v| -v)),
    ])
}

/// The geometric transform of a Model (applies to its geometry, not to its children).
fn geometric(p: &Props) -> Option<Mat4> {
    let t = p.v3("GeometricTranslation", [0.0; 3]);
    let r = p.v3("GeometricRotation", [0.0; 3]);
    let s = p.v3("GeometricScaling", [1.0; 3]);
    if t == [0.0; 3] && r == [0.0; 3] && s == [1.0; 3] {
        return None;
    }
    Some(chain(&[translate(t), euler(r, 0), scale(s)]))
}

impl Document {
    /// The top-level record of this name.
    pub fn top(&self, name: &str) -> Option<&Record> {
        self.records.iter().find(|n| n.name == name)
    }

    fn settings(&self) -> Option<Props<'_>> {
        // 6.x writers put it among the objects.
        self.top("GlobalSettings").or_else(|| self.top("Objects")?.child("GlobalSettings")).map(Props::of)
    }

    /// The turn from the file's axis system (`GlobalSettings`) to +Y up, +Z front, +X right.
    /// The identity when the file does not say or what it says is not a rotation or mirror.
    pub fn axes_to_y_up(&self) -> Mat4 {
        let p = self.settings();
        let get = |name: &str, d: f64| p.as_ref().and_then(|p| p.f(name)).unwrap_or(d);
        let axis = |a: f64, s: f64| -> Option<[f64; 3]> {
            let mut v = [0.0; 3];
            *v.get_mut(usize::try_from(a as i64).ok()?)? = if s < 0.0 { -1.0 } else { 1.0 };
            Some(v)
        };
        let (Some(right), Some(up), Some(front)) = (
            axis(get("CoordAxis", 0.0), get("CoordAxisSign", 1.0)),
            axis(get("UpAxis", 1.0), get("UpAxisSign", 1.0)),
            axis(get("FrontAxis", 2.0), get("FrontAxisSign", 1.0)),
        ) else {
            return IDENTITY;
        };
        // Two axes on the same file axis: not a rotation.
        if !(0..3).all(|k| right[k] != 0.0 || up[k] != 0.0 || front[k] != 0.0) {
            return IDENTITY;
        }
        let mut m = IDENTITY;
        for (r, row) in [right, up, front].iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                m[c * 4 + r] = *v;
            }
        }
        m
    }

    /// Centimetres per file unit (`GlobalSettings` `UnitScaleFactor`), when the file says.
    pub fn unit_scale(&self) -> Option<f64> {
        self.settings()?.f("UnitScaleFactor").filter(|s| *s > 0.0)
    }

    /// The scene the records describe. `resolve` is asked for texture image files (see the
    /// module documentation).
    pub fn scene(&self, limits: &Limits, resolve: Resolver) -> Result<Scene> {
        if self.version != 0 && self.version < 6000 {
            return Err(crate::Error::Unsupported(format!(
                "FBX {}.{} (only FBX 6.x and 7.x files are read)",
                self.version / 1000,
                self.version % 1000 / 100
            )));
        }
        let objects = self.top("Objects").ok_or_else(|| malformed(KIND, "no Objects section"))?;
        let mut b = Build {
            limits,
            budget: Budget { left: Cell::new(self.left), limit: limits.max_data_bytes },
            resolve,
            by_key: objects.children.iter().filter_map(|o| Some((key(o.props.first())?, o))).collect(),
            children: HashMap::new(),
            scene: Scene::default(),
            material_of: HashMap::new(),
            texture_of: HashMap::new(),
            image_of: HashMap::new(),
            triangles: 0,
            skipped: 0,
        };
        // Connections: (child, parent, property) in file order, and the same by parent, so each
        // object finds its children without scanning them all.
        let mut links: Vec<(Key, Key)> = vec![];
        for e in self.top("Connections").into_iter().flat_map(|c| c.children.iter()).filter(|e| e.name == "C" || e.name == "Connect") {
            let (Some(child), Some(parent)) = (key(e.props.get(1)), key(e.props.get(2))) else { continue };
            links.push((child, parent));
            b.children.entry(parent).or_default().push((child, e.str_at(3)));
        }

        let models: Vec<(Key, &Record)> =
            objects.children.iter().filter(|o| o.name == "Model").filter_map(|o| Some((key(o.props.first())?, o))).collect();
        let mut node_of: HashMap<Key, usize> = HashMap::new();
        for (id, o) in &models {
            if node_of.contains_key(id) {
                continue;
            }
            b.node_room()?;
            node_of.insert(*id, b.scene.nodes.len());
            b.scene.nodes.push(Node {
                name: short_name(o.first_str().unwrap_or("")).to_string(),
                transform: model_local(&Props::of(o)),
                ..Node::default()
            });
        }

        // Hierarchy: the first Model→Model link is the parent; anything else is a root.
        let mut parent: HashMap<usize, usize> = HashMap::new();
        for (c, p) in &links {
            if let (Some(ci), Some(pi)) = (node_of.get(c), node_of.get(p))
                && ci != pi
            {
                parent.entry(*ci).or_insert(*pi);
            }
        }
        for i in 0..b.scene.nodes.len() {
            match parent.get(&i).and_then(|p| b.scene.nodes.get_mut(*p)) {
                Some(p) => p.children.push(i),
                None => b.scene.roots.push(i),
            }
        }

        // One mesh per Model with Mesh geometry (material slots are per Model). Models instancing
        // the same geometry with the same materials share one mesh rather than each building a
        // copy.
        let mut mesh_of: HashMap<(Key, Vec<usize>), usize> = HashMap::new();
        for (id, o) in &models {
            let Some(ni) = node_of.get(id).copied() else { continue };
            let Some((gid, geom)) = b.geometry_of(*id) else { continue };
            let mut mats: Vec<usize> = vec![];
            for (mid, _) in b.children_of(*id, "Material") {
                mats.extend(b.material(mid)?);
            }
            let name = b.scene.nodes.get(ni).map(|n| n.name.clone()).unwrap_or_default();
            let mesh_index = match mesh_of.get(&(gid, mats.clone())) {
                Some(i) => *i,
                None => {
                    let Some(mesh) = b.mesh(&name, geom, &mats)? else { continue };
                    b.budget.take(mesh_bytes(&mesh), &format!("mesh `{name}`"))?;
                    if b.scene.meshes.len() >= limits.max_meshes {
                        return Err(too_large(KIND, format!("more than {} meshes", limits.max_meshes)));
                    }
                    b.scene.meshes.push(mesh);
                    mesh_of.insert((gid, mats), b.scene.meshes.len() - 1);
                    b.scene.meshes.len() - 1
                }
            };
            match geometric(&Props::of(o)) {
                Some(transform) => {
                    b.node_room()?;
                    let gi = b.scene.nodes.len();
                    b.scene.nodes.push(Node { name: format!("{name} geometry"), transform, mesh: Some(mesh_index), ..Node::default() });
                    if let Some(n) = b.scene.nodes.get_mut(ni) {
                        n.children.push(gi);
                    }
                }
                None => {
                    if let Some(n) = b.scene.nodes.get_mut(ni) {
                        n.mesh = Some(mesh_index);
                    }
                }
            }
        }
        let mut scene = b.scene;
        count_warning(&mut scene.warnings, b.skipped, "polygons with fewer than three corners or a vertex that is not there were left out");
        Ok(scene)
    }
}

/// What connections name an object by: its id, or its `Class::Name`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key<'a> {
    Id(i64),
    Name(&'a str),
}

fn key(p: Option<&Prop>) -> Option<Key<'_>> {
    match p? {
        Prop::Int(i) => Some(Key::Id(*i)),
        Prop::Str(s) => Some(Key::Name(s)),
        _ => None,
    }
}

fn short_name(full: &str) -> &str {
    full.split_once("::").map_or(full, |(_, n)| n)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mapping {
    PolygonVertex,
    Vertex,
    Polygon,
    AllSame,
}

/// How a geometry layer element (normals, UVs) is indexed.
struct Layer {
    mapping: Mapping,
    index: Option<Vec<i64>>,
    /// Entries in the element's array.
    len: usize,
}

impl Layer {
    /// The layer and its flat array of `width` numbers per entry.
    fn of(geom: &Record, element: &str, data: &str, index: &[&str], width: usize) -> Option<(Layer, Vec<f64>)> {
        let e = geom.child(element)?;
        let mapping = match e.child("MappingInformationType").and_then(|m| m.str_at(0)).unwrap_or("ByPolygonVertex") {
            "ByPolygonVertex" => Mapping::PolygonVertex,
            "ByVertex" | "ByVertice" | "ByControlPoint" => Mapping::Vertex,
            "ByPolygon" => Mapping::Polygon,
            "AllSame" => Mapping::AllSame,
            _ => return None,
        };
        let reference = e.child("ReferenceInformationType").and_then(|m| m.str_at(0)).unwrap_or("Direct");
        let index = (reference != "Direct").then(|| index.iter().find_map(|n| e.child(n)).map(Record::ints)).flatten();
        let data = e.child(data)?.floats();
        Some((Layer { mapping, index, len: data.len() / width }, data))
    }

    /// The entry for corner `pv` of the geometry, at vertex `v` of polygon `poly`; [`NONE`] when
    /// the file has none there.
    fn at(layer: Option<&Layer>, pv: usize, v: usize, poly: usize) -> u32 {
        let entry = |l: &Layer| {
            let i = match l.mapping {
                Mapping::PolygonVertex => pv,
                Mapping::Vertex => v,
                Mapping::Polygon => poly,
                Mapping::AllSame => 0,
            };
            let j = match &l.index {
                Some(ix) => usize::try_from(*ix.get(i)?).ok()?,
                None => i,
            };
            u32::try_from(j).ok().filter(|_| j < l.len)
        };
        layer.and_then(entry).unwrap_or(NONE)
    }
}

fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// Lambert/Phong surface properties onto metallic-roughness.
fn material(name: &str, record: &Record) -> Material {
    let p = Props::of(record);
    let phong = record.child("ShadingModel").and_then(|s| s.str_at(0)).is_some_and(|s| s.eq_ignore_ascii_case("phong"));
    let factor = p.f("DiffuseFactor").unwrap_or(1.0);
    let d = p.v3("DiffuseColor", p.v3("Diffuse", [0.8; 3]));
    let alpha = match p.f("Opacity") {
        Some(o) => o,
        None => {
            let tc = p.v3("TransparentColor", [0.0; 3]);
            1.0 - (tc[0] + tc[1] + tc[2]) / 3.0 * p.f("TransparencyFactor").unwrap_or(0.0)
        }
    }
    .clamp(0.0, 1.0);
    let lin = |c: f64| srgb_to_linear((c * factor).clamp(0.0, 1.0) as f32);
    let e = p.v3("EmissiveColor", [0.0; 3]);
    let ef = p.f("EmissiveFactor").unwrap_or(1.0);
    let roughness = if phong {
        let ns = p.f("ShininessExponent").or_else(|| p.f("Shininess")).unwrap_or(20.0).max(0.0);
        (2.0 / (ns + 2.0)).sqrt() as f32
    } else {
        1.0
    };
    Material {
        name: name.to_string(),
        base_color: [lin(d[0]), lin(d[1]), lin(d[2]), alpha as f32],
        metallic: 0.0,
        roughness,
        emissive: e.map(|c| srgb_to_linear((c * ef).clamp(0.0, 1.0) as f32)),
        alpha_mode: if alpha < 0.999 { AlphaMode::Blend } else { AlphaMode::Opaque },
        ..Material::default()
    }
}

/// A scene under construction.
struct Build<'a, 'r> {
    limits: &'a Limits,
    budget: Budget,
    resolve: Resolver<'r>,
    by_key: HashMap<Key<'a>, &'a Record>,
    children: HashMap<Key<'a>, Vec<(Key<'a>, Option<&'a str>)>>,
    scene: Scene,
    material_of: HashMap<Key<'a>, usize>,
    texture_of: HashMap<Key<'a>, Option<usize>>,
    /// Images by the source used (the embedded Video, else the file name the resolver took, else
    /// the first name): Texture objects aliasing one image share it instead of each keeping a
    /// copy.
    image_of: HashMap<(Option<Key<'a>>, String), usize>,
    triangles: usize,
    skipped: usize,
}

impl<'a> Build<'a, '_> {
    /// The objects of `kind` connected under `id`, with the property each connects to.
    fn children_of(&self, id: Key<'a>, kind: &str) -> Vec<(Key<'a>, Option<&'a str>)> {
        let all = self.children.get(&id).map(Vec::as_slice).unwrap_or_default();
        all.iter().filter(|l| self.by_key.get(&l.0).is_some_and(|o| o.name == kind)).copied().collect()
    }

    fn node_room(&self) -> Result<()> {
        if self.scene.nodes.len() >= self.limits.max_nodes {
            return Err(too_large(KIND, format!("more than {} nodes", self.limits.max_nodes)));
        }
        Ok(())
    }

    /// The mesh geometry of Model `id`, and what it is shared by.
    fn geometry_of(&self, id: Key<'a>) -> Option<(Key<'a>, &'a Record)> {
        // 6.x: the Model holds its own mesh.
        if let Some(model) = self.by_key.get(&id).copied().filter(|o| o.child("Vertices").is_some()) {
            return Some((id, model));
        }
        let (gid, _) = self.children_of(id, "Geometry").into_iter().next()?;
        Some((gid, self.by_key.get(&gid).copied()?))
    }

    /// The image of Texture `tid`.
    fn texture(&mut self, tid: Key<'a>) -> Result<Option<usize>> {
        if let Some(t) = self.texture_of.get(&tid) {
            return Ok(*t);
        }
        let image = self.image(tid)?;
        self.texture_of.insert(tid, image);
        Ok(image)
    }

    fn image(&mut self, tid: Key<'a>) -> Result<Option<usize>> {
        let Some(tex) = self.by_key.get(&tid).copied() else { return Ok(None) };
        let files: Vec<String> = ["RelativeFilename", "FileName"].iter().filter_map(|k| Some(tex.child(k)?.str_at(0)?.replace('\\', "/"))).collect();
        for f in &files {
            if !f.is_empty() && !self.scene.external.contains(f) {
                self.scene.external.push(f.clone());
            }
        }
        let embedded = self.children_of(tid, "Video").into_iter().find_map(|(v, _)| {
            let content = self.by_key.get(&v)?.child("Content")?;
            let bytes = match content.props.first()? {
                Prop::Bytes(b) if !b.is_empty() => Some(b.clone()),
                Prop::Str(_) => {
                    let s: String = content.props.iter().filter_map(Prop::as_str).collect();
                    base64_decode(&s).filter(|b| !b.is_empty())
                }
                _ => None,
            }?;
            Some(((Some(v), String::new()), bytes))
        });
        let supplied = |resolve: &mut Resolver| {
            files.iter().find_map(|f| {
                let (name, bytes) = resolve(f).map(|b| (f.clone(), b)).or_else(|| {
                    let base = f.rsplit('/').next()?;
                    Some((base.to_string(), resolve(base)?))
                })?;
                Some(((None, name), bytes))
            })
        };
        let (source, data) = match embedded.or_else(|| supplied(&mut self.resolve)) {
            Some((source, bytes)) => (source, Some(bytes)),
            None => match files.first() {
                Some(f) => ((None, f.clone()), None),
                None => return Ok(None),
            },
        };
        if let Some(i) = self.image_of.get(&source) {
            return Ok(Some(*i));
        }
        self.budget.take(Some(data.as_ref().map_or(0, Vec::len)), "an image")?;
        if self.scene.images.len() >= self.limits.max_materials {
            return Err(too_large(KIND, format!("more than {} images", self.limits.max_materials)));
        }
        let uri = if source.0.is_some() { files.first().cloned() } else { Some(source.1.clone()) };
        self.scene.images.push(Image { name: short_name(tex.first_str().unwrap_or("")).to_string(), uri, mime_type: None, data });
        self.image_of.insert(source, self.scene.images.len() - 1);
        Ok(Some(self.scene.images.len() - 1))
    }

    /// The scene's material for Material object `mid`, made on first use.
    fn material(&mut self, mid: Key<'a>) -> Result<Option<usize>> {
        if let Some(m) = self.material_of.get(&mid) {
            return Ok(Some(*m));
        }
        let Some(record) = self.by_key.get(&mid).copied() else { return Ok(None) };
        let mut m = material(short_name(record.first_str().unwrap_or("")), record);
        for (tid, prop) in self.children_of(mid, "Texture") {
            let slot = match prop {
                Some("DiffuseColor") => &mut m.base_color_texture,
                Some("NormalMap") => &mut m.normal_texture,
                Some("EmissiveColor") => &mut m.emissive_texture,
                _ => continue,
            };
            if slot.is_none() {
                *slot = self.texture(tid)?.map(|image| TextureRef { image, ..TextureRef::default() });
            }
        }
        if m.emissive_texture.is_some() && m.emissive == [0.0; 3] {
            m.emissive = [1.0; 3];
        }
        if self.scene.materials.len() >= self.limits.max_materials {
            return Err(too_large(KIND, format!("more than {} materials", self.limits.max_materials)));
        }
        self.scene.materials.push(m);
        self.material_of.insert(mid, self.scene.materials.len() - 1);
        Ok(Some(self.scene.materials.len() - 1))
    }

    /// A Mesh geometry; `materials[k]` is the scene's material for the geometry's slot `k`.
    /// `None` when it has no polygon.
    fn mesh(&mut self, name: &str, geom: &Record, materials: &[usize]) -> Result<Option<Mesh>> {
        let limits = self.limits;
        let (Some(vertices), Some(pvi)) = (geom.child("Vertices"), geom.child("PolygonVertexIndex")) else { return Ok(None) };
        let pool = |what: &str, n: usize| {
            if n > limits.max_vertices { Err(too_large(KIND, format!("more than {} {what}", limits.max_vertices))) } else { Ok(()) }
        };
        let positions: Vec<[f64; 3]> = vertices.floats().as_chunks::<3>().0.iter().map(|c| c.map(finite)).collect();
        pool("vertices", positions.len())?;
        let normals = Layer::of(geom, "LayerElementNormal", "Normals", &["NormalsIndex", "NormalIndex"], 3);
        let uvs = Layer::of(geom, "LayerElementUV", "UV", &["UVIndex"], 2);
        let slots = geom.child("LayerElementMaterial").and_then(|e| {
            let mapping = e.child("MappingInformationType").and_then(|m| m.str_at(0)).unwrap_or("AllSame");
            Some((mapping == "ByPolygon", e.child("Materials")?.ints()))
        });
        let (normal_layer, uv_layer) = (normals.as_ref().map(|l| &l.0), uvs.as_ref().map(|l| &l.0));

        let mut parts: Vec<Part> = vec![];
        let mut part_of: HashMap<Option<usize>, usize> = HashMap::new();
        let mut face: Vec<Corner> = vec![];
        let (mut broken, mut poly) = (false, 0usize);
        for (pv, raw) in pvi.ints().into_iter().enumerate() {
            // The last corner of a polygon is stored as `-index - 1`.
            let (v, last) = if raw < 0 { (!raw, true) } else { (raw, false) };
            match u32::try_from(v).ok().filter(|v| (*v as usize) < positions.len()) {
                Some(p) => face.push(Corner {
                    position: p,
                    normal: Layer::at(normal_layer, pv, p as usize, poly),
                    uv: Layer::at(uv_layer, pv, p as usize, poly),
                }),
                None => broken = true,
            }
            if face.len() > limits.max_face_corners {
                return Err(too_large(KIND, format!("a polygon of more than {} corners", limits.max_face_corners)));
            }
            if !last {
                continue;
            }
            let (corners, p_index, was_broken) = (std::mem::take(&mut face), poly, std::mem::take(&mut broken));
            poly += 1;
            if was_broken || corners.len() < 3 {
                self.skipped += 1;
                continue;
            }
            self.triangles = self.triangles.saturating_add(corners.len() - 2);
            if self.triangles > limits.max_triangles {
                return Err(too_large(KIND, format!("more than {} triangles", limits.max_triangles)));
            }
            let slot = slots.as_ref().and_then(|(by_poly, ids)| {
                let k = if *by_poly { *ids.get(p_index)? } else { *ids.first()? };
                materials.get(usize::try_from(k).ok()?).copied()
            });
            let slot = slot.or_else(|| materials.first().copied());
            let at = match part_of.get(&slot) {
                Some(at) => *at,
                None => {
                    if parts.len() >= limits.max_parts {
                        return Err(too_large(KIND, format!("more than {} material slots in a mesh", limits.max_parts)));
                    }
                    parts.push(Part { material: slot, ..Part::default() });
                    part_of.insert(slot, parts.len() - 1);
                    parts.len() - 1
                }
            };
            let Some(part) = parts.get_mut(at) else { continue };
            part.face_sizes.push(corners.len() as u32);
            part.corners.extend(corners);
        }
        if parts.is_empty() {
            return Ok(None);
        }
        for p in &mut parts {
            if p.face_sizes.iter().all(|n| *n == 3) {
                p.face_sizes = vec![];
            }
        }
        let normals: Vec<[f64; 3]> = normals.map(|(_, d)| d.as_chunks::<3>().0.iter().map(|c| c.map(finite)).collect()).unwrap_or_default();
        pool("normals", normals.len())?;
        // FBX's v axis points up the image.
        let uvs: Option<Vec<[f32; 2]>> =
            uvs.map(|(_, d)| d.as_chunks::<2>().0.iter().map(|c| [finite(c[0]) as f32, 1.0 - finite(c[1]) as f32]).collect());
        pool("UVs", uvs.as_ref().map_or(0, Vec::len))?;
        Ok(Some(Mesh { name: name.to_string(), positions, normals, uv_sets: uvs.into_iter().collect(), parts, ..Mesh::default() }))
    }
}
