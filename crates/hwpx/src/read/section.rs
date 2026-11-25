//! Sections: page size and margins, columns, headers and footers, page numbers.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::collections::{HashMap, HashSet};

use craft_wordmodel::props::{Align, Borders, ParaProps};
use craft_wordmodel::section::{Columns, SectionStart};
use craft_wordmodel::{PartKind, SectionProps};

use super::props::{border, pt};
use super::{A4, Blk, Converter, Item, P};
use crate::header::{BorderFill, num, num_or};
use crate::units::{round_even, tw, tw_f};
use crate::xml::Element;

/// Most table cells made up during one reading (merged-cell rows and empty cells a table
/// declares but doesn't hold). A table may declare 10,000 rows of 64 columns with one cell in
/// them; repeated, that is millions of cells from a small file. Past this, declared rows without
/// cells are left out.
pub(crate) const MAX_MADE_CELLS: i64 = 200_000;

/// The largest page size or margin read from a file, in HWPUNIT (about 2,000 inches). Real pages
/// are far smaller; the bound keeps every sum of page lengths in range.
const MAX_PAGE_LENGTH: i64 = 14_400_000;

#[derive(Clone, Debug)]
pub(crate) struct Margins {
    pub left: i64,
    pub right: i64,
    pub top: i64,
    pub bottom: i64,
    pub header: i64,
    pub footer: i64,
    pub gutter: i64,
}

impl Default for Margins {
    fn default() -> Self {
        Margins { left: 8504, right: 8504, top: 5668, bottom: 4252, header: 4252, footer: 4252, gutter: 0 }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PageBorder {
    pub bf: BorderFill,
    pub paper: bool,
    pub area: String,
    /// left, right, top, bottom
    pub off: [i64; 4],
}

/// A header or footer of the section being read.
#[derive(Clone, Debug)]
pub(crate) struct Hf {
    pub kind: &'static str,
    pub which: &'static str,
    /// Names the story: two references with one name show one story.
    pub name: String,
    /// None: the same story as another reference (see first_page_parts).
    pub blocks: Option<Vec<Blk>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HideFirst {
    pub header: bool,
    pub footer: bool,
    pub pagenum: bool,
    pub master: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Section {
    pub first_section: bool,
    pub width: i64,
    pub height: i64,
    pub landscape: bool,
    pub vertical: bool,
    pub margin: Margins,
    pub start_page: i64,
    pub outline: Option<i64>,
    pub tab_stop: i64,
    /// (pos, formatType, sideChar)
    pub page_num: Option<(String, String, String)>,
    pub hf: Vec<Hf>,
    /// ((kind, which), story name) of the section's own references.
    pub refs: Vec<((&'static str, &'static str), String)>,
    pub have_secpr: bool,
    pub title_pg: bool,
    /// Page of the current paragraph (Hancom's layout).
    pub page_index: i64,
    pub hide_first: Option<HideFirst>,
    pub page_border: Option<PageBorder>,
    /// The master page (Hancom: 바탕쪽): its paragraphs.
    pub master: Option<Element>,
}

impl Default for Section {
    fn default() -> Self {
        Section {
            first_section: false,
            width: A4.0,
            height: A4.1,
            landscape: false,
            vertical: false,
            margin: Margins::default(),
            start_page: 0,
            outline: None,
            tab_stop: 8000,
            page_num: None,
            hf: Vec::new(),
            refs: Vec::new(),
            have_secpr: false,
            title_pg: false,
            page_index: 0,
            hide_first: None,
            page_border: None,
            master: None,
        }
    }
}

/// What a section passes on to the ones after it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Carry {
    pub hf_n: i64,
    pub even_odd: bool,
    /// The references the sections so far left in force, by (kind, which).
    pub inherited_refs: Vec<((&'static str, &'static str), String)>,
    /// The last header/footer of each kind a section made: its story name and its own blocks
    /// (before the section's master page and background went in), for a later section that
    /// inherits it but needs one of its own to add its master page or background to.
    pub inherited_hf: HashMap<(&'static str, &'static str), (String, Vec<Blk>)>,
    /// The part each story name was made into.
    pub parts: HashMap<String, u32>,
    /// Headers that carry their section's master page or page background: a later section that
    /// would inherit one takes a plain copy instead (Hancom draws both in their own section
    /// only).
    pub decorated: HashSet<String>,
    /// The page number (pos, formatType, sideChar) each inherited story (kind, which) shows.
    pub page_num_of: HashMap<(&'static str, &'static str), (String, String, String)>,
    /// Master page parts read so far, by manifest id: sections naming the same one don't read
    /// it again.
    pub masters: HashMap<String, Option<Element>>,
}

fn set_ref(refs: &mut Vec<((&'static str, &'static str), String)>, key: (&'static str, &'static str), name: String) {
    match refs.iter_mut().find(|(k, _)| *k == key) {
        Some(r) => r.1 = name,
        None => refs.push((key, name)),
    }
}

impl Converter<'_> {
    pub(crate) fn sec_pr(&mut self, el: &Element) {
        if self.sec.have_secpr {
            return;
        }
        self.sec.have_secpr = true;
        if el.get("outlineShapeIDRef").is_some() {
            self.sec.outline = Some(num(el.get("outlineShapeIDRef")));
        }
        if el.get("tabStop").is_some_and(|s| !s.is_empty()) {
            self.sec.tab_stop = num_or(el.get("tabStop"), 8000);
        }
        self.sec.vertical = el.get("textDirection").filter(|s| !s.is_empty()).unwrap_or("HORIZONTAL").starts_with("VERTICAL");
        if let Some(pg) = el.find("hp:pagePr") {
            self.sec.width = num_or(pg.get("width"), A4.0).clamp(0, MAX_PAGE_LENGTH);
            self.sec.height = num_or(pg.get("height"), A4.1).clamp(0, MAX_PAGE_LENGTH);
            // WIDELY is the usual (portrait) value; NARROWLY turns the sheet sideways.
            self.sec.landscape = pg.get("landscape") == Some("NARROWLY");
            if let Some(m) = pg.find("hp:margin") {
                let mm = &mut self.sec.margin;
                for (k, slot) in [
                    ("left", &mut mm.left),
                    ("right", &mut mm.right),
                    ("top", &mut mm.top),
                    ("bottom", &mut mm.bottom),
                    ("header", &mut mm.header),
                    ("footer", &mut mm.footer),
                    ("gutter", &mut mm.gutter),
                ] {
                    if m.get(k).is_some() {
                        // bounded, so the sums the page geometry takes of them can't overflow
                        *slot = num(m.get(k)).clamp(0, MAX_PAGE_LENGTH);
                    }
                }
            }
        }
        // A master page is a part of its own that the section names (`hp:masterPage idRef`,
        // Hancom's files; the viewer opens no other form), or, as craft-hwp rebuilds HWP 5.0,
        // a paragraph list in the section definition.
        for mp in el.findall("hp:masterPage") {
            let part = match mp.find("hp:subList") {
                Some(_) => None,
                None => mp.get("idRef").and_then(|id| self.master_part(id).cloned()),
            };
            let page = part.as_ref().unwrap_or(mp);
            let kind = page.get("type").or(mp.get("type")).unwrap_or("BOTH");
            if kind == "BOTH"
                && let Some(sub) = page.find("hp:subList")
            {
                self.sec.master = Some(sub.clone());
                break;
            }
        }
        if let Some(pbf) = el.findall("hp:pageBorderFill").find(|x| x.get("type").unwrap_or("BOTH") == "BOTH")
            && let Some(bf) = self.h.border_fills.get(&num(pbf.get("borderFillIDRef")))
        {
            let off = pbf.find("hp:offset");
            let o = |k: &str| off.map(|x| num(x.get(k))).unwrap_or(0);
            self.sec.page_border = Some(PageBorder {
                bf: bf.clone(),
                paper: pbf.get("textBorder") == Some("PAPER"),
                area: pbf.get("fillArea").unwrap_or("PAPER").to_string(),
                off: [o("left"), o("right"), o("top"), o("bottom")],
            });
        }
        match el.find("hp:startNum") {
            Some(sn) if num(sn.get("page")) > 0 => self.sec.start_page = num(sn.get("page")),
            _ => {
                if self.sec.first_section && self.h.begin_page > 1 {
                    self.sec.start_page = self.h.begin_page;
                }
            }
        }
    }

    /// The master page part a section names by its manifest id (read once).
    fn master_part(&mut self, id: &str) -> Option<&Element> {
        if !self.carry.masters.contains_key(id) {
            let href = match self.pkg.item(id) {
                Some(it) => it.href.clone(),
                None => format!("Contents/{id}.xml"),
            };
            let part = self.pkg.xml(&href).ok();
            self.carry.masters.insert(id.to_string(), part);
        }
        self.carry.masters.get(id).and_then(Option::as_ref)
    }

    /// Twips of the text column (the cell's, inside a table).
    pub(crate) fn line_width(&self) -> i64 {
        if let Some(w) = self.text_width {
            return w;
        }
        let (s, m) = (&self.sec, &self.sec.margin);
        let page_w = if s.landscape { s.width.max(s.height) } else { s.width };
        let (cols, gap) = self.columns;
        // less the binding margin, as objects are placed (viewer, 2026-11-25: two tables that
        // fit the width without it go one under the other with it)
        tw_f((page_w - m.left - m.right - m.gutter.max(0) - gap * (cols - 1)) as f64 / cols.max(1) as f64)
    }

    /// HWPUNIT height of the current section's body (page minus margins, header, footer).
    pub(crate) fn body_height(&self) -> i64 {
        let (s, m) = (&self.sec, &self.sec.margin);
        let page_h = if s.landscape { s.width.min(s.height) } else { s.height };
        page_h - m.top - m.header - m.bottom - m.footer
    }

    pub(crate) fn finish_section(&mut self) {
        // A master page or page background is drawn in its own section only, on every page of it
        // whichever header the page shows (viewer, 2026-11-25).
        let decorated = self.has_background() || self.sec.master.is_some();
        if !decorated {
            self.plain_inherited_headers();
        }
        match self.sec.page_num.as_ref().map(|(pos, _, _)| pos.as_str()) {
            // no page number from here on: an inherited story loses its number, keeps its text
            // (viewer, 2026-11-25)
            Some("NONE") => self.stop_page_number(),
            Some("") | None => {}
            Some(_) => self.add_page_number(),
        }
        let has_master = self.sec.master.is_some();
        self.sec.title_pg = self.sec.hide_first.is_some_and(|h| h.header || h.footer || h.pagenum || (h.master && has_master));
        if decorated {
            self.background_header();
        }
        // this section's own headers and footers, as later sections may inherit them
        for h in &self.sec.hf {
            if let Some(blocks) = &h.blocks {
                self.carry.inherited_hf.insert((h.kind, h.which), (h.name.clone(), blocks.clone()));
            }
        }
        if self.sec.title_pg {
            self.first_page_parts();
        }
        self.master_page();
        self.backgrounds_in_headers();
        let hf = std::mem::take(&mut self.sec.hf);
        for h in hf.iter() {
            if let Some(blocks) = &h.blocks {
                let kind = if h.kind == "header" { PartKind::Header } else { PartKind::Footer };
                let id = self.doc.add_part(kind, super::blocks(blocks.clone(), true, &mut self.origins));
                self.carry.parts.insert(h.name.clone(), id);
                if decorated && h.kind == "header" && h.which != "first" {
                    self.carry.decorated.insert(h.name.clone());
                }
            }
            set_ref(&mut self.sec.refs, (h.kind, h.which), h.name.clone());
        }
        self.sec.hf = hf;
        for (k, v) in self.sec.refs.clone() {
            set_ref(&mut self.carry.inherited_refs, k, v);
        }
    }

    fn add_page_number(&mut self) {
        let Some((pos, fmt, side)) = self.sec.page_num.clone() else { return };
        let kind: &'static str = if pos.contains("BOTTOM") { "footer" } else { "header" };
        let align = if pos.ends_with("LEFT") {
            Align::Left
        } else if pos.ends_with("RIGHT") || pos.contains("SIDE") {
            Align::Right
        } else {
            Align::Center
        };
        let switch = match fmt.as_str() {
            "ROMAN_CAPITAL" => " \\* ROMAN",
            "ROMAN_SMALL" => " \\* roman",
            "LATIN_CAPITAL" => " \\* ALPHABETIC",
            "LATIN_SMALL" => " \\* alphabetic",
            _ => "",
        };
        let props = self.run_props(Some("0"));
        let mut items = Vec::new();
        if !side.is_empty() {
            items.push(Item::Text(format!("{side} "), props.clone()));
        }
        items.push(super::field(&format!("PAGE{switch}"), &props));
        if !side.is_empty() {
            items.push(Item::Text(format!(" {side}"), props.clone()));
        }
        let para = P { props: ParaProps { align: Some(align), ..ParaProps::default() }, items, page_num: true, ..P::default() };
        let number = (pos, fmt, side);
        // The number is on every page: in the default story, and in the even pages' own story
        // when there is one (viewer, 2026-11-25).
        for which in ["default", "even"] {
            let own = self.sec.hf.iter().any(|h| h.kind == kind && h.which == which);
            let inherited = self.carry.inherited_refs.iter().any(|(k, _)| *k == (kind, which));
            if !own && !inherited && which == "even" {
                continue; // the default story is on even pages too
            }
            self.number_story(kind, which, &para, &number);
        }
    }

    /// Story (`kind`, `which`) of this section showing the page number `para`: its own story
    /// with the number in place of any other, a copy of an inherited one (unless that already
    /// shows this number), or a story of just the number.
    fn number_story(&mut self, kind: &'static str, which: &'static str, para: &P, number: &(String, String, String)) {
        if let Some(h) = self.sec.hf.iter_mut().find(|h| h.kind == kind && h.which == which) {
            if let Some(b) = h.blocks.as_mut() {
                b.retain(|b| !matches!(b, Blk::P(p) if p.page_num));
                b.push(Blk::P(Box::new(para.clone())));
            }
            self.carry.page_num_of.insert((kind, which), number.clone());
            return;
        }
        if self.carry.inherited_refs.iter().any(|(k, _)| *k == (kind, which)) {
            // An earlier section's header/footer continues here. If it shows this page number
            // it does here too; if not (text only, or another number: the viewer shows this
            // section's number in its place), this section takes a copy of its own with the
            // number.
            let Some((_, blocks)) = self.carry.inherited_hf.get(&(kind, which)).cloned() else { return };
            let shows = blocks.iter().any(|b| matches!(b, Blk::P(p) if p.page_num));
            if shows && self.carry.page_num_of.get(&(kind, which)) == Some(number) {
                return;
            }
            let mut blocks: Vec<Blk> = blocks.into_iter().filter(|b| !matches!(b, Blk::P(p) if p.page_num)).collect();
            blocks.push(Blk::P(Box::new(para.clone())));
            self.carry.page_num_of.insert((kind, which), number.clone());
            self.carry.hf_n += 1;
            let name = format!("{kind}{}", self.carry.hf_n);
            self.sec.hf.push(Hf { kind, which, name, blocks: Some(blocks) });
            return;
        }
        self.carry.page_num_of.insert((kind, which), number.clone());
        self.carry.hf_n += 1;
        self.sec.hf.push(Hf { kind, which, name: format!("{kind}{}", self.carry.hf_n), blocks: Some(vec![Blk::P(Box::new(para.clone()))]) });
    }

    /// A section asking for no page number: each story it would inherit showing one becomes a
    /// copy of its own without it (its text kept), as its own stories are.
    fn stop_page_number(&mut self) {
        for kind in ["header", "footer"] {
            for which in ["default", "even"] {
                if let Some(h) = self.sec.hf.iter_mut().find(|h| h.kind == kind && h.which == which) {
                    if let Some(b) = h.blocks.as_mut() {
                        b.retain(|b| !matches!(b, Blk::P(p) if p.page_num));
                    }
                } else if self.carry.inherited_refs.iter().any(|(k, _)| *k == (kind, which))
                    && let Some((_, blocks)) = self.carry.inherited_hf.get(&(kind, which)).cloned()
                    && blocks.iter().any(|b| matches!(b, Blk::P(p) if p.page_num))
                {
                    let blocks: Vec<Blk> = blocks.into_iter().filter(|b| !matches!(b, Blk::P(p) if p.page_num)).collect();
                    self.carry.hf_n += 1;
                    let name = format!("{kind}{}", self.carry.hf_n);
                    self.sec.hf.push(Hf { kind, which, name, blocks: Some(blocks) });
                }
                self.carry.page_num_of.remove(&(kind, which));
            }
        }
    }

    /// A section without a master page or background that would inherit an earlier section's
    /// header carrying one takes a plain copy of that header instead.
    fn plain_inherited_headers(&mut self) {
        for which in ["default", "even"] {
            if self.sec.hf.iter().any(|h| h.kind == "header" && h.which == which) {
                continue;
            }
            let Some((_, name)) = self.carry.inherited_refs.iter().find(|(k, _)| *k == ("header", which)) else { continue };
            if !self.carry.decorated.contains(name) {
                continue;
            }
            let Some((_, blocks)) = self.carry.inherited_hf.get(&("header", which)).cloned() else { continue };
            self.carry.hf_n += 1;
            let name = format!("header{}", self.carry.hf_n);
            self.sec.hf.push(Hf { kind: "header", which, name, blocks: Some(blocks) });
        }
    }

    /// First-page header/footer for hp:pageHiding: empty where hidden, else the default.
    fn first_page_parts(&mut self) {
        let Some(hide) = self.sec.hide_first else { return };
        for kind in ["header", "footer"] {
            let default = self.sec.hf.iter().find(|h| h.kind == kind && h.which == "default").map(|h| (h.name.clone(), h.blocks.clone()));
            // without one of its own, the section shows an earlier section's (with that one's
            // page number, when it has one)
            let shown = default.clone().or_else(|| {
                self.carry
                    .inherited_refs
                    .iter()
                    .any(|(k, _)| *k == (kind, "default"))
                    .then(|| self.carry.inherited_hf.get(&(kind, "default")).cloned().map(|(name, blocks)| (name, Some(blocks))))
                    .flatten()
            });
            // the story shown, own or inherited, has a page number to hide
            let has_pn = shown.as_ref().and_then(|(_, b)| b.as_ref()).is_some_and(|b| b.iter().any(|b| matches!(b, Blk::P(p) if p.page_num)));
            let hidden = if kind == "header" { hide.header } else { hide.footer };
            let blocks = if hidden {
                vec![]
            } else if hide.pagenum && has_pn {
                shown
                    .as_ref()
                    .and_then(|(_, b)| b.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|b| !matches!(b, Blk::P(p) if p.page_num))
                    .collect()
            } else if kind == "header" && hide.master && self.sec.master.is_some() {
                // the first page hides the master page only: a header of its own, without it
                shown.as_ref().and_then(|(_, b)| b.clone()).unwrap_or_default()
            } else if let Some((name, _)) = default {
                self.sec.hf.push(Hf { kind, which: "first", name, blocks: None });
                continue;
            } else if let Some((_, name)) = self.carry.inherited_refs.iter().find(|(k, _)| *k == (kind, "default")).cloned() {
                set_ref(&mut self.sec.refs, (kind, "first"), name);
                continue;
            } else {
                continue;
            };
            self.carry.hf_n += 1;
            let name = format!("{kind}{}", self.carry.hf_n);
            self.sec.hf.push(Hf { kind, which: "first", name, blocks: Some(blocks) });
        }
    }

    /// The model's section properties for one column region of an HWP section (the first one
    /// starts a new page).
    pub(crate) fn section_props(&self, cols: i64, gap: i64, first: bool, new_page: bool) -> SectionProps {
        let s = &self.sec;
        let mut sp = SectionProps::default();
        for ((kind, which), name) in &s.refs {
            let Some(&id) = self.carry.parts.get(name) else { continue };
            let set = if *kind == "header" { &mut sp.headers } else { &mut sp.footers };
            match *which {
                "first" => set.first = Some(id),
                "even" => set.even = Some(id),
                _ => set.default = Some(id),
            }
        }
        sp.start = if first || new_page { SectionStart::NextPage } else { SectionStart::Continuous };
        let (mut w, mut h) = (s.width, s.height);
        if s.landscape {
            (w, h) = (w.max(h), w.min(h));
        }
        let (pw, ph) = (pt(tw(w)), pt(tw(h)));
        if pw >= 36.0 {
            sp.page_w = pw;
        }
        if ph >= 36.0 {
            sp.page_h = ph;
        }
        sp.landscape = s.landscape || sp.page_w > sp.page_h;
        let mut m = s.margin.clone();
        if s.vertical {
            // Hancom turns the margins with the text (a line starts at the "left" margin, at
            // the top of the sheet); the model's margins stay on the sheet.
            let o = s.margin.clone();
            m = Margins { top: o.left, right: o.top + o.header, bottom: o.right, left: o.bottom + o.footer, header: 0, footer: 0, gutter: o.gutter };
        }
        // HWP measures the body from top+header (and bottom+footer); the header itself starts
        // at the top margin.
        let limit = sp.page_w.max(sp.page_h);
        let len = |hu: i64| pt(tw(hu)).abs().min(limit);
        sp.margin_top = len(m.top + m.header);
        sp.margin_right = len(m.right);
        sp.margin_bottom = len(m.bottom + m.footer);
        sp.margin_left = len(m.left);
        sp.header = len(m.top);
        sp.footer = len(m.bottom);
        sp.gutter = len(m.gutter);
        if let Some(b) = self.page_borders() {
            sp.page_borders = Some(b);
        }
        if s.start_page != 0 && first {
            sp.page_num_start = u32::try_from(s.start_page).ok();
        }
        sp.columns = if cols > 1 {
            Columns { count: u32::try_from(cols).unwrap_or(1), space: pt(tw(gap)).max(0.0), ..Columns::default() }
        } else {
            Columns { count: 1, space: 36.0, ..Columns::default() }
        };
        sp.title_page = s.title_pg && first;
        if s.vertical {
            sp.text_direction = craft_wordmodel::props::TextDirection::from_ooxml("tbRl");
        }
        sp
    }

    /// A page border measured from the paper, as the section's page borders.
    fn page_borders(&self) -> Option<Borders> {
        let pb = self.sec.page_border.as_ref().filter(|pb| pb.paper)?;
        let side = |b: &crate::header::Border, off: i64| {
            border(b).filter(|_| b.visible()).map(|mut x| {
                x.space = round_even(off as f64 / 100.0).min(31) as f32;
                x
            })
        };
        let b = Borders {
            top: side(&pb.bf.top, pb.off[2]),
            left: side(&pb.bf.left, pb.off[0]),
            bottom: side(&pb.bf.bottom, pb.off[3]),
            right: side(&pb.bf.right, pb.off[1]),
            ..Borders::default()
        };
        (b.top.is_some() || b.left.is_some() || b.bottom.is_some() || b.right.is_some()).then_some(b)
    }
}
