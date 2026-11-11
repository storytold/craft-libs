//! CSS colours, the paint syntax of SVG: parsing ([`parse_color`]) and the canonical
//! `#rrggbb` form ([`Color::to_css`]), with luminance and blending for palette work.
//!
//! The grammar is a subset of <https://www.w3.org/TR/css-color-4/>: hex, `rgb()`/`hsl()` and
//! the named colours. Alpha is checked and dropped: a [`Color`] is opaque.

/// An 8-bit sRGB color, no alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b }
    }

    /// Canonical CSS form: lowercase `#rrggbb`.
    pub fn to_css(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Perceived (relative) luminance in `[0, 1]`, the WCAG weighting of
    /// sRGB channels. 0 is black, 1 is white.
    pub fn luminance(&self) -> f32 {
        let r = channel_value(self.r);
        let g = channel_value(self.g);
        let b = channel_value(self.b);
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// Blend from `self` to `other` by `t` in `[0, 1]`: `t = 0.5` is a
    /// plain midpoint, not perceptually flat. `t` outside the range is
    /// clamped, so UI dials can pass anything.
    pub fn blend(self, other: Color, t: f32) -> Color {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        Color { r: lerp_channel(self.r, other.r, t), g: lerp_channel(self.g, other.g, t), b: lerp_channel(self.b, other.b, t) }
    }
}

fn channel_value(v: u8) -> f32 {
    let c = f32::from(v) / 255.0;
    // WCAG 2.x "relative luminance" is plain channel weighting; css-color-4
    // applies the sRGB transfer function first. We use the WCAG form, which
    // is what palette contrast helpers want.
    c
}

fn lerp_channel(a: u8, b: u8, t: f32) -> u8 {
    let l = f32::from(a) + (f32::from(b) - f32::from(a)) * t;
    (l + 0.5).clamp(0.0, 255.0) as u8
}

/// Why a color string could not be parsed. `Display` names the offending
/// input so callers can echo it without re-wrapping.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid color {input:?}")]
pub struct ColorParseError {
    input: String,
}

/// Parse a CSS color string (<https://www.w3.org/TR/css-color-4/>, which SVG 1.1 §4.2
/// and SVG 2 paint both defer to): the 148 named colours, `#rgb`, `#rgba`, `#rrggbb`,
/// `#rrggbbaa`, `rgb()`/`rgba()` with integer 0–255 or percentage channels, and
/// `hsl()`/`hsla()`, in comma or space syntax with an optional alpha (`/ a` or a fourth
/// comma value). Alpha is checked and then dropped.
///
/// Anything else — `transparent`, `currentColor`, `color()`, out-of-range channels —
/// is an error, not a guess: silent color loss is worse than a reported failure.
/// Callers that understand `none`/`currentColor` (SVG paint) resolve them first.
pub fn parse_color(input: &str) -> Result<Color, ColorParseError> {
    let err = || ColorParseError { input: input.to_string() };
    // Cap input length before work: a caller feeding untrusted text cannot
    // make this quadratic.
    if input.len() > 64 {
        return Err(err());
    }
    let lower = input.trim().to_ascii_lowercase();
    if let Some(digits) = lower.strip_prefix('#') {
        return parse_hex(digits.trim()).ok_or_else(err);
    }
    if let Some((name, rest)) = lower.split_once('(') {
        return parse_function(name.trim(), rest).ok_or_else(err);
    }
    NAMED.iter().find(|(name, _)| *name == lower).map(|(_, rgb)| Color::new((rgb >> 16) as u8, (rgb >> 8) as u8, *rgb as u8)).ok_or_else(err)
}

/// Hex digits after `#`: 3 or 4 (each doubled), 6 or 8; the alpha digits are dropped.
fn parse_hex(digits: &str) -> Option<Color> {
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = |s: &str| u8::from_str_radix(s, 16).ok();
    match digits.len() {
        3 | 4 => {
            let d = |i: usize| value(digits.get(i..=i)?).map(|v| v * 17);
            Some(Color::new(d(0)?, d(1)?, d(2)?))
        }
        6 | 8 => {
            let d = |i: usize| value(digits.get(i..i + 2)?);
            Some(Color::new(d(0)?, d(2)?, d(4)?))
        }
        _ => None,
    }
}

/// `rgb(…)`, `rgba(…)`, `hsl(…)`, `hsla(…)` after the `(`, which must close with `)`.
fn parse_function(name: &str, rest: &str) -> Option<Color> {
    let body = rest.trim().strip_suffix(')')?;
    // Legacy comma syntax, or modern space syntax with `/ alpha`.
    let parts: Vec<&str> = if body.contains(',') {
        if body.contains('/') {
            return None;
        }
        body.split(',').map(str::trim).collect()
    } else {
        let (channels, alpha) = match body.split_once('/') {
            Some((channels, alpha)) => (channels, Some(alpha.trim())),
            None => (body, None),
        };
        let mut parts: Vec<&str> = channels.split_whitespace().collect();
        parts.extend(alpha);
        parts
    };
    if !(parts.len() == 3 || parts.len() == 4) {
        return None;
    }
    if let Some(alpha) = parts.get(3) {
        let a = match alpha.strip_suffix('%') {
            Some(p) => number(p)? / 100.0,
            None => number(alpha)?,
        };
        if !(0.0..=1.0).contains(&a) {
            return None;
        }
    }
    let (Some(a), Some(b), Some(c)) = (parts.first(), parts.get(1), parts.get(2)) else {
        return None;
    };
    match name {
        "rgb" | "rgba" => Some(Color::new(rgb_channel(a)?, rgb_channel(b)?, rgb_channel(c)?)),
        "hsl" | "hsla" => {
            let hue = number(a.strip_suffix("deg").unwrap_or(a))?;
            Some(hsl_to_rgb(hue.rem_euclid(360.0), percent(b)?, percent(c)?))
        }
        _ => None,
    }
}

/// A finite plain decimal number.
fn number(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit() || b"+-.".contains(&b)) {
        return None;
    }
    text.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// `0%`–`100%` as a fraction.
fn percent(text: &str) -> Option<f64> {
    let p = number(text.strip_suffix('%')?)?;
    (0.0..=100.0).contains(&p).then_some(p / 100.0)
}

/// An `rgb()` channel: an integer 0–255 or a percentage. Out of range is an error.
fn rgb_channel(text: &str) -> Option<u8> {
    if text.ends_with('%') {
        return Some((percent(text)? * 255.0).round() as u8);
    }
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u8>().ok()
}

/// css-color-4 §7.1 HSL to RGB: `hue` in [0, 360), saturation/lightness in [0, 1].
fn hsl_to_rgb(hue: f64, s: f64, l: f64) -> Color {
    let channel = |n: f64| {
        let k = (n + hue / 30.0) % 12.0;
        let a = s * l.min(1.0 - l);
        let v = l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0);
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    Color::new(channel(0.0), channel(8.0), channel(4.0))
}

/// css-color-4 §6.1 named colours as `0xrrggbb`, lowercase.
const NAMED: &[(&str, u32)] = &[
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

impl std::str::FromStr for Color {
    type Err = ColorParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_color(s)
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_css())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::ok_expect)]

    use super::*;

    fn c(input: &str) -> Color {
        parse_color(input).ok().expect("test input should parse")
    }

    #[test]
    fn parses_hex_forms() {
        assert_eq!(c("#e8573f"), Color::new(0xe8, 0x57, 0x3f));
        assert_eq!(c("#E8573F"), Color::new(0xe8, 0x57, 0x3f));
        assert_eq!(c(" #000000 "), Color::new(0, 0, 0));
        assert_eq!(c("#fff"), Color::new(255, 255, 255));
        assert_eq!(c("#000"), Color::new(0, 0, 0));
        assert_eq!(c("#abc"), Color::new(0xaa, 0xbb, 0xcc));
    }

    #[test]
    fn parses_css_keywords() {
        assert_eq!(c("black"), Color::new(0, 0, 0));
        assert_eq!(c("WHITE"), Color::new(255, 255, 255));
        assert_eq!(c("Silver"), Color::new(192, 192, 192));
        assert_eq!(c("cyan"), Color::new(0, 255, 255));
        // css-color-4 keeps both spellings for the two aliases.
        assert_eq!(c("magenta"), c("fuchsia"));
        assert_eq!(c("gray"), c("grey"));
    }

    #[test]
    fn parses_rgb_function() {
        assert_eq!(c("rgb(232, 87, 63)"), Color::new(232, 87, 63));
        assert_eq!(c("rgb(232,87,63)"), Color::new(232, 87, 63));
        assert_eq!(c("rgb(232 87 63)"), Color::new(232, 87, 63));
        // 0–255 only; out-of-range integers are a parse error, not a clamp.
        assert!(parse_color("rgb(256, 0, 0)").is_err());
        assert!(parse_color("rgb(-1, 0, 0)").is_err());
        assert!(parse_color("rgb(1 2)").is_err());
        assert!(parse_color("rgb(1, 2, 3, 4)").is_err());
    }

    #[test]
    fn rejects_incomplete_and_unsupported_forms() {
        assert!(parse_color("").is_err());
        assert!(parse_color("#").is_err());
        assert!(parse_color("#ff").is_err());
        assert!(parse_color("#fffff").is_err());
        assert!(parse_color("#fffffff").is_err());
        assert!(parse_color("transparent").is_err());
        assert!(parse_color("rgba(1, 2, 3, 1.5)").is_err());
        assert!(parse_color("hsl(120, 50, 50%)").is_err());
        assert!(parse_color("rgb(150%, 0%, 0%)").is_err());
        assert!(parse_color("rgb(1, 2 / 3)").is_err());
        assert!(parse_color("currentColor").is_err());
        assert!(parse_color("#ggg").is_err());
        assert!(parse_color("rgb(1, 2").is_err());
        // Nothing from a too-long string is parsed.
        assert!(parse_color(&"k".repeat(65)).is_err());
    }

    #[test]
    fn parses_alpha_percentage_and_hsl_forms() {
        assert_eq!(c("#f008"), Color::new(255, 0, 0));
        assert_eq!(c("#00ff0080"), Color::new(0, 255, 0));
        assert_eq!(c("rgba(1, 2, 3, 0.5)"), Color::new(1, 2, 3));
        assert_eq!(c("rgb(1 2 3 / 50%)"), Color::new(1, 2, 3));
        assert_eq!(c("rgb(100%, 0%, 50%)"), Color::new(255, 0, 128));
        assert_eq!(c("RGB(10,20,30)"), Color::new(10, 20, 30));
        assert_eq!(c("hsl(0, 100%, 50%)"), Color::new(255, 0, 0));
        assert_eq!(c("hsl(120deg 100% 25%)"), Color::new(0, 128, 0));
        assert_eq!(c("hsla(240, 100%, 50%, 0.3)"), Color::new(0, 0, 255));
        assert_eq!(c("hsl(-120, 100%, 50%)"), Color::new(0, 0, 255));
        assert_eq!(c("hsl(0, 0%, 100%)"), Color::new(255, 255, 255));
        assert_eq!(c("rebeccapurple"), Color::new(0x66, 0x33, 0x99));
        assert_eq!(c("CornflowerBlue"), Color::new(0x64, 0x95, 0xed));
        assert_eq!(NAMED.len(), 148);
    }

    #[test]
    fn resolves_fromstr_and_display() {
        let color: Color = "#3f88e8".parse().ok().expect("hex parses");
        assert_eq!(color.to_string(), "#3f88e8");
        // Display is canonical lowercase six-digit.
        assert_eq!(c("#ABC").to_string(), "#aabbcc");
    }

    #[test]
    fn luminance_is_zero_to_one() {
        assert_eq!(c("#000000").luminance(), 0.0);
        assert_eq!(c("#ffffff").luminance(), 1.0);
        let mid = c("#7f7f7f").luminance();
        assert!((0.2..0.7).contains(&mid), "middle gray ≈ {mid}");
        // Refined weighting only shifts one step, never flips extremes.
        assert!(c("#e8573f").luminance() < c("#ffffff").luminance());
    }

    #[test]
    fn blend_interpolates_and_clamps() {
        let black = c("#000000");
        let white = c("#ffffff");
        assert_eq!(black.blend(white, 0.0), black);
        assert_eq!(black.blend(white, 1.0), white);
        assert_eq!(black.blend(white, 0.5), c("#808080"));
        // Out-of-range and non-finite t are clamped, never wrap.
        assert_eq!(black.blend(white, 5.0), white);
        assert_eq!(black.blend(white, f32::NAN), black);
    }
}
