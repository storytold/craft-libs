//! Smart objects: the placed-layer blocks (`SoLd`, `SoLE`, `PlLd`), the files they show
//! (`lnk2`, `lnkD`, `lnk3`, `lnkE`), and the geometry of a placement.
//!
//! A placed layer names a linked file by its unique id and says where the file's corners sit in
//! the document: a quad, which is a projective map of the content rectangle when it is not a
//! parallelogram, and optionally a warp (`warp` in `SoLd`): one of the named styles with bend
//! and horizontal / vertical distortion, or a custom envelope whose quilt mesh is a grid of
//! bicubic Bézier patches.
//!
//! The custom mesh is evaluated exactly (bicubic Bézier patches over `meshPoints`, split by
//! `quiltSliceX` / `quiltSliceY`). Photoshop does not publish the curves of its named styles;
//! the styles here are clean-room geometric definitions of what each name describes (Arc,
//! Arch, Bulge, Shell, Flag, Wave, Fish, Rise, Fisheye, Inflate, Squeeze, Twist), matched by
//! behaviour, not numerically.
//!
//! The blocks themselves stay raw in the model ([`crate::TaggedBlock`]) and write back
//! unchanged; these are read-only views. Came from EffectCraft's `effectcraft-psd`.

use std::ops::Range;

use crate::descriptor::{Descriptor, UnicodeString, Value};
use crate::error::{PsdError, Result};
use crate::file::PsdFile;
use crate::io::{Reader, read_pascal};
use crate::layer::LayerRecord;
use crate::tagged::TaggedBlock;

/// The most pixels a baked smart object may have: 2^24 (256 MiB as float RGBA). The size comes
/// from the file's transform, which is untrusted.
pub const MAX_BAKE_PIXELS: u64 = 1 << 24;

/// A placed layer's warp.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Warp {
    /// `warpNone`, `warpArc`, `warpArcLower`, `warpArcUpper`, `warpArch`, `warpBulge`,
    /// `warpShellLower`, `warpShellUpper`, `warpFlag`, `warpWave`, `warpFish`, `warpRise`,
    /// `warpFisheye`, `warpInflate`, `warpSqueeze`, `warpTwist`, `warpCustom`.
    pub style: String,
    /// Bend, −100..100 (%).
    pub bend: f64,
    /// Horizontal distortion, −100..100 (%).
    pub horizontal: f64,
    /// Vertical distortion, −100..100 (%).
    pub vertical: f64,
    /// The style is applied rotated a quarter turn (`warpRotate` = vertical).
    pub rotate_vertical: bool,
    /// The warped area in content pixels: left, top, right, bottom.
    pub bounds: [f64; 4],
    /// Custom envelope: patch columns.
    pub cols: usize,
    /// Custom envelope: patch rows.
    pub rows: usize,
    /// Custom envelope: the (3·cols+1)×(3·rows+1) control points (content pixels, row-major).
    pub mesh: Vec<[f64; 2]>,
}

/// The eight numbers of a transform entry (`Trnf`, `nonAffineTransform`), when it has exactly eight.
fn eight(d: &Descriptor, key: &str) -> Option<[f64; 8]> {
    d.numbers(key).try_into().ok()
}

/// The corner quad (top-left, top-right, bottom-right, bottom-left) from x0 y0 … x3 y3.
fn quad_from(t: [f64; 8]) -> [[f64; 2]; 4] {
    [[t[0], t[1]], [t[2], t[3]], [t[4], t[5]], [t[6], t[7]]]
}

impl Warp {
    /// Reads a `warp` descriptor; `None` for no warp (the identity, or a custom warp without a
    /// usable mesh).
    pub fn from_descriptor(d: &Descriptor) -> Option<Warp> {
        let style = d.enumerated("warpStyle").map_or_else(|| "warpNone".to_string(), |id| String::from_utf8_lossy(id.as_bytes()).into_owned());
        let side = |b: &Descriptor, k: &str| b.number(k).unwrap_or(0.0);
        let bounds = d.object("bounds").map_or([0.0; 4], |b| [side(b, "Left"), side(b, "Top "), side(b, "Rght"), side(b, "Btom")]);
        let mut w = Warp {
            bend: d.number("warpValue").unwrap_or(0.0),
            horizontal: d.number("warpPerspective").unwrap_or(0.0),
            vertical: d.number("warpPerspectiveOther").unwrap_or(0.0),
            rotate_vertical: d.enumerated("warpRotate").is_some_and(|id| id.is("Vrtc")),
            bounds,
            cols: 1,
            rows: 1,
            mesh: vec![],
            style,
        };
        if let Some(env) = d.object("customEnvelopeWarp") {
            let cols = env.numbers("quiltSliceX").len().saturating_sub(1).max(1);
            let rows = env.numbers("quiltSliceY").len().saturating_sub(1).max(1);
            if let Some(Value::ObjectArray(pts)) = env.get("meshPoints") {
                let xs = pts.body.numbers("Hrzn");
                let ys = pts.body.numbers("Vrtc");
                let n = cols.checked_mul(3).zip(rows.checked_mul(3)).and_then(|(c, r)| (c + 1).checked_mul(r + 1));
                if n.is_some_and(|n| xs.len() == n && ys.len() == n) {
                    w.cols = cols;
                    w.rows = rows;
                    w.mesh = xs.into_iter().zip(ys).map(|(x, y)| [x, y]).collect();
                }
            }
        }
        let identity = w.style == "warpNone" || (w.style != "warpCustom" && w.bend == 0.0 && w.horizontal == 0.0 && w.vertical == 0.0);
        let custom_ok = w.style != "warpCustom" || !w.mesh.is_empty();
        (!identity && custom_ok).then_some(w)
    }

    /// The warp of a content point (content pixels → content pixels).
    pub fn apply(&self, p: [f64; 2], w: f64, h: f64) -> [f64; 2] {
        let [l, t, r, b] = if self.bounds[2] > self.bounds[0] && self.bounds[3] > self.bounds[1] { self.bounds } else { [0.0, 0.0, w, h] };
        let (bw, bh) = (r - l, b - t);
        let (u, v) = ((p[0] - l) / bw, (p[1] - t) / bh);
        if self.style == "warpCustom" {
            // A mesh that does not match cols × rows (public fields) leaves the point where it is.
            return self.mesh_point(u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)).unwrap_or(p);
        }
        // Styles work in a centred frame (pixels), optionally rotated a quarter turn.
        let (x, y, fw, fh) = if self.rotate_vertical { (bh * (v - 0.5), -bw * (u - 0.5), bh, bw) } else { (bw * (u - 0.5), bh * (v - 0.5), bw, bh) };
        let [mut x2, mut y2] = style_map(&self.style, self.bend / 100.0, x, y, fw, fh);
        // Distortions: a perspective-like taper across the width (horizontal) or height.
        let (hd, vd) =
            if self.rotate_vertical { (self.vertical / 100.0, self.horizontal / 100.0) } else { (self.horizontal / 100.0, self.vertical / 100.0) };
        let s = (x2 / (fw / 2.0)).clamp(-1.5, 1.5);
        let tt = (y2 / (fh / 2.0)).clamp(-1.5, 1.5);
        y2 *= 1.0 + hd * s;
        x2 *= 1.0 + vd * tt;
        let (ox, oy) = if self.rotate_vertical { (-y2, x2) } else { (x2, y2) };
        [l + bw / 2.0 + ox, t + bh / 2.0 + oy]
    }

    /// A point of the custom quilt mesh at (u, v) ∈ [0, 1]².
    fn mesh_point(&self, u: f64, v: f64) -> Option<[f64; 2]> {
        let (cols, rows) = (self.cols, self.rows);
        if cols == 0 || rows == 0 {
            return None;
        }
        let pu = (u * cols as f64).min(cols as f64 - 1e-9);
        let pv = (v * rows as f64).min(rows as f64 - 1e-9);
        let (ci, cj) = (pu.floor() as usize, pv.floor() as usize);
        let (fu, fv) = (pu - ci as f64, pv - cj as f64);
        let stride = cols.checked_mul(3)?.checked_add(1)?;
        let bern = |t: f64| {
            let s = 1.0 - t;
            [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t]
        };
        let (bu, bv) = (bern(fu), bern(fv));
        let mut out = [0.0; 2];
        for (j, bvj) in bv.iter().enumerate() {
            for (i, bui) in bu.iter().enumerate() {
                let idx = cj.saturating_mul(3).saturating_add(j).saturating_mul(stride).saturating_add(ci.saturating_mul(3)).saturating_add(i);
                let q = self.mesh.get(idx)?;
                let k = bui * bvj;
                out[0] += q[0] * k;
                out[1] += q[1] * k;
            }
        }
        Some(out)
    }
}

/// A named style in a centred frame `fw`×`fh` (y down), bend `b` ∈ [−1, 1].
fn style_map(style: &str, b: f64, x: f64, y: f64, fw: f64, fh: f64) -> [f64; 2] {
    use std::f64::consts::PI;
    let (hw, hh) = (fw / 2.0, fh / 2.0);
    let s = x / hw; // −1..1 across
    let t = y / hh; // −1..1 down
    // Concentric arcs: the midline bends through an angle of b·π, rows keep their distance.
    let arc = |x: f64, y: f64| -> [f64; 2] {
        let theta = b * PI;
        if theta.abs() < 1e-9 {
            return [x, y];
        }
        let r = fw / theta;
        let phi = x / hw * theta / 2.0;
        let rr = r - y;
        // Arc centre below the shape for upward bends; keep the midline's centre fixed.
        [rr * phi.sin(), r - rr * phi.cos()]
    };
    let bump = 1.0 - s * s;
    match style {
        "warpArc" => arc(x, y),
        "warpArcLower" => {
            let a = arc(x, y);
            let k = (t + 1.0) / 2.0;
            [x + (a[0] - x) * k, y + (a[1] - y) * k]
        }
        "warpArcUpper" => {
            let a = arc(x, y);
            let k = (1.0 - t) / 2.0;
            [x + (a[0] - x) * k, y + (a[1] - y) * k]
        }
        "warpArch" => [x, y - b * hh * bump],
        "warpBulge" => [x, y + t * b * hh * bump],
        "warpShellLower" => [x * (1.0 - b * 0.25 * (1.0 - t) / 2.0), y + (t + 1.0) / 2.0 * b * hh * bump],
        "warpShellUpper" => [x * (1.0 - b * 0.25 * (t + 1.0) / 2.0), y - (1.0 - t) / 2.0 * b * hh * bump],
        "warpFlag" => [x, y - b * hh * 0.5 * (PI * s).sin()],
        "warpWave" => [x + b * hw * 0.1 * (PI * t).sin(), y - b * hh * 0.5 * (PI * s).sin() * (1.0 - 0.5 * t)],
        "warpFish" => [x, y + t * b * hh * (PI * (s + 1.0) / 2.0).sin() * (1.0 - 0.5 * (s + 1.0) / 2.0)],
        "warpRise" => [x, y - b * hh * (PI * s / 2.0).sin()],
        "warpFisheye" | "warpInflate" => {
            let k = if style == "warpFisheye" { (1.0 - (s * s + t * t) / 2.0).max(0.0) } else { 1.0 };
            [x * (1.0 + b * 0.5 * k * (1.0 - t * t)), y * (1.0 + b * 0.5 * k * bump)]
        }
        "warpSqueeze" => [x * (1.0 - b * 0.5 * (1.0 - t * t)), y * (1.0 + b * 0.5 * bump)],
        "warpTwist" => {
            let r = ((s * s + t * t) / 2.0).sqrt().min(1.0);
            let a = b * PI / 2.0 * (1.0 - r);
            let (sn, cs) = a.sin_cos();
            [x * cs - y * sn, x * sn + y * cs]
        }
        _ => [x, y],
    }
}

/// The projective map taking the unit square's corners (0,0), (1,0), (1,1), (0,1) to `q`
/// (Heckbert's square-to-quad), as a 3×3 matrix applied to (u, v, 1).
pub fn square_to_quad(q: [[f64; 2]; 4]) -> [[f64; 3]; 3] {
    let [p0, p1, p2, p3] = q;
    let sx = p0[0] - p1[0] + p2[0] - p3[0];
    let sy = p0[1] - p1[1] + p2[1] - p3[1];
    if sx.abs() < 1e-12 && sy.abs() < 1e-12 {
        return [[p1[0] - p0[0], p2[0] - p1[0], p0[0]], [p1[1] - p0[1], p2[1] - p1[1], p0[1]], [0.0, 0.0, 1.0]];
    }
    let (dx1, dx2) = (p1[0] - p2[0], p3[0] - p2[0]);
    let (dy1, dy2) = (p1[1] - p2[1], p3[1] - p2[1]);
    let den = dx1 * dy2 - dx2 * dy1;
    let g = (sx * dy2 - dx2 * sy) / den;
    let h = (dx1 * sy - sx * dy1) / den;
    [[p1[0] - p0[0] + g * p1[0], p3[0] - p0[0] + h * p3[0], p0[0]], [p1[1] - p0[1] + g * p1[1], p3[1] - p0[1] + h * p3[1], p0[1]], [g, h, 1.0]]
}

/// The baked size of placed content with document-space bounds `bb` at `k` pixels per document pixel.
pub(crate) fn size_for(bb: [f64; 4], k: f64) -> (u32, u32) {
    (((bb[2] - bb[0]) * k).ceil().clamp(1.0, 16384.0) as u32, ((bb[3] - bb[1]) * k).ceil().clamp(1.0, 16384.0) as u32)
}

/// A placed layer (smart object): `SoLd` / `SoLE` (placed layer data) or the older `PlLd`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SmartObject {
    /// Unique id of the linked file ([`LinkedFile::uuid`]) the layer shows.
    pub uuid: String,
    /// Where the content's corners sit in document pixels: top-left, top-right, bottom-right,
    /// bottom-left. The non-affine transform when the block has one.
    pub quad: [[f64; 2]; 4],
    /// The content's size in pixels (`Sz  `), when recorded.
    pub size: Option<[f64; 2]>,
    /// The placed layer's warp (`warp`), when it has one.
    pub warp: Option<Warp>,
}

impl SmartObject {
    /// Reads the data of a `SoLd` / `SoLE` block.
    pub fn from_placed_layer_data(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let _kind = r.array::<4>()?;
        let _version = r.u32()?;
        let _descriptor_version = r.u32()?;
        let d = Descriptor::read(&mut r)?;
        let uuid = d.text("Idnt").unwrap_or_default();
        let t = eight(&d, "Trnf").unwrap_or([0.0; 8]);
        // The non-affine transform when present (perspective), else the transform.
        let t = eight(&d, "nonAffineTransform").unwrap_or(t);
        let size = d.object("Sz  ").and_then(|s| Some([s.number("Wdth")?, s.number("Hght")?]));
        let warp = d.object("warp").and_then(Warp::from_descriptor);
        Ok(SmartObject { uuid, quad: quad_from(t), size, warp })
    }

    /// Reads the data of a `PlLd` block. A transform that is not finite is an error.
    pub fn from_placed_layer(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let _kind = r.array::<4>()?;
        let _version = r.u32()?;
        let uuid = String::from_utf8_lossy(&read_pascal(&mut r, 1)?).into_owned();
        // Page, total pages, anti-alias policy, layer type.
        r.skip(16)?;
        let mut t = [0.0; 8];
        for v in &mut t {
            *v = r.f64()?;
            if !v.is_finite() {
                return Err(PsdError::invalid("PlLd: transform is not finite"));
            }
        }
        Ok(SmartObject { uuid, quad: quad_from(t), size: None, warp: None })
    }

    /// The quad as an affine placement of a `w`×`h` content rectangle: (centre, scale x, scale
    /// y, rotation in degrees). Perspective and skew are dropped.
    pub fn placement(&self, w: f64, h: f64) -> ([f64; 2], f64, f64, f64) {
        let q = self.quad;
        let centre = [(q[0][0] + q[1][0] + q[2][0] + q[3][0]) / 4.0, (q[0][1] + q[1][1] + q[2][1] + q[3][1]) / 4.0];
        let top = [q[1][0] - q[0][0], q[1][1] - q[0][1]];
        let left = [q[3][0] - q[0][0], q[3][1] - q[0][1]];
        let sx = top[0].hypot(top[1]) / w.max(1e-9);
        let sy = left[0].hypot(left[1]) / h.max(1e-9);
        // Mirrored content (left edge turning the other way) keeps a negative y scale.
        let cross = top[0] * left[1] - top[1] * left[0];
        (centre, sx, if cross < 0.0 { -sy } else { sy }, top[1].atan2(top[0]).to_degrees())
    }

    /// Whether the corner quad is a parallelogram (an affine placement).
    pub fn is_affine(&self) -> bool {
        let q = self.quad;
        let ex = q[0][0] + q[2][0] - q[1][0] - q[3][0];
        let ey = q[0][1] + q[2][1] - q[1][1] - q[3][1];
        ex.abs() < 1e-3 && ey.abs() < 1e-3
    }

    /// Whether placing needs baking (a perspective quad or a warp).
    pub fn needs_bake(&self) -> bool {
        !self.is_affine() || self.warp.is_some()
    }

    /// A content point (pixels of a `w`×`h` content) in document pixels: the warp, then the quad.
    pub fn map(&self, p: [f64; 2], w: f64, h: f64) -> [f64; 2] {
        self.map_with(&square_to_quad(self.quad), p, w, h)
    }

    /// [`Self::map`] with the quad's projective matrix already solved (`square_to_quad(self.quad)`).
    pub(crate) fn map_with(&self, m: &[[f64; 3]; 3], p: [f64; 2], w: f64, h: f64) -> [f64; 2] {
        let p = match &self.warp {
            Some(wp) => wp.apply(p, w, h),
            None => p,
        };
        let (u, v) = (p[0] / w.max(1e-9), p[1] / h.max(1e-9));
        let z = m[2][0] * u + m[2][1] * v + m[2][2];
        let z = if z.abs() < 1e-12 { 1e-12 } else { z };
        [(m[0][0] * u + m[0][1] * v + m[0][2]) / z, (m[1][0] * u + m[1][1] * v + m[1][2]) / z]
    }

    /// The document-space bounds (x0, y0, x1, y1) of the placed (warped) content.
    pub fn placed_bounds(&self, w: f64, h: f64) -> [f64; 4] {
        self.bounds_with(&square_to_quad(self.quad), w, h)
    }

    pub(crate) fn bounds_with(&self, m: &[[f64; 3]; 3], w: f64, h: f64) -> [f64; 4] {
        let n = 32;
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for j in 0..=n {
            for i in 0..=n {
                let p = self.map_with(m, [i as f64 / n as f64 * w, j as f64 / n as f64 * h], w, h);
                b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
            }
        }
        [b[0].floor(), b[1].floor(), b[2].ceil(), b[3].ceil()]
    }

    /// The baked size for a `w`×`h` content at `k` pixels per document pixel.
    pub fn placed_size(&self, w: f64, h: f64, k: f64) -> (u32, u32) {
        size_for(self.placed_bounds(w, h), k)
    }

    /// The scale and size to bake a `w`×`h` content at, wanting `k` pixels per document pixel but
    /// never more than [`MAX_BAKE_PIXELS`] pixels, nor more than 4× the `doc` size across either
    /// way: a quad corner far outside the document lowers `k` (possibly below 1) instead of asking
    /// for a gigantic image. Returns the scale actually used and the size in pixels.
    pub fn bake_scale(&self, w: f64, h: f64, k: f64, doc: (u32, u32)) -> (f64, (u32, u32)) {
        let bb = self.placed_bounds(w, h);
        let (bw, bh) = ((bb[2] - bb[0]).max(1.0), (bb[3] - bb[1]).max(1.0));
        if !(bw.is_finite() && bh.is_finite() && k.is_finite() && k > 0.0) {
            return (1.0, (1, 1));
        }
        let (max_w, max_h) = ((f64::from(doc.0) * 4.0).max(1.0), (f64::from(doc.1) * 4.0).max(1.0));
        let capped = k.min(max_w / bw).min(max_h / bh).min((MAX_BAKE_PIXELS as f64 / (bw * bh)).sqrt());
        // Rounding up could step over a cap, so a lowered scale rounds down.
        let side = |len: f64| if capped < k { (len * capped).floor() } else { (len * capped).ceil() }.clamp(1.0, 16384.0) as u32;
        (capped, (side(bw), side(bh)))
    }
}

impl LayerRecord {
    /// The layer's smart object placement: `SoLd` or `SoLE` when one reads, else `PlLd`. A
    /// malformed block counts as absent.
    pub fn smart_object(&self) -> Option<SmartObject> {
        let read = |key: &[u8; 4], f: fn(&[u8]) -> Result<SmartObject>| self.block(key).and_then(|b| f(&b.data).ok());
        read(b"SoLd", SmartObject::from_placed_layer_data)
            .or_else(|| read(b"SoLE", SmartObject::from_placed_layer_data))
            .or_else(|| read(b"PlLd", SmartObject::from_placed_layer))
    }
}

/// A file embedded in (or linked from) the document: one record of a linked layer block
/// (`lnk2`, `lnkD`, `lnk3`, `lnkE`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkedFile {
    /// `liFD` (embedded data), `liFE` (external file) or `liFA` (alias).
    pub kind: [u8; 4],
    /// Unique id, which a [`SmartObject::uuid`] names.
    pub uuid: String,
    /// Original file name.
    pub name: String,
    /// File type code (`8BPS`, `PNGf`, `JPEG`…), may be blank.
    pub file_type: String,
    /// Where the embedded bytes are in the block's data (absent for externally linked files).
    pub data: Option<Range<usize>>,
}

/// `true` for the keys of the linked layer blocks.
pub fn is_linked_layer_key(key: &[u8; 4]) -> bool {
    matches!(key, b"lnk2" | b"lnkD" | b"lnk3" | b"lnkE")
}

/// The records of a linked layer block's data (spec: "Linked Layer"): length, kind, version,
/// unique id, original file name, type, creator, data length, optional descriptors, then the
/// data. A damaged record ends the list; the records before it are returned.
pub fn linked_files(data: &[u8]) -> Vec<LinkedFile> {
    let mut out = Vec::new();
    let mut r = Reader::new(data);
    while r.remaining() >= 8 {
        let Ok(len) = r.u64() else { break };
        let start = r.pos();
        let Ok(item) = r.bytes_u64(len) else { break };
        match linked_file(item, start) {
            Ok(f) => out.push(f),
            Err(_) => break,
        }
        // Records are padded to a multiple of 4.
        let pad = ((4 - len % 4) % 4).min(r.remaining() as u64) as usize;
        if r.skip(pad).is_err() {
            break;
        }
    }
    out
}

/// One record; `at` is where it starts in the block's data.
fn linked_file(item: &[u8], at: usize) -> Result<LinkedFile> {
    let mut r = Reader::new(item);
    let kind = r.array::<4>()?;
    let version = r.u32()?;
    let uuid = String::from_utf8_lossy(&read_pascal(&mut r, 1)?).into_owned();
    let name = UnicodeString::read(&mut r)?.to_string_lossy();
    let file_type = String::from_utf8_lossy(&r.array::<4>()?).trim().to_string();
    let _creator = r.array::<4>()?;
    let data_len = r.u64()?;
    if r.u8()? != 0 {
        let _version = r.u32()?;
        Descriptor::read(&mut r)?;
    }
    let mut out = LinkedFile { kind, uuid, name, file_type, data: None };
    let embedded = |r: &mut Reader<'_>| -> Result<Range<usize>> {
        let s = at + r.pos();
        let n = r.bytes_u64(data_len)?.len();
        Ok(s..s + n)
    };
    match &kind {
        b"liFD" => out.data = Some(embedded(&mut r)?),
        b"liFE" => {
            let _version = r.u32()?;
            Descriptor::read(&mut r)?;
            if version > 3 {
                // Year, month / day / hour / minute, seconds.
                r.skip(4 + 4 + 8)?;
            }
            let _file_size = r.u64()?;
            if version > 2 && data_len > 0 && r.remaining() as u64 >= data_len {
                out.data = Some(embedded(&mut r)?);
            }
        }
        _ => {}
    }
    Ok(out)
}

impl PsdFile {
    /// Every linked file of the document's linked layer blocks, each with the block that holds
    /// it ([`LinkedFile::data`] is a range of that block's data).
    pub fn linked_files(&self) -> Vec<(&TaggedBlock, LinkedFile)> {
        self.global_blocks
            .iter()
            .filter(|b| is_linked_layer_key(&b.key))
            .flat_map(|b| linked_files(&b.data).into_iter().map(move |f| (b, f)))
            .collect()
    }

    /// The embedded bytes of the linked file `uuid`, when the document embeds it.
    pub fn linked_data(&self, uuid: &str) -> Option<&[u8]> {
        self.linked_files().into_iter().find_map(|(b, f)| if f.uuid == uuid { b.data.get(f.data?) } else { None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{Id, ObjectArray};
    use crate::io::{WriteExt, write_pascal};

    fn so(quad: [[f64; 2]; 4], warp: Option<Warp>) -> SmartObject {
        SmartObject { uuid: "x".into(), quad, size: None, warp }
    }

    fn list(v: &[f64]) -> Value {
        Value::List(v.iter().map(|&x| Value::Double(x)).collect())
    }

    /// The data of a `SoLd` block placing `uuid` at `quad`.
    fn placed_layer_data(uuid: &str, quad: [f64; 8], extra: impl FnOnce(Descriptor) -> Descriptor) -> Vec<u8> {
        let d = Descriptor::new("null").with("Idnt", Value::Text(UnicodeString::new_nul(uuid))).with("Trnf", list(&quad));
        let mut out = b"soLD".to_vec();
        out.put_u32(4);
        out.put_u32(16);
        extra(d).write(&mut out);
        out
    }

    /// One linked layer record, with its length field and padding.
    fn linked_record(kind: &[u8; 4], uuid: &str, name: &str, file_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut item = kind.to_vec();
        item.put_u32(2);
        write_pascal(&mut item, uuid.as_bytes(), 1);
        UnicodeString::new(name).write(&mut item);
        item.put(file_type);
        item.put(b"    ");
        item.put_u64(data.len() as u64);
        item.put_u8(0);
        if kind == b"liFD" {
            item.put(data);
        } else {
            item.put_u32(16);
            Descriptor::new("ExternalFileLink").write(&mut item);
            item.put_u64(0);
        }
        let mut out = Vec::new();
        out.put_u64(item.len() as u64);
        out.put(&item);
        out.resize(out.len().div_ceil(4) * 4, 0);
        out
    }

    #[test]
    fn placed_layer_data_gives_the_id_quad_size_and_warp() {
        let quad = [20.0, 4.0, 40.0, 4.0, 40.0, 44.0, 20.0, 44.0];
        let data = placed_layer_data("uuid-1", quad, |d| {
            let size = Descriptor::new("Pnt ").with("Wdth", Value::Double(20.0)).with("Hght", Value::Double(10.0));
            let bounds = ["Top ", "Left", "Btom", "Rght"]
                .iter()
                .zip([0.0, 0.0, 10.0, 20.0])
                .fold(Descriptor::new("classFloatRect"), |b, (k, v)| b.with(k, Value::UnitFloat { unit: *b"#Pxl", value: v }));
            let warp = Descriptor::new("warp")
                .with("warpStyle", Value::Enumerated { type_id: Id::new("warpStyle"), value: Id::new("warpArc") })
                .with("warpValue", Value::Double(25.0))
                .with("warpRotate", Value::Enumerated { type_id: Id::new("Ornt"), value: Id::new("Vrtc") })
                .with("bounds", Value::Descriptor(bounds));
            d.with("Sz  ", Value::Descriptor(size)).with("warp", Value::Descriptor(warp))
        });
        let s = SmartObject::from_placed_layer_data(&data).unwrap();
        assert_eq!(s.uuid, "uuid-1");
        assert_eq!(s.quad, [[20.0, 4.0], [40.0, 4.0], [40.0, 44.0], [20.0, 44.0]]);
        assert_eq!(s.size, Some([20.0, 10.0]));
        let w = s.warp.clone().unwrap();
        assert_eq!((w.style.as_str(), w.bend, w.rotate_vertical, w.bounds), ("warpArc", 25.0, true, [0.0, 0.0, 20.0, 10.0]));
        assert!(s.is_affine() && s.needs_bake());
        let (centre, sx, sy, rotation) = s.placement(20.0, 10.0);
        assert_eq!((centre, sx, sy, rotation), ([30.0, 24.0], 1.0, 4.0, 0.0));
        // Every truncation is an error, never a panic.
        for cut in 0..data.len() {
            assert!(SmartObject::from_placed_layer_data(&data[..cut]).is_err(), "cut {cut}");
        }
    }

    #[test]
    fn the_non_affine_transform_wins_and_an_identity_warp_is_none() {
        let data = placed_layer_data("p", [0.0, 0.0, 10.0, 0.0, 10.0, 10.0, 0.0, 10.0], |d| {
            let none = Descriptor::new("warp").with("warpStyle", Value::Enumerated { type_id: Id::new("warpStyle"), value: Id::new("warpNone") });
            d.with("nonAffineTransform", list(&[10.0, 10.0, 50.0, 0.0, 50.0, 40.0, 10.0, 30.0])).with("warp", Value::Descriptor(none))
        });
        let s = SmartObject::from_placed_layer_data(&data).unwrap();
        assert_eq!(s.quad, [[10.0, 10.0], [50.0, 0.0], [50.0, 40.0], [10.0, 30.0]]);
        assert!(s.warp.is_none() && !s.is_affine());
    }

    #[test]
    fn a_custom_envelope_reads_its_quilt_mesh_and_a_wrong_sized_one_is_no_warp() {
        let envelope = |points: usize| {
            let unfl = |v: Vec<f64>| Value::UnitFloats { unit: *b"#Pxl", values: v };
            let body =
                Descriptor::new("rationalPoint").with("Hrzn", unfl((0..points).map(|i| i as f64).collect())).with("Vrtc", unfl(vec![1.0; points]));
            let env = Descriptor::new("customEnvelopeWarp")
                .with("quiltSliceX", unfl(vec![0.0, 10.0, 20.0]))
                .with("quiltSliceY", unfl(vec![0.0, 10.0]))
                .with("meshPoints", Value::ObjectArray(ObjectArray { prefix: 16, body }));
            Descriptor::new("warp")
                .with("warpStyle", Value::Enumerated { type_id: Id::new("warpStyle"), value: Id::new("warpCustom") })
                .with("customEnvelopeWarp", Value::Descriptor(env))
        };
        // Two columns and one row of patches: 7 × 4 control points.
        let w = Warp::from_descriptor(&envelope(28)).unwrap();
        assert_eq!((w.cols, w.rows, w.mesh.len()), (2, 1, 28));
        assert_eq!(w.mesh[5], [5.0, 1.0]);
        assert_eq!(Warp::from_descriptor(&envelope(27)), None);
    }

    #[test]
    fn the_older_placed_layer_block_gives_the_id_and_quad_and_refuses_a_transform_that_is_not_finite() {
        let block = |t: [f64; 8]| {
            let mut out = b"plcL".to_vec();
            out.put_u32(3);
            write_pascal(&mut out, b"old-id", 1);
            for v in [1u32, 1, 16, 2] {
                out.put_u32(v);
            }
            for v in t {
                out.put(&v.to_be_bytes());
            }
            out
        };
        let s = SmartObject::from_placed_layer(&block([1.0, 2.0, 3.0, 2.0, 3.0, 4.0, 1.0, 4.0])).unwrap();
        assert_eq!((s.uuid.as_str(), s.quad), ("old-id", [[1.0, 2.0], [3.0, 2.0], [3.0, 4.0], [1.0, 4.0]]));
        assert!(SmartObject::from_placed_layer(&block([f64::NAN; 8])).is_err());
        assert!(SmartObject::from_placed_layer(&block([f64::INFINITY; 8])).is_err());
    }

    #[test]
    fn a_layer_prefers_placed_layer_data_and_ignores_a_malformed_block() {
        let good = placed_layer_data("new", [0.0; 8], |d| d);
        let mut old = b"plcL".to_vec();
        old.put_u32(3);
        write_pascal(&mut old, b"old", 1);
        old.extend_from_slice(&[0; 16 + 64]);
        let mut layer = LayerRecord { blocks: vec![TaggedBlock::new(*b"PlLd", old), TaggedBlock::new(*b"SoLd", good)], ..Default::default() };
        assert_eq!(layer.smart_object().unwrap().uuid, "new");
        layer.blocks[1].data.truncate(9);
        assert_eq!(layer.smart_object().unwrap().uuid, "old");
        layer.blocks.clear();
        assert!(layer.smart_object().is_none());
    }

    #[test]
    fn linked_files_lists_embedded_and_external_records_and_stops_at_damage() {
        let mut data = linked_record(b"liFD", "uuid-1", "a.png", b"PNGf", b"PNGDATA");
        let first = data.len();
        data.extend(linked_record(b"liFE", "uuid-2", "b.psd", b"8BPS", b""));
        let files = linked_files(&data);
        assert_eq!(files.len(), 2);
        assert_eq!(
            (files[0].kind, files[0].uuid.as_str(), files[0].name.as_str(), files[0].file_type.as_str()),
            (*b"liFD", "uuid-1", "a.png", "PNGf")
        );
        assert_eq!(&data[files[0].data.clone().unwrap()], b"PNGDATA");
        assert_eq!((files[1].kind, files[1].uuid.as_str(), files[1].data.clone()), (*b"liFE", "uuid-2", None));
        // A record whose length runs past the block, or that is cut short, ends the list.
        let second_end = first + 8 + u64::from_be_bytes(data[first..first + 8].try_into().unwrap()) as usize;
        for cut in first..second_end {
            assert_eq!(linked_files(&data[..cut]).len(), 1, "cut {cut}");
        }
        let mut hostile = data.clone();
        hostile[..8].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(linked_files(&hostile).is_empty());
        // An embedded length larger than the record is damage, not an out-of-range slice.
        let mut long = linked_record(b"liFD", "u", "n", b"JPEG", b"abc");
        let at = long.windows(8).rposition(|w| w == 3u64.to_be_bytes()).unwrap();
        long[at..at + 8].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(linked_files(&long).is_empty());
    }

    #[test]
    fn a_document_finds_the_embedded_bytes_of_a_smart_object() {
        let mut file = crate::testgen::small(crate::Version::Psd, crate::Compression::Raw);
        file.global_blocks.push(TaggedBlock::new(*b"lnk2", linked_record(b"liFD", "uuid-1", "a.png", b"PNGf", b"PNGDATA")));
        file.global_blocks.push(TaggedBlock::new(*b"lnkE", linked_record(b"liFE", "uuid-2", "b.psd", b"8BPS", b"")));
        assert_eq!(file.linked_files().len(), 2);
        assert_eq!(file.linked_data("uuid-1"), Some(b"PNGDATA".as_slice()));
        assert_eq!(file.linked_data("uuid-2"), None);
        assert_eq!(file.linked_data("nope"), None);
        // The blocks are untouched: the file still writes and reads back.
        let again = PsdFile::from_bytes(&file.to_bytes().unwrap()).unwrap();
        assert_eq!(again.linked_data("uuid-1"), Some(b"PNGDATA".as_slice()));
    }

    #[test]
    fn perspective_quad_maps_corners_and_is_projective() {
        let q = [[10.0, 10.0], [50.0, 0.0], [50.0, 40.0], [10.0, 30.0]];
        let s = so(q, None);
        assert!(!s.is_affine() && s.needs_bake());
        for (c, want) in [([0.0, 0.0], q[0]), ([20.0, 0.0], q[1]), ([20.0, 10.0], q[2]), ([0.0, 10.0], q[3])] {
            let p = s.map(c, 20.0, 10.0);
            assert!((p[0] - want[0]).abs() < 1e-9 && (p[1] - want[1]).abs() < 1e-9, "{c:?} → {p:?}");
        }
        // Projective, not bilinear: the content centre is where the diagonals cross.
        let c = s.map([10.0, 5.0], 20.0, 10.0);
        assert!((c[0] - 30.0).abs() > 0.5, "{c:?}");
        let affine = so([[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]], None);
        assert!(affine.is_affine() && !affine.needs_bake());
    }

    #[test]
    fn custom_mesh_and_styles_warp() {
        // An identity 4×4 mesh shifted 3 px right in its interior row: interior points move.
        let mut mesh = vec![];
        for j in 0..4 {
            for i in 0..4 {
                mesh.push([i as f64 * 20.0 / 3.0 + if j == 1 || j == 2 { 3.0 } else { 0.0 }, j as f64 * 10.0 / 3.0]);
            }
        }
        let w = Warp { style: "warpCustom".into(), bounds: [0.0, 0.0, 20.0, 10.0], cols: 1, rows: 1, mesh, ..Default::default() };
        let p = w.apply([10.0, 5.0], 20.0, 10.0);
        assert!((p[0] - 12.25).abs() < 1e-9 && (p[1] - 5.0).abs() < 1e-9, "{p:?}");
        assert_eq!(w.apply([0.0, 0.0], 20.0, 10.0), [0.0, 0.0]);
        // Arc: the top centre rises, the bottom centre too (concentric arcs); Bulge: top up,
        // bottom down; Flag keeps the centre.
        let style = |s: &str| Warp { style: s.into(), bend: 50.0, bounds: [0.0, 0.0, 200.0, 100.0], ..Default::default() };
        let top = style("warpArc").apply([100.0, 0.0], 200.0, 100.0);
        let corner = style("warpArc").apply([0.0, 0.0], 200.0, 100.0);
        assert!(top[1] < corner[1], "arc {top:?} {corner:?}");
        let b = style("warpBulge");
        assert!(b.apply([100.0, 0.0], 200.0, 100.0)[1] < 0.0 && b.apply([100.0, 100.0], 200.0, 100.0)[1] > 100.0);
        assert!((style("warpFlag").apply([100.0, 50.0], 200.0, 100.0)[1] - 50.0).abs() < 1e-9);
        // Vertical rotation swaps the roles of the axes.
        let mut v = style("warpBulge");
        v.rotate_vertical = true;
        let l = v.apply([0.0, 50.0], 200.0, 100.0);
        assert!(l[0] < 0.0 && (l[1] - 50.0).abs() < 1e-9, "{l:?}");
        // Horizontal distortion: the right side grows taller.
        let d = Warp { style: "warpArc".into(), horizontal: 50.0, bounds: [0.0, 0.0, 200.0, 100.0], ..Default::default() };
        assert!(d.apply([200.0, 0.0], 200.0, 100.0)[1] < d.apply([0.0, 0.0], 200.0, 100.0)[1]);
    }

    #[test]
    fn custom_warp_with_a_missing_or_short_mesh_does_not_panic() {
        let base = Warp { style: "warpCustom".into(), bounds: [0.0, 0.0, 20.0, 10.0], cols: 1, rows: 1, ..Default::default() };
        // Empty and too-short meshes leave the point where it is.
        assert_eq!(base.apply([5.0, 5.0], 20.0, 10.0), [5.0, 5.0]);
        let short = Warp { mesh: vec![[0.0, 0.0]; 5], ..base.clone() };
        assert_eq!(short.apply([5.0, 5.0], 20.0, 10.0), [5.0, 5.0]);
        for (cols, rows) in [(0, 0), (0, 3), (usize::MAX, 1), (1, usize::MAX), (usize::MAX, usize::MAX)] {
            let w = Warp { cols, rows, ..base.clone() };
            assert_eq!(w.apply([5.0, 5.0], 20.0, 10.0), [5.0, 5.0]);
        }
    }

    #[test]
    fn an_absurd_quad_corner_bakes_within_four_times_the_document_and_2_pow_24_pixels() {
        let doc = (1920u32, 1080u32);
        for far in [1e4, 1e6, 1e12] {
            let s = SmartObject { uuid: "x".into(), quad: [[0.0, 0.0], [far, 5.0], [far, far], [0.0, 20.0]], size: None, warp: None };
            assert!(s.needs_bake());
            // What the importer asks for: the unbounded size is the 16384² clamp, 4 GiB as float RGBA.
            assert_eq!(s.placed_size(100.0, 100.0, 4.0), (16384, 16384));
            let (k, (w, h)) = s.bake_scale(100.0, 100.0, 4.0, doc);
            assert!(k > 0.0 && k <= 4.0, "{k}");
            assert!(w <= doc.0 * 4 && h <= doc.1 * 4, "{w}x{h}");
            assert!(u64::from(w) * u64::from(h) <= MAX_BAKE_PIXELS, "{w}x{h}");
        }
        // A quad inside the document keeps the scale and the size it always had.
        let s = SmartObject { uuid: "x".into(), quad: [[0.0, 0.0], [400.0, 50.0], [400.0, 150.0], [0.0, 200.0]], size: None, warp: None };
        let (k, size) = s.bake_scale(100.0, 100.0, 2.0, doc);
        assert_eq!((k, size), (2.0, s.placed_size(100.0, 100.0, 2.0)));
        // Not-a-number corners stay small too.
        let s = SmartObject { uuid: "x".into(), quad: [[f64::NAN; 2]; 4], size: None, warp: None };
        let (_, (w, h)) = s.bake_scale(100.0, 100.0, 4.0, doc);
        assert!(u64::from(w) * u64::from(h) <= MAX_BAKE_PIXELS);
    }
}
