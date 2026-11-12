//! PLY (polygon file format), read.
//!
//! From the format's public description: a text header naming elements and their properties,
//! then the elements' rows in ASCII or binary (little- or big-endian). [`read`] takes the
//! `vertex` element (`x`, `y`, `z` of any numeric type; `nx`, `ny`, `nz`; `s`/`t`, `u`/`v` or
//! `texture_u`/`texture_v`; `red`, `green`, `blue` and `alpha`) and the `face` element's
//! `vertex_indices` list, polygons kept whole, with its `texcoord` list (a `u v` per corner,
//! which then is the UV set instead of the vertices' own). A `comment TextureFile <name>` line
//! gives the mesh a material whose base colour texture is that file: named in [`Image::uri`]
//! and [`Scene::external`], never opened. Every other element and property is passed over. A
//! file without faces is a point cloud: a mesh with positions and no parts.
//!
//! Positions are kept as stored, so the indices of a point cloud stay the file's; a face that
//! names a vertex that is not there, or one whose position is not finite, is left out and
//! counted in a warning.
//!
//! [`parse_header`] and [`Rows`] are public for callers that need more than a mesh from a PLY
//! file (every scalar property as a column, `edge` elements, thinning a large cloud): they
//! read the same header and rows with the same checks.

use crate::{Corner, Image, Limits, Material, Mesh, Part, Result, Scene, TextureRef, count_warning, malformed, too_large};

const KIND: &str = "PLY";
/// Largest header read (bytes).
pub const MAX_HEADER: usize = 1 << 20;
/// Most elements a header may declare.
pub const MAX_ELEMENTS: usize = 64;
/// Most properties one element may have.
pub const MAX_PROPERTIES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scalar {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Scalar {
    fn parse(s: &str) -> Option<Scalar> {
        Some(match s {
            "char" | "int8" => Scalar::I8,
            "uchar" | "uint8" => Scalar::U8,
            "short" | "int16" => Scalar::I16,
            "ushort" | "uint16" => Scalar::U16,
            "int" | "int32" => Scalar::I32,
            "uint" | "uint32" => Scalar::U32,
            "float" | "float32" => Scalar::F32,
            "double" | "float64" => Scalar::F64,
            _ => return None,
        })
    }

    /// Bytes of one value in a binary file.
    pub fn size(self) -> usize {
        match self {
            Scalar::I8 | Scalar::U8 => 1,
            Scalar::I16 | Scalar::U16 => 2,
            Scalar::I32 | Scalar::U32 | Scalar::F32 => 4,
            Scalar::F64 => 8,
        }
    }

    pub fn is_integer(self) -> bool {
        !matches!(self, Scalar::F32 | Scalar::F64)
    }

    /// The value at the start of `b`.
    pub fn read(self, b: &[u8], big: bool) -> Option<f64> {
        fn take<const N: usize>(b: &[u8], big: bool) -> Option<[u8; N]> {
            let mut a: [u8; N] = b.get(..N)?.try_into().ok()?;
            if big {
                a.reverse();
            }
            Some(a)
        }
        Some(match self {
            Scalar::I8 => f64::from(i8::from_le_bytes(take(b, big)?)),
            Scalar::U8 => f64::from(*b.first()?),
            Scalar::I16 => f64::from(i16::from_le_bytes(take(b, big)?)),
            Scalar::U16 => f64::from(u16::from_le_bytes(take(b, big)?)),
            Scalar::I32 => f64::from(i32::from_le_bytes(take(b, big)?)),
            Scalar::U32 => f64::from(u32::from_le_bytes(take(b, big)?)),
            Scalar::F32 => f64::from(f32::from_le_bytes(take(b, big)?)),
            Scalar::F64 => f64::from_le_bytes(take(b, big)?),
        })
    }

    /// A colour channel of this type as 0..1: integers by their range, floats as they are.
    pub fn unit(self, v: f64) -> f32 {
        let v = match self {
            Scalar::F32 | Scalar::F64 => v,
            Scalar::U8 | Scalar::I8 => v / 255.0,
            Scalar::U16 | Scalar::I16 => v / 65_535.0,
            Scalar::U32 | Scalar::I32 => v / 4_294_967_295.0,
        };
        if v.is_finite() { v.clamp(0.0, 1.0) as f32 } else { 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Scalar(Scalar),
    /// The count's type and the items' type.
    List(Scalar, Scalar),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub name: String,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub name: String,
    /// Rows the header declares; not yet checked against the file.
    pub count: u64,
    pub properties: Vec<Property>,
}

impl Element {
    /// Bytes per row in a binary file, when every property is a scalar.
    pub fn row_size(&self) -> Option<usize> {
        self.properties.iter().try_fold(0usize, |n, p| match p.kind {
            Kind::Scalar(s) => n.checked_add(s.size()),
            Kind::List(..) => None,
        })
    }

    /// The first scalar property with one of `names` (ASCII case ignored), and its type.
    pub fn find(&self, names: &[&str]) -> Option<(usize, Scalar)> {
        names.iter().find_map(|n| {
            self.properties.iter().enumerate().find_map(|(i, p)| match p.kind {
                Kind::Scalar(s) if p.name.eq_ignore_ascii_case(n) => Some((i, s)),
                _ => None,
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Encoding {
    Ascii,
    Little,
    Big,
}

/// What the header says, and where the rows start.
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub encoding: Encoding,
    pub elements: Vec<Element>,
    /// Offset of the first row.
    pub body: usize,
    /// The text of the `comment` lines.
    pub comments: Vec<String>,
}

impl Header {
    /// Rows the header declares for the element `name` (0 when it has none).
    pub fn count(&self, name: &str) -> u64 {
        self.elements.iter().find(|e| e.name == name).map_or(0, |e| e.count)
    }
}

/// Does `bytes` start like a PLY file?
pub fn is_ply(bytes: &[u8]) -> bool {
    bytes.starts_with(b"ply") && bytes.get(3).is_some_and(|b| matches!(b, b'\n' | b'\r'))
}

fn property(e: &mut Element, it: &mut std::str::SplitWhitespace<'_>) -> Result<()> {
    if e.properties.len() >= MAX_PROPERTIES {
        return Err(too_large(KIND, format!("more than {MAX_PROPERTIES} properties in an element")));
    }
    let first = it.next().unwrap_or("");
    let kind = if first == "list" {
        let count = it.next().and_then(Scalar::parse).ok_or_else(|| malformed(KIND, "a list property without a count type"))?;
        let item = it.next().and_then(Scalar::parse).ok_or_else(|| malformed(KIND, "a list property without an item type"))?;
        if !count.is_integer() {
            return Err(malformed(KIND, "a list's count must be an integer"));
        }
        Kind::List(count, item)
    } else {
        Kind::Scalar(Scalar::parse(first).ok_or_else(|| malformed(KIND, format!("unknown property type `{}`", short(first))))?)
    };
    let name = it.next().ok_or_else(|| malformed(KIND, "a property without a name"))?.to_string();
    e.properties.push(Property { name, kind });
    Ok(())
}

fn short(s: &str) -> String {
    s.chars().take(32).collect()
}

/// Read the header: the encoding, the elements with their properties, and where the rows start.
pub fn parse_header(bytes: &[u8]) -> Result<Header> {
    if !is_ply(bytes) {
        return Err(malformed(KIND, "not a PLY file (it must start with `ply`)"));
    }
    let head = bytes.get(..bytes.len().min(MAX_HEADER)).unwrap_or(bytes);
    // `end_header` at the start of a line, so a comment that mentions it does not end the header.
    let end = head
        .windows(11)
        .position(|w| matches!(w.first(), Some(b'\n' | b'\r')) && w.get(1..) == Some(b"end_header"))
        .map(|p| p + 1)
        .ok_or_else(|| malformed(KIND, "no `end_header` in the first megabyte"))?;
    // The rows start after the end of that line (`\n`, `\r\n` or a lone `\r`).
    let mut body = end + 10;
    while head.get(body).is_some_and(|b| *b == b' ' || *b == b'\t') {
        body += 1;
    }
    match (head.get(body), head.get(body + 1)) {
        (Some(b'\r'), Some(b'\n')) => body += 2,
        (Some(b'\n' | b'\r'), _) => body += 1,
        (None, _) => {}
        _ => return Err(malformed(KIND, "`end_header` is not on a line of its own")),
    }
    let text = String::from_utf8_lossy(head.get(..end).unwrap_or_default());
    let mut encoding = None;
    let mut elements: Vec<Element> = Vec::new();
    let mut comments = Vec::new();
    for line in text.split(['\n', '\r']).skip(1) {
        if let Some(c) = line.strip_prefix("comment ") {
            comments.push(c.trim().to_string());
            continue;
        }
        let mut it = line.split_whitespace();
        match it.next() {
            Some("format") => {
                encoding = Some(match it.next() {
                    Some("ascii") => Encoding::Ascii,
                    Some("binary_little_endian") => Encoding::Little,
                    Some("binary_big_endian") => Encoding::Big,
                    other => return Err(malformed(KIND, format!("unknown format `{}`", short(other.unwrap_or(""))))),
                });
            }
            Some("element") => {
                let name = it.next().ok_or_else(|| malformed(KIND, "an element without a name"))?.to_string();
                let count = it
                    .next()
                    .and_then(|c| c.parse::<u64>().ok())
                    .ok_or_else(|| malformed(KIND, format!("element `{}` has no count", short(&name))))?;
                if elements.len() >= MAX_ELEMENTS {
                    return Err(too_large(KIND, format!("more than {MAX_ELEMENTS} elements")));
                }
                elements.push(Element { name, count, properties: Vec::new() });
            }
            Some("property") => property(elements.last_mut().ok_or_else(|| malformed(KIND, "a property before any element"))?, &mut it)?,
            // `comment`, `obj_info` and blank lines.
            _ => {}
        }
    }
    let encoding = encoding.ok_or_else(|| malformed(KIND, "no `format` line"))?;
    Ok(Header { encoding, elements, body, comments })
}

fn space(b: &u8) -> bool {
    b.is_ascii_whitespace()
}

/// The values of the rows, read one after another in the header's order.
pub enum Rows<'a> {
    Binary { b: &'a [u8], pos: usize, big: bool },
    Ascii { tokens: std::slice::Split<'a, u8, fn(&u8) -> bool>, left: usize },
}

impl<'a> Rows<'a> {
    pub fn new(h: &Header, bytes: &'a [u8]) -> Rows<'a> {
        let b = bytes.get(h.body..).unwrap_or_default();
        match h.encoding {
            Encoding::Ascii => Rows::Ascii { tokens: b.split(space as fn(&u8) -> bool), left: b.len() },
            Encoding::Little => Rows::Binary { b, pos: 0, big: false },
            Encoding::Big => Rows::Binary { b, pos: 0, big: true },
        }
    }

    /// Bytes not yet read. Every row takes at least one, so an element that declares more
    /// rows than this is damaged.
    pub fn remaining(&self) -> usize {
        match self {
            Rows::Binary { b, pos, .. } => b.len().saturating_sub(*pos),
            Rows::Ascii { left, .. } => *left,
        }
    }

    /// The next value, of type `t`.
    pub fn value(&mut self, t: Scalar) -> Result<f64> {
        match self {
            Rows::Binary { b, pos, big } => {
                let v = b.get(*pos..).and_then(|r| t.read(r, *big)).ok_or_else(|| malformed(KIND, "the file ends early"))?;
                *pos += t.size();
                Ok(v)
            }
            Rows::Ascii { tokens, left } => loop {
                let tok = tokens.next().ok_or_else(|| malformed(KIND, "the file ends early"))?;
                *left = left.saturating_sub(tok.len() + 1);
                if tok.is_empty() {
                    continue;
                }
                let s = std::str::from_utf8(tok).map_err(|_| malformed(KIND, "a value is not a number"))?;
                // Integers written as decimals (`255.0`) are taken too.
                return s.parse::<f64>().map_err(|_| malformed(KIND, format!("`{}` is not a number", short(s))));
            },
        }
    }

    /// Pass over `n` values of type `t`.
    pub fn skip(&mut self, t: Scalar, n: usize) -> Result<()> {
        match self {
            Rows::Binary { b, pos, .. } => {
                *pos = n
                    .checked_mul(t.size())
                    .and_then(|k| pos.checked_add(k))
                    .filter(|p| *p <= b.len())
                    .ok_or_else(|| malformed(KIND, "the file ends early"))?;
                Ok(())
            }
            Rows::Ascii { .. } => (0..n).try_for_each(|_| self.value(t).map(|_| ())),
        }
    }

    /// A list's length, of type `t`, at most `max`.
    pub fn count(&mut self, t: Scalar, max: usize) -> Result<usize> {
        let n = self.value(t)?;
        if !(n >= 0.0 && n <= max as f64 && n.fract() == 0.0) {
            return Err(too_large(KIND, format!("a list of more than {max} items (or a damaged length)")));
        }
        Ok(n as usize)
    }

    /// Checks that the rows of `e` can be in what is left of the file, before any is read: a
    /// row takes at least a byte, and exactly [`Element::row_size`] in a binary file.
    pub fn check(&self, e: &Element) -> Result<()> {
        let per_row = match self {
            Rows::Binary { .. } => e.row_size().unwrap_or(1).max(1),
            Rows::Ascii { .. } => 1,
        };
        match e.count.checked_mul(per_row as u64) {
            Some(n) if n <= self.remaining() as u64 => Ok(()),
            _ => Err(malformed(KIND, format!("the file is too short for its {} `{}` rows", e.count, short(&e.name)))),
        }
    }

    /// Pass over all rows of `e`.
    pub fn skip_element(&mut self, e: &Element, max_list: usize) -> Result<()> {
        self.check(e)?;
        if let (Rows::Binary { pos, .. }, Some(size)) = (&mut *self, e.row_size()) {
            // `check` has shown that the block fits.
            *pos = pos.saturating_add((e.count as usize).saturating_mul(size));
            return Ok(());
        }
        for _ in 0..e.count {
            for p in &e.properties {
                match p.kind {
                    Kind::Scalar(t) => self.skip(t, 1)?,
                    Kind::List(c, t) => {
                        let n = self.count(c, max_list)?;
                        self.skip(t, n)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Where the vertex element keeps what a mesh takes.
struct VertexColumns {
    xyz: [usize; 3],
    normal: Option<[usize; 3]>,
    uv: Option<[usize; 2]>,
    rgb: Option<[(usize, Scalar); 3]>,
    alpha: Option<(usize, Scalar)>,
}

impl VertexColumns {
    fn of(e: &Element) -> Result<VertexColumns> {
        let at = |names: &[&str]| e.find(names).map(|(i, _)| i);
        let three = |a: &[&str], b: &[&str], c: &[&str]| Some([at(a)?, at(b)?, at(c)?]);
        let xyz = three(&["x"], &["y"], &["z"]).ok_or_else(|| malformed(KIND, "the vertices have no `x`, `y` and `z`"))?;
        let uv = (|| Some([at(&["s", "u", "texture_u"])?, at(&["t", "v", "texture_v"])?]))();
        let rgb =
            (|| Some([e.find(&["red", "r", "diffuse_red"])?, e.find(&["green", "g", "diffuse_green"])?, e.find(&["blue", "b", "diffuse_blue"])?]))();
        Ok(VertexColumns { xyz, normal: three(&["nx"], &["ny"], &["nz"]), uv, rgb, alpha: e.find(&["alpha", "a"]) })
    }
}

fn vertices(e: &Element, rows: &mut Rows<'_>, mesh: &mut Mesh, limits: &Limits) -> Result<()> {
    let w = VertexColumns::of(e)?;
    if e.count > limits.max_vertices as u64 {
        return Err(too_large(KIND, format!("{} vertices (limit {})", e.count, limits.max_vertices)));
    }
    rows.check(e)?;
    let mut uvs = Vec::new();
    let mut slots = vec![0.0f64; e.properties.len()];
    for _ in 0..e.count {
        for (p, slot) in e.properties.iter().zip(&mut slots) {
            match p.kind {
                Kind::Scalar(t) => *slot = rows.value(t)?,
                Kind::List(c, t) => {
                    let n = rows.count(c, limits.max_face_corners)?;
                    rows.skip(t, n)?;
                }
            }
        }
        let v = |i: usize| slots.get(i).copied().unwrap_or(0.0);
        mesh.positions.push(w.xyz.map(v));
        if let Some(n) = w.normal {
            mesh.normals.push(n.map(v));
        }
        if let Some([s, t]) = w.uv {
            // PLY texture coordinates have their origin at the bottom left.
            uvs.push([v(s) as f32, 1.0 - v(t) as f32]);
        }
        if let Some(rgb) = w.rgb {
            let c = rgb.map(|(i, t)| t.unit(v(i)));
            mesh.colors.push([c[0], c[1], c[2], w.alpha.map_or(1.0, |(i, t)| t.unit(v(i)))]);
        }
    }
    if w.uv.is_some() {
        mesh.uv_sets.push(uvs);
    }
    Ok(())
}

struct Faces {
    part: Part,
    /// A UV per face corner, from the faces' `texcoord` lists.
    uvs: Vec<[f32; 2]>,
    triangles: usize,
    bad: usize,
}

fn faces(e: &Element, list: usize, rows: &mut Rows<'_>, vertex_count: u64, out: &mut Faces, limits: &Limits) -> Result<()> {
    rows.check(e)?;
    let texcoord = e.properties.iter().position(|p| p.name == "texcoord" && matches!(p.kind, Kind::List(..)));
    let (mut face, mut coords): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    for _ in 0..e.count {
        coords.clear();
        for (i, p) in e.properties.iter().enumerate() {
            match p.kind {
                Kind::Scalar(t) => rows.skip(t, 1)?,
                Kind::List(c, t) if i == list => {
                    let n = rows.count(c, limits.max_face_corners)?;
                    face.clear();
                    for _ in 0..n {
                        face.push(rows.value(t)?);
                    }
                }
                // `u v` per corner.
                Kind::List(c, t) if Some(i) == texcoord => {
                    for _ in 0..rows.count(c, limits.max_face_corners.saturating_mul(2))? {
                        coords.push(rows.value(t)?);
                    }
                }
                Kind::List(c, t) => {
                    let n = rows.count(c, limits.max_face_corners)?;
                    rows.skip(t, n)?;
                }
            }
        }
        let corners: Option<Vec<Corner>> = face
            .iter()
            .map(|v| (*v >= 0.0 && *v < vertex_count as f64 && v.fract() == 0.0).then(|| u32::try_from(*v as u64).ok().map(Corner::at)).flatten())
            .collect();
        let Some(mut corners) = corners.filter(|c| c.len() >= 3) else {
            out.bad += 1;
            continue;
        };
        // Coordinates that do not match the corners are passed over.
        if coords.len() == corners.len() * 2 {
            if out.uvs.len().saturating_add(corners.len()) > limits.max_vertices {
                return Err(too_large(KIND, format!("more than {} texture coordinates", limits.max_vertices)));
            }
            for (corner, uv) in corners.iter_mut().zip(coords.as_chunks::<2>().0) {
                corner.uv = out.uvs.len() as u32;
                // The origin is at the bottom left.
                out.uvs.push([uv[0] as f32, 1.0 - uv[1] as f32]);
            }
        }
        out.triangles = out.triangles.saturating_add(corners.len() - 2);
        if out.triangles > limits.max_triangles {
            return Err(too_large(KIND, format!("more than {} triangles", limits.max_triangles)));
        }
        out.part.face_sizes.push(corners.len() as u32);
        out.part.corners.extend(corners);
    }
    Ok(())
}

/// Read a PLY file into a scene of one mesh (unnamed: PLY has no names). The elements may come
/// in any order.
pub fn read(bytes: &[u8], limits: &Limits) -> Result<Scene> {
    let h = parse_header(bytes)?;
    let vertex_count = h.elements.iter().find(|e| e.name == "vertex").ok_or_else(|| malformed(KIND, "no `vertex` element"))?.count;
    let mut rows = Rows::new(&h, bytes);
    let mut mesh = Mesh::default();
    let mut out = Faces { part: Part::default(), uvs: Vec::new(), triangles: 0, bad: 0 };
    let (mut seen_vertices, mut seen_faces) = (false, false);
    for e in &h.elements {
        let list = e.properties.iter().position(|p| matches!(p.name.as_str(), "vertex_indices" | "vertex_index") && matches!(p.kind, Kind::List(..)));
        match (e.name.as_str(), list) {
            ("vertex", _) if !seen_vertices => {
                seen_vertices = true;
                vertices(e, &mut rows, &mut mesh, limits)?;
            }
            ("face", Some(list)) if !seen_faces => {
                seen_faces = true;
                faces(e, list, &mut rows, vertex_count, &mut out, limits)?;
            }
            _ => rows.skip_element(e, limits.max_face_corners)?,
        }
        // Nothing after the vertices and faces is wanted.
        if seen_vertices && (seen_faces || !h.elements.iter().any(|e| e.name == "face")) {
            break;
        }
    }
    // Faces that name a vertex whose position is not a number are left out.
    let finite = |c: &Corner| mesh.positions.get(c.position as usize).is_some_and(|p| p.iter().all(|x| x.is_finite()));
    let mut part = out.part;
    if !mesh.positions.iter().flatten().all(|x| x.is_finite()) {
        let mut kept = Part::default();
        for f in part.faces() {
            if f.iter().all(finite) {
                kept.face_sizes.push(f.len() as u32);
                kept.corners.extend(f);
            } else {
                out.bad += 1;
            }
        }
        part = kept;
    }
    if !mesh.normals.is_empty() {
        part.corners.iter_mut().for_each(|c| c.normal = c.position);
    }
    if !out.uvs.is_empty() {
        // Per-corner coordinates replace the vertices' own.
        mesh.uv_sets = vec![std::mem::take(&mut out.uvs)];
    } else if !mesh.uv_sets.is_empty() {
        part.corners.iter_mut().for_each(|c| c.uv = c.position);
    }
    if part.face_sizes.iter().all(|n| *n == 3) {
        part.face_sizes.clear();
    }
    // `comment TextureFile <name>` (MeshLab and others): the image the UVs are for.
    let texture = h.comments.iter().find_map(|c| {
        let (key, name) = c.split_once(char::is_whitespace)?;
        (key.eq_ignore_ascii_case("TextureFile") && !name.trim().is_empty()).then(|| name.trim().to_string())
    });
    let textured = texture.is_some() && !part.corners.is_empty();
    if textured {
        part.material = Some(0);
    }
    if !part.corners.is_empty() {
        mesh.parts.push(part);
    }
    let mut scene = Scene::from_meshes(vec![mesh], Vec::new());
    if let Some(name) = texture.filter(|_| textured) {
        let texture = TextureRef { image: 0, ..TextureRef::default() };
        scene.materials.push(Material { base_color_texture: Some(texture), ..Material::plain("", [1.0; 4]) });
        scene.images.push(Image { uri: Some(name.clone()), ..Image::default() });
        scene.external.push(name);
    }
    count_warning(&mut scene.warnings, out.bad, "face(s) could not be read and were left out");
    Ok(scene)
}
