//! Codecs of IDML attribute values.

/// A number as IDML writes it: at most nine decimals, no trailing zeros, `0` for anything that
/// is not finite.
pub fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let r = (v * 1e9).round() / 1e9;
    if r == r.trunc() && r.abs() < 1e15 {
        let i = r as i64;
        return i.to_string();
    }
    let s = format!("{r}");
    if s == "-0" { "0".into() } else { s }
}

/// A point: `x y`.
pub fn point_str(x: f64, y: f64) -> String {
    format!("{} {}", num(x), num(y))
}

/// The numbers of a whitespace-separated list; empty when any of them is NaN or infinite
/// (dropping just those would shift the rest).
pub fn nums(s: &str) -> Vec<f64> {
    let v: Vec<f64> = s.split_whitespace().filter_map(|v| v.parse().ok()).collect();
    if v.iter().all(|x| x.is_finite()) { v } else { Vec::new() }
}

/// An `ItemTransform`: the six coefficients `a b c d tx ty`, or `None` when the value is not
/// six finite numbers.
pub fn matrix(s: &str) -> Option<[f64; 6]> {
    <[f64; 6]>::try_from(nums(s)).ok()
}

/// A transform as an `ItemTransform` value.
pub fn matrix_str(m: &[f64; 6]) -> String {
    m.iter().map(|v| num(*v)).collect::<Vec<_>>().join(" ")
}

/// A point (`x y`; further numbers are ignored), or `None` when the value has fewer than two
/// numbers or one that is not finite.
pub fn point(s: &str) -> Option<(f64, f64)> {
    match nums(s).as_slice() {
        [x, y, ..] => Some((*x, *y)),
        _ => None,
    }
}

/// An IDML tint (percent; negative means the colour's own tint, 100 %) as 0..=1.
pub fn tint(percent: f64) -> f32 {
    if percent < 0.0 || percent.is_nan() { 1.0 } else { (percent / 100.0).min(1.0) as f32 }
}

/// A name inside a `Self` id: `%` → `%25`, `:` → `%3a`.
pub fn escape_id(s: &str) -> String {
    s.replace('%', "%25").replace(':', "%3a")
}

/// The reverse of [`escape_id`].
pub fn unescape_id(s: &str) -> String {
    s.replace("%3a", ":").replace("%3A", ":").replace("%25", "%")
}

/// A `Self` id for a number: `u` and its hex digits.
pub fn uid(n: u64) -> String {
    format!("u{n:x}")
}

/// The font family of an `AppliedFont` value. InDesign can write the family with its `$ID/`
/// prefix (`$ID/Arial`), and `$ID/` alone for no font; some writers append the style after a
/// tab. Spaces are part of a family's name.
pub fn font_family(v: &str) -> Option<String> {
    let v = v.trim_matches(['\n', '\r']);
    let family = v.split('\t').next().unwrap_or(v);
    let family = family.trim_start().strip_prefix("$ID/").unwrap_or(family);
    (!family.trim().is_empty()).then(|| family.to_string())
}

/// A style name as an IDML `Name`. `builtins` pairs a caller's name for a built-in style with
/// its IDML name (`("[None]", "$ID/[No character style]")`). Any other name has its groups
/// turned around (`A/B` → `A:B`), with a literal `:` or `\` escaped as `\:` or `\\` so it is not
/// read back as a group separator.
pub fn style_name_out(builtins: &[(&str, &str)], name: &str) -> String {
    builtins
        .iter()
        .find(|(o, _)| *o == name)
        .map(|(_, i)| i.to_string())
        .unwrap_or_else(|| name.replace('\\', "\\\\").replace(':', "\\:").replace('/', ":"))
}

/// An IDML `Name` as the caller's style name: the reverse of [`style_name_out`].
pub fn style_name_in(builtins: &[(&str, &str)], name: &str) -> String {
    if let Some((o, _)) = builtins.iter().find(|(_, i)| *i == name) {
        return o.to_string();
    }
    let name = name.strip_prefix("$ID/").unwrap_or(name);
    let mut out = String::with_capacity(name.len());
    let mut chars = name.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.clone().next() {
                Some(next @ ('\\' | ':')) => {
                    out.push(next);
                    chars.next();
                }
                _ => out.push('\\'),
            },
            ':' => out.push('/'),
            _ => out.push(c),
        }
    }
    out
}

/// The `Self` id of a style: `kind` (`ParagraphStyle`, `CharacterStyle`, …), `/`, and its IDML
/// name escaped.
pub fn style_self(kind: &str, builtins: &[(&str, &str)], name: &str) -> String {
    format!("{kind}/{}", escape_id(&style_name_out(builtins, name)))
}

/// The value of property `key` in an XMP packet, written as an element (`<xmp:CreateDate>…<`) or
/// as an attribute (`xmp:CreateDate="…"`).
pub fn xmp_value<'a>(xmp: &'a str, key: &str) -> Option<&'a str> {
    if let Some(i) = xmp.find(&format!("<{key}>")) {
        let rest = xmp.get(i + key.len() + 2..)?;
        rest.get(..rest.find('<')?)
    } else {
        let i = xmp.find(&format!("{key}=\""))?;
        let rest = xmp.get(i + key.len() + 2..)?;
        rest.get(..rest.find('"')?)
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 with a line break every 76 characters, as embedded image `Contents` are written.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4 + data.len() / 57);
    let digit = |n: u32, shift: u32| char::from(B64[(n >> shift) as usize & 63]);
    for (i, c) in data.chunks(3).enumerate() {
        if i > 0 && i % 19 == 0 {
            out.push('\n');
        }
        let byte = |k: usize| u32::from(c.get(k).copied().unwrap_or(0));
        let n = byte(0) << 16 | byte(1) << 8 | byte(2);
        out.push(digit(n, 18));
        out.push(digit(n, 12));
        out.push(if c.len() > 1 { digit(n, 6) } else { '=' });
        out.push(if c.len() > 2 { digit(n, 0) } else { '=' });
    }
    out
}

/// Lenient base64 decoding: whitespace and any other character outside the alphabet is skipped,
/// and the URL-safe `-` and `_` are read too.
pub fn base64_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for b in s.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        };
        buf = buf << 6 | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}

/// Decode `%XX` escapes; anything else (including a malformed escape) is kept as written.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |i: usize| b.get(i).and_then(|c| char::from(*c).to_digit(16));
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'%'
            && let (Some(h), Some(l)) = (hex(i + 1), hex(i + 2))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// An IDML link URI as a file system path (`file:/a%20b` → `/a b`, `file:///C:/x` → `C:/x`). A
/// URI naming another computer keeps it as a network path (`file://host/s/x` → `//host/s/x`).
pub fn uri_to_path(uri: &str) -> String {
    let rest = match uri.strip_prefix("file://") {
        Some(rest) => {
            let host_end = rest.find(['/', '\\']).unwrap_or(rest.len());
            let host = rest.get(..host_end).unwrap_or_default();
            if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
                rest.get(host_end..).unwrap_or(rest)
            } else if matches!(host.as_bytes(), [letter, b':'] if letter.is_ascii_alphabetic()) {
                rest
            } else {
                return format!("//{}", percent_decode(rest));
            }
        }
        None => uri.strip_prefix("file:").unwrap_or(uri),
    };
    // `file:///C:/…` → `/C:/…` → `C:/…`
    let rest = match rest.as_bytes() {
        [b'/', _, b':', _, ..] => rest.get(1..).unwrap_or(rest),
        _ => rest,
    };
    percent_decode(rest)
}

/// A file path as an IDML link URI (`file:/abs/path`, with spaces, `%` and `#` percent-encoded).
/// A value that is a URI already is kept.
pub fn path_to_uri(path: &str) -> String {
    if path.contains(':') && !path.starts_with('/') && path.chars().nth(1) != Some(':') {
        return path.to_string();
    }
    let mut out = String::from("file:");
    for c in path.chars() {
        match c {
            '\\' => out.push('/'),
            ' ' => out.push_str("%20"),
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            c => out.push(c),
        }
    }
    out
}
