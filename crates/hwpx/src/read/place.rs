//! Where a floating table or drawing sits (`hp:pos`, `hp:outMargin`, `textWrap`).
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use craft_wordmodel::para::{Anchor, Wrap, WrapText};

use super::text::floor_div;
use super::{Converter, signed32};
use crate::header::{num, truthy};
use crate::units::{emu_signed, trunc};
use crate::xml::Element;

/// Where and how an object sits. Lengths in HWPUNIT.
#[derive(Clone, Debug)]
pub(crate) struct Placement {
    pub inline: bool,
    /// Distance from text: left, right, top, bottom.
    pub dist: [i64; 4],
    pub z: i64,
    pub h_rel: Anchor,
    pub v_rel: Anchor,
    pub x: i64,
    pub y: i64,
    pub align_h: String,
    pub align_v: String,
    pub wrap: Wrap,
    pub wrap_text: WrapText,
}

/// HWPUNIT -> points through whole twips cut down, as distances from text always were.
pub(crate) fn dist_pt(hu: i64) -> f32 {
    floor_div(emu_signed(hu as f64), 635) as f32 / 20.0
}

impl Converter<'_> {
    pub(crate) fn placement(&self, el: &Element) -> Placement {
        let pos = el.find("hp:pos");
        let om = el.find("hp:outMargin");
        let d = |k: &str| om.map(|o| num(o.get(k))).unwrap_or(0);
        let mut p = Placement {
            inline: pos.is_none_or(|p| truthy(p.get("treatAsChar"))),
            dist: [d("left"), d("right"), d("top"), d("bottom")],
            z: num(el.get("zOrder")).max(0),
            h_rel: Anchor::Column,
            v_rel: Anchor::Paragraph,
            x: 0,
            y: 0,
            align_h: "LEFT".into(),
            align_v: "TOP".into(),
            wrap: Wrap::TopAndBottom,
            wrap_text: WrapText::BothSides,
        };
        let Some(pos) = pos else { return p };
        p.h_rel = match pos.get("horzRelTo").unwrap_or("COLUMN") {
            "PAPER" => Anchor::Page,
            "PAGE" => Anchor::Margin,
            _ => Anchor::Column,
        };
        p.v_rel = match pos.get("vertRelTo").unwrap_or("PARA") {
            "PAPER" => Anchor::Page,
            "PAGE" => Anchor::Margin,
            _ => Anchor::Paragraph,
        };
        // 32-bit values written unsigned (4294966596 is -700)
        p.x = signed32(pos.get("horzOffset"));
        p.y = signed32(pos.get("vertOffset"));
        // held in the body (flowWithText), an object placed from its paragraph doesn't go above
        // the paragraph's top; across, it goes where its offset says (Hancom's viewer, 2026-11-25)
        if p.v_rel == Anchor::Paragraph && truthy(pos.get("flowWithText")) {
            p.y = p.y.max(0);
        }
        p.align_h = pos.get("horzAlign").unwrap_or("LEFT").into();
        p.align_v = pos.get("vertAlign").unwrap_or("TOP").into();
        p.wrap_text = match el.get("textFlow").unwrap_or("") {
            "LEFT_ONLY" => WrapText::Left,
            "RIGHT_ONLY" => WrapText::Right,
            "LARGEST_ONLY" => WrapText::Largest,
            _ => WrapText::BothSides,
        };
        p.wrap = match el.get("textWrap").unwrap_or("TOP_AND_BOTTOM") {
            "SQUARE" | "TIGHT" => Wrap::Square,
            // "Through": text may fill the object's empty inside (frames drawn as outlines).
            "THROUGH" | "IN_FRONT_OF_TEXT" => Wrap::InFrontOfText,
            "BEHIND_TEXT" => Wrap::BehindText,
            _ => Wrap::TopAndBottom,
        };
        p
    }

    /// HWPUNIT offsets from the reference edge, resolving the object's alignment.
    pub(crate) fn offsets(&self, p: &Placement, w: f64, h: f64) -> (i64, i64) {
        let (s, m) = (&self.sec, &self.sec.margin);
        let (page_w, page_h) = if s.landscape { (s.width.max(s.height), s.width.min(s.height)) } else { (s.width, s.height) };
        let body_w = page_w - m.left - m.right - m.gutter;
        let ref_w = match p.h_rel {
            Anchor::Page => page_w,
            // inside a table cell the column is the cell's text (viewer: an object aligned right of
            // the column sits at the cell's right edge); `text_width` is in twips
            Anchor::Column if let Some(w) = self.text_width => w.saturating_mul(5).max(0),
            // the current column, whose left edge the offset is written from
            Anchor::Column if self.columns.0 > 1 => {
                let (n, gap) = (self.columns.0, self.columns.1.max(0));
                ((body_w - gap * (n - 1)) / n).max(0)
            }
            _ => body_w,
        } as f64;
        let ref_h = match p.v_rel {
            Anchor::Page => page_h,
            Anchor::Margin => self.body_height(),
            _ => 0,
        } as f64;
        let (mut x, mut y) = (p.x as f64, p.y as f64);
        match p.align_h.as_str() {
            "CENTER" => x += (ref_w - w) / 2.0,
            "RIGHT" | "OUTSIDE" => x = ref_w - w - x,
            _ => {}
        }
        if ref_h != 0.0 {
            match p.align_v.as_str() {
                "CENTER" => y += (ref_h - h) / 2.0,
                "BOTTOM" | "OUTSIDE" => y = ref_h - h - y,
                _ => {}
            }
        }
        (trunc(x), trunc(y))
    }
}
