//! The values style properties are written in (ODF 1.3 Part 3, section 18): lengths, angles,
//! percentages, colours and borders.

/// A length (`0.889in`, `2.258cm`, `12pt`, `5mm`, `1pc`, `96px`) in points. `None` for a
/// length without a unit, a percentage, or one that is not a finite number.
pub fn length_pt(s: &str) -> Option<f64> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic() || c == '%')?;
    let (num, unit) = (s.get(..split)?, s.get(split..)?);
    let n: f64 = num.trim().parse().ok().filter(|n: &f64| n.is_finite())?;
    let pt = match unit {
        "pt" => n,
        "in" | "inch" => n * 72.0,
        "cm" => n * 72.0 / 2.54,
        "mm" => n * 72.0 / 25.4,
        "pc" => n * 12.0,
        "px" => n * 0.75,
        _ => return None,
    };
    pt.is_finite().then_some(pt)
}

/// A percentage (`50%`) as a fraction (0.5).
pub fn percent(s: &str) -> Option<f64> {
    let n: f64 = s.trim().strip_suffix('%')?.trim().parse().ok()?;
    (n.is_finite()).then_some(n / 100.0)
}

/// An angle (`45`, `45deg`, `0.785rad`, `50grad`) in degrees. A bare number is degrees.
pub fn angle_deg(s: &str) -> Option<f64> {
    let s = s.trim();
    let (n, factor) = if let Some(n) = s.strip_suffix("deg") {
        (n, 1.0)
    } else if let Some(n) = s.strip_suffix("grad") {
        (n, 360.0 / 400.0)
    } else if let Some(n) = s.strip_suffix("rad") {
        (n, 180.0 / std::f64::consts::PI)
    } else {
        (s, 1.0)
    };
    n.trim().parse::<f64>().ok().map(|v| v * factor).filter(|d| d.is_finite())
}

/// A colour (`#3366ff`) as `0xRRGGBB`. `None` for `transparent` or anything that is not six
/// hexadecimal digits after `#`.
pub fn rgb(s: &str) -> Option<u32> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(h, 16).ok()
}

/// A border (`fo:border`, `fo:border-left`, `style:diagonal-tl-br`, …: §20.183, written as in
/// XSL-FO): its width, line style and colour.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Border {
    /// The width in points, when the border states one. `thin`, `medium` and `thick` are 0.75,
    /// 1.75 and 2.5 points, the widths office suites write for them.
    pub width: Option<f64>,
    /// The line style as written: `solid`, `double`, `dotted`, `dashed`, `dash-dot`, … `none`
    /// (and `hidden`) for no line.
    pub style: String,
    /// The colour as `0xRRGGBB`, when stated.
    pub color: Option<u32>,
}

impl Border {
    /// No line.
    pub fn is_none(&self) -> bool {
        matches!(self.style.as_str(), "none" | "hidden")
    }
}

/// A border value. An empty value, `none` and `hidden` are a border of style `none`; a value
/// that states no line style is `solid`.
pub fn border(s: &str) -> Border {
    let mut b = Border { style: String::new(), ..Border::default() };
    for part in s.split_whitespace() {
        if let Some(c) = rgb(part) {
            b.color = Some(c);
        } else if let Some(w) = length_pt(part) {
            b.width = Some(w);
        } else {
            match part {
                "thin" => b.width = Some(0.75),
                "medium" => b.width = Some(1.75),
                "thick" => b.width = Some(2.5),
                style => b.style = style.to_string(),
            }
        }
    }
    if b.style.is_empty() {
        b.style = if s.trim().is_empty() { "none" } else { "solid" }.to_string();
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_in_their_units() {
        assert_eq!(length_pt("1in"), Some(72.0));
        assert_eq!(length_pt(" 2.54cm "), Some(72.0));
        assert_eq!(length_pt("10mm").map(|v| (v * 1000.0).round()), Some(28346.0));
        for bad in ["12", "50%", "1em", "inf pt", "NaNpt", ""] {
            assert_eq!(length_pt(bad), None, "{bad}");
        }
        assert_eq!(angle_deg("90"), Some(90.0));
        assert_eq!(angle_deg("100grad"), Some(90.0));
        assert!(angle_deg("3.14159265rad").is_some_and(|d| (d - 180.0).abs() < 1e-6));
        assert_eq!(percent("50%"), Some(0.5));
        assert_eq!((rgb("#3366FF"), rgb("transparent"), rgb("#36f")), (Some(0x3366ff), None, None));
    }

    #[test]
    fn borders_in_any_order() {
        assert_eq!(border("0.74pt solid #000000"), Border { width: Some(0.74), style: "solid".into(), color: Some(0) });
        assert_eq!(border("#ff0000 double 2.5pt"), Border { width: Some(2.5), style: "double".into(), color: Some(0xff0000) });
        assert_eq!(border("thin").style, "solid");
        assert!(border("none").is_none() && border("").is_none() && border("1pt hidden").is_none());
    }
}
