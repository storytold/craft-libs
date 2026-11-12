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

use std::borrow::Cow;
use std::ops::Range;

use crate::descriptor::{Descriptor, UnicodeString, Value};
use crate::error::{PsdError, Result};
use crate::file::PsdFile;
use crate::io::{Reader, WriteExt, read_pascal};
use crate::layer::LayerRecord;

mod write;
pub use write::PlacedLayerOptions;

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
    d.get_numbers(key).try_into().ok()
}

/// The corner quad (top-left, top-right, bottom-right, bottom-left) from x0 y0 … x3 y3.
fn quad_from(t: [f64; 8]) -> [[f64; 2]; 4] {
    [[t[0], t[1]], [t[2], t[3]], [t[4], t[5]], [t[6], t[7]]]
}

/// The patch boundaries of a custom envelope: a list of numbers, or (as Photoshop writes a
/// `quiltWarp`) an object array whose body holds them under the same key.
fn quilt_slices(env: &Descriptor, key: &str) -> Vec<f64> {
    match env.get(key) {
        Some(Value::ObjectArray(o)) => o.body.get_numbers(key),
        _ => env.get_numbers(key),
    }
}

impl Warp {
    /// Reads a `warp` descriptor; `None` for no warp (the identity, or a custom warp without a
    /// usable mesh).
    pub fn from_descriptor(d: &Descriptor) -> Option<Warp> {
        let style = d.get_enum("warpStyle").unwrap_or_else(|| "warpNone".to_string());
        let side = |b: &Descriptor, k: &str| b.get_number(k).unwrap_or(0.0);
        let bounds = d.get_object("bounds").map_or([0.0; 4], |b| [side(b, "Left"), side(b, "Top "), side(b, "Rght"), side(b, "Btom")]);
        let mut w = Warp {
            bend: d.get_number("warpValue").unwrap_or(0.0),
            horizontal: d.get_number("warpPerspective").unwrap_or(0.0),
            vertical: d.get_number("warpPerspectiveOther").unwrap_or(0.0),
            rotate_vertical: d.get_enum("warpRotate").as_deref() == Some("Vrtc"),
            bounds,
            cols: 1,
            rows: 1,
            mesh: vec![],
            style,
        };
        if let Some(env) = d.get_object("customEnvelopeWarp") {
            let cols = quilt_slices(env, "quiltSliceX").len().saturating_sub(1).max(1);
            let rows = quilt_slices(env, "quiltSliceY").len().saturating_sub(1).max(1);
            if let Some(Value::ObjectArray(pts)) = env.get("meshPoints") {
                let xs = pts.body.get_numbers("Hrzn");
                let ys = pts.body.get_numbers("Vrtc");
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
        let uuid = d.get_text("Idnt").unwrap_or_default();
        let t = eight(&d, "Trnf").unwrap_or([0.0; 8]);
        // The non-affine transform when present (perspective), else the transform.
        let t = eight(&d, "nonAffineTransform").unwrap_or(t);
        let size = d.get_object("Sz  ").and_then(|s| Some([s.get_number("Wdth")?, s.get_number("Hght")?]));
        // A split warp is in `quiltWarp`, with `warp` holding none.
        let warp = d.get_object("quiltWarp").and_then(Warp::from_descriptor).or_else(|| d.get_object("warp").and_then(Warp::from_descriptor));
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

/// The four-character file type Photoshop records for an embedded file, from its first bytes:
/// `8BPS` (`8BPB` for a PSB), `PNGf`, `JPEG`, `TIFF`, `GIFf`, or four spaces.
pub fn file_type(bytes: &[u8]) -> [u8; 4] {
    match bytes {
        [b'8', b'B', b'P', b'S', 0, 2, ..] => *b"8BPB",
        [b'8', b'B', b'P', b'S', ..] => *b"8BPS",
        [0x89, b'P', b'N', b'G', ..] => *b"PNGf",
        [0xFF, 0xD8, ..] => *b"JPEG",
        [b'I', b'I', 42, 0, ..] | [b'M', b'M', 0, 42, ..] => *b"TIFF",
        [b'G', b'I', b'F', ..] => *b"GIFf",
        _ => *b"    ",
    }
}

/// One embedded-file record (`liFD`, version 7: no open descriptor, no child document, no
/// modification time, unlocked) with its length field and padding to 4, ready to append to a
/// linked layer block's data. The type is [`file_type`] of the bytes; the id is cut to 255 bytes.
pub fn encode_embedded_file(uuid: &str, name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut item = b"liFD".to_vec();
    item.put_u32(7);
    let id = uuid.as_bytes().get(..uuid.len().min(255)).unwrap_or_default();
    item.put_u8(id.len() as u8);
    item.put(id);
    // NUL-terminated, the terminator counted (as Photoshop writes and reads it).
    UnicodeString::new_nul(name).write(&mut item);
    item.put(&file_type(bytes));
    item.put(b"8BIM");
    item.put_u64(bytes.len() as u64);
    item.put_u8(0);
    item.put(bytes);
    item.put_u32(0); // child document id (version 5)
    item.put_f64(0.0); // asset modification time (version 6)
    item.put_u8(0); // asset locked (version 7)
    let mut out = Vec::with_capacity(item.len() + 11);
    out.put_u64(item.len() as u64);
    out.put(&item);
    out.resize(out.len().next_multiple_of(4), 0);
    out
}

/// One record of a linked layer block's data, unparsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkedRecord<'a> {
    /// The record's unique id, when its header is readable (lossy UTF-8).
    pub uuid: Option<&'a [u8]>,
    /// The whole record: length field, record and padding.
    pub raw: &'a [u8],
}

impl LinkedRecord<'_> {
    /// The unique id as text, when it is readable and valid UTF-8.
    pub fn uuid_str(&self) -> Option<&str> {
        std::str::from_utf8(self.uuid?).ok()
    }
}

/// The record inside `data` at its start: (the record without its length field, the record
/// with length field and padding). `None` when the length runs past the data.
fn framed_record(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let len = usize::try_from(u64::from_be_bytes(*data.first_chunk::<8>()?)).ok()?;
    let end = len.checked_add(8)?;
    let item = data.get(8..end)?;
    let padded = end.checked_next_multiple_of(4)?.min(data.len());
    Some((item, data.get(..padded)?))
}

/// The unique id bytes of a record (after its type and version), when they are present.
fn record_uuid(item: &[u8]) -> Option<&[u8]> {
    let n = usize::from(*item.get(8)?);
    item.get(9..9usize.checked_add(n)?)
}

/// The records of a linked layer block's data, in order, each with its unique id when its header
/// is readable. A tail that is no complete record is returned as one record without an id.
pub fn linked_records(data: &[u8]) -> Vec<LinkedRecord<'_>> {
    let mut out = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let Some((item, raw)) = framed_record(rest) else {
            out.push(LinkedRecord { uuid: None, raw: rest });
            break;
        };
        out.push(LinkedRecord { uuid: record_uuid(item), raw });
        rest = rest.get(raw.len()..).unwrap_or_default();
    }
    out
}

/// The unique ids of every record of a linked layer block's data that has a readable one.
pub fn linked_uuids(data: &[u8]) -> Vec<String> {
    linked_records(data).iter().filter_map(|r| r.uuid.map(|u| String::from_utf8_lossy(u).into_owned())).collect()
}

/// A linked layer block's data with the records `keep` accepts (by unique id, lossy UTF-8) and
/// every record without a readable id, then the records of `add` (see
/// [`encode_embedded_file`]) appended. `None` when nothing would change.
pub fn rebuild_linked_block(data: &[u8], keep: &dyn Fn(&str) -> bool, add: &[Vec<u8>]) -> Option<Vec<u8>> {
    let records = linked_records(data);
    let kept = |r: &LinkedRecord<'_>| r.uuid.is_none_or(|u| keep(&String::from_utf8_lossy(u)));
    if records.iter().all(kept) && add.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(data.len());
    for r in records.iter().filter(|r| kept(r)) {
        out.extend_from_slice(r.raw);
    }
    for a in add {
        out.extend_from_slice(a);
    }
    Some(out)
}

/// Only the complete records `keep(uuid, raw record)` accepts, for copying smart objects to
/// another document: unlike [`rebuild_linked_block`], records without an id and a damaged tail
/// are left out, so a copy never carries unrelated embedded files. An id must be valid UTF-8 to
/// be selected: one that is not, but whose lossy text `keep` accepts, is an error, as is a
/// selected record that is cut short. `Ok(None)` when every record is selected and complete
/// (the data can be shared as it is).
pub fn select_linked_records(data: &[u8], keep: &dyn Fn(&str, &[u8]) -> bool) -> Result<Option<Vec<u8>>> {
    let cut_short = || PsdError::invalid("a selected linked-layer record is cut short");
    let append = |out: &mut Vec<u8>, bytes: &[u8]| -> Result<()> {
        out.try_reserve(bytes.len()).map_err(|_| PsdError::OutOfMemory("not enough memory to copy linked-layer records"))?;
        out.extend_from_slice(bytes);
        Ok(())
    };
    let mut selected: Option<Vec<u8>> = None;
    let mut at = 0usize;
    while let Some(rest) = data.get(at..).filter(|r| !r.is_empty()) {
        let Some((item, raw)) = framed_record(rest) else {
            if let Some(item) = rest.get(8..)
                && selection_uuid(item, rest, keep)?.is_some_and(|uuid| keep(uuid, rest))
            {
                return Err(cut_short());
            }
            if selected.is_none() {
                let mut prefix = Vec::new();
                append(&mut prefix, data.get(..at).ok_or_else(cut_short)?)?;
                selected = Some(prefix);
            }
            break;
        };
        if selection_uuid(item, raw, keep)?.is_some_and(|uuid| keep(uuid, raw)) {
            if let Some(out) = &mut selected {
                append(out, raw)?;
            }
        } else if selected.is_none() {
            let mut prefix = Vec::new();
            append(&mut prefix, data.get(..at).ok_or_else(cut_short)?)?;
            selected = Some(prefix);
        }
        at = at.checked_add(raw.len()).ok_or_else(cut_short)?;
    }
    Ok(selected)
}

/// A record's id for selection: exact UTF-8, or an error when only its lossy text is selected.
fn selection_uuid<'a>(item: &'a [u8], raw: &[u8], keep: &dyn Fn(&str, &[u8]) -> bool) -> Result<Option<&'a str>> {
    let Some(bytes) = record_uuid(item).filter(|b| !b.is_empty()) else { return Ok(None) };
    match std::str::from_utf8(bytes) {
        Ok(uuid) => Ok(Some(uuid)),
        Err(_) if keep(&String::from_utf8_lossy(bytes), raw) => Err(PsdError::invalid("a linked-layer record id is not valid UTF-8")),
        Err(_) => Ok(None),
    }
}

/// Whether the complete record `record` (length field and padding included) is in a linked layer
/// block's data, without decoding any file bytes.
pub fn contains_linked_record(data: &[u8], record: &[u8]) -> bool {
    let mut rest = data;
    while let Some((_, raw)) = framed_record(rest) {
        if raw == record {
            return true;
        }
        let Some(next) = rest.get(raw.len()..) else { return false };
        rest = next;
    }
    false
}

/// The record with id `uuid` copied under `new_uuid` (at most 255 bytes), with its length
/// field and padding; the rest of the record (embedded, external or alias data) is kept.
pub fn copy_linked_record(data: &[u8], uuid: &str, new_uuid: &str) -> Option<Vec<u8>> {
    let record = linked_records(data).into_iter().find(|r| r.uuid == Some(uuid.as_bytes()))?;
    let (item, _) = framed_record(record.raw)?;
    let old_end = 9usize.checked_add(usize::from(*item.get(8)?))?;
    let new_len = u8::try_from(new_uuid.len()).ok()?;
    let mut copy = item.get(..8)?.to_vec();
    copy.push(new_len);
    copy.extend_from_slice(new_uuid.as_bytes());
    copy.extend_from_slice(item.get(old_end..)?);
    let mut out = Vec::with_capacity(copy.len() + 11);
    out.put_u64(copy.len() as u64);
    out.extend(copy);
    out.resize(out.len().next_multiple_of(4), 0);
    Some(out)
}

impl PsdFile {
    /// Every linked file of the document's linked layer blocks, each with the index of the
    /// global block that holds it ([`LinkedFile::data`] is a range of that block's data). A
    /// block left in the source ([`crate::TaggedBlock::stored`]) is read for it.
    pub fn linked_files(&self) -> Result<Vec<(usize, LinkedFile)>> {
        let mut out = Vec::new();
        for (i, b) in self.global_blocks.iter().enumerate().filter(|(_, b)| is_linked_layer_key(&b.key)) {
            out.extend(linked_files(&b.encoded()?).into_iter().map(|f| (i, f)));
        }
        Ok(out)
    }

    /// The embedded bytes of the linked file `uuid`, when the document embeds it: borrowed from
    /// a block in memory, read from the source for a block left there.
    pub fn linked_data(&self, uuid: &str) -> Result<Option<Cow<'_, [u8]>>> {
        for b in self.global_blocks.iter().filter(|b| is_linked_layer_key(&b.key)) {
            let data = b.encoded()?;
            let Some(range) = linked_files(&data).into_iter().find(|f| f.uuid == uuid).and_then(|f| f.data) else { continue };
            return Ok(match data {
                Cow::Borrowed(d) => d.get(range).map(Cow::Borrowed),
                Cow::Owned(d) => d.get(range).map(|e| Cow::Owned(e.to_vec())),
            });
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{Id, ObjectArray};
    use crate::io::{WriteExt, write_pascal};
    use crate::tagged::TaggedBlock;

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

    /// The embedded files of a block, as (id, name, bytes).
    fn embedded(data: &[u8]) -> Vec<(String, String, Vec<u8>)> {
        linked_files(data).into_iter().filter_map(|f| Some((f.uuid, f.name, data.get(f.data?)?.to_vec()))).collect()
    }

    #[test]
    fn embedded_file_records_read_back_and_check() {
        let mut data = encode_embedded_file("abc-1", "a.png", &[0x89, b'P', b'N', b'G', 1]);
        data.extend(encode_embedded_file("def-2", "b.psb", b"8BPS\0\x02rest"));
        assert_eq!(
            embedded(&data),
            [("abc-1".into(), "a.png".into(), vec![0x89, b'P', b'N', b'G', 1]), ("def-2".into(), "b.psb".into(), b"8BPS\0\x02rest".to_vec())]
        );
        assert_eq!(linked_files(&data).iter().map(|f| f.file_type.clone()).collect::<Vec<_>>(), ["PNGf", "8BPB"]);
        TaggedBlock::new(*b"lnk2", data).check_structure().unwrap();
        for (bytes, ty) in [
            (&b"8BPS\0\x01"[..], b"8BPS"),
            (&[0xFF, 0xD8, 0xFF][..], b"JPEG"),
            (b"II*\0", b"TIFF"),
            (b"MM\0*", b"TIFF"),
            (b"GIF89a", b"GIFf"),
            (b"%PDF", b"    "),
        ] {
            assert_eq!(&file_type(bytes), ty);
        }
    }

    #[test]
    fn blocks_are_pruned_and_extended() {
        let (a, b, c) =
            (encode_embedded_file("a", "a.png", &[1]), encode_embedded_file("b", "b.psb", &[2, 2]), encode_embedded_file("c", "c", &[7; 3]));
        let mut data = a.clone();
        data.extend(&b);
        assert_eq!(linked_uuids(&data), ["a", "b"]);
        assert_eq!(rebuild_linked_block(&data, &|_| true, &[]), None);
        let out = rebuild_linked_block(&data, &|u| u != "a", std::slice::from_ref(&c)).unwrap();
        assert_eq!(out, [b.clone(), c.clone()].concat());
        // A malformed tail is kept as it is.
        let mut bad = a.clone();
        bad.extend([0, 0, 0, 0, 0, 0, 1, 0, 9]);
        assert!(rebuild_linked_block(&bad, &|_| true, std::slice::from_ref(&c)).unwrap().starts_with(&bad));
        assert!(linked_records(&[0, 0, 0, 0, 0, 0, 0, 200, 1, 2]).iter().all(|r| r.uuid.is_none()));
        assert!(linked_records(&[]).is_empty());
    }

    #[test]
    fn copying_a_record_keeps_an_unknown_external_payload() {
        let mut raw = encode_embedded_file("old", "missing.psb", &[7; 5]);
        raw[8..12].copy_from_slice(b"liFE");
        let copy = copy_linked_record(&raw, "old", "independent-copy").unwrap();
        assert_eq!(linked_uuids(&copy), ["independent-copy"]);
        let original_len = u64::from_be_bytes(raw[..8].try_into().unwrap()) as usize;
        let copied_len = u64::from_be_bytes(copy[..8].try_into().unwrap()) as usize;
        assert_eq!(&copy[8..16], &raw[8..16]);
        assert_eq!(&copy[17 + "independent-copy".len()..8 + copied_len], &raw[20..8 + original_len]);
        assert!(copy_linked_record(&raw, "missing", "new").is_none());
        assert!(copy_linked_record(&raw, "old", &"x".repeat(256)).is_none());
    }

    #[test]
    fn selected_records_leave_out_other_files_and_unidentified_pieces() {
        let (a, b) = (encode_embedded_file("a", "a.psb", &[1; 3]), encode_embedded_file("b", "b.psb", &[2; 4]));
        assert!(select_linked_records(&a, &|uuid, _| uuid == "a").unwrap().is_none(), "a block of only selected records can be shared");
        let mut data = a.clone();
        data.extend(&b);
        data.extend([0, 0, 0, 0, 0, 0, 1, 0, 9]);
        assert_eq!(select_linked_records(&data, &|uuid, _| uuid == "a").unwrap(), Some(a.clone()));
        assert_eq!(select_linked_records(&data, &|uuid, _| uuid == "b").unwrap(), Some(b));
        assert_eq!(select_linked_records(&data, &|_, _| false).unwrap(), Some(Vec::new()));
        // A complete but unidentified record is skipped without losing a later selected one.
        let mut unreadable = 1u64.to_be_bytes().to_vec();
        unreadable.extend([7, 0, 0, 0]);
        unreadable.extend(&a);
        assert_eq!(select_linked_records(&unreadable, &|uuid, _| uuid == "a").unwrap(), Some(a));
    }

    #[test]
    fn selected_external_and_alias_records_keep_unknown_payloads_verbatim() {
        for kind in [*b"liFE", *b"liFA", *b"next"] {
            let mut raw = encode_embedded_file("selected", "missing.psb", &[7; 5]);
            raw[8..12].copy_from_slice(&kind);
            raw[12..16].copy_from_slice(&42u32.to_be_bytes()); // an unknown future version
            let mut data = encode_embedded_file("unselected", "other.psb", &[9; 3]);
            data.extend(&raw);
            assert_eq!(select_linked_records(&data, &|uuid, _| uuid == "selected").unwrap(), Some(raw));
        }
    }

    #[test]
    fn a_cut_selected_record_is_an_error_not_a_missing_file() {
        let mut raw = encode_embedded_file("selected", "missing.psb", &[7; 5]);
        raw[..8].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(select_linked_records(&raw, &|uuid, _| uuid == "selected").unwrap_err().to_string().contains("cut short"));
        assert_eq!(select_linked_records(&raw, &|uuid, _| uuid == "other").unwrap(), Some(Vec::new()));
    }

    #[test]
    fn records_are_matched_whole() {
        let (a, b, different) =
            (encode_embedded_file("a", "a.psb", &[1; 3]), encode_embedded_file("b", "b.psb", &[2; 4]), encode_embedded_file("a", "a.psb", &[9; 3]));
        let combined = [a.clone(), b.clone()].concat();
        assert!(contains_linked_record(&combined, &a) && contains_linked_record(&combined, &b));
        assert!(!contains_linked_record(&combined, &different));
        assert_eq!(select_linked_records(&a, &|_, raw| !contains_linked_record(&combined, raw)).unwrap(), Some(Vec::new()));
        assert!(select_linked_records(&different, &|_, raw| !contains_linked_record(&combined, raw)).unwrap().is_none());
    }

    #[test]
    fn empty_ids_are_unidentified_even_when_asked_for() {
        let raw = encode_embedded_file("", "unidentified.psb", &[7; 5]);
        assert_eq!(select_linked_records(&raw, &|_, _| true).unwrap(), Some(Vec::new()));
        let mut cut = raw;
        cut[..8].copy_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(select_linked_records(&cut, &|_, _| true).unwrap(), Some(Vec::new()));
    }

    #[test]
    fn selection_never_takes_a_lossy_id_for_a_record_identity() {
        let (mut a, mut b) = (encode_embedded_file("x", "a.psb", &[1; 3]), encode_embedded_file("x", "b.psb", &[2; 4]));
        a[17] = 0xff;
        b[17] = 0xfe;
        let mut data = [a, b].concat();
        assert_eq!(linked_uuids(&data), ["\u{fffd}", "\u{fffd}"]);
        assert!(select_linked_records(&data, &|uuid, _| uuid == "\u{fffd}").unwrap_err().to_string().contains("not valid UTF-8"));
        let valid = encode_embedded_file("valid", "valid.psb", &[3; 5]);
        data.extend(&valid);
        assert_eq!(select_linked_records(&data, &|uuid, _| uuid == "valid").unwrap(), Some(valid));
        // A literal replacement character can itself be a valid UTF-8 id.
        let unicode = encode_embedded_file("\u{fffd}", "unicode.psb", &[4; 2]);
        assert!(select_linked_records(&unicode, &|uuid, _| uuid == "\u{fffd}").unwrap().is_none());
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
        let first = file.global_blocks.len();
        file.global_blocks.push(TaggedBlock::new(*b"lnk2", linked_record(b"liFD", "uuid-1", "a.png", b"PNGf", b"PNGDATA")));
        file.global_blocks.push(TaggedBlock::new(*b"lnkE", linked_record(b"liFE", "uuid-2", "b.psd", b"8BPS", b"")));
        let embedded = |f: &PsdFile, uuid: &str| f.linked_data(uuid).unwrap().map(Cow::into_owned);
        assert_eq!(
            file.linked_files().unwrap().iter().map(|(i, f)| (*i, f.uuid.as_str())).collect::<Vec<_>>(),
            [(first, "uuid-1"), (first + 1, "uuid-2")]
        );
        assert_eq!(embedded(&file, "uuid-1").as_deref(), Some(b"PNGDATA".as_slice()));
        assert_eq!(embedded(&file, "uuid-2"), None);
        assert_eq!(embedded(&file, "nope"), None);
        // The blocks are untouched: the file still writes and reads back.
        let again = PsdFile::from_bytes(&file.to_bytes().unwrap()).unwrap();
        assert_eq!(embedded(&again, "uuid-1").as_deref(), Some(b"PNGDATA".as_slice()));
        // With the linked-file blocks left in the source, the same files and bytes are found.
        let bytes = file.to_bytes().unwrap();
        let (deferred, _) =
            PsdFile::from_source_deferring(crate::Source::new(bytes), |key, _| is_linked_layer_key(key), |_| Ok(false), |_| false).unwrap();
        assert!(deferred.global_blocks.iter().filter(|b| is_linked_layer_key(&b.key)).all(|b| b.data.is_empty() && b.stored.is_some()));
        assert_eq!(deferred.linked_files().unwrap(), file.linked_files().unwrap());
        assert_eq!(embedded(&deferred, "uuid-1").as_deref(), Some(b"PNGDATA".as_slice()));
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
