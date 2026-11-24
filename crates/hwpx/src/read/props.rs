//! Character and paragraph shapes of the header as `craft-wordmodel` properties, styles and
//! list definitions.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use craft_wordmodel::numbering::{AbstractNum, Level, LevelSuffix, Num};
use craft_wordmodel::props::{
    Align, Border, BorderStyle, Borders, CharProps, EmphasisMark, LineSpacing, NumRef, ParaProps, Rgb, TabAlign, TabLeader, TabStop, TextColor,
    Underline, VertAlign,
};
use craft_wordmodel::section::NumFormat;
use craft_wordmodel::styles::{Style, StyleKind, StyleSheet};

use super::Converter;
use super::drawing::MAX_LEN_PT;
use crate::header::{CharPr, ParaPr};
use crate::units::{round_even, tw, tw_f};

/// Twips as points, the way a DOCX reader takes them.
pub(crate) fn pt(twips: i64) -> f32 {
    twips as f32 / 20.0
}

pub(crate) fn rgb(hex: &str) -> Option<Rgb> {
    Rgb::parse(hex)
}

fn align(a: &str) -> Align {
    match a {
        "LEFT" => Align::Left,
        "RIGHT" => Align::Right,
        "CENTER" => Align::Center,
        "DISTRIBUTE" | "DISTRIBUTE_SPACE" => Align::Distribute,
        _ => Align::Justify,
    }
}

pub(crate) fn underline(shape: &str) -> Underline {
    Underline::from_ooxml(match shape {
        "DOT" | "CIRCLE" => "dotted",
        "DASH" => "dash",
        "DASH_DOT" => "dotDash",
        "DASH_DOT_DOT" => "dotDotDash",
        "LONG_DASH" => "dashLong",
        "DOUBLE_SLIM" => "double",
        "SLIM_THICK" | "THICK_SLIM" | "SLIM_THICK_SLIM" | "THICK3D" | "THICKREV3D" => "thick",
        "WAVE" => "wave",
        "DOUBLEWAVE" => "wavyDouble",
        _ => "single",
    })
}

pub(crate) fn border_style(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "SOLID" => "single",
        "DOT" | "CIRCLE" => "dotted",
        "DASH" | "LONG_DASH" => "dashed",
        "DASH_DOT" => "dotDash",
        "DASH_DOT_DOT" => "dotDotDash",
        "DOUBLE_SLIM" => "double",
        "SLIM_THICK" => "thinThickSmallGap",
        "THICK_SLIM" => "thickThinSmallGap",
        "SLIM_THICK_SLIM" => "thinThickThinSmallGap",
        "WAVE" => "wave",
        "DOUBLEWAVE" => "doubleWave",
        "THICK3D" | "3D" => "threeDEmboss",
        "THICKREV3D" | "REV3D" => "threeDEngrave",
        _ => return None,
    })
}

/// One border of a border fill: `None` when it isn't drawn (`w:val="nil"` in a DOCX).
pub(crate) fn border(b: &crate::header::Border) -> Option<Border> {
    let style = border_style(&b.kind).filter(|_| b.visible())?;
    let eighths = round_even(b.width_mm * 72.0 / 25.4 * 8.0).clamp(2, 96);
    Some(Border {
        style: BorderStyle::from_ooxml(style),
        width: (eighths as f32 / 8.0).clamp(0.0, 12.0),
        color: b.color.as_deref().and_then(rgb),
        space: 0.0,
    })
}

pub(crate) fn num_fmt(f: &str) -> NumFormat {
    NumFormat::from_ooxml(match f {
        "CIRCLED_DIGIT" => "decimalEnclosedCircle",
        "ROMAN_CAPITAL" => "upperRoman",
        "ROMAN_SMALL" => "lowerRoman",
        "LATIN_CAPITAL" | "CIRCLED_LATIN_CAPITAL" => "upperLetter",
        "LATIN_SMALL" | "CIRCLED_LATIN_SMALL" => "lowerLetter",
        "HANGUL_SYLLABLE" | "CIRCLED_HANGUL_SYLLABLE" => "ganada",
        "HANGUL_JAMO" | "CIRCLED_HANGUL_JAMO" => "chosung",
        "HANGUL_PHONETIC" => "koreanDigital",
        "IDEOGRAPH" | "CIRCLED_IDEOGRAPH" => "ideographTraditional",
        "2DIGIT" => "decimalZero",
        _ => "decimal",
    })
}

fn tab_align(t: &str) -> TabAlign {
    match t {
        "RIGHT" => TabAlign::Right,
        "CENTER" => TabAlign::Center,
        "DECIMAL" => TabAlign::Decimal,
        _ => TabAlign::Left,
    }
}

fn tab_leader(l: &str) -> TabLeader {
    match l {
        "SOLID" => TabLeader::Underscore,
        "DOT" | "DASH_DOT_DOT" | "CIRCLE" => TabLeader::Dot,
        "DASH" | "LONG_DASH" | "DASH_DOT" => TabLeader::Hyphen,
        _ => TabLeader::None,
    }
}

/// A bullet character as an app can draw it. Bullets in the private-use range are glyphs of
/// symbol fonts, which a font without them can't show: the ones whose glyph Hancom's viewer was
/// seen to draw become that character, the rest "•".
fn bullet_char(ch: char) -> char {
    match ch as u32 {
        0xF0FC => '✓', // drawn as a check mark (2026-11-17)
        0xE000..=0xF8FF => '•',
        _ => ch,
    }
}

/// Hancom's level placeholders (`^1`, `^2`, ...) as the model's (`%1`, `%2`, ...).
pub(crate) fn level_placeholders(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '^'
            && let Some(&d) = chars.peek()
            && d.is_ascii_digit()
        {
            out.push('%');
            continue;
        }
        out.push(c);
    }
    out
}

impl Converter<'_> {
    /// A character shape as run properties (the shape's own spacing).
    pub(crate) fn char_props(&self, cp: &CharPr) -> CharProps {
        let mut c = CharProps::default();
        let latin = self.h.font_face("latin", cp.fonts.get("latin").copied());
        let hangul = self.h.font_face("hangul", cp.fonts.get("hangul").copied());
        if latin.is_some() || hangul.is_some() {
            let a = latin.or(hangul).unwrap_or("");
            let e = hangul.or(latin).unwrap_or("");
            c.font = Some(a.to_string());
            c.font_ea = Some(e.to_string());
            c.font_cs = Some(a.to_string());
        }
        if cp.bold {
            c.bold = Some(true);
            c.bold_cs = Some(true);
        }
        if cp.italic {
            c.italic = Some(true);
            c.italic_cs = Some(true);
        }
        if let Some(s) = &cp.strike {
            if s == "DOUBLE_SLIM" {
                c.double_strike = Some(true);
            } else {
                c.strike = Some(true);
            }
        }
        if cp.outline {
            c.outline = Some(true);
        }
        if cp.shadow {
            c.shadow = Some(true);
        }
        if cp.emboss {
            c.emboss = Some(true);
        }
        if cp.engrave {
            c.engrave = Some(true);
        }
        if let Some(col) = cp.text_color.as_deref().and_then(rgb) {
            c.color = Some(TextColor::Rgb(col));
        }
        let spacing = round_even(cp.spacing as f64 * cp.height as f64 / 500.0);
        if spacing != 0 {
            c.spacing = Some(pt(spacing).clamp(-1584.0, 1584.0));
        }
        if cp.ratio != 100 {
            c.scale = Some(cp.ratio.clamp(1, 600) as f32);
        }
        let half_points = round_even(cp.height as f64 / 50.0).max(2);
        let size = (half_points as f32 / 2.0).clamp(1.0, 1638.0);
        c.size = Some(size);
        c.size_cs = Some(size);
        if let Some(u) = &cp.underline {
            c.underline = Some(underline(u));
            c.underline_color = cp.underline_color.as_deref().and_then(rgb);
        }
        if let Some(s) = cp.shade_color.as_deref().filter(|s| *s != "FFFFFF").and_then(rgb) {
            c.shading = Some(s);
        }
        if cp.sup {
            c.vert_align = Some(VertAlign::Superscript);
        } else if cp.sub {
            c.vert_align = Some(VertAlign::Subscript);
        }
        if !matches!(cp.sym_mark.as_str(), "NONE" | "") {
            c.emphasis = Some(EmphasisMark::from_ooxml("dot"));
        }
        c
    }

    /// The run properties of character shape `id`, cached.
    pub(crate) fn run_props(&mut self, id: Option<&str>) -> CharProps {
        let key = crate::header::num(id);
        if let Some(c) = self.char_cache.get(&key) {
            return c.clone();
        }
        let c = self.char_props(&self.h.char_pr_id(key));
        self.char_cache.insert(key, c.clone());
        c
    }

    /// A paragraph shape as paragraph properties. `font_h`: the HWPUNIT height the line spacing
    /// is measured from; `trim`: no line spacing after the one line (exact font height);
    /// `exact`: percentages as exact heights (text-only paragraphs).
    pub(crate) fn para_props(
        &mut self,
        pp: &ParaPr,
        font_h: i64,
        page_break: bool,
        numbering: bool,
        trim: bool,
        exact: bool,
    ) -> (ParaProps, Spacing) {
        let pp = &pp.in_hwpunit(font_h);
        let mut p = ParaProps::default();
        if pp.keep_next {
            p.keep_next = Some(true);
        }
        if pp.keep_lines {
            p.keep_lines = Some(true);
        }
        if page_break || pp.page_break_before {
            p.page_break_before = Some(true);
        }
        p.widow_control = Some(pp.widow);
        if numbering {
            p.numbering = self.num_ref(pp);
        }
        if pp.border_fill != 0
            && let Some(bf) = self.h.border_fills.get(&pp.border_fill).cloned()
        {
            let sides = [&bf.top, &bf.left, &bf.bottom, &bf.right];
            if sides.iter().any(|b| b.visible()) {
                p.borders = Some(Borders {
                    top: border(&bf.top),
                    left: border(&bf.left),
                    bottom: border(&bf.bottom),
                    right: border(&bf.right),
                    ..Default::default()
                });
            }
            if let Some(f) = bf.fill.as_deref().filter(|f| *f != "FFFFFF").and_then(rgb) {
                p.shading = Some(f);
            }
        }
        let tabs = self.h.tab_prs.get(&pp.tab_pr).cloned().unwrap_or_default();
        let mut stops: Vec<TabStop> = tabs
            .iter()
            .filter(|t| t.pos > 0)
            .map(|t| TabStop { pos: pt(tw(t.pos)).min(MAX_LEN_PT), align: tab_align(&t.kind), leader: tab_leader(&t.leader) })
            .collect();
        if self.h.auto_tab_right.contains(&pp.tab_pr) {
            // The automatic tab at the right end (Hancom: 오른쪽 끝 자동 탭): a right tab where the line ends (tab
            // positions count from the text column's left edge).
            let end = self.line_width() - tw(pp.right);
            if end > 0 && tabs.iter().all(|t| tw(t.pos) < end) {
                stops.push(TabStop { pos: pt(end).min(MAX_LEN_PT), align: TabAlign::Right, leader: TabLeader::None });
            }
        }
        if !stops.is_empty() {
            p.tabs = Some(stops);
        }
        // Line spacing: HWP percentages are relative to the font size (measured: 10pt at 160%
        // -> 16pt pitch). exact=true for text-only paragraphs.
        let v = pp.ls_value;
        let (line, exact_rule) = match pp.ls_type.as_str() {
            "FIXED" => (tw(v), true),
            "AT_LEAST" => (tw(v), false),
            _ if trim => (tw(font_h), true),
            "BETWEEN_LINES" => (tw(font_h + v), exact),
            _ => (tw_f(font_h as f64 * v as f64 / 100.0), exact),
        };
        let line = pt(line.max(20)).abs();
        p.line_spacing = Some(if exact_rule { LineSpacing::Exactly(line) } else { LineSpacing::AtLeast(line) });
        let (mut left, right, ind) = (tw(pp.left), tw(pp.right), tw(pp.indent));
        let mut first = 0;
        if ind > 0 {
            first = ind;
        } else if ind < 0 {
            left += -ind;
            first = ind;
        }
        p.indent_left = Some(pt(left));
        p.indent_right = Some(pt(right));
        if ind != 0 {
            p.indent_first = Some(pt(first));
        }
        p.align = Some(align(&pp.align));
        if p.align == Some(Align::Justify) {
            p.kashida = Some(craft_wordmodel::props::Kashida::None);
        }
        if pp.heading_type == "OUTLINE" {
            p.outline_level = Some(pp.heading_level.clamp(0, 8) as u8);
        }
        (p, Spacing { before: tw(pp.before).max(0), after: tw(pp.after).max(0) })
    }

    /// The list a paragraph shape puts its paragraphs in (outline, numbering or bullets).
    fn num_ref(&mut self, pp: &ParaPr) -> Option<NumRef> {
        let key = match pp.heading_type.as_str() {
            "OUTLINE" => match self.sec.outline {
                Some(o) if self.h.numberings.contains_key(&o) => ("num", o),
                _ => return None,
            },
            "NUMBER" if self.h.numberings.contains_key(&pp.heading_ref) => ("num", pp.heading_ref),
            "BULLET" if self.h.bullets.contains_key(&pp.heading_ref) => ("bul", pp.heading_ref),
            _ => return None,
        };
        let id = match self.num_ids.iter().find(|(k, _)| *k == key) {
            Some((_, id)) => *id,
            None => {
                let id = self.num_ids.len() as u32 + 1;
                self.num_ids.push((key, id));
                id
            }
        };
        Some(NumRef { num: id, level: pp.heading_level.clamp(0, 8) as u8 })
    }

    /// The lists the paragraphs used, one abstract definition each.
    pub(crate) fn numbering(&self) -> craft_wordmodel::numbering::Numbering {
        let mut out = craft_wordmodel::numbering::Numbering::default();
        for &((kind, r), nid) in &self.num_ids {
            let mut levels = Vec::new();
            for i in 0..9i64 {
                let mut lv = Level { indent: 0.0, hanging: 0.0, suffix: LevelSuffix::Space, align: Align::Left, ..Level::default() };
                if kind == "bul" {
                    let ch = bullet_char(self.h.bullets.get(&r).and_then(|c| c.chars().next()).unwrap_or('•'));
                    lv.start = 1;
                    lv.format = NumFormat::Bullet;
                    lv.text = ch.to_string();
                } else {
                    let l = self.h.numberings.get(&r).and_then(|n| n.levels.get(&(i + 1)));
                    lv.format = l.map(|l| num_fmt(&l.fmt)).unwrap_or(NumFormat::Decimal);
                    let text = l.map(|l| l.text.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| format!("^{}.", i + 1));
                    lv.text = level_placeholders(&text).chars().take(64).collect();
                    lv.start = l.map(|l| l.start).filter(|s| *s != 0).and_then(|s| u32::try_from(s).ok()).unwrap_or(1);
                }
                levels.push(lv);
            }
            out.abstracts.push(AbstractNum { id: nid, levels, ..AbstractNum::default() });
            out.nums.push(Num { id: nid, abstract_id: nid, ..Num::default() });
        }
        out
    }

    /// The header's styles as a style sheet: the first style ("Normal" in Hancom's files) gives
    /// the document defaults too.
    pub(crate) fn style_sheet(&mut self) -> StyleSheet {
        let base = self.h.style(0).cloned();
        let base_cp = self.h.char_pr_id(base.as_ref().map(|s| s.char_pr).unwrap_or(0));
        let base_pp = self.h.para_pr_id(base.as_ref().map(|s| s.para_pr).unwrap_or(0));
        let mut sheet = StyleSheet { styles: Vec::new(), ..StyleSheet::default() };
        let mut chr = self.char_props(&base_cp);
        chr.lang = Some("ko-KR".into());
        chr.lang_ea = Some("ko-KR".into());
        sheet.default_chr = chr;
        let (mut dp, sp) = self.para_props(&base_pp, base_cp.height, false, false, false, false);
        dp.space_before = Some(pt(sp.before));
        dp.space_after = Some(pt(sp.after));
        sheet.default_para = dp;
        sheet.default_para_present = true;
        if self.h.style(0).is_none() {
            sheet.styles.push(Style { id: "Normal".into(), name: "Normal".into(), kind: StyleKind::Paragraph, builtin: true, ..Style::default() });
        }
        for st in self.h.styles.clone() {
            let id = style_id(st.id);
            let name = if !st.name.is_empty() {
                st.name.clone()
            } else if !st.eng_name.is_empty() {
                st.eng_name.clone()
            } else {
                id.clone()
            };
            if st.kind == "CHAR" {
                sheet.styles.push(Style {
                    id,
                    name,
                    kind: StyleKind::Character,
                    builtin: true,
                    chr: self.char_props(&self.h.char_pr_id(st.char_pr)),
                    ..Style::default()
                });
                continue;
            }
            let cp = self.h.char_pr_id(st.char_pr);
            let pp = self.h.para_pr_id(st.para_pr);
            let (mut para, sp) = self.para_props(&pp, cp.height, false, false, false, false);
            para.space_before = Some(pt(sp.before));
            para.space_after = Some(pt(sp.after));
            let next = self.h.style(st.next).map(|_| style_id(st.next));
            sheet.styles.push(Style {
                id,
                name,
                kind: StyleKind::Paragraph,
                next,
                quick: true,
                builtin: true,
                para,
                chr: self.char_props(&cp),
                ..Style::default()
            });
        }
        sheet
    }
}

/// The model's id of header style `id`: the first style is the default paragraph style, which the
/// model calls `Normal`.
pub(crate) fn style_id(id: i64) -> String {
    if id == 0 { "Normal".into() } else { format!("S{id}") }
}

/// A paragraph's space before and after, twips, kept apart until the paragraphs around it are
/// known ([`super::merge_spacing`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Spacing {
    pub before: i64,
    pub after: i64,
}
