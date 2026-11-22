//! A section's master page (Hancom: 바탕쪽), page background (Hancom: 쪽 배경) and a page border measured from the body.
//!
//! The model has no master page and one background for the whole document only, so they ride
//! in the section's headers: objects behind the text that take no room, in paragraphs of
//! (almost) no height, repeated on every page like the header.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use craft_wordmodel::InlineObject;
use craft_wordmodel::connector::{LineCap, ShapeExtra};
use craft_wordmodel::fill::Fill;
use craft_wordmodel::line_dash::LineDash;
use craft_wordmodel::para::{Anchor, FileOrders, Float, ShapeKind, Wrap};
use craft_wordmodel::props::{CharProps, Rgb};

use super::drawing::gradient_fill;
use super::media::pixel_size;
use super::props::rgb;
use super::section::{Hf, Section};
use super::{Blk, Converter, Item, P};
use crate::header::Border;
use crate::units::{emu, round_even};
use crate::xml::Element;

/// The most page background tiles laid one by one; more are drawn as one picture.
const MAX_BACKGROUND_TILES: i64 = 10_000;

/// What a background box shows.
enum BoxFill {
    Shape(Option<Fill>, Option<(Rgb, f32, LineDash)>),
    Picture(String, [f32; 4]),
}

fn emu_pt(e: i64) -> f32 {
    (e as f64 / 12_700.0) as f32
}

/// The line of a page border measured from the body, or None. A border measured from the paper
/// is the section's own page border.
fn frame_border(s: &Section) -> Option<&Border> {
    let pb = s.page_border.as_ref().filter(|pb| !pb.paper)?;
    [&pb.bf.top, &pb.bf.left, &pb.bf.bottom, &pb.bf.right].into_iter().find(|b| b.visible())
}

/// Master pages count "page" positions from the paper (offsets from the sheet's edges match the
/// viewer).
fn set_paper_positions(el: &mut Element, depth: usize) {
    if depth > 256 {
        return;
    }
    if el.tag == "hp:pos" {
        for k in ["horzRelTo", "vertRelTo"] {
            if el.get(k) == Some("PAGE") {
                el.set(k, "PAPER");
            }
        }
    }
    for c in el.children.iter_mut() {
        set_paper_positions(c, depth + 1);
    }
}

impl Converter<'_> {
    pub(crate) fn has_background(&self) -> bool {
        let Some(pb) = &self.sec.page_border else { return false };
        let filled = pb.bf.image.is_some() || pb.bf.fill.as_deref().is_some_and(|f| !matches!(f, "FFFFFF" | "none"));
        filled || frame_border(&self.sec).is_some()
    }

    /// A section with a master page or background needs a default header to carry it: its own,
    /// a copy of the one it inherits, or a new empty one.
    pub(crate) fn background_header(&mut self) {
        let has_default_header = self.sec.hf.iter().any(|h| h.kind == "header" && h.which == "default");
        if !(self.has_background() || self.sec.master.is_some()) || has_default_header {
            return;
        }
        let inherits = self.carry.inherited_refs.iter().any(|(k, _)| *k == ("header", "default"));
        match self.carry.inherited_hf.get(&("header", "default")).cloned() {
            // the earlier section's header continues here: this section takes a copy of its own
            // to add its master page to
            Some((_, blocks)) if inherits => {
                self.carry.hf_n += 1;
                let name = format!("header{}", self.carry.hf_n);
                self.sec.hf.push(Hf { kind: "header", which: "default", name, blocks: Some(blocks) });
            }
            _ if inherits => self.missing("master pages or backgrounds not shown (inherited header)"),
            _ => {
                self.carry.hf_n += 1;
                let name = format!("header{}", self.carry.hf_n);
                self.sec.hf.push(Hf { kind: "header", which: "default", name, blocks: Some(vec![]) });
            }
        }
    }

    /// The master page's objects at the start of the default header: behind everything, taking
    /// no room.
    pub(crate) fn master_page(&mut self) {
        let Some(mut master) = self.sec.master.take() else { return };
        set_paper_positions(&mut master, 0);
        let mut hf = std::mem::take(&mut self.sec.hf);
        for h in hf.iter_mut() {
            if h.kind == "header"
                && h.which == "default"
                && let Some(blocks) = h.blocks.as_mut()
            {
                let mut made = self.blocks(master.findall("hp:p"), true);
                for b in made.iter_mut() {
                    if let Blk::P(bp) = b {
                        let tiny = P::tiny();
                        (bp.props, bp.mark, bp.spacing) = (tiny.props, tiny.mark, tiny.spacing);
                        for i in bp.items.iter_mut() {
                            if let Item::Obj(o, _) = i
                                && let Some(f) = o.float_mut()
                                && f.wrap != Wrap::Inline
                            {
                                f.wrap = Wrap::BehindText;
                            }
                        }
                    }
                }
                made.append(blocks);
                *blocks = made;
            }
        }
        self.sec.hf = hf;
        self.sec.master = Some(master);
    }

    /// The page background and body-measured page border at the start of every header.
    pub(crate) fn backgrounds_in_headers(&mut self) {
        let mut hf = std::mem::take(&mut self.sec.hf);
        for h in hf.iter_mut() {
            if h.kind == "header"
                && let Some(blocks) = h.blocks.as_mut()
            {
                let objs = self.page_background();
                if !objs.is_empty() {
                    let items = objs.into_iter().map(|o| Item::Obj(Box::new(o), CharProps::default())).collect();
                    blocks.insert(0, Blk::P(Box::new(P { items, ..P::tiny() })));
                }
            }
        }
        self.sec.hf = hf;
    }

    fn page_background(&mut self) -> Vec<InlineObject> {
        if !self.has_background() {
            return vec![];
        }
        let Some(bf) = self.sec.page_border.as_ref().map(|pb| pb.bf.clone()) else { return vec![] };
        let frame = self.page_frame();
        if bf.image.is_none() && !bf.fill.as_deref().is_some_and(|f| !matches!(f, "FFFFFF" | "none")) {
            return frame.into_iter().collect();
        }
        let mut out = self.page_fill();
        out.extend(frame);
        out
    }

    /// A page border measured from the body, as a box around it.
    fn page_frame(&mut self) -> Option<InlineObject> {
        let b = frame_border(&self.sec).cloned()?;
        let pb = self.sec.page_border.as_ref()?;
        let (m, off) = (self.sec.margin.clone(), pb.off);
        let s = &self.sec;
        let (w, h) = if s.landscape { (s.width.max(s.height), s.width.min(s.height)) } else { (s.width, s.height) };
        let (x, y) = (m.left - off[0], m.top + m.header - off[2]);
        let fw = w - m.left - m.right + off[0] + off[1];
        let fh = h - m.top - m.header - m.bottom - m.footer + off[2] + off[3];
        let dash = match b.kind.as_str() {
            "DOT" | "CIRCLE" => "sysDot",
            "DASH" => "dash",
            "DASH_DOT" => "dashDot",
            "DASH_DOT_DOT" => "sysDashDotDot",
            "LONG_DASH" => "lgDash",
            _ => "solid",
        };
        let width = emu_pt(round_even(b.width_mm * 36000.0)).clamp(0.0, 100.0);
        let color = b.color.as_deref().and_then(rgb).unwrap_or(Rgb(0, 0, 0));
        Some(self.background_box(Anchor::Page, x, y, fw, fh, BoxFill::Shape(None, Some((color, width, LineDash::from_ooxml(dash))))))
    }

    /// The page background: a colour, a gradient, a stretched picture or picture tiles.
    fn page_fill(&mut self) -> Vec<InlineObject> {
        let Some(pb) = self.sec.page_border.clone() else { return vec![] };
        let bf = &pb.bf;
        let s = &self.sec;
        let (mut w, mut h) = if s.landscape { (s.width.max(s.height), s.width.min(s.height)) } else { (s.width, s.height) };
        let (mut rel, x, y) = (Anchor::Page, 0i64, 0i64);
        if pb.area != "PAPER" {
            // the fill covers the page or the border (`fillArea` PAGE or BORDER), not the
            // paper: here the body area
            let m = &s.margin;
            rel = Anchor::Margin;
            w = w - m.left - m.right;
            h = h - m.top - m.header - m.bottom - m.footer;
        }
        let Some((id, mode)) = bf.image.clone() else {
            let fill = match &bf.gradient {
                // the same mapping as drawn shapes
                Some(g) => gradient_fill(&g.kind, g.angle, &g.colors.iter().filter_map(|c| rgb(c)).collect::<Vec<_>>()),
                None => rgb(bf.fill.as_deref().unwrap_or("FFFFFF")).map(|color| Fill::Solid { color, transparency: 0.0 }),
            };
            return vec![self.background_box(rel, x, y, w, h, BoxFill::Shape(fill, None))];
        };
        let pkg = self.pkg;
        let Some((name, data)) = pkg.binary(&id) else { return vec![] };
        let media = self.media(name, data);
        let px = pixel_size(data).filter(|(a, b)| *a > 0 && *b > 0);
        let Some((pw, ph)) = px.filter(|_| mode.starts_with("TILE")) else {
            return vec![self.background_box(rel, x, y, w, h, BoxFill::Picture(media, [0.0; 4]))];
        };
        // Tiles at 96 dpi (Hancom's size for a picture without resolution), laid one by one;
        // tiles at the far edges are cropped to the area.
        let (tw, th) = (i64::from(pw) * 75, i64::from(ph) * 75);
        let tiles = |len: i64, tile: i64| if tile > 0 { (len.max(0) + tile - 1) / tile } else { 0 };
        if tiles(w, tw).saturating_mul(tiles(h, th)) > MAX_BACKGROUND_TILES {
            // Too many tiles to lay one by one (a tiny picture over a page): the picture
            // stretched over the area instead, and said so.
            self.missing("page background tiles (one stretched picture instead)");
            return vec![self.background_box(rel, x, y, w, h, BoxFill::Picture(media, [0.0; 4]))];
        }
        let mut out = Vec::new();
        let mut ty = 0i64;
        let mut guard = 0;
        while ty < h && guard < MAX_BACKGROUND_TILES {
            let mut tx = 0i64;
            while tx < w && guard < MAX_BACKGROUND_TILES {
                guard += 1;
                let (cw, ch) = (tw.min(w - tx), th.min(h - ty));
                let cut = |part: i64, whole: i64| {
                    (round_even((1.0 - part as f64 / whole as f64) * 100000.0).clamp(0, 100_000) as f32 / 100_000.0).clamp(0.0, 1.0)
                };
                let crop = [0.0, 0.0, cut(cw, tw), cut(ch, th)];
                out.push(self.background_box(rel, x + tx, y + ty, cw, ch, BoxFill::Picture(media.clone(), crop)));
                tx += tw;
            }
            ty += th;
        }
        out
    }

    /// A box behind the text at `x`, `y` from the page or margin, `w` × `h` (HWPUNIT).
    fn background_box(&mut self, rel: Anchor, x: i64, y: i64, w: i64, h: i64, fill: BoxFill) -> InlineObject {
        self.pic_n += 1;
        let float = Float {
            wrap: Wrap::BehindText,
            z: FileOrders::default().model_order(Some(0)),
            h_rel: rel,
            v_rel: rel,
            x: emu_pt(emu(x)),
            y: emu_pt(emu(y)),
            ..Float::default()
        };
        let (w, h) = (emu_pt(emu(w)).clamp(0.0, 14_400.0), emu_pt(emu(h)).clamp(0.0, 14_400.0));
        match fill {
            BoxFill::Picture(media, crop) => InlineObject::Image { media, w, h, alt: String::new(), float, crop, fx: Default::default(), ole: None },
            BoxFill::Shape(fill, line) => {
                let mut extra = ShapeExtra { id: self.pic_n, cap: LineCap::from_ooxml("flat"), ..ShapeExtra::default() };
                if let Some((_, _, dash)) = &line {
                    extra.dash = dash.clone();
                }
                InlineObject::Shape {
                    kind: ShapeKind::Rectangle,
                    w,
                    h,
                    fill,
                    stroke: line.as_ref().map(|l| l.0),
                    stroke_width: line.as_ref().map_or(0.0, |l| l.1),
                    float,
                    story: None,
                    freeform: None,
                    effects: Default::default(),
                    extra,
                }
            }
        }
    }
}
