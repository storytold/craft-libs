//! sRGB, OKLab and OKLCH colour with conversions and gamut mapping.
//!
//! [`Srgba`] is the float colour the pickers edit; [`Hsb`] and [`Lab`] are its other spaces.
//!
//! Conversions use Björn Ottosson's OKLab matrices at the full `f64` precision of CSS Color 4, so
//! sRGB white has lightness 1, an sRGB grey has no chroma beyond `f64` rounding, and an sRGB
//! colour survives a round trip through OKLCH unchanged.

/// An sRGB colour with 8-bit channels and unmultiplied alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba(pub [u8; 4]);

/// A colour in OKLab: lightness `l` in 0..=1, opponent axes `a` (green-red) and `b` (blue-yellow).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oklab {
    /// Perceptual lightness, 0 (black) to 1 (white).
    pub l: f64,
    /// Green-red axis.
    pub a: f64,
    /// Blue-yellow axis.
    pub b: f64,
}

/// A colour in OKLCH: OKLab in polar form. Used for building ramps and moving lightness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oklch {
    /// Perceptual lightness, 0 (black) to 1 (white).
    pub l: f64,
    /// Chroma, 0 for greys; sRGB tops out near 0.32.
    pub c: f64,
    /// Hue angle in degrees, 0..360.
    pub h: f64,
}

/// Linear-light sRGB, each channel 0..=1 when in gamut.
type Linear = [f64; 3];

/// Slack when testing whether a linear channel is inside 0..=1.
const GAMUT_EPS: f64 = 1e-6;

/// The OKLCH chroma below which a colour is a grey, which [`Oklab::to_oklch`] gives chroma 0 and
/// hue 0. sRGB greys convert to a chroma of `f64` rounding (up to about 1e-15) in an arbitrary
/// hue, and colour derivation keeps a colour's hue and chroma while it moves the lightness: a
/// lightness search that stops where a contrast test flips lands between the points where the
/// channels of a tinted colour round to their next 8-bit value, so a residual tint becomes a step
/// in one channel. Every 8-bit colour that is not a grey has a chroma above 0.001 (`#feffff`
/// has the least), the resolution of a picker's chroma field.
pub const ACHROMATIC: f64 = 1e-6;

impl Rgba {
    /// Opaque black.
    pub const BLACK: Rgba = Rgba::rgb(0, 0, 0);
    /// Opaque white.
    pub const WHITE: Rgba = Rgba::rgb(255, 255, 255);
    /// Fully transparent.
    pub const TRANSPARENT: Rgba = Rgba([0, 0, 0, 0]);

    /// An opaque colour from channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba([r, g, b, 255])
    }

    /// Parses `#RRGGBB` or `#RRGGBBAA` (case-insensitive, `#` required).
    pub fn from_hex(s: &str) -> Option<Rgba> {
        let digits = s.strip_prefix('#')?;
        if !digits.is_ascii() || !(digits.len() == 6 || digits.len() == 8) {
            return None;
        }
        let mut out = [255u8; 4];
        for (i, slot) in out.iter_mut().enumerate().take(digits.len() / 2) {
            *slot = u8::from_str_radix(digits.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        Some(Rgba(out))
    }

    /// Lowercase `#rrggbb`, or `#rrggbbaa` when not opaque.
    pub fn to_hex(self) -> String {
        let [r, g, b, a] = self.0;
        if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
    }

    /// The alpha channel, 0..=255.
    pub const fn a(self) -> u8 {
        self.0[3]
    }

    /// Linear-light channels (alpha ignored).
    pub fn to_linear(self) -> [f64; 3] {
        [srgb_to_linear(self.0[0]), srgb_to_linear(self.0[1]), srgb_to_linear(self.0[2])]
    }

    /// This colour in OKLab (alpha ignored).
    pub fn to_oklab(self) -> Oklab {
        Oklab::from_linear(self.to_linear())
    }

    /// This colour in OKLCH (alpha ignored).
    pub fn to_oklch(self) -> Oklch {
        self.to_oklab().to_oklch()
    }

    /// The same colour with a different alpha.
    pub const fn with_alpha(self, a: u8) -> Rgba {
        Rgba([self.0[0], self.0[1], self.0[2], a])
    }

    /// The same colour with its alpha multiplied by `factor`, rounded as egui's
    /// `Color32::gamma_multiply` rounds it.
    pub fn faded(self, factor: f32) -> Rgba {
        self.with_alpha((f32::from(self.0[3]) * factor + 0.5) as u8)
    }
}

/// egui's colours are premultiplied, so a translucent one comes back to its straight channels only
/// as closely as 8 bits allow, and a fully transparent one is transparent black.
impl From<ecolor::Color32> for Rgba {
    fn from(c: ecolor::Color32) -> Rgba {
        Rgba(c.to_srgba_unmultiplied())
    }
}

/// Decodes one sRGB channel to linear light.
pub fn srgb_to_linear(v: u8) -> f64 {
    srgb_decode(f64::from(v) / 255.0)
}

/// Decodes an encoded sRGB value in 0..=1 to linear light.
pub fn srgb_decode(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Encodes linear light as an sRGB value in 0..=1 (not clamped).
pub fn linear_to_srgb(v: f64) -> f64 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

fn to_u8(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn in_gamut(lin: Linear) -> bool {
    lin.iter().all(|&c| (-GAMUT_EPS..=1.0 + GAMUT_EPS).contains(&c))
}

impl Oklab {
    /// From linear-light sRGB.
    pub fn from_linear([r, g, b]: [f64; 3]) -> Oklab {
        let l = (0.412_221_469_470_762_9 * r + 0.536_332_537_261_734_9 * g + 0.051_445_993_267_502_2 * b).cbrt();
        let m = (0.211_903_495_817_825_1 * r + 0.680_699_550_645_234_4 * g + 0.107_396_953_536_940_6 * b).cbrt();
        let s = (0.088_302_459_190_056_4 * r + 0.281_718_839_136_121_5 * g + 0.629_978_701_673_822_2 * b).cbrt();
        Oklab {
            l: 0.210_454_268_309_314 * l + 0.793_617_774_702_305_4 * m - 0.004_072_043_011_619_3 * s,
            a: 1.977_998_532_431_168_4 * l - 2.428_592_242_048_58 * m + 0.450_593_709_617_411 * s,
            b: 0.025_904_042_465_547_8 * l + 0.782_771_712_457_529_6 * m - 0.808_675_754_923_077_4 * s,
        }
    }

    /// To linear-light sRGB; channels may fall outside 0..=1 for out-of-gamut colours.
    pub fn to_linear(self) -> [f64; 3] {
        let l = (self.l + 0.396_337_777_376_174_9 * self.a + 0.215_803_757_309_913_6 * self.b).powi(3);
        let m = (self.l - 0.105_561_345_815_658_6 * self.a - 0.063_854_172_825_813_3 * self.b).powi(3);
        let s = (self.l - 0.089_484_177_529_811_9 * self.a - 1.291_485_548_019_409_2 * self.b).powi(3);
        [
            4.076_741_636_075_958 * l - 3.307_711_539_258_063 * m + 0.230_969_903_182_104_3 * s,
            -1.268_437_973_285_031_5 * l + 2.609_757_349_287_688 * m - 0.341_319_376_002_657 * s,
            -0.004_196_076_138_675_6 * l - 0.703_418_617_935_936_3 * m + 1.707_614_694_074_611_7 * s,
        ]
    }

    /// Polar form. A colour with chroma below [`ACHROMATIC`] is a grey, with chroma 0 and hue 0.
    pub fn to_oklch(self) -> Oklch {
        let c = self.a.hypot(self.b);
        if c < ACHROMATIC {
            return Oklch { l: self.l, c: 0.0, h: 0.0 };
        }
        Oklch { l: self.l, c, h: self.b.atan2(self.a).to_degrees().rem_euclid(360.0) }
    }
}

impl Oklch {
    /// This colour in OKLab.
    pub fn to_oklab(self) -> Oklab {
        let (sin, cos) = self.h.to_radians().sin_cos();
        Oklab { l: self.l, a: self.c * cos, b: self.c * sin }
    }

    /// Whether the colour is inside the sRGB gamut.
    pub fn in_gamut(self) -> bool {
        in_gamut(self.to_oklab().to_linear())
    }

    /// Maps into sRGB by reducing chroma at fixed lightness and hue (binary search for the
    /// largest in-gamut chroma, as in CSS Color 4 without its final clip). Lightness is clamped
    /// to 0..=1 first, so the result is always in gamut.
    pub fn gamut_map(self) -> Oklch {
        let l = self.l.clamp(0.0, 1.0);
        let mut out = Oklch { l, c: self.c.max(0.0), h: self.h };
        if out.in_gamut() {
            return out;
        }
        let (mut lo, mut hi) = (0.0, out.c);
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if (Oklch { c: mid, ..out }).in_gamut() {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        out.c = lo;
        out
    }

    /// Gamut-maps and encodes as opaque sRGB.
    pub fn to_rgba(self) -> Rgba {
        let lin = self.gamut_map().to_oklab().to_linear();
        Rgba::rgb(to_u8(linear_to_srgb(lin[0])), to_u8(linear_to_srgb(lin[1])), to_u8(linear_to_srgb(lin[2])))
    }
}

impl From<Rgba> for Oklch {
    fn from(c: Rgba) -> Oklch {
        c.to_oklch()
    }
}

/// An encoded sRGB colour with float channels in 0..=1 and straight alpha: the value a colour
/// picker edits. Eight bits lose the precision that HSB and OKLCH edits need.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Srgba {
    /// Red, encoded.
    pub r: f32,
    /// Green, encoded.
    pub g: f32,
    /// Blue, encoded.
    pub b: f32,
    /// Alpha, straight.
    pub a: f32,
}

/// A colour in HSB (HSV) computed on encoded sRGB, as Photoshop does. egui's `Hsva` works in
/// linear light and puts 50 % grey at 22 % brightness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsb {
    /// Hue in degrees, 0..360; 0 for greys.
    pub h: f32,
    /// Saturation, 0..=1.
    pub s: f32,
    /// Brightness, 0..=1.
    pub b: f32,
}

/// A colour in CIE L*a*b* relative to D50.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lab {
    /// Lightness, 0..=100.
    pub l: f32,
    /// Green-red axis, about -128..=127 for sRGB.
    pub a: f32,
    /// Blue-yellow axis.
    pub b: f32,
}

/// Linear sRGB to D50 XYZ, Bradford-adapted as in the ICC sRGB profile.
const SRGB_TO_XYZ_D50: [[f64; 3]; 3] =
    [[0.436_074_7, 0.385_064_9, 0.143_080_4], [0.222_504_5, 0.716_878_6, 0.060_616_9], [0.013_932_2, 0.097_104_5, 0.714_173_3]];
/// D50 XYZ to linear sRGB.
const XYZ_D50_TO_SRGB: [[f64; 3]; 3] =
    [[3.133_856_1, -1.616_866_7, -0.490_614_6], [-0.978_768_4, 1.916_141_5, 0.033_454_0], [0.071_945_3, -0.228_991_4, 1.405_242_7]];
/// The D50 white point (ICC PCS).
const D50: [f64; 3] = [0.964_22, 1.0, 0.825_21];
const LAB_EPS: f64 = 216.0 / 24389.0;
const LAB_KAPPA: f64 = 24389.0 / 27.0;

fn mat_mul(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    m.map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
}

fn lab_f(t: f64) -> f64 {
    if t > LAB_EPS { t.cbrt() } else { (LAB_KAPPA * t + 16.0) / 116.0 }
}

fn lab_finv(f: f64) -> f64 {
    let f3 = f * f * f;
    if f3 > LAB_EPS { f3 } else { (116.0 * f - 16.0) / LAB_KAPPA }
}

fn encode_clamped(lin: f64) -> f32 {
    linear_to_srgb(lin.clamp(0.0, 1.0)) as f32
}

impl Srgba {
    /// Opaque black.
    pub const BLACK: Srgba = Srgba::rgb(0.0, 0.0, 0.0);
    /// Opaque white.
    pub const WHITE: Srgba = Srgba::rgb(1.0, 1.0, 1.0);

    /// An opaque colour from encoded channels.
    pub const fn rgb(r: f32, g: f32, b: f32) -> Srgba {
        Srgba { r, g, b, a: 1.0 }
    }

    /// A colour from encoded channels and alpha.
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Srgba {
        Srgba { r, g, b, a }
    }

    /// The same colour with a different alpha.
    pub const fn with_alpha(self, a: f32) -> Srgba {
        Srgba { a, ..self }
    }

    /// The colour rounded to 8 bits per channel.
    pub fn to_rgba(self) -> Rgba {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Rgba([q(self.r), q(self.g), q(self.b), q(self.a)])
    }

    /// Whether both colours have the same 8-bit red, green and blue (alpha ignored).
    pub fn same_rgb8(self, other: Srgba) -> bool {
        self.to_rgba().0[..3] == other.to_rgba().0[..3]
    }

    fn linear(self) -> [f64; 3] {
        [self.r, self.g, self.b].map(|v| srgb_decode(f64::from(v)))
    }

    /// This colour in HSB.
    pub fn to_hsb(self) -> Hsb {
        let (mx, mn) = (self.r.max(self.g).max(self.b), self.r.min(self.g).min(self.b));
        let d = mx - mn;
        let sextant = if d <= 0.0 {
            0.0
        } else if mx == self.r {
            ((self.g - self.b) / d).rem_euclid(6.0)
        } else if mx == self.g {
            (self.b - self.r) / d + 2.0
        } else {
            (self.r - self.g) / d + 4.0
        };
        Hsb { h: 60.0 * sextant, s: if mx > 0.0 { d / mx } else { 0.0 }, b: mx }
    }

    /// An opaque colour from HSB; the hue wraps and saturation and brightness clamp to 0..=1.
    pub fn from_hsb(hsb: Hsb) -> Srgba {
        let sextant = (hsb.h / 60.0).rem_euclid(6.0);
        let (s, v) = (hsb.s.clamp(0.0, 1.0), hsb.b.clamp(0.0, 1.0));
        let chroma = v * s;
        let x = chroma * (1.0 - (sextant.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match sextant as u32 {
            0 => (chroma, x, 0.0),
            1 => (x, chroma, 0.0),
            2 => (0.0, chroma, x),
            3 => (0.0, x, chroma),
            4 => (x, 0.0, chroma),
            _ => (chroma, 0.0, x),
        };
        let m = v - chroma;
        Srgba::rgb(r + m, g + m, b + m)
    }

    /// This colour in CIE L*a*b* (D50, Bradford-adapted).
    pub fn to_lab(self) -> Lab {
        let xyz = mat_mul(&SRGB_TO_XYZ_D50, self.linear());
        let [fx, fy, fz] = [0, 1, 2].map(|i| lab_f(xyz[i] / D50[i]));
        Lab { l: (116.0 * fy - 16.0) as f32, a: (500.0 * (fx - fy)) as f32, b: (200.0 * (fy - fz)) as f32 }
    }

    /// An opaque colour from CIE L*a*b* (D50). A colour outside sRGB clamps each linear channel,
    /// as PhotoCraft's Lab field does.
    pub fn from_lab(lab: Lab) -> Srgba {
        let fy = (f64::from(lab.l) + 16.0) / 116.0;
        let fx = fy + f64::from(lab.a) / 500.0;
        let fz = fy - f64::from(lab.b) / 200.0;
        let xyz = [lab_finv(fx) * D50[0], lab_finv(fy) * D50[1], lab_finv(fz) * D50[2]];
        let [r, g, b] = mat_mul(&XYZ_D50_TO_SRGB, xyz);
        Srgba::rgb(encode_clamped(r), encode_clamped(g), encode_clamped(b))
    }

    /// This colour in OKLCH.
    pub fn to_oklch(self) -> Oklch {
        Oklab::from_linear(self.linear()).to_oklch()
    }

    /// An opaque colour from OKLCH, mapped into sRGB by chroma reduction ([`Oklch::gamut_map`]).
    pub fn from_oklch(c: Oklch) -> Srgba {
        let [r, g, b] = c.gamut_map().to_oklab().to_linear();
        Srgba::rgb(encode_clamped(r), encode_clamped(g), encode_clamped(b))
    }
}

impl From<Rgba> for Srgba {
    fn from(c: Rgba) -> Srgba {
        let [r, g, b, a] = c.0.map(|v| f32::from(v) / 255.0);
        Srgba { r, g, b, a }
    }
}

impl From<Srgba> for Rgba {
    fn from(c: Srgba) -> Rgba {
        c.to_rgba()
    }
}

impl From<ecolor::Color32> for Srgba {
    fn from(c: ecolor::Color32) -> Srgba {
        Rgba::from(c).into()
    }
}

impl From<Srgba> for ecolor::Color32 {
    fn from(c: Srgba) -> ecolor::Color32 {
        let [r, g, b, a] = c.to_rgba().0;
        ecolor::Color32::from_rgba_unmultiplied(r, g, b, a)
    }
}

/// Hex text and float arrays for the two colour types (F04.09): the one set of conversions that
/// replaces the apps' near-identical `hex`, `parse_hex`, `rgb3` and `c32` helpers. `#` is
/// optional on input and written on output; three-digit `#RGB` doubles each digit (web colours);
/// six or eight digits are `#RRGGBB[AA]`. Output is lower-case.
impl Srgba {
    /// Parses `#RGB`, `#RRGGBB` or `#RRGGBBAA` (the `#` optional, case-insensitive); alpha 1 when
    /// not given. `None` for anything else.
    // pc: crates/ui-egui/src/dialogs/color_picker_ui.rs:64 `parse_hex`, plus `Rgba::from_hex`.
    pub fn from_hex(text: &str) -> Option<Srgba> {
        let h = text.trim().trim_start_matches('#');
        // `from_str_radix` accepts a leading '+', so require plain hex digits first.
        if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let digit = |i: usize| u8::from_str_radix(h.get(i..=i)?, 16).ok();
        let pair = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        let (r, g, b, a) = match h.len() {
            3 => (digit(0)? * 17, digit(1)? * 17, digit(2)? * 17, 255),
            6 => (pair(0)?, pair(2)?, pair(4)?, 255),
            8 => (pair(0)?, pair(2)?, pair(4)?, pair(6)?),
            _ => return None,
        };
        let f = |v: u8| f32::from(v) / 255.0;
        Some(Srgba { r: f(r), g: f(g), b: f(b), a: f(a) })
    }

    /// `#rrggbb`, or `#rrggbbaa` when the alpha is below 1.
    // pc: crates/ui-egui/src/dialogs/color_picker_ui.rs:59 `hex`.
    pub fn to_hex(self) -> String {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        if self.a < 1.0 {
            format!("#{:02x}{:02x}{:02x}{:02x}", q(self.r), q(self.g), q(self.b), q(self.a))
        } else {
            format!("#{:02x}{:02x}{:02x}", q(self.r), q(self.g), q(self.b))
        }
    }

    /// The encoded channels without alpha, for engines that take `[f32; 3]`.
    pub fn to_rgb_array(self) -> [f32; 3] {
        [self.r, self.g, self.b]
    }

    /// An opaque colour from encoded channels.
    pub fn from_rgb_array([r, g, b]: [f32; 3]) -> Srgba {
        Srgba::rgb(r, g, b)
    }

    /// The encoded channels with alpha.
    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// A colour from encoded channels with alpha.
    pub fn from_array([r, g, b, a]: [f32; 4]) -> Srgba {
        Srgba::new(r, g, b, a)
    }
}

impl Rgba {
    /// The channels as `[f32; 3]` in 0–1, without alpha.
    pub fn to_rgb_array(self) -> [f32; 3] {
        Srgba::from(self).to_rgb_array()
    }

    /// An opaque colour from 0–1 channels, rounded to 8 bits.
    pub fn from_rgb_array(rgb: [f32; 3]) -> Rgba {
        Srgba::from_rgb_array(rgb).to_rgba()
    }
}
