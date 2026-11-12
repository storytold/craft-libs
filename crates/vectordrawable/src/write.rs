//! Writes a drawable as pretty-printed XML: elements two spaces deeper per level, one attribute
//! per line four spaces past its element (as Android Studio formats drawables). Numbers are
//! compact (at most three decimals, no `-0`) and always finite; text is escaped for attribute
//! values.

use std::fmt::Write as _;

use kurbo::{BezPath, PathEl};

use crate::model::{Cap, Color, Fill, GradientKind, Group, Item, ItemKind, Join, Path, Stroke};

/// Groups nested deeper than this are written without their children, so writing a drawable
/// never recurses without bound. Android Studio's own drawables nest a handful of levels.
pub const MAX_DEPTH: usize = 256;

const ANDROID: &str = "http://schemas.android.com/apk/res/android";
const AAPT: &str = "http://schemas.android.com/aapt";

/// A number as Android reads it: rounded to 3 decimals, no trailing zeros, `0` for anything
/// non-finite.
pub fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    // Android reads these as 32-bit floats, so clamp to that range; this also keeps the scaling
    // below from overflowing to infinity.
    let v = v.clamp(-f64::from(f32::MAX), f64::from(f32::MAX));
    let r = (v * 1000.0).round() / 1000.0;
    let s = format!("{r}");
    if s == "-0" { "0".into() } else { s }
}

/// `s` for an attribute value: markup escaped, whitespace that parsers would fold kept as
/// character references, and characters XML 1.0 forbids dropped.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

/// `#RRGGBB`, or `#AARRGGBB` when not opaque.
pub fn color(c: Color) -> String {
    // A non-finite component is 0.
    let [r, g, b, a] = c.map(|v| (if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 } * 255.0).round() as u8);
    if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{a:02X}{r:02X}{g:02X}{b:02X}") }
}

/// SVG path syntax, absolute commands.
pub fn path_data(p: &BezPath) -> String {
    let mut s = String::new();
    let pt = |x: f64, y: f64| format!("{},{}", num(x), num(y));
    for el in p.elements() {
        if !s.is_empty() {
            s.push(' ');
        }
        match *el {
            PathEl::MoveTo(a) => {
                let _ = write!(s, "M{}", pt(a.x, a.y));
            }
            PathEl::LineTo(a) => {
                let _ = write!(s, "L{}", pt(a.x, a.y));
            }
            PathEl::QuadTo(a, b) => {
                let _ = write!(s, "Q{} {}", pt(a.x, a.y), pt(b.x, b.y));
            }
            PathEl::CurveTo(a, b, c) => {
                let _ = write!(s, "C{} {} {}", pt(a.x, a.y), pt(b.x, b.y), pt(c.x, c.y));
            }
            PathEl::ClosePath => s.push('Z'),
        }
    }
    s
}

struct Out {
    s: String,
}

impl Out {
    /// `<tag` and its attributes, one per line; `close` ends it with `/>`, else `>`.
    fn open(&mut self, depth: usize, tag: &str, attrs: &[(&str, String)], close: bool) {
        let pad = "  ".repeat(depth);
        let _ = write!(self.s, "{pad}<{tag}");
        for (i, (k, v)) in attrs.iter().enumerate() {
            if i == 0 && attrs.len() == 1 {
                let _ = write!(self.s, " {k}=\"{v}\"");
            } else {
                let _ = write!(self.s, "\n{pad}    {k}=\"{v}\"");
            }
        }
        self.s.push_str(if close { " />\n" } else { ">\n" });
    }

    fn close(&mut self, depth: usize, tag: &str) {
        let _ = writeln!(self.s, "{}</{tag}>", "  ".repeat(depth));
    }
}

/// Whether a path that is written (see [`MAX_DEPTH`]) has a gradient fill.
fn uses_gradient(items: &[Item], depth: usize) -> bool {
    items.iter().any(|it| match &it.kind {
        ItemKind::Path(p) => matches!(p.fill, Some(Fill::Gradient(_))),
        ItemKind::Group(g) => depth < MAX_DEPTH && uses_gradient(&g.children, depth + 1),
    })
}

/// The whole file: a `width`×`height` dp drawable with a viewport of the same size.
pub fn document(width: f64, height: f64, items: &[Item]) -> String {
    let mut o = Out { s: String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n") };
    let mut attrs = vec![("xmlns:android", ANDROID.to_string())];
    if uses_gradient(items, 1) {
        attrs.push(("xmlns:aapt", AAPT.to_string()));
    }
    attrs.extend([
        ("android:width", format!("{}dp", num(width))),
        ("android:height", format!("{}dp", num(height))),
        ("android:viewportWidth", num(width)),
        ("android:viewportHeight", num(height)),
    ]);
    o.open(0, "vector", &attrs, false);
    if items.is_empty() {
        // Android refuses a <vector> without a <path> ("no path defined"): one empty, unfilled path.
        o.open(1, "path", &[("android:pathData", "M0,0".to_string())], true);
    }
    for it in items {
        item(&mut o, 1, it);
    }
    o.close(0, "vector");
    o.s
}

fn item(o: &mut Out, depth: usize, it: &Item) {
    match &it.kind {
        ItemKind::Path(p) => path(o, depth, p),
        ItemKind::Group(g) => group(o, depth, g),
    }
}

fn group(o: &mut Out, depth: usize, g: &Group) {
    let mut attrs = Vec::new();
    if let Some(n) = &g.name {
        attrs.push(("android:name", esc(n)));
    }
    if let Some(x) = &g.transform {
        let (sx, sy) = x.scale;
        if num(x.rotation) != "0" {
            attrs.push(("android:rotation", num(x.rotation)));
        }
        if num(sx) != "1" {
            attrs.push(("android:scaleX", num(sx)));
        }
        if num(sy) != "1" {
            attrs.push(("android:scaleY", num(sy)));
        }
        if num(x.translate.0) != "0" {
            attrs.push(("android:translateX", num(x.translate.0)));
        }
        if num(x.translate.1) != "0" {
            attrs.push(("android:translateY", num(x.translate.1)));
        }
    }
    let children: &[Item] = if depth < MAX_DEPTH { &g.children } else { &[] };
    if children.is_empty() && g.clip.is_none() {
        o.open(depth, "group", &attrs, true);
        return;
    }
    o.open(depth, "group", &attrs, false);
    if let Some(c) = &g.clip {
        o.open(depth + 1, "clip-path", &[("android:pathData", path_data(c))], true);
    }
    for c in children {
        item(o, depth + 1, c);
    }
    o.close(depth, "group");
}

fn stroke_attrs(attrs: &mut Vec<(&str, String)>, s: &Stroke) {
    attrs.push(("android:strokeColor", color(s.color)));
    attrs.push(("android:strokeWidth", num(s.width)));
    match s.cap {
        Cap::Butt => {}
        Cap::Round => attrs.push(("android:strokeLineCap", "round".to_string())),
        Cap::Square => attrs.push(("android:strokeLineCap", "square".to_string())),
    }
    match s.join {
        Join::Round => attrs.push(("android:strokeLineJoin", "round".to_string())),
        Join::Bevel => attrs.push(("android:strokeLineJoin", "bevel".to_string())),
        Join::Miter if num(s.miter) != "4" => attrs.push(("android:strokeMiterLimit", num(s.miter))),
        Join::Miter => {}
    }
}

fn path(o: &mut Out, depth: usize, p: &Path) {
    let mut attrs = Vec::new();
    if let Some(n) = &p.name {
        attrs.push(("android:name", esc(n)));
    }
    attrs.push(("android:pathData", path_data(&p.data)));
    if let Some(Fill::Solid(c)) = &p.fill {
        attrs.push(("android:fillColor", color(*c)));
    }
    if let Some(s) = &p.stroke {
        stroke_attrs(&mut attrs, s);
    }
    if p.even_odd && p.fill.is_some() {
        attrs.push(("android:fillType", "evenOdd".to_string()));
    }
    let Some(Fill::Gradient(g)) = &p.fill else {
        o.open(depth, "path", &attrs, true);
        return;
    };
    o.open(depth, "path", &attrs, false);
    o.open(depth + 1, "aapt:attr", &[("name", "android:fillColor".to_string())], false);
    let mut ga = Vec::new();
    match g.kind {
        GradientKind::Linear { start, end } => {
            ga.push(("android:type", "linear".to_string()));
            ga.push(("android:startX", num(start.0)));
            ga.push(("android:startY", num(start.1)));
            ga.push(("android:endX", num(end.0)));
            ga.push(("android:endY", num(end.1)));
        }
        GradientKind::Radial { center, radius } => {
            ga.push(("android:type", "radial".to_string()));
            ga.push(("android:centerX", num(center.0)));
            ga.push(("android:centerY", num(center.1)));
            ga.push(("android:gradientRadius", num(radius)));
        }
        GradientKind::Sweep { center } => {
            ga.push(("android:type", "sweep".to_string()));
            ga.push(("android:centerX", num(center.0)));
            ga.push(("android:centerY", num(center.1)));
        }
    }
    o.open(depth + 2, "gradient", &ga, false);
    for (off, c) in &g.stops {
        o.open(depth + 3, "item", &[("android:offset", num(*off)), ("android:color", color(*c))], true);
    }
    o.close(depth + 2, "gradient");
    o.close(depth + 1, "aapt:attr");
    o.close(depth, "path");
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use kurbo::{Rect, Shape as _};

    use super::*;
    use crate::model::{Gradient, GroupTransform};

    const RED: Color = [1.0, 0.0, 0.0, 1.0];

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
        Rect::new(x0, y0, x1, y1).to_path(0.1)
    }

    fn filled(data: BezPath, fill: Fill) -> Item {
        let bbox = data.bounding_box();
        Item { kind: ItemKind::Path(Path { name: None, data, fill: Some(fill), stroke: None, even_odd: false }), bbox }
    }

    fn group(children: Vec<Item>) -> Item {
        Item { kind: ItemKind::Group(Group { name: None, transform: None, clip: None, children }), bbox: Rect::ZERO }
    }

    /// The written file, checked to be well-formed and free of numbers Android can't read.
    fn written(width: f64, height: f64, items: &[Item]) -> String {
        let xml = document(width, height, items);
        if let Err(e) = craft_xml::parse(xml.as_bytes()) {
            panic!("not well-formed: {e}\n{xml}");
        }
        for bad in ["NaN", "inf", "=\"-0\"", "e+", "e-"] {
            assert!(!xml.contains(bad), "{bad} in\n{xml}");
        }
        xml
    }

    #[test]
    fn numbers_are_short_finite_and_never_negative_zero() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.12345), "0.123");
        assert_eq!(num(-0.0004), "0");
        assert_eq!(num(-2.5), "-2.5");
        assert_eq!(num(1e-9), "0");
        assert_eq!(num(12345678.9), "12345678.9");
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(num(f64::INFINITY), "0");
        assert!(!num(f64::MAX).contains("inf") && !num(-f64::MAX).contains("inf"), "clamped to the f32 range");
    }

    #[test]
    fn colours_are_rrggbb_or_aarrggbb() {
        assert_eq!(color([26.0 / 255.0, 43.0 / 255.0, 60.0 / 255.0, 1.0]), "#1A2B3C");
        assert_eq!(color([26.0 / 255.0, 43.0 / 255.0, 60.0 / 255.0, 128.0 / 255.0]), "#801A2B3C");
        assert_eq!(color([2.0, -1.0, f32::NAN, 1.0]), "#FF0000");
        assert_eq!(color([0.0, 0.0, 0.0, f32::NAN]), "#00000000");
    }

    #[test]
    fn text_is_escaped_for_an_attribute() {
        assert_eq!(esc("a&b<c>\"d'"), "a&amp;b&lt;c&gt;&quot;d&apos;");
        assert_eq!(esc("tab\tline\nret\r"), "tab&#9;line&#10;ret&#13;");
        assert_eq!(esc("bell\u{7}\u{FFFE}ok"), "bellok");
    }

    #[test]
    fn the_file_is_pretty_printed() {
        let clip = Group {
            name: None,
            transform: None,
            clip: Some(rect(0.0, 0.0, 24.0, 24.0)),
            children: vec![filled(rect(4.0, 4.0, 20.0, 12.0), Fill::Solid(RED))],
        };
        let xml = written(24.0, 24.0, &[Item { kind: ItemKind::Group(clip), bbox: Rect::new(0.0, 0.0, 24.0, 24.0) }]);
        let want = "<?xml version=\"1.0\" encoding=\"utf-8\"?>
<vector
    xmlns:android=\"http://schemas.android.com/apk/res/android\"
    android:width=\"24dp\"
    android:height=\"24dp\"
    android:viewportWidth=\"24\"
    android:viewportHeight=\"24\">
  <group>
    <clip-path android:pathData=\"M0,0 L24,0 L24,24 L0,24 Z\" />
    <path
        android:pathData=\"M4,4 L20,4 L20,12 L4,12 Z\"
        android:fillColor=\"#FF0000\" />
  </group>
</vector>
";
        assert_eq!(xml, want);
    }

    #[test]
    fn an_empty_drawable_is_still_one_android_loads() {
        // Android throws "no path defined" for a <vector> without a <path>.
        let xml = written(10.0, 10.0, &[]);
        assert!(xml.ends_with("android:viewportHeight=\"10\">\n  <path android:pathData=\"M0,0\" />\n</vector>\n"), "{xml}");
    }

    #[test]
    fn names_are_escaped_and_a_name_cannot_add_markup() {
        let mut it = filled(rect(0.0, 0.0, 1.0, 1.0), Fill::Solid(RED));
        if let ItemKind::Path(p) = &mut it.kind {
            p.name = Some("Icon & \"co\" <1>\" android:pathData=\"M9,9".into());
        }
        let xml = written(1.0, 1.0, &[it]);
        let root = craft_xml::parse(xml.as_bytes()).unwrap();
        let path = root.elements().next().unwrap();
        assert_eq!(path.attr("android:name"), Some("Icon & \"co\" <1>\" android:pathData=\"M9,9"));
        assert_eq!(path.attr("android:pathData"), Some("M0,0 L1,0 L1,1 L0,1 Z"));
    }

    #[test]
    fn a_stroke_writes_only_what_differs_from_androids_defaults() {
        let stroke = |cap, join, miter| {
            let s = Stroke { color: [0.0, 0.0, 1.0, 0.5], width: 2.5, cap, join, miter };
            let p = Path { name: None, data: rect(0.0, 0.0, 1.0, 1.0), fill: None, stroke: Some(s), even_odd: true };
            written(1.0, 1.0, &[Item { kind: ItemKind::Path(p), bbox: Rect::ZERO }])
        };
        let plain = stroke(Cap::Butt, Join::Miter, 4.0);
        assert!(plain.contains("android:strokeColor=\"#800000FF\"") && plain.contains("android:strokeWidth=\"2.5\""), "{plain}");
        for absent in ["strokeLineCap", "strokeLineJoin", "strokeMiterLimit", "fillColor", "fillType"] {
            assert!(!plain.contains(absent), "{absent} in {plain}");
        }
        let round = stroke(Cap::Round, Join::Round, 10.0);
        assert!(round.contains("android:strokeLineCap=\"round\"") && round.contains("android:strokeLineJoin=\"round\""), "{round}");
        assert!(!round.contains("strokeMiterLimit"), "a miter limit means nothing on a round join: {round}");
        let square = stroke(Cap::Square, Join::Bevel, 4.0);
        assert!(square.contains("android:strokeLineCap=\"square\"") && square.contains("android:strokeLineJoin=\"bevel\""), "{square}");
        assert!(stroke(Cap::Butt, Join::Miter, 2.0).contains("android:strokeMiterLimit=\"2\""));
    }

    #[test]
    fn an_even_odd_fill_is_marked_and_a_group_writes_its_transform() {
        let mut it = filled(rect(0.0, 0.0, 1.0, 1.0), Fill::Solid(RED));
        if let ItemKind::Path(p) = &mut it.kind {
            p.even_odd = true;
        }
        let xf = GroupTransform { translate: (3.0, -4.0), rotation: 90.0, scale: (2.0, 1.0) };
        let g = Group { name: Some("g".into()), transform: Some(xf), clip: None, children: vec![it] };
        let xml = written(8.0, 8.0, &[Item { kind: ItemKind::Group(g), bbox: Rect::ZERO }]);
        assert!(xml.contains("android:fillType=\"evenOdd\""), "{xml}");
        for want in ["android:name=\"g\"", "android:rotation=\"90\"", "android:scaleX=\"2\"", "android:translateX=\"3\"", "android:translateY=\"-4\""]
        {
            assert!(xml.contains(want), "{want} not in {xml}");
        }
        assert!(!xml.contains("scaleY"), "1 is the default: {xml}");
        // An identity transform and no children: one empty element.
        let empty = Group {
            name: None,
            transform: Some(GroupTransform { translate: (0.0, 0.0), rotation: 0.0, scale: (1.0, 1.0) }),
            clip: None,
            children: vec![],
        };
        assert!(written(8.0, 8.0, &[Item { kind: ItemKind::Group(empty), bbox: Rect::ZERO }]).contains("  <group />\n"));
    }

    #[test]
    fn gradients_are_inline_resources_and_declare_the_aapt_namespace() {
        let stops = vec![(0.0, RED), (1.0, [0.0, 0.0, 1.0, 0.5])];
        let kinds = [
            (GradientKind::Linear { start: (0.0, 0.0), end: (10.0, 0.0) }, "android:type=\"linear\"", "android:endX=\"10\""),
            (GradientKind::Radial { center: (5.0, 5.0), radius: 5.0 }, "android:type=\"radial\"", "android:gradientRadius=\"5\""),
            (GradientKind::Sweep { center: (5.0, 6.0) }, "android:type=\"sweep\"", "android:centerY=\"6\""),
        ];
        for (kind, type_attr, own) in kinds {
            let xml =
                written(10.0, 10.0, &[group(vec![filled(rect(0.0, 0.0, 10.0, 10.0), Fill::Gradient(Gradient { kind, stops: stops.clone() }))])]);
            assert!(xml.contains("xmlns:aapt=\"http://schemas.android.com/aapt\""), "{xml}");
            assert!(xml.contains("<aapt:attr name=\"android:fillColor\">") && xml.contains(type_attr) && xml.contains(own), "{xml}");
            assert!(xml.contains("android:offset=\"0\"\n") && xml.contains("android:color=\"#800000FF\""), "{xml}");
            assert!(!xml.contains("android:fillColor=\""), "the fill is the gradient alone: {xml}");
        }
        assert!(!written(1.0, 1.0, &[filled(rect(0.0, 0.0, 1.0, 1.0), Fill::Solid(RED))]).contains("aapt"));
    }

    #[test]
    fn groups_past_the_depth_cap_are_written_empty() {
        let mut deep = filled(rect(0.0, 0.0, 1.0, 1.0), Fill::Gradient(Gradient { kind: GradientKind::Sweep { center: (0.0, 0.0) }, stops: vec![] }));
        for _ in 0..MAX_DEPTH {
            deep = group(vec![deep]);
        }
        // MAX_DEPTH groups: the innermost is at the cap and is written without the path.
        let xml = document(1.0, 1.0, std::slice::from_ref(&deep));
        assert_eq!(xml.matches("<group").count(), MAX_DEPTH);
        assert!(!xml.contains("<path") && !xml.contains("aapt"), "nothing past the cap is written or declared");
        // One group fewer and the path is written.
        let ItemKind::Group(outer) = deep.kind else { panic!("a group") };
        let xml = document(1.0, 1.0, &outer.children);
        assert!(xml.contains("<path") && xml.contains("xmlns:aapt"));
    }
}
