use std::fmt;

/// An sRGB colour with unmultiplied alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    pub const BLACK: Rgba = Rgba::rgb(0, 0, 0);
    pub const WHITE: Rgba = Rgba::rgb(0xFF, 0xFF, 0xFF);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba([r, g, b, 0xFF])
    }

    /// An opaque colour from `0xRRGGBB`; higher bits are ignored.
    pub const fn from_rgb_u32(rgb: u32) -> Rgba {
        let [_, r, g, b] = rgb.to_be_bytes();
        Rgba::rgb(r, g, b)
    }

    pub const fn r(self) -> u8 {
        self.0[0]
    }

    pub const fn g(self) -> u8 {
        self.0[1]
    }

    pub const fn b(self) -> u8 {
        self.0[2]
    }

    pub const fn a(self) -> u8 {
        self.0[3]
    }

    pub const fn is_opaque(self) -> bool {
        self.0[3] == 0xFF
    }

    /// `#rrggbb` or `#rrggbbaa`, either case; the `#` is optional.
    pub fn parse_hex(s: &str) -> Option<Rgba> {
        let digits = s.strip_prefix('#').unwrap_or(s);
        if !matches!(digits.len(), 6 | 8) || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let mut out = [0xFF; 4];
        for (slot, i) in out.iter_mut().zip((0..digits.len()).step_by(2)) {
            *slot = u8::from_str_radix(digits.get(i..i + 2)?, 16).ok()?;
        }
        Some(Rgba(out))
    }

    /// `#rrggbb` for an opaque colour, `#rrggbbaa` otherwise.
    pub fn to_hex(self) -> String {
        let [r, g, b, a] = self.0;
        if a == 0xFF { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
    }

    /// Per-channel interpolation of the encoded values (alpha included), `t` clamped to 0..=1.
    pub fn lerp_gamma(self, to: Rgba, t: f32) -> Rgba {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        let mut out = self.0;
        for (o, (&a, &b)) in out.iter_mut().zip(self.0.iter().zip(to.0.iter())) {
            *o = channel(f32::from(a) + (f32::from(b) - f32::from(a)) * t);
        }
        Rgba(out)
    }

    /// Like [`Rgba::lerp_gamma`] with `-t`: the same distance away from `from`, clamped per channel.
    pub(crate) fn step_away(self, from: Rgba, t: f32) -> Rgba {
        let mut out = self.0;
        for (o, (&a, &b)) in out.iter_mut().zip(self.0.iter().zip(from.0.iter())) {
            *o = channel(f32::from(a) - (f32::from(b) - f32::from(a)) * t);
        }
        out[3] = self.0[3];
        Rgba(out)
    }

    /// This colour painted over `bg` (source-over, mixed in encoded sRGB as egui does).
    pub fn over(self, bg: Rgba) -> Rgba {
        let fa = f32::from(self.a()) / 255.0;
        let ba = f32::from(bg.a()) / 255.0;
        let out_a = fa + ba * (1.0 - fa);
        if out_a <= 0.0 {
            return Rgba([0, 0, 0, 0]);
        }
        let mut out = [0; 4];
        for (o, (&f, &b)) in out.iter_mut().zip(self.0.iter().zip(bg.0.iter())).take(3) {
            *o = channel((f32::from(f) * fa + f32::from(b) * ba * (1.0 - fa)) / out_a);
        }
        out[3] = channel(out_a * 255.0);
        Rgba(out)
    }

    /// WCAG 2 relative luminance of the colour's RGB, ignoring alpha.
    pub(crate) fn luminance(self) -> f32 {
        fn linear(c: u8) -> f32 {
            let c = f32::from(c) / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        }
        0.2126 * linear(self.r()) + 0.7152 * linear(self.g()) + 0.0722 * linear(self.b())
    }
}

fn channel(v: f32) -> u8 {
    // `as` saturates and maps NaN to 0.
    (v + 0.5).floor().clamp(0.0, 255.0) as u8
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Rgba {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Rgba {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        Rgba::parse_hex(&s).ok_or_else(|| serde::de::Error::custom(format!("not a #rrggbb or #rrggbbaa colour: {s:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip_and_rejects() {
        assert_eq!(Rgba::parse_hex("#121316"), Some(Rgba::from_rgb_u32(0x121316)));
        assert_eq!(Rgba::parse_hex("2D81FF80"), Some(Rgba([0x2D, 0x81, 0xFF, 0x80])));
        assert_eq!(Rgba([0x2D, 0x81, 0xFF, 0x80]).to_hex(), "#2d81ff80");
        assert_eq!(Rgba::from_rgb_u32(0xABCDEF).to_hex(), "#abcdef");
        for bad in ["", "#", "#12345", "#1234567", "#12345g", "#+12345", "#ééé", "#123456789"] {
            assert_eq!(Rgba::parse_hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn mixing() {
        let a = Rgba::rgb(0, 100, 200);
        let b = Rgba::rgb(100, 200, 0);
        assert_eq!(a.lerp_gamma(b, 0.5), Rgba::rgb(50, 150, 100));
        assert_eq!(a.lerp_gamma(b, 2.0), b);
        assert_eq!(a.lerp_gamma(b, f32::NAN), a);
        assert_eq!(Rgba::rgb(100, 100, 100).step_away(Rgba::rgb(200, 0, 100), 0.5), Rgba::rgb(50, 150, 100));
        assert_eq!(Rgba([255, 255, 255, 128]).over(Rgba::BLACK), Rgba::rgb(128, 128, 128));
        assert_eq!(Rgba([0, 0, 0, 0]).over(Rgba([0, 0, 0, 0])), Rgba([0, 0, 0, 0]));
    }
}
