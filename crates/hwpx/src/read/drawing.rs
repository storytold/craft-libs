//! Pictures, shapes, text boxes, groups and form objects.
//!
//! Objects "treated as characters" stay on their text line; the others float at their position
//! with their text wrapping, so they no longer push the text around.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::sync::Arc;

use craft_wordmodel::connector::{ArrowKind, ArrowSize, ArrowStyle, LineCap, MAX_CORNER, ShapeExtra};
use craft_wordmodel::fill::{Fill, Gradient, GradientFlip, GradientKind, PathShape};
use craft_wordmodel::freeform::{FreePath, Freeform};
use craft_wordmodel::geom::Spin;
use craft_wordmodel::para::{FileOrders, Float, GroupChild, MAX_GROUP_CHILDREN, ShapeKind, Wrap};
use craft_wordmodel::props::{Align, CharProps, ParaProps, Rgb, valign_from_anchor};
use craft_wordmodel::wordart::GradStop;
use craft_wordmodel::{InlineObject, PartKind, TEXT_BOX_INSET};

use super::media::{image_ext, pixel_size};
use super::place::Placement;
use super::props::{Spacing, rgb};
use super::text::ext_of;
use super::{Blk, Converter, Item, P, blocks};
use crate::header::{color, num, truthy};
use crate::units::{emu_signed, round_even, trunc};
use crate::xml::Element;

/// Drawing objects that can be group members.
const OBJECTS: &[&str] =
    &["hp:pic", "hp:container", "hp:rect", "hp:ellipse", "hp:arc", "hp:line", "hp:polygon", "hp:curve", "hp:connectLine", "hp:ole", "hp:textart"];
/// Deepest group nesting flattened into the outermost group.
const MAX_GROUP_DEPTH: usize = 8;
/// Largest length kept, points (200 inches).
const MAX_LEN_PT: f32 = 14_400.0;
/// The stacking order floating objects start from (Hancom's z-order is added to it).
const Z_BASE: i64 = 251_658_240;
/// A text box's inner margins when the file gives none (1mm, EMU).
const TEXT_INSET: i64 = 36_000;

/// HWPUNIT -> EMU keeping the sign (a fractional value is cut first).
fn emu(hu: f64) -> i64 {
    emu_signed(hu)
}

/// EMU -> points.
fn emu_pt(e: i64) -> f32 {
    (e as f64 / 12_700.0) as f32
}

/// HWPUNIT -> points through whole EMU.
fn hu_pt(hu: f64) -> f32 {
    emu_pt(emu(hu))
}

/// Degrees within one turn, as a 60000ths-of-a-degree angle read back.
fn turn(deg: i64) -> f32 {
    deg.saturating_mul(60000).rem_euclid(21_600_000) as f32 / 60_000.0
}

fn dash(style: &str) -> Option<&'static str> {
    Some(match style {
        "DASH" => "dash",
        "DOT" | "CIRCLE" => "sysDot",
        "DASH_DOT" => "dashDot",
        "DASH_DOT_DOT" => "lgDashDotDot",
        "LONG_DASH" => "lgDash",
        _ => return None,
    })
}

fn arrow(style: &str) -> Option<ArrowKind> {
    Some(match style {
        "ARROW" => ArrowKind::from_ooxml("arrow"),
        "SPEAR" => ArrowKind::from_ooxml("triangle"),
        "CONCAVE_ARROW" => ArrowKind::from_ooxml("stealth"),
        "EMPTY_DIAMOND" | "FILLED_DIAMOND" | "EMPTY_BOX" | "FILLED_BOX" => ArrowKind::from_ooxml("diamond"),
        "EMPTY_CIRCLE" | "FILLED_CIRCLE" => ArrowKind::from_ooxml("oval"),
        _ => return None,
    })
}

/// A gradient through `cols` (hc:gradation types: 1/LINEAR, 2/RADIAL drawn round with a light
/// centre, as the viewer draws it, 3/CONICAL drawn round too, 4/SQUARE).
pub(crate) fn gradient_fill(kind: &str, angle: i64, cols: &[Rgb]) -> Option<Fill> {
    let n = cols.len().saturating_sub(1).max(1) as f64;
    let stops = cols
        .iter()
        .enumerate()
        .map(|(k, c)| GradStop { pos: trunc(100000.0 * k as f64 / n) as f32 / 1000.0, color: *c, transparency: 0.0 })
        .collect();
    let kind = match kind {
        "1" => "LINEAR",
        "2" => "RADIAL",
        "3" => "CONICAL",
        "4" => "SQUARE",
        k => k,
    };
    let kind = match kind {
        "RADIAL" | "CONICAL" => GradientKind::Path { shape: PathShape::Circle, focus: [50.0; 4] },
        "SQUARE" => GradientKind::Path { shape: PathShape::Rect, focus: [50.0; 4] },
        _ => GradientKind::Linear { angle: turn(angle), scaled: false },
    };
    Fill::Gradient(Arc::new(Gradient { stops, kind, tile: None, rot_with_shape: true, flip: GradientFlip::None })).sanitized()
}

/// The object's size: its current, frame or original size, whichever is given first. A
/// level or upright line has no height or no width (its original size is only the space its
/// points are given in).
pub(crate) fn size_of(el: &Element) -> (i64, i64) {
    let line = matches!(el.tag.as_str(), "hp:line" | "hp:connectLine");
    for tag in ["hp:curSz", "hp:sz", "hp:orgSz"] {
        if let Some(s) = el.find(tag) {
            let (w, h) = (num(s.get("width")), num(s.get("height")));
            if (w > 0 && h > 0) || (line && w.max(h) > 0 && w.min(h) >= 0) {
                return (w, h);
            }
        }
    }
    (0, 0)
}

fn org_size(el: &Element, w: f64, h: f64) -> (f64, f64) {
    match el.find("hp:orgSz") {
        Some(o) if num(o.get("width")) > 0 && num(o.get("height")) > 0 => (num(o.get("width")) as f64, num(o.get("height")) as f64),
        _ => (w, h),
    }
}

/// Whether a picture is mirrored left to right and top to bottom: its `hp:flip` with the scale
/// matrix that draws it so (negative on that axis). Hancom's viewer mirrors only by the matrix.
fn mirrored(el: &Element) -> (bool, bool) {
    let Some(fl) = el.find("hp:flip") else { return (false, false) };
    let sca = el.find("hp:renderingInfo").and_then(|r| r.find("hc:scaMatrix"));
    let negative = |k: &str| sca.and_then(|m| m.get(k)).and_then(|v| v.trim().parse::<f64>().ok()).is_some_and(|v| v < 0.0);
    (truthy(fl.get("horizontal")) && negative("e1"), truthy(fl.get("vertical")) && negative("e5"))
}

fn angle_of(el: &Element) -> i64 {
    el.find("hp:rotationInfo").map(|r| num(r.get("angle"))).unwrap_or(0)
}

/// A turned object's frame (hp:sz) is the box around the turned shape; the model wants the
/// shape's own size (hp:curSz), turned about the frame's centre. Group members are already
/// placed unturned.
fn unrotated(el: &Element, x: f64, y: f64, w: f64, h: f64, grouped: bool) -> (f64, f64, f64, f64) {
    let Some(cur) = el.find("hp:curSz") else { return (x, y, w, h) };
    if grouped || angle_of(el) == 0 {
        return (x, y, w, h);
    }
    let (cw, ch) = (num(cur.get("width")) as f64, num(cur.get("height")) as f64);
    if cw <= 0.0 || ch <= 0.0 || (cw == w && ch == h) {
        return (x, y, w, h);
    }
    (x + (w - cw) / 2.0, y + (h - ch) / 2.0, cw, ch)
}

/// hp:imgClip -> crop fractions (left, top, right, bottom). The clip is in the picture's own
/// space: hp:imgDim when given, else its pixels at 96 dpi (75 HWPUNIT each), widened to the clip
/// if larger.
fn crop(el: &Element, data: &[u8]) -> [f32; 4] {
    let Some(clip) = el.find("hp:imgClip") else { return [0.0; 4] };
    let (l, t, r, b) = (num(clip.get("left")), num(clip.get("top")), num(clip.get("right")), num(clip.get("bottom")));
    let (mut fw, mut fh) = match el.find("hp:imgDim").filter(|d| num(d.get("dimwidth")) > 0) {
        Some(d) => (num(d.get("dimwidth")), num(d.get("dimheight"))),
        None => {
            let px = pixel_size(data).unwrap_or((0, 0));
            (i64::from(px.0) * 75, i64::from(px.1) * 75)
        }
    };
    fw = fw.max(r);
    fh = fh.max(b);
    if fw <= 0 || fh <= 0 || r <= l || b <= t || (l, t, r, b) == (0, 0, fw, fh) {
        return [0.0; 4];
    }
    let (fwf, fhf) = (fw as f64, fh as f64);
    let sides = [l as f64 / fwf, t as f64 / fhf, (fw - r) as f64 / fwf, (fh - b) as f64 / fhf];
    sides.map(|v| (round_even(v * 100000.0).clamp(0, 100_000) as f32 / 100_000.0).clamp(0.0, 1.0))
}

/// A polygon's or curve's outline in its frame (`w` × `h` points); turn = (degrees, shape w,
/// h, frame w, h, HWPUNIT) turns its points clockwise about the frame's centre. `None`: drawn as
/// a rectangle.
fn freeform(el: &Element, w: f32, h: f32, turn: Option<(i64, f64, f64, f64, f64)>) -> Option<Freeform> {
    let tag = el.tag.as_str();
    let org = el.find("hp:orgSz");
    let mut pw = org.map(|o| num(o.get("width")).max(1)).unwrap_or(1);
    let mut ph = org.map(|o| num(o.get("height")).max(1)).unwrap_or(1);
    let mut pts: Vec<(i64, i64)> = if tag == "hp:polygon" {
        el.findall("hc:pt").map(|p| (num(p.get("x")), num(p.get("y")))).collect()
    } else {
        // hp:curve: segments, drawn through their end points
        let segs: Vec<&Element> = el.findall("hp:seg").collect();
        let mut v: Vec<(i64, i64)> = segs.first().map(|s| (num(s.get("x1")), num(s.get("y1")))).into_iter().collect();
        v.extend(segs.iter().map(|s| (num(s.get("x2")), num(s.get("y2")))));
        v
    };
    let (Some(&p0), Some(&pn)) = (pts.first(), pts.last()) else { return None };
    if pts.len() < 2 {
        return None;
    }
    let close = tag == "hp:polygon" && p0 == pn;
    if let Some((deg, sw, sh, fw, fh)) = turn {
        let rad = (deg as f64).to_radians();
        let (c, s) = (rad.cos(), rad.sin());
        let (pwf, phf) = (pw as f64, ph as f64);
        pts = pts
            .iter()
            .map(|&(px, py)| {
                let dx = px as f64 * sw / pwf - sw / 2.0;
                let dy = py as f64 * sh / phf - sh / 2.0;
                (round_even(fw / 2.0 + dx * c - dy * s), round_even(fh / 2.0 + dx * s + dy * c))
            })
            .collect();
        pw = round_even(fw).max(1);
        ph = round_even(fh).max(1);
    }
    // A path drawn back to where it started is closed too.
    let closed = close || (pts.len() > 2 && pts.first() == pts.last());
    let (sx, sy) = (f64::from(w) / pw as f64, f64::from(h) / ph as f64);
    let pts = pts.iter().map(|&(x, y)| [(x as f64 * sx) as f32, (y as f64 * sy) as f32]).collect();
    let f = Freeform { w, h, paths: vec![FreePath { pts, closed }], alpha: 1.0, ink: None, order: None }.sanitized();
    (!f.paths.is_empty()).then_some(f)
}

fn fill(el: &Element) -> Option<Fill> {
    let fb = el.find("hc:fillBrush")?;
    if let Some(c) = fb.find("hc:winBrush").and_then(|wb| color(wb.get("faceColor"))).and_then(|c| rgb(&c)) {
        return Some(Fill::Solid { color: c, transparency: 0.0 });
    }
    let gr = fb.find("hc:gradation")?;
    let cols: Vec<Rgb> = gr.findall("hc:color").filter_map(|c| color(c.get("value"))).filter_map(|c| rgb(&c)).collect();
    if cols.len() >= 2 {
        return gradient_fill(gr.get("type").unwrap_or("LINEAR"), num(gr.get("angle")), &cols);
    }
    cols.first().map(|c| Fill::Solid { color: *c, transparency: 0.0 })
}

/// The outline: colour, width (points), and its dash and arrowheads.
fn outline(el: &Element, extra: &mut ShapeExtra) -> (Option<Rgb>, f32) {
    let Some(ls) = el.find("hp:lineShape") else { return (Some(Rgb(0, 0, 0)), 0.75) };
    let style = ls.get("style").unwrap_or("SOLID");
    if style == "NONE" {
        return (None, 0.0);
    }
    let w = emu(num(ls.get("width")) as f64).max(3175);
    let col = color(ls.get("color")).and_then(|c| rgb(&c)).unwrap_or(Rgb(0, 0, 0));
    if let Some(d) = dash(style) {
        extra.dash = craft_wordmodel::line_dash::LineDash::from_ooxml(d);
    }
    let style_of = |kind| ArrowStyle { kind, width: ArrowSize::from_ooxml("med"), length: ArrowSize::from_ooxml("med") };
    if let Some(a) = arrow(ls.get("headStyle").unwrap_or("")) {
        extra.arrow_start = true;
        extra.start_style = style_of(a);
    }
    if let Some(a) = arrow(ls.get("tailStyle").unwrap_or("")) {
        extra.arrow_end = true;
        extra.end_style = style_of(a);
    }
    (Some(col), emu_pt(w).clamp(0.0, 100.0))
}

/// What a shape's text box says about its text: inner margins (EMU) and vertical alignment.
struct BoxText {
    inset: [i64; 4],
    anchor: Option<&'static str>,
}

impl Converter<'_> {
    /// A top-level hp:pic, shape or hp:container as an item of its run.
    pub(crate) fn drawing(&mut self, el: &Element, props: &CharProps) -> Option<Item> {
        let place = self.placement(el);
        let (mut w, mut h) = size_of(el);
        if let Some(sz) = el.find("hp:sz")
            && angle_of(el) != 0
            && num(sz.get("width")) > 0
        {
            (w, h) = (num(sz.get("width")), num(sz.get("height"))); // the frame around the turned shape
        }
        let (w, h) = (w as f64, h as f64);
        let (obj, inner) = if el.tag == "hp:container" {
            // A group stays one object, so its members keep their places relative to each other
            // whether the group is inline or floating.
            let (ow, oh) = org_size(el, w, h);
            let mut children = Vec::new();
            self.members(el, (0.0, 0.0, 1.0, 1.0), 0, &mut children);
            if children.is_empty() {
                return None;
            }
            let (fw, fh) = frame_size(w, h);
            let (cw, ch) = (hu_pt(ow).clamp(0.0, MAX_LEN_PT), hu_pt(oh).clamp(0.0, MAX_LEN_PT));
            let (ch_w, ch_h) = if cw > 0.0 && ch > 0.0 { (cw, ch) } else { (fw.max(1.0), fh.max(1.0)) };
            (InlineObject::Group { w: fw, h: fh, float: Float::default(), ch_w, ch_h, children, canvas: None }, (0.0, 0.0, w, h))
        } else {
            self.body(el, w, h, false)?
        };
        Some(self.frame_box(obj, props, &place, w, h, inner))
    }

    /// Places `obj` (its frame `w` × `h` HWPUNIT) on its line or floating.
    pub(crate) fn frame(&mut self, obj: InlineObject, props: &CharProps, p: &Placement, w: f64, h: f64) -> Item {
        self.frame_box(obj, props, p, w, h, (0.0, 0.0, w, h))
    }

    /// Places `obj` drawn in the box `inner` (x, y, w, h HWPUNIT, from the frame's top left) of
    /// its frame `w` × `h`: a turned object's frame is the box around it, and the object keeps
    /// its own size, turned about the frame's centre.
    fn frame_box(&mut self, mut obj: InlineObject, props: &CharProps, p: &Placement, w: f64, h: f64, inner: (f64, f64, f64, f64)) -> Item {
        self.pic_n += 1;
        let n = self.pic_n;
        // one side may be 0 (a level or upright line)
        let (w, h) = if w > 0.0 || h > 0.0 { (w.max(0.0), h.max(0.0)) } else { (1.0, 1.0) };
        let own_box = inner.2 != w || inner.3 != h;
        let (fw, fh) = if own_box { frame_size(inner.2, inner.3) } else { frame_size(w, h) };
        let mut float = if p.inline {
            Float::default()
        } else {
            let (mut x, mut y) = self.offsets(p, w, h);
            if own_box {
                (x, y) = (x.saturating_add(trunc(inner.0)), y.saturating_add(trunc(inner.1)));
            }
            let dist = p.dist.map(|d| emu_pt(emu(d as f64)).clamp(0.0, 1584.0));
            let z = u32::try_from(Z_BASE.saturating_add(p.z)).ok();
            Float {
                wrap: p.wrap,
                wrap_text: if p.wrap == Wrap::Square { p.wrap_text } else { Default::default() },
                z: FileOrders::default().model_order(z),
                h_rel: p.h_rel,
                v_rel: p.v_rel,
                x: hu_pt(x as f64),
                y: hu_pt(y as f64),
                dist: dist[0].max(dist[1]),
                dist_top: dist[2],
                dist_bottom: dist[3],
                ..Float::default()
            }
        };
        if let InlineObject::Shape { extra, .. } = &mut obj {
            extra.id = n;
        }
        // The object's own turn and flips.
        let own = obj.float_mut().map(|f| Spin::new(f.rot, f.flip_h, f.flip_v)).unwrap_or_default();
        float.set_spin(own);
        // An inline turned object keeps its line to the frame: drawn in the frame's size, it
        // overhangs it; in its own size, its turned bounds are the frame.
        float.bare_turn = p.inline && float.rot != 0.0 && !own_box;
        resize(&mut obj, fw, fh);
        if let Some(f) = obj.float_mut() {
            *f = float;
        }
        Item::Obj(Box::new(obj), props.clone())
    }

    /// Members of a group, mapped into the outermost group's space by `t` (x0, y0, sx, sy:
    /// outer = x0 + inner × sx, points). Members' offsets and sizes are stored in their group's
    /// own coordinates (its original size); a member can be larger than the group.
    fn members(&mut self, el: &Element, t: (f32, f32, f32, f32), depth: usize, out: &mut Vec<GroupChild>) {
        let (x0, y0, sx, sy) = t;
        for c in el.children.iter().filter(|c| OBJECTS.contains(&c.tag.as_str())) {
            if out.len() >= MAX_GROUP_CHILDREN {
                return;
            }
            let off = c.find("hp:offset");
            let (dx, dy) = off.map(|o| (num(o.get("x")) as f64, num(o.get("y")) as f64)).unwrap_or((0.0, 0.0));
            let (cw, ch) = size_of(c);
            let (cw, ch) = (cw as f64, ch as f64);
            let fit = |v: f32, lo: f32| if v.is_finite() { v.clamp(lo, MAX_LEN_PT) } else { 0.0 };
            if c.tag == "hp:container" {
                if depth + 1 >= MAX_GROUP_DEPTH {
                    self.missing("nesting too deep");
                    continue;
                }
                let (ow, oh) = org_size(c, cw, ch);
                let (ox, oy) = (hu_pt(dx), hu_pt(dy));
                let (ew, eh) = (hu_pt(cw).max(0.0), hu_pt(ch).max(0.0));
                let (iw, ih) = (hu_pt(ow).max(0.0), hu_pt(oh).max(0.0));
                let (iw, ih) = if iw > 0.0 && ih > 0.0 { (iw, ih) } else { (ew.max(1.0), eh.max(1.0)) };
                let (kx, ky) = (ew / iw, eh / ih);
                let before = out.len();
                self.members(c, (x0 + ox * sx, y0 + oy * sy, sx * kx, sy * ky), depth + 1, out);
                if out.len() > before {
                    self.pic_n += 1; // the nested group's own id
                }
                continue;
            }
            let Some((mut obj, (bx, by, bw, bh))) = self.body_at(c, dx, dy, cw, ch, true) else { continue };
            let (w, h) = (fit(hu_pt(bw) * sx, 0.0), fit(hu_pt(bh) * sy, 0.0));
            if let InlineObject::Shape { extra, .. } = &mut obj {
                extra.id = self.pic_n;
            }
            resize(&mut obj, w, h);
            out.push(GroupChild { x: fit(x0 + hu_pt(bx) * sx, -MAX_LEN_PT), y: fit(y0 + hu_pt(by) * sy, -MAX_LEN_PT), obj });
        }
    }

    /// A top-level picture or shape in its frame `w` × `h` (HWPUNIT), and the box it was drawn in.
    fn body(&mut self, el: &Element, w: f64, h: f64, grouped: bool) -> Option<(InlineObject, (f64, f64, f64, f64))> {
        self.body_at(el, 0.0, 0.0, w, h, grouped)
    }

    fn body_at(&mut self, el: &Element, x: f64, y: f64, w: f64, h: f64, grouped: bool) -> Option<(InlineObject, (f64, f64, f64, f64))> {
        match el.tag.as_str() {
            "hp:pic" => {
                let (x, y, w, h) = unrotated(el, x, y, w, h, grouped);
                let obj = self.picture(el)?;
                Some((obj, (x, y, w.max(1.0), h.max(1.0))))
            }
            "hp:rect" | "hp:ellipse" | "hp:arc" | "hp:line" | "hp:polygon" | "hp:curve" | "hp:connectLine" => {
                Some(self.shape(el, x, y, w, h, grouped))
            }
            tag => {
                self.missing(tag);
                None
            }
        }
    }

    /// The picture's bytes stored under a media key (the same entry once).
    pub(crate) fn media(&mut self, name: &str, data: &[u8]) -> String {
        if let Some(k) = self.media_keys.get(name) {
            return k.clone();
        }
        let key = format!("image{}.{}", self.media_keys.len() + 1, image_ext(&ext_of(name), data));
        self.doc.media.insert(key.clone(), Arc::new(data.to_vec()));
        self.media_keys.insert(name.to_string(), key.clone());
        key
    }

    fn picture(&mut self, el: &Element) -> Option<InlineObject> {
        let img = el.find("hc:img").or_else(|| el.iter("hc:img").next());
        // the package outlives the converter: its bytes are copied only into a new media entry
        let pkg = self.pkg;
        let Some((name, data)) = img.and_then(|i| pkg.binary(i.get("binaryItemIDRef").unwrap_or(""))) else {
            self.missing("picture (missing image data)");
            return None;
        };
        let crop = crop(el, data);
        let media = self.media(name, data);
        self.pic_n += 1;
        let mut float = Float::default();
        // mirrored as the drawing matrices draw it (viewer, 2026-11-25: hp:flip alone doesn't
        // mirror; the scale matrix negative on that axis does)
        let (flip_h, flip_v) = mirrored(el);
        float.set_spin(Spin::new(turn(angle_of(el)), flip_h, flip_v));
        Some(InlineObject::Image { media, w: 0.0, h: 0.0, alt: String::new(), float, crop, fx: Default::default(), ole: None })
    }

    fn shape(&mut self, el: &Element, x: f64, y: f64, w: f64, h: f64, grouped: bool) -> (InlineObject, (f64, f64, f64, f64)) {
        let mut angle = angle_of(el);
        let (mut x, mut y, mut w, mut h) = (x, y, w, h);
        let mut turned = None;
        if angle != 0 && !grouped && matches!(el.tag.as_str(), "hp:polygon" | "hp:curve") {
            // Turn the points themselves inside the frame: an app may not draw a turned shape
            // whose box sticks out of its frame.
            let (_, _, uw, uh) = unrotated(el, x, y, w, h, grouped);
            turned = Some((angle, uw, uh, w, h));
            angle = 0;
        } else {
            (x, y, w, h) = unrotated(el, x, y, w, h, grouped);
        }
        let (w, h) = (w.max(0.0), h.max(0.0));
        let tag = el.tag.as_str();
        let is_line = matches!(tag, "hp:line" | "hp:connectLine");
        let (mut flip_h, mut flip_v) = (false, false);
        if is_line {
            let s = el.find("hc:startPt").or_else(|| el.find("hp:startPt"));
            let e = el.find("hc:endPt").or_else(|| el.find("hp:endPt"));
            if let (Some(s), Some(e)) = (s, e) {
                flip_h = num(e.get("x")) < num(s.get("x"));
                flip_v = num(e.get("y")) < num(s.get("y"));
            }
        }
        if let Some(fl) = el.find("hp:flip") {
            flip_h = truthy(fl.get("horizontal")) != flip_h;
            flip_v = truthy(fl.get("vertical")) != flip_v;
        }
        let mut extra = ShapeExtra { cap: LineCap::from_ooxml("flat"), ..ShapeExtra::default() };
        let mut kind = match tag {
            "hp:rect" if num(el.get("ratio")) > 0 => {
                extra.corner = u32::try_from((num(el.get("ratio")).min(50) * 1000).clamp(0, i64::from(MAX_CORNER))).ok();
                ShapeKind::RoundedRectangle
            }
            "hp:ellipse" => ShapeKind::Ellipse,
            "hp:line" | "hp:connectLine" => ShapeKind::Line,
            _ => {
                if tag == "hp:arc" {
                    // no arc shape in the model yet: its frame is drawn, and said so
                    self.missing("arcs (drawn as their frame)");
                }
                ShapeKind::Rectangle
            }
        };
        let (pw, ph) = (emu_pt(emu(w)).clamp(0.0, MAX_LEN_PT), emu_pt(emu(h)).clamp(0.0, MAX_LEN_PT));
        let ff = if matches!(tag, "hp:polygon" | "hp:curve") { freeform(el, pw, ph, turned).map(Arc::new) } else { None };
        let fill = if is_line || kind.is_open() { None } else { fill(el) };
        let (stroke, stroke_width) = outline(el, &mut extra);
        let mut story = None;
        let dt = el.find("hp:drawText");
        if let Some(sub) = dt.and_then(|d| d.find("hp:subList")) {
            let made = self.apart(|c| c.blocks(sub.findall("hp:p"), true));
            let tm = dt.and_then(|d| d.find("hp:textMargin"));
            let ins = |k: &str| tm.map(|t| emu(num(t.get(k)) as f64)).unwrap_or(TEXT_INSET);
            let anchor = match sub.get("vertAlign").unwrap_or("TOP") {
                "CENTER" => "ctr",
                "BOTTOM" => "b",
                _ => "t",
            };
            story = Some(self.text_box(made, true, &BoxText { inset: [ins("left"), ins("top"), ins("right"), ins("bottom")], anchor: Some(anchor) }));
            kind = ShapeKind::TextBox;
        }
        if ff.is_some() {
            kind = ShapeKind::Freeform;
        }
        self.pic_n += 1;
        let mut float = Float::default();
        float.set_spin(Spin::new(turn(angle), flip_h, flip_v));
        let obj =
            InlineObject::Shape { kind, w: pw, h: ph, fill, stroke, stroke_width, float, story, freeform: ff, effects: Default::default(), extra };
        (obj, (x, y, w, h))
    }

    /// A text box story holding `made`.
    fn text_box(&mut self, made: Vec<Blk>, need_final_p: bool, bt: &BoxText) -> u32 {
        let id = self.doc.add_part(PartKind::TextBox, blocks(made, need_final_p, &mut self.origins));
        if let Some(part) = self.doc.parts.get_mut(&id) {
            if let Some(a) = bt.anchor {
                part.body.anchor = valign_from_anchor(a);
            }
            let inset = bt.inset.map(|v| u32::try_from(v.clamp(0, i64::from(craft_wordmodel::MAX_TEXT_BOX_INSET))).unwrap_or(0));
            part.body.inset = (inset != TEXT_BOX_INSET).then_some(inset);
        }
        id
    }

    /// A box without fill or outline holding `made`, `w` × `h` HWPUNIT.
    fn plain_box(&mut self, story: u32, w: f64, h: f64, fill: Option<Fill>, stroke: Option<(Rgb, f32)>) -> InlineObject {
        self.pic_n += 1;
        InlineObject::Shape {
            kind: ShapeKind::TextBox,
            w: emu_pt(emu(w)).clamp(0.0, MAX_LEN_PT),
            h: emu_pt(emu(h)).clamp(0.0, MAX_LEN_PT),
            fill,
            stroke: stroke.map(|s| s.0),
            stroke_width: stroke.map_or(0.0, |s| s.1),
            float: Float::default(),
            story: Some(story),
            freeform: None,
            effects: Default::default(),
            extra: ShapeExtra { cap: LineCap::from_ooxml("flat"), ..ShapeExtra::default() },
        }
    }

    /// A table on a text line, or behind or in front of the text: an invisible text box (on the
    /// line, or floating without wrapping) holding the table. A floating table of the model
    /// always pushes the text aside.
    pub(crate) fn inline_table(&mut self, el: &Element, props: &CharProps) -> Vec<Item> {
        let mut made = self.apart(|c| c.table(el, true));
        if made.is_empty() {
            return vec![];
        }
        let sz = el.find("hp:sz");
        let (w, h) = sz.map(|s| (num(s.get("width")), num(s.get("height")))).unwrap_or((0, 0));
        if matches!(made.last(), Some(Blk::Table(_))) {
            made.push(Blk::P(Box::new(P::tiny())));
        }
        let story = self.text_box(made, false, &BoxText { inset: [0; 4], anchor: None });
        let obj = self.plain_box(story, w as f64, h as f64, None, None);
        let place = self.placement(el);
        vec![self.frame(obj, props, &place, w as f64, h as f64)]
    }

    /// A form object (Hancom: 양식 개체) as a text box showing its caption: a button, a radio
    /// button or check box with its mark, or a box.
    pub(crate) fn form(&mut self, el: &Element, props: &CharProps) -> Vec<Item> {
        let sz = el.find("hp:sz");
        let (w, h) = sz.map(|s| (num(s.get("width")), num(s.get("height")))).unwrap_or((0, 0));
        let on = !matches!(el.get("value").unwrap_or("0"), "0" | "");
        let mark = match el.tag.as_str() {
            "hp:radioBtn" => {
                if on {
                    "◉ "
                } else {
                    "○ "
                }
            }
            "hp:checkBtn" => {
                if on {
                    "☑ "
                } else {
                    "☐ "
                }
            }
            _ => "",
        };
        let caption = el.get("caption").filter(|s| !s.is_empty()).or_else(|| el.get("text")).unwrap_or("");
        let text = format!("{mark}{caption}{}", if el.tag == "hp:comboBox" { " ▾" } else { "" });
        let fprops = if el.get("charPrIDRef").is_some() { self.run_props(el.get("charPrIDRef")) } else { props.clone() };
        let boxed = !matches!(el.tag.as_str(), "hp:radioBtn" | "hp:checkBtn") && el.get("drawFrame").unwrap_or("1") != "0";
        let align = if el.tag == "hp:btn" { Align::Center } else { Align::Left };
        let para = P {
            props: ParaProps { align: Some(align), ..ParaProps::default() },
            items: if text.is_empty() { vec![] } else { vec![Item::Text(text, fprops)] },
            spacing: Some(Spacing::default()),
            ..P::default()
        };
        let fill = (el.tag == "hp:btn").then_some(Fill::Solid { color: Rgb(0xE0, 0xE0, 0xE0), transparency: 0.0 });
        let stroke = boxed.then_some((Rgb(0x80, 0x80, 0x80), 0.5));
        let story = self.text_box(vec![Blk::P(Box::new(para))], false, &BoxText { inset: [18000, 0, 18000, 0], anchor: Some("ctr") });
        let obj = self.plain_box(story, w as f64, h as f64, fill, stroke);
        let place = self.placement(el);
        vec![self.frame(obj, props, &place, w as f64, h as f64)]
    }

    /// An equation (Hancom: 수식) as its script in italics: the model has no reader for Hancom's equation script.
    pub(crate) fn equation(&mut self, el: &Element, props: &CharProps) -> Option<Item> {
        let script = el.find("hp:script").map(|s| s.text.trim().to_string()).unwrap_or_default();
        self.missing("equations (as text)");
        if script.is_empty() {
            return None;
        }
        let p = CharProps { italic: Some(true), ..props.clone() };
        Some(self.text_item(&script, &p))
    }
}

/// A frame of `w` × `h` HWPUNIT in points.
fn frame_size(w: f64, h: f64) -> (f32, f32) {
    (emu_pt(emu(w)).clamp(0.0, MAX_LEN_PT), emu_pt(emu(h)).clamp(0.0, MAX_LEN_PT))
}

/// Gives a picture, shape or group the size `w` × `h` (points), stretching a freeform's outline
/// with it.
fn resize(obj: &mut InlineObject, w: f32, h: f32) {
    match obj {
        InlineObject::Image { w: ow, h: oh, .. } | InlineObject::Group { w: ow, h: oh, .. } => (*ow, *oh) = (w, h),
        InlineObject::Shape { w: ow, h: oh, freeform, .. } => {
            if let Some(f) = freeform
                && (f.w != w || f.h != h)
            {
                let (sx, sy) = (if f.w > 0.0 { w / f.w } else { 1.0 }, if f.h > 0.0 { h / f.h } else { 1.0 });
                let f = Arc::make_mut(f);
                for p in f.paths.iter_mut().flat_map(|p| p.pts.iter_mut()) {
                    *p = [p[0] * sx, p[1] * sy];
                }
                (f.w, f.h) = (w, h);
            }
            (*ow, *oh) = (w, h);
        }
        _ => {}
    }
}
