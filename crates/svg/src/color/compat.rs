//! Floating-point CSS colour compatibility for existing SVG consumers.
//!
//! Ported from EffectCraft `crates/svg/src/color.rs`: Brandon Thomas's original
//! implementation at `94e1c86d89a866771f80ad1610afda9aa12e85d8`, with NetterSilver's
//! finite-channel checks at `e244d642350787aaa7028470855368165012d029`.
//! The shared keyword table replaces the identical local table; allocation limits
//! and precision/grammar regressions are added here.
use super::NAMED;

/// Parse the legacy SVG colour grammar without quantizing RGB or alpha from `f64`.
///
/// This compatibility entry point clamps finite channels, accepts `transparent`, loose
/// comma/space/slash separators and unitless HSL saturation/lightness, and ignores arguments
/// after alpha. It deliberately differs from the strict [`super::parse_color_alpha`].
/// `none`, `currentColor`, and `url()` remain the caller's paint policy.
///
/// Inputs above 64 MiB are refused before allocation; argument storage is limited to four
/// entries and hex storage to eight digits. Invalid or over-budget input returns `None`.
pub fn parse_rgba_compat(s: &str) -> Option<[f64; 4]> {
    if s.len() > 64 * 1024 * 1024 {
        return None;
    }
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    if lower == "transparent" {
        return Some([0.0; 4]);
    }
    if let Some(h) = lower.strip_prefix('#') {
        if !matches!(h.len(), 3 | 4 | 6 | 8) {
            return None;
        }
        let d: Vec<u32> = h.chars().map(|c| c.to_digit(16)).collect::<Option<_>>()?;
        let v = |i: usize| d[i] as f64 / 15.0;
        let w = |i: usize| (d[i] * 16 + d[i + 1]) as f64 / 255.0;
        return Some(match d.len() {
            3 => [v(0), v(1), v(2), 1.0],
            4 => [v(0), v(1), v(2), v(3)],
            6 => [w(0), w(2), w(4), 1.0],
            8 => [w(0), w(2), w(4), w(6)],
            _ => return None,
        });
    }
    if let Some(args) = func(&lower, "rgba").or_else(|| func(&lower, "rgb")) {
        let a: Vec<&str> = split_args(args);
        if a.len() < 3 {
            return None;
        }
        let comp = |t: &str| -> Option<f64> {
            Some(if let Some(p) = t.strip_suffix('%') {
                p.trim().parse::<f64>().ok().filter(|v| v.is_finite())? / 100.0
            } else {
                t.parse::<f64>().ok().filter(|v| v.is_finite())? / 255.0
            })
        };
        let alpha = a.get(3).map_or(Some(1.0), |t| alpha(t))?;
        return Some([comp(a[0])?.clamp(0.0, 1.0), comp(a[1])?.clamp(0.0, 1.0), comp(a[2])?.clamp(0.0, 1.0), alpha]);
    }
    if let Some(args) = func(&lower, "hsla").or_else(|| func(&lower, "hsl")) {
        let a: Vec<&str> = split_args(args);
        if a.len() < 3 {
            return None;
        }
        let h = a[0].trim_end_matches("deg").parse::<f64>().ok().filter(|v| v.is_finite())?.rem_euclid(360.0) / 360.0;
        let s = a[1].trim_end_matches('%').parse::<f64>().ok().filter(|v| v.is_finite())?.clamp(0.0, 100.0) / 100.0;
        let l = a[2].trim_end_matches('%').parse::<f64>().ok().filter(|v| v.is_finite())?.clamp(0.0, 100.0) / 100.0;
        let al = a.get(3).map_or(Some(1.0), |t| alpha(t))?;
        let [r, g, b] = hsl(h, s, l);
        return Some([r, g, b, al]);
    }
    NAMED
        .iter()
        .find(|(n, _)| *n == lower)
        .map(|(_, v)| [((v >> 16) & 255) as f64 / 255.0, ((v >> 8) & 255) as f64 / 255.0, (v & 255) as f64 / 255.0, 1.0])
}

fn func<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    s.strip_prefix(name)?.trim_start().strip_prefix('(')?.strip_suffix(')')
}

fn split_args(s: &str) -> Vec<&str> {
    s.split(|c: char| c == ',' || c == '/' || c.is_whitespace()).filter(|t| !t.is_empty()).take(4).collect()
}

fn alpha(t: &str) -> Option<f64> {
    Some(
        if let Some(p) = t.strip_suffix('%') {
            p.parse::<f64>().ok().filter(|v| v.is_finite())? / 100.0
        } else {
            t.parse::<f64>().ok().filter(|v| v.is_finite())?
        }
        .clamp(0.0, 1.0),
    )
}

/// CSS HSL → RGB (CSS Color 3 §4.2.4).
fn hsl(h: f64, s: f64, l: f64) -> [f64; 3] {
    let m2 = if l <= 0.5 { l * (s + 1.0) } else { l + s - l * s };
    let m1 = l * 2.0 - m2;
    let hue = |mut h: f64| {
        if h < 0.0 {
            h += 1.0;
        }
        if h > 1.0 {
            h -= 1.0;
        }
        if h * 6.0 < 1.0 {
            m1 + (m2 - m1) * h * 6.0
        } else if h * 2.0 < 1.0 {
            m2
        } else if h * 3.0 < 2.0 {
            m1 + (m2 - m1) * (2.0 / 3.0 - h) * 6.0
        } else {
            m1
        }
    };
    [hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0)]
}

#[cfg(test)]
mod tests {
    use super::parse_rgba_compat;

    #[test]
    fn non_finite_rejected() {
        for s in [
            "rgb(nan,0,0)",
            "rgb(inf,0,0)",
            "rgba(0,0,0,nan)",
            "rgba(0,0,0,inf)",
            "hsl(nan,100%,50%)",
            "hsl(120,nan%,50%)",
            "hsl(120,100%,inf%)",
            "hsl(120,100%,50%,nan)",
        ] {
            assert_eq!(parse_rgba_compat(s), None, "{s}");
        }
    }

    #[test]
    fn colors() {
        assert_eq!(parse_rgba_compat("#f00"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(parse_rgba_compat("#00ff0080").map(|c| (c[1], (c[3] * 255.0).round())), Some((1.0, 128.0)));
        assert_eq!(parse_rgba_compat("rgb(255, 0, 0)"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(parse_rgba_compat("rgba(0,0,255,0.5)"), Some([0.0, 0.0, 1.0, 0.5]));
        assert_eq!(parse_rgba_compat("rgb(100%, 50%, 0%)"), Some([1.0, 0.5, 0.0, 1.0]));
        assert_eq!(parse_rgba_compat("hsl(120, 100%, 50%)"), Some([0.0, 1.0, 0.0, 1.0]));
        assert_eq!(parse_rgba_compat("CornflowerBlue"), Some([100.0 / 255.0, 149.0 / 255.0, 237.0 / 255.0, 1.0]));
        assert_eq!(parse_rgba_compat("transparent"), Some([0.0; 4]));
        assert_eq!(parse_rgba_compat("nonsense"), None);
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::parse_rgba_compat;

    #[test]
    fn float_precision_and_legacy_grammar() {
        assert_eq!(parse_rgba_compat("rgb(100%,50%,0%)"), Some([1.0, 0.5, 0.0, 1.0]));
        assert_eq!(parse_rgba_compat("rgba(0,0,255,0.123456789123)"), Some([0.0, 0.0, 1.0, 0.123456789123]));
        assert_eq!(parse_rgba_compat("rgb(300,-1,0)"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(parse_rgba_compat("hsl(120,50,50)"), Some([0.25, 0.75, 0.25, 1.0]));
        assert_eq!(parse_rgba_compat("rgb(1,2,3,0.5,ignored)"), parse_rgba_compat("rgb(1,2,3,0.5)"));
        assert_eq!(parse_rgba_compat("# fff"), None);
        assert_eq!(parse_rgba_compat(&format!("rgb(1,2,3){}", " ".repeat(65))), parse_rgba_compat("rgb(1,2,3)"));
    }

    #[test]
    fn bounded_arguments_and_hex() {
        assert_eq!(super::split_args(&"1,".repeat(100_000)).len(), 4);
        assert_eq!(parse_rgba_compat(&format!("#{}", "f".repeat(100_000))), None);
        assert_eq!(parse_rgba_compat(&" ".repeat(64 * 1024 * 1024 + 1)), None);
    }
}
