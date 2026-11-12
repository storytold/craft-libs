//! Tolerant readers and writers of plain values.

use crate::Json;

/// Rounds to six decimals, the precision values are written with; a non-finite value becomes 0.
pub fn round6(v: f64) -> f64 {
    if v.is_finite() { (v * 1e6).round() / 1e6 } else { 0.0 }
}

/// The numbers of a value: `5`, `[5]`, `[1, 2, 3]`, or the same wrapped once more (`[[x, y]]`,
/// as a key's `s` may be). `None` when anything in it is not a number.
pub fn nums(j: &Json) -> Option<Vec<f64>> {
    match j {
        Json::Number(n) => Some(vec![n.as_f64()?]),
        Json::Array(a) => {
            if a.len() == 1
                && let Some(inner @ Json::Array(_)) = a.first()
            {
                return nums(inner);
            }
            a.iter().map(Json::as_f64).collect()
        }
        _ => None,
    }
}

/// A colour as `[r, g, b, a]` in 0..=1. Missing components are 0 and a missing alpha is 1. Some
/// files write 0..=255: when a colour component is above 1, the three are divided by 255.
pub fn color(j: &Json) -> Option<[f64; 4]> {
    let v = nums(j)?;
    let g = |i: usize, d: f64| v.get(i).copied().unwrap_or(d);
    let mut c = [*v.first()?, g(1, 0.0), g(2, 0.0), g(3, 1.0)];
    if c.iter().take(3).any(|x| *x > 1.0) {
        for x in c.iter_mut().take(3) {
            *x /= 255.0;
        }
    }
    Some(c)
}

/// A `#rrggbb` colour (a solid layer's `sc`) as components in 0..=1. A pair that is not two hex
/// digits is 0.
pub fn hex_color(sc: &str) -> [f32; 3] {
    let h = sc.trim_start_matches('#');
    let c = |i: usize| {
        let pair = h.get(i..i + 2).filter(|x| x.bytes().all(|b| b.is_ascii_hexdigit()));
        f32::from(pair.and_then(|x| u8::from_str_radix(x, 16).ok()).unwrap_or(0)) / 255.0
    };
    [c(0), c(2), c(4)]
}

/// Components in 0..=1 as `#rrggbb`.
pub fn to_hex(c: [f32; 3]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let [r, g, bl] = c;
    format!("#{:02x}{:02x}{:02x}", b(r), b(g), b(bl))
}

/// The image type `bytes` hold, by their whole signature; `None` for anything else. These are
/// the types an image asset may embed.
pub fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}
