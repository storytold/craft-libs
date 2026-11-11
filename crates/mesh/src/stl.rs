//! STL (stereolithography) meshes, binary and ASCII, read and written.
//!
//! From the public description of the format (3D Systems' StereoLithography Interface
//! Specification, 1989, and its widely documented colour extensions):
//!
//! - Binary: an 80-byte header, a little-endian `u32` facet count, then 50 bytes per facet: a
//!   normal and three vertices (12 `f32`) and a 16-bit attribute word. Bytes after the last
//!   facet are ignored. A count of 0 over a file that holds facets (some writers leave it
//!   unset) is read for the facets that are there, with a warning; a count larger than the
//!   file holds is a truncated file and an error.
//! - ASCII: `solid name`, then `facet normal nx ny nz` / `outer loop` / `vertex x y z`… /
//!   `endloop` / `endfacet`, closed by `endsolid`; several solids may follow each other.
//!   Loops with more than three vertices are fan-triangulated.
//! - Colour (binary only): with `COLOR=r g b a` (four bytes) in the header (Materialise Magics),
//!   that is the part's colour and a facet whose attribute has bit 15 clear has its own colour
//!   (red in bits 0–4, green 5–9, blue 10–14); otherwise (VisCAM / SolidView) a facet with bit 15
//!   set has its own colour (blue in bits 0–4, green 5–9, red 10–14). Each colour becomes a
//!   plain material named `#RRGGBB` and a [`Part`] of the mesh; uncoloured files get neither.
//!
//! Detection: a file whose size is exactly what its binary header says is binary (many binary
//! headers begin with `solid`); otherwise a file that starts with `solid` and has a `facet` or
//! `endsolid` is ASCII; otherwise it is binary.
//!
//! STL has no units, no UVs and no shared vertices: every facet keeps its own three positions.
//! Its normal is the face normal of its counter-clockwise winding; the stored normal is used
//! only when the triangle is degenerate (stored normals are often zero or wrong). Facets with a
//! non-finite coordinate are left out with a warning.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::{
    Corner, Error, Limits, Material, Mesh, Part, Result, Scene, count_warning, face_normal, is_mirror, malformed, srgb_to_linear, too_large,
    transform_point,
};

const KIND: &str = "STL";
/// Header (80) plus facet count (4).
const HEADER: usize = 84;
/// Bytes per binary facet.
const FACET: usize = 50;

/// Does `bytes` look like an ASCII STL (`solid` first, and a `facet` or `endsolid` in the first
/// 512 bytes)? Binary STL has no signature to test for.
pub fn is_ascii(bytes: &[u8]) -> bool {
    let head = bytes.get(..bytes.len().min(512)).unwrap_or(&[]);
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start();
    t.get(..5).is_some_and(|k| k.eq_ignore_ascii_case("solid")) && (t.contains("facet") || t.contains("endsolid"))
}

/// Read an STL file (binary or ASCII, detected from the contents) into a scene of one mesh.
pub fn read(bytes: &[u8], limits: &Limits) -> Result<Scene> {
    let count = bytes.get(80..HEADER).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(|b| u32::from_le_bytes(b) as usize);
    let needed = count.and_then(|n| n.checked_mul(FACET)).and_then(|n| n.checked_add(HEADER));
    if needed != Some(bytes.len()) && is_ascii(bytes) {
        return ascii(bytes, limits);
    }
    match count {
        Some(count) => binary(bytes, count, limits),
        None => Err(malformed(KIND, "neither a binary file (shorter than its 84-byte header) nor an ASCII one (no `solid`)")),
    }
}

/// Facets grouped into parts by colour, in order of first use.
struct Builder<'a> {
    mesh: Mesh,
    colors: Vec<Option<[u8; 3]>>,
    slots: HashMap<Option<[u8; 3]>, usize>,
    facets: usize,
    skipped: usize,
    limits: &'a Limits,
}

impl<'a> Builder<'a> {
    fn new(limits: &'a Limits) -> Self {
        Builder { mesh: Mesh::default(), colors: Vec::new(), slots: HashMap::new(), facets: 0, skipped: 0, limits }
    }

    /// Add one triangle; `stored` is the file's facet normal.
    fn tri(&mut self, color: Option<[u8; 3]>, stored: [f64; 3], v: [[f64; 3]; 3]) -> Result<()> {
        if v.iter().flatten().any(|c| !c.is_finite()) {
            self.skipped += 1;
            return Ok(());
        }
        if self.facets >= self.limits.max_triangles {
            return Err(too_large(KIND, format!("more than {} triangles", self.limits.max_triangles)));
        }
        if self.mesh.positions.len().saturating_add(3) > self.limits.max_vertices {
            return Err(too_large(KIND, format!("more than {} vertices", self.limits.max_vertices)));
        }
        let unit = |n: [f64; 3]| {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            (len.is_finite() && len > 0.0).then(|| n.map(|x| x / len))
        };
        let n = face_normal(v).or_else(|| unit(stored)).unwrap_or([0.0, 0.0, 1.0]);
        let slot = *self.slots.entry(color).or_insert_with(|| {
            self.mesh.parts.push(Part::default());
            self.colors.push(color);
            self.mesh.parts.len() - 1
        });
        let (Some(part), Ok(base), Ok(normal)) =
            (self.mesh.parts.get_mut(slot), u32::try_from(self.mesh.positions.len()), u32::try_from(self.mesh.normals.len()))
        else {
            return Err(too_large(KIND, "more vertices than a 32-bit index holds"));
        };
        self.mesh.normals.push(n);
        for (k, p) in (0u32..).zip(v) {
            self.mesh.positions.push(p);
            part.corners.push(Corner { position: base + k, normal, uv: crate::NONE });
        }
        self.facets += 1;
        Ok(())
    }

    fn finish(mut self, name: String, mut warnings: Vec<String>) -> Result<Scene> {
        count_warning(&mut warnings, self.skipped, "facet(s) with a non-finite coordinate were left out");
        if self.facets == 0 {
            return Err(malformed(KIND, "no facets"));
        }
        let mut materials = Vec::new();
        let coloured = self.colors.iter().any(Option::is_some);
        for (part, color) in self.mesh.parts.iter_mut().zip(&self.colors) {
            let material = match color {
                Some(c) => {
                    let lin = c.map(|x| srgb_to_linear(f32::from(x) / 255.0));
                    Material::plain(format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]), [lin[0], lin[1], lin[2], 1.0])
                }
                // Uncoloured facets next to coloured ones: a neutral grey of their own.
                None if coloured => Material::plain("STL", [0.6, 0.6, 0.6, 1.0]),
                None => continue,
            };
            part.material = Some(materials.len());
            materials.push(material);
        }
        self.mesh.name = name;
        let mut scene = Scene::from_meshes(vec![self.mesh], materials);
        scene.warnings = warnings;
        Ok(scene)
    }
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    b.get(at..at + 4).and_then(|s| <[u8; 4]>::try_from(s).ok()).map_or(f32::NAN, f32::from_le_bytes)
}

/// A 5-bit channel expanded to 8 bits.
fn five(x: u16) -> u8 {
    let x = (x & 31) as u8;
    (x << 3) | (x >> 2)
}

/// The Magics `COLOR=` part colour in the header (RGB; alpha is ignored).
fn header_color(header: &[u8]) -> Option<[u8; 3]> {
    let at = header.windows(6).position(|w| w == b"COLOR=")?;
    let c = header.get(at + 6..at + 10)?;
    Some([*c.first()?, *c.get(1)?, *c.get(2)?])
}

fn binary(bytes: &[u8], count: usize, limits: &Limits) -> Result<Scene> {
    let header = bytes.get(..80).ok_or_else(|| malformed(KIND, "header truncated"))?;
    let magics = header_color(header);
    let body = bytes.get(HEADER..).unwrap_or(&[]);
    let room = body.len() / FACET;
    let mut warnings = Vec::new();
    if count > room {
        return Err(malformed(KIND, format!("the header counts {count} facets, the file holds {room} (truncated)")));
    }
    // Some writers leave the count unset.
    let take = if count == 0 { room } else { count };
    if count == 0 && room > 0 {
        warnings.push(format!("the header counts no facets, the file holds {room}"));
    }
    if take > limits.max_triangles {
        return Err(too_large(KIND, format!("{take} triangles (limit {})", limits.max_triangles)));
    }
    let mut b = Builder::new(limits);
    for f in body.as_chunks::<FACET>().0.iter().take(take) {
        let n = [f32_at(f, 0), f32_at(f, 4), f32_at(f, 8)];
        let v = [0usize, 1, 2].map(|k| [f32_at(f, 12 + 12 * k), f32_at(f, 16 + 12 * k), f32_at(f, 20 + 12 * k)]);
        let attr = f.get(48..50).and_then(|a| <[u8; 2]>::try_from(a).ok()).map_or(0, u16::from_le_bytes);
        let color = match magics {
            Some(part) => Some(if attr & 0x8000 == 0 { [five(attr), five(attr >> 5), five(attr >> 10)] } else { part }),
            None => (attr & 0x8000 != 0).then(|| [five(attr >> 10), five(attr >> 5), five(attr)]),
        };
        b.tri(color, n.map(f64::from), v.map(|p| p.map(f64::from)))?;
    }
    let text = String::from_utf8_lossy(header);
    let name = text.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    b.finish(name.strip_prefix("solid").unwrap_or(name).trim().to_string(), warnings)
}

/// Three numbers from the rest of an ASCII line.
fn three<'a>(it: &mut impl Iterator<Item = &'a str>) -> Option<[f64; 3]> {
    Some([it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?])
}

fn ascii(bytes: &[u8], limits: &Limits) -> Result<Scene> {
    let text = String::from_utf8_lossy(bytes);
    let mut b = Builder::new(limits);
    let mut name: Option<String> = None;
    let mut normal = [0.0f64; 3];
    let mut loop_v: Vec<[f64; 3]> = vec![];
    for (ln, line) in text.lines().enumerate() {
        let line = line.trim();
        let mut it = line.split_whitespace();
        let Some(key) = it.next() else { continue };
        match key.to_ascii_lowercase().as_str() {
            "solid" if name.is_none() => name = Some(line.get(5..).unwrap_or("").trim().to_string()),
            "facet" => {
                // `facet normal nx ny nz`
                normal = it.next().and_then(|_| three(&mut it)).unwrap_or([0.0; 3]);
                loop_v.clear();
            }
            "vertex" => {
                if loop_v.len() >= limits.max_face_corners {
                    return Err(too_large(KIND, format!("line {}: a loop of more than {} vertices", ln + 1, limits.max_face_corners)));
                }
                loop_v.push(three(&mut it).ok_or_else(|| malformed(KIND, format!("line {}: expected three numbers", ln + 1)))?);
            }
            "endloop" | "endfacet" => {
                if let Some((&first, rest)) = loop_v.split_first() {
                    for w in rest.windows(2) {
                        if let [a, c] = w {
                            b.tri(None, normal, [first, *a, *c])?;
                        }
                    }
                }
                loop_v.clear();
            }
            _ => {}
        }
    }
    b.finish(name.unwrap_or_default(), Vec::new())
}

/// Every triangle of the scene in world space, with its face normal.
fn triangles(scene: &Scene, limits: &Limits, mut each: impl FnMut([f64; 3], [[f64; 3]; 3])) -> Result<()> {
    for (index, world) in scene.instances(limits)? {
        let Some(mesh) = scene.meshes.get(index) else { continue };
        for t in mesh.parts.iter().flat_map(Part::triangles) {
            let p = t.map(|c| mesh.positions.get(c.position as usize).copied());
            let [Some(a), Some(b), Some(c)] = p else { return Err(malformed(KIND, "a corner index is out of range")) };
            // A mirrored placement turns the winding; turn it back so the facet faces out.
            let v = if is_mirror(&world) { [a, c, b] } else { [a, b, c] }.map(|p| transform_point(&world, p));
            each(face_normal(v).unwrap_or([0.0; 3]), v);
        }
    }
    Ok(())
}

/// Binary STL of every mesh of the scene, placed by its nodes, as one solid. `name` goes in the
/// header (cut to 80 bytes; a leading `solid` would make readers take the file for ASCII, so it
/// is written after a prefix). Colours are not written.
pub fn write_binary(scene: &Scene, name: &str, limits: &Limits) -> Result<Vec<u8>> {
    let mut header = format!("STL {name}").into_bytes();
    header.resize(80, b' ');
    let mut out = header;
    out.extend_from_slice(&[0; 4]);
    let mut n = 0u64;
    triangles(scene, limits, |normal, v| {
        n += 1;
        for x in std::iter::once(normal).chain(v).flatten() {
            out.extend_from_slice(&(x as f32).to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    })?;
    let count = u32::try_from(n).map_err(|_| Error::TooLarge("STL: more triangles than the 32-bit count holds".into()))?;
    if let Some(slot) = out.get_mut(80..HEADER) {
        slot.copy_from_slice(&count.to_le_bytes());
    }
    Ok(out)
}

/// ASCII STL of every mesh of the scene, placed by its nodes, as one solid named `name` (its
/// graphic ASCII characters).
pub fn write_ascii(scene: &Scene, name: &str, limits: &Limits) -> Result<String> {
    let name: String = name.chars().filter(|c| c.is_ascii_graphic()).collect();
    let mut s = format!("solid {name}\n");
    triangles(scene, limits, |n, v| {
        let _ = write!(s, "  facet normal {:e} {:e} {:e}\n    outer loop\n", n[0], n[1], n[2]);
        for p in v {
            let _ = writeln!(s, "      vertex {:e} {:e} {:e}", p[0], p[1], p[2]);
        }
        s += "    endloop\n  endfacet\n";
    })?;
    let _ = writeln!(s, "endsolid {name}");
    Ok(s)
}
