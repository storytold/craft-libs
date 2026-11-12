//! Output encoding: the colour space (primaries + transfer curve) and sample format a render is
//! delivered in. Previews are 8-bit sRGB, or 8-bit in a monitor's own primaries when the frontend
//! has a display profile ([`DisplaySpace`]); exports may ask for a wide-gamut space and/or 16-bit or
//! linear float samples.
//!
//! The per-pixel stage works in scene-linear Rec.2020 and ends by converting to the target's
//! primaries, gamut mapping into the *target* gamut, and encoding. Tone curves and grain operate on
//! sRGB-curve-encoded values of the target primaries (identical to the sRGB path when the target is
//! sRGB); afterwards the values are re-encoded with the target's own curve.

use crate::color::{ADOBE_RGB, DISPLAY_P3, Mat3, PROPHOTO, REC2020, RgbSpace, SRGB};
use serde::{Deserialize, Serialize};

/// The RGB space an image is rendered into.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputSpace {
    #[default]
    Srgb,
    DisplayP3,
    /// Primaries and gamma of the published Adobe RGB (1998) specification ("compatible": our own
    /// profile, built from the published numbers).
    AdobeRgb,
    ProPhoto,
    Rec2020,
}

/// The encoding curve of an [`OutputSpace`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OutputTrc {
    /// IEC 61966-2-1 (sRGB, Display P3).
    Srgb,
    /// Pure power law: encoded = linear^(1/g).
    Gamma(f32),
    /// ITU-R BT.709 / BT.2020 OETF.
    Rec709,
}

impl OutputTrc {
    /// Linear → encoded (input clamped to 0..1).
    pub fn encode(self, v: f32) -> f32 {
        let v = v.clamp(0.0, 1.0);
        match self {
            OutputTrc::Srgb => crate::color::transfer::linear_to_srgb(v),
            OutputTrc::Gamma(g) => v.powf(1.0 / g),
            OutputTrc::Rec709 => {
                if v < 0.018 {
                    v * 4.5
                } else {
                    1.099 * v.powf(0.45) - 0.099
                }
            }
        }
    }

    /// Encoded → linear (input clamped to 0..1).
    pub fn decode(self, e: f32) -> f32 {
        let e = e.clamp(0.0, 1.0);
        match self {
            OutputTrc::Srgb => crate::color::transfer::srgb_to_linear(e),
            OutputTrc::Gamma(g) => e.powf(g),
            OutputTrc::Rec709 => {
                if e < 0.081 {
                    e / 4.5
                } else {
                    ((e + 0.099) / 1.099).powf(1.0 / 0.45)
                }
            }
        }
    }

    /// Code for the GPU kernel: 0 sRGB, 1 gamma, 2 Rec.709.
    pub fn code(self) -> (u32, f32) {
        match self {
            OutputTrc::Srgb => (0, 1.0),
            OutputTrc::Gamma(g) => (1, g),
            OutputTrc::Rec709 => (2, 1.0),
        }
    }
}

impl OutputSpace {
    pub const ALL: [OutputSpace; 5] = [OutputSpace::Srgb, OutputSpace::DisplayP3, OutputSpace::AdobeRgb, OutputSpace::ProPhoto, OutputSpace::Rec2020];

    pub fn rgb_space(self) -> RgbSpace {
        match self {
            OutputSpace::Srgb => SRGB,
            OutputSpace::DisplayP3 => DISPLAY_P3,
            OutputSpace::AdobeRgb => ADOBE_RGB,
            OutputSpace::ProPhoto => PROPHOTO,
            OutputSpace::Rec2020 => REC2020,
        }
    }

    pub fn trc(self) -> OutputTrc {
        match self {
            OutputSpace::Srgb | OutputSpace::DisplayP3 => OutputTrc::Srgb,
            OutputSpace::AdobeRgb => OutputTrc::Gamma(563.0 / 256.0),
            OutputSpace::ProPhoto => OutputTrc::Gamma(1.8),
            OutputSpace::Rec2020 => OutputTrc::Rec709,
        }
    }

    /// User-facing name.
    pub fn label(self) -> &'static str {
        match self {
            OutputSpace::Srgb => "sRGB",
            OutputSpace::DisplayP3 => "Display P3",
            OutputSpace::AdobeRgb => "Adobe RGB (1998) compatible",
            OutputSpace::ProPhoto => "ProPhoto RGB",
            OutputSpace::Rec2020 => "Rec. 2020",
        }
    }

    pub fn parse(s: &str) -> Option<OutputSpace> {
        let k: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        Some(match k.as_str() {
            "srgb" => OutputSpace::Srgb,
            "displayp3" | "p3" => OutputSpace::DisplayP3,
            "adobergb" | "adobergb1998" | "adobergb1998compatible" | "widegamutrgb" => OutputSpace::AdobeRgb,
            "prophoto" | "prophotorgb" | "rommrgb" => OutputSpace::ProPhoto,
            "rec2020" | "bt2020" | "itur2020" => OutputSpace::Rec2020,
            _ => return None,
        })
    }

    /// Linear Rec.2020 → linear target RGB (Bradford-adapted for D50 targets), row-major.
    pub fn from_working(self) -> [[f32; 3]; 3] {
        REC2020.to_space(&self.rgb_space()).to_f32()
    }

    /// Luminance weights of the target's linear RGB, as the gamut mapper uses them. sRGB keeps the
    /// rounded BT.709 weights the pipeline has always used (byte-identical previews and exports).
    pub fn luma(self) -> [f32; 3] {
        match self {
            OutputSpace::Srgb => [0.2126, 0.7152, 0.0722],
            o => o.rgb_space().luma().map(|v| v as f32),
        }
    }
}

/// A monitor's RGB as a preview target (from its ICC display profile): previews render into the
/// display's own primaries, gamut mapped into the display's gamut, encoded with the sRGB curve.
/// The frontend then takes that encoding to the display's real device values
/// (`lightcraft_codecs::display`). Exports never use it.
#[derive(Clone, Copy, Debug)]
pub struct DisplaySpace {
    /// Linear Rec.2020 → linear display RGB, and back.
    pub from_working: [[f32; 3]; 3],
    pub to_working: [[f32; 3]; 3],
    /// Luminance weights of the display's linear RGB (gamut mapping).
    pub luma: [f32; 3],
    /// Identifies the display profile (part of render keys).
    pub id: u64,
}

impl DisplaySpace {
    /// From the display's linear RGB → linear Rec.2020 matrix (display white → (1, 1, 1)).
    /// `None` for a singular or non-finite matrix.
    pub fn new(to_working: Mat3, id: u64) -> Option<DisplaySpace> {
        let from = to_working.inverse()?;
        let luma = REC2020.to_xyz().mul(&to_working).0[1];
        let all = to_working.0.iter().chain(from.0.iter()).flatten().chain(luma.iter());
        if all.clone().any(|v| !v.is_finite()) || (luma.iter().sum::<f64>() - 1.0).abs() > 1e-3 {
            return None;
        }
        Some(DisplaySpace { from_working: from.to_f32(), to_working: to_working.to_f32(), luma: luma.map(|v| v as f32), id })
    }

    /// A display that is exactly `space`'s primaries (tests, and the sRGB identity).
    pub fn of(space: OutputSpace, id: u64) -> Option<DisplaySpace> {
        DisplaySpace::new(space.rgb_space().to_space(&REC2020), id)
    }
}

impl PartialEq for DisplaySpace {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id
    }
}

impl Eq for DisplaySpace {}

/// Soft proofing: render as if the result were converted to `space` (colours outside its gamut
/// are mapped into it, then shown in the render's own space), optionally painting what the
/// destination can't hold (`dest_warning`, red) and what the display can't show
/// (`display_warning`, blue) — Lightroom's destination and monitor gamut warnings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Proof {
    pub space: OutputSpace,
    pub dest_warning: bool,
    pub display_warning: bool,
    /// An output-device profile to proof against instead of `space` (a printer / paper profile):
    /// the id [`register_proof_profile`] gave its table; 0 = none. An id with no table is ignored.
    #[serde(skip_serializing_if = "is_zero_id")]
    pub profile: u64,
}

fn is_zero_id(v: &u64) -> bool {
    *v == 0
}

/// A soft-proof table for an output-device profile, built by the caller's CMS: `n³` entries over
/// working-space (linear Rec.2020) RGB encoded with the sRGB curve, index `(r * n + g) * n + b`,
/// each the colour the device reproduces (linear Rec.2020, paper white already applied when the
/// proof simulates paper and ink) and, in `[3]`, how far outside the device gamut the colour is
/// (0 inside; above [`PROOF_GAMUT_TOLERANCE`] counts as outside).
#[derive(Clone, Debug, PartialEq)]
pub struct ProofLut {
    n: usize,
    data: Vec<[f32; 4]>,
}

/// Gamut distance (sRGB-curve code values, 0..1) above which a colour is out of the proof gamut.
pub const PROOF_GAMUT_TOLERANCE: f32 = 0.02;

impl ProofLut {
    /// `None` unless `data` holds `n³` finite entries with `2 ≤ n ≤ 129`.
    pub fn new(n: usize, data: Vec<[f32; 4]>) -> Option<ProofLut> {
        let ok = (2..=129).contains(&n) && data.len() == n * n * n && data.iter().flatten().all(|v| v.is_finite());
        ok.then_some(ProofLut { n, data })
    }

    /// The proofed colour and gamut distance of working-space `rgb` (trilinear).
    pub fn sample(&self, rgb: [f32; 3]) -> ([f32; 3], f32) {
        let m = (self.n - 1) as f32;
        let f = rgb.map(|v| crate::color::transfer::linear_to_srgb(v.clamp(0.0, 1.0)) * m);
        let i0 = f.map(|v| (v.floor() as usize).min(self.n - 2));
        let t = [f[0] - i0[0] as f32, f[1] - i0[1] as f32, f[2] - i0[2] as f32];
        let mut acc = [0.0f32; 4];
        for corner in 0..8usize {
            let b = [corner >> 2 & 1, corner >> 1 & 1, corner & 1];
            let w: f32 = (0..3).map(|k| if b[k] == 1 { t[k] } else { 1.0 - t[k] }).product();
            let idx = ((i0[0] + b[0]) * self.n + i0[1] + b[1]) * self.n + i0[2] + b[2];
            if let Some(e) = self.data.get(idx) {
                for k in 0..4 {
                    acc[k] += e[k] * w;
                }
            }
        }
        ([acc[0], acc[1], acc[2]], acc[3])
    }
}

/// Proof tables by id (a few: the profiles a session proofs against).
static PROOF_PROFILES: std::sync::Mutex<Vec<(u64, std::sync::Arc<ProofLut>)>> = std::sync::Mutex::new(Vec::new());
const PROOF_PROFILES_KEPT: usize = 8;

/// Make `lut` available to renders as [`Proof::profile`] `id` (any nonzero number that identifies
/// the profile and its options, e.g. a hash). Returns `id` (`1` for `0`).
pub fn register_proof_profile(id: u64, lut: ProofLut) -> u64 {
    let id = id.max(1);
    let mut v = PROOF_PROFILES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    v.retain(|(k, _)| *k != id);
    v.push((id, std::sync::Arc::new(lut)));
    let extra = v.len().saturating_sub(PROOF_PROFILES_KEPT);
    v.drain(..extra);
    id
}

/// The table registered as `id`.
pub fn proof_profile(id: u64) -> Option<std::sync::Arc<ProofLut>> {
    if id == 0 {
        return None;
    }
    let v = PROOF_PROFILES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    v.iter().find(|(k, _)| *k == id).map(|(_, l)| l.clone())
}

/// Warning colours (sRGB-encoded): destination gamut red, display gamut blue.
pub const PROOF_DEST_WARNING: [f32; 3] = [1.0, 0.0, 0.0];
pub const PROOF_DISPLAY_WARNING: [f32; 3] = [0.0, 0.25, 1.0];

/// [`Proof`] as the per-pixel stage uses it.
#[derive(Clone, Debug)]
pub struct ProofParams {
    /// The output-device table (replaces `to_proof` / `proof_to_out` when present).
    pub table: Option<std::sync::Arc<ProofLut>>,
    /// Linear Rec.2020 → linear proof RGB, its luminance weights, linear proof → linear output RGB.
    pub to_proof: [[f32; 3]; 3],
    pub luma: [f32; 3],
    pub proof_to_out: [[f32; 3]; 3],
    pub dest_warning: bool,
    pub display_warning: bool,
}

impl Proof {
    pub fn key(self) -> u64 {
        ((self.space as u64 + 1) | (self.dest_warning as u64) << 8 | (self.display_warning as u64) << 9) ^ self.profile.rotate_left(16)
    }

    pub fn params(self, out: OutputSpace) -> ProofParams {
        ProofParams {
            table: proof_profile(self.profile),
            to_proof: self.space.from_working(),
            luma: self.space.luma(),
            proof_to_out: self.space.rgb_space().to_space(&out.rgb_space()).to_f32(),
            dest_warning: self.dest_warning,
            display_warning: self.display_warning,
        }
    }

    /// [`Self::params`] for a render into a display's primaries: the display warning then shows
    /// what *this* display can't show.
    pub fn params_display(self, d: &DisplaySpace) -> ProofParams {
        let proof_to_working = self.space.rgb_space().to_space(&REC2020).to_f32();
        ProofParams { proof_to_out: mul33(&d.from_working, &proof_to_working), ..self.params(OutputSpace::Srgb) }
    }
}

fn mul33(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

/// Sample format of a render.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputDepth {
    /// 8-bit display-encoded ([`crate::Rendered::image`] only).
    #[default]
    U8,
    /// 16-bit display-encoded RGB in [`crate::Rendered::deep`].
    U16,
    /// 32-bit float *linear* RGB (target primaries, 0..1) in [`crate::Rendered::deep`].
    F32Linear,
    /// 32-bit float linear RGB (target primaries) of the HDR render: SDR white = 1, highlights up
    /// to [`craft_develop_model::Hdr::peak`]. With HDR off this is [`OutputDepth::F32Linear`].
    F32Hdr,
}

/// High-bit-depth RGB samples (3 per pixel, interleaved, row-major).
#[derive(Clone, Debug, PartialEq)]
pub enum DeepSamples {
    U16(Vec<u16>),
    F32(Vec<f32>),
}

/// A high-bit-depth render ([`OutputDepth::U16`] / [`OutputDepth::F32Linear`]).
#[derive(Clone, Debug, PartialEq)]
pub struct DeepImage {
    pub width: usize,
    pub height: usize,
    pub space: OutputSpace,
    pub samples: DeepSamples,
}

impl DeepImage {
    /// The `w × h` part of the image at (`x`, `y`), clamped to the image (at least 1 × 1 when the
    /// image has pixels). Samples are RGB, three per pixel, row-major.
    pub fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> DeepImage {
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        let w = w.min(self.width.saturating_sub(x)).max(usize::from(self.width > 0));
        let h = h.min(self.height.saturating_sub(y)).max(usize::from(self.height > 0));
        fn cut<T: Copy>(v: &[T], width: usize, x: usize, y: usize, w: usize, h: usize) -> Vec<T> {
            let mut out = Vec::with_capacity(w.saturating_mul(h).saturating_mul(3));
            for row in y..y + h {
                let start = row.saturating_mul(width).saturating_add(x).saturating_mul(3);
                if let Some(r) = v.get(start..start.saturating_add(w * 3)) {
                    out.extend_from_slice(r);
                }
            }
            out
        }
        let samples = match &self.samples {
            DeepSamples::U16(v) => DeepSamples::U16(cut(v, self.width, x, y, w, h)),
            DeepSamples::F32(v) => DeepSamples::F32(cut(v, self.width, x, y, w, h)),
        };
        DeepImage { width: w, height: h, space: self.space, samples }
    }

    /// The same image reduced to 8 bits, display-encoded with the space's curve.
    pub fn to_rgba8(&self) -> crate::raster::Rgba8 {
        let mut out = crate::raster::Rgba8::new(self.width, self.height);
        let trc = self.space.trc();
        match &self.samples {
            DeepSamples::U16(v) => {
                for (p, c) in out.data.iter_mut().zip(v.as_chunks::<3>().0) {
                    *p = [c[0], c[1], c[2]].map(|x| ((x as u32 * 255 + 32767) / 65535) as u8).into_rgba();
                }
            }
            DeepSamples::F32(v) => {
                for (p, c) in out.data.iter_mut().zip(v.as_chunks::<3>().0) {
                    *p = [c[0], c[1], c[2]].map(|x| (trc.encode(x) * 255.0 + 0.5) as u8).into_rgba();
                }
            }
        }
        out
    }
}

trait IntoRgba {
    fn into_rgba(self) -> [u8; 4];
}

impl IntoRgba for [u8; 3] {
    fn into_rgba(self) -> [u8; 4] {
        [self[0], self[1], self[2], 255]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A crop keeps the right pixels and never reads past the image, whatever the request.
    #[test]
    fn deep_crop_keeps_the_window_and_clamps() {
        // 4 × 3, pixel (x, y) holds x + 10 y in every channel
        let v: Vec<u16> = (0..3).flat_map(|y| (0..4).flat_map(move |x| [x + 10 * y; 3])).collect();
        let d = DeepImage { width: 4, height: 3, space: OutputSpace::Srgb, samples: DeepSamples::U16(v) };
        let c = d.crop(1, 1, 2, 2);
        assert_eq!((c.width, c.height), (2, 2));
        assert_eq!(c.samples, DeepSamples::U16(vec![11, 11, 11, 12, 12, 12, 21, 21, 21, 22, 22, 22]));
        for (x, y, w, h) in [(9, 9, 5, 5), (0, 0, usize::MAX, usize::MAX), (3, 2, 0, 0)] {
            let c = d.crop(x, y, w, h);
            let DeepSamples::U16(s) = &c.samples else { panic!("a U16 image crops to U16") };
            assert!(c.width >= 1 && c.height >= 1 && c.width <= 4 && c.height <= 3);
            assert_eq!(s.len(), c.width * c.height * 3, "{x} {y} {w} {h}");
        }
    }

    #[test]
    fn trcs_invert() {
        for o in OutputSpace::ALL {
            let t = o.trc();
            for i in 0..=100 {
                let x = i as f32 / 100.0;
                assert!((t.decode(t.encode(x)) - x).abs() < 1e-5, "{o:?} {x}");
            }
        }
    }

    #[test]
    fn parse_and_serde_names() {
        for o in OutputSpace::ALL {
            let s = serde_json::to_value(o).unwrap();
            assert_eq!(OutputSpace::parse(s.as_str().unwrap()), Some(o));
            assert_eq!(OutputSpace::parse(o.label()), Some(o), "{}", o.label());
        }
    }

    #[test]
    fn white_maps_to_white() {
        for o in OutputSpace::ALL {
            let m = o.from_working();
            for row in m {
                assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-4, "{o:?}");
            }
            assert!((o.luma().iter().sum::<f32>() - 1.0).abs() < 1e-4);
        }
    }
}
