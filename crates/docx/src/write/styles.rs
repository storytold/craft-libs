use super::{Level, LevelSuffix, ParaProps, Style, StyleKind, TablePart, VAlign, W, Writer, props, xml};
use crate::write::props::open_change;
use craft_wordmodel::Document;
use craft_wordmodel::props::CharProps;
use craft_wordmodel::styles::{StyleSheet, TableStyleParts};
use std::borrow::Cow;

/// Built-in names Word keeps in lower case in styles.xml.
fn file_style_name(name: &str) -> String {
    if let Some((f, _)) = crate::read::LOWER_NAMES.iter().find(|(_, ui)| ui.eq_ignore_ascii_case(name)) {
        return (*f).to_string();
    }
    for (pre, f) in [("Heading ", "heading "), ("TOC ", "toc "), ("Index ", "index ")] {
        if let Some(d) = name.strip_prefix(pre)
            && d.len() == 1
            && d.chars().all(|c| c.is_ascii_digit())
        {
            return format!("{f}{d}");
        }
    }
    name.to_string()
}

/// Does `c` name a complex-script font (a font or a theme slot)?
fn names_cs(c: &CharProps) -> bool {
    c.font_cs.is_some() || c.font_cs_theme.is_some()
}

/// Does `c` name an East Asian font (a font or a theme slot)?
fn names_ea(c: &CharProps) -> bool {
    c.font_ea.is_some() || c.font_ea_theme.is_some()
}

/// The styles as `styles.xml` writes them, when that differs from `s`; `None` when it doesn't.
/// Word draws each font slot the run defaults leave out in Times New Roman, where WordCraft draws
/// the Latin font the text resolves to (the body font when that is unset too). So each slot the
/// run defaults leave out is named, with the Latin font; a theme slot stays a theme slot. A
/// paragraph style that sets a Latin font, where neither it nor a style it is based on names the
/// slot, names it with that font too, so its text keeps its font. What else would change
/// (a run's own Latin font, a character style's) the run names itself ([`run_as_saved`]).
pub(super) fn saved_styles(s: &StyleSheet) -> Option<StyleSheet> {
    let d = &s.default_chr;
    let open_font = d.font.is_none() && d.font_ascii_theme.is_none() && d.font_hansi_theme.is_none();
    let (open_cs, open_ea) = (!names_cs(d), !names_ea(d));
    if !open_font && !open_cs && !open_ea {
        return None;
    }
    let mut out = s.clone();
    let latin = d.font.clone().unwrap_or_else(|| craft_wordmodel::styles::BODY_FONT.to_string());
    let c = &mut out.default_chr;
    if open_font {
        c.font = Some(latin.clone());
    }
    if open_cs {
        c.font_cs = Some(latin.clone());
    }
    if open_ea {
        c.font_ea = Some(latin);
    }
    for (st, o) in s.styles.iter().zip(out.styles.iter_mut()) {
        let Some(font) = st.chr.font.clone().filter(|_| st.kind == StyleKind::Paragraph) else { continue };
        let up = s.chain(&st.id);
        if open_cs && !up.iter().any(|x| names_cs(&x.chr)) {
            o.chr.font_cs = Some(font.clone());
        }
        if open_ea && !up.iter().any(|x| names_ea(&x.chr)) {
            o.chr.font_ea = Some(font);
        }
    }
    Some(out)
}

/// `c`, a run's formatting in a paragraph of style `para_style` (`table`: the run formatting the
/// table style gives the cell), as it is written: where the styles as saved (`saved`, from
/// [`saved_styles`]) would draw its complex-script or East Asian text in another font than
/// WordCraft draws it in under `mem`, the run names that font itself, so the text keeps its font
/// in Word and after reopening. A tracked change's before copy the same.
pub(super) fn run_as_saved<'a>(
    mem: &StyleSheet,
    saved: Option<&StyleSheet>,
    para_style: Option<&str>,
    table: Option<&CharProps>,
    c: Cow<'a, CharProps>,
) -> Cow<'a, CharProps> {
    let Some(saved) = saved else { return c };
    let fonts = |p: &CharProps| {
        let (m, s) = (mem.resolve_char_in(para_style, table, p), saved.resolve_char_in(para_style, table, p));
        // Without an East Asian font, that text is drawn in the Latin one.
        let (m_ea, s_ea) = (m.font_ea.unwrap_or(m.font), s.font_ea.unwrap_or(s.font));
        ((m.font_cs != s.font_cs).then_some(m.font_cs), (m_ea != s_ea).then_some(m_ea))
    };
    let own = fonts(&c);
    let old = c.fmt_change.as_deref().map(|ch| fonts(&ch.old)).unwrap_or((None, None));
    if own == (None, None) && old == (None, None) {
        return c;
    }
    let mut p = c.into_owned();
    let name = |p: &mut CharProps, (cs, ea): (Option<String>, Option<String>)| {
        if cs.is_some() {
            p.font_cs = cs;
        }
        if ea.is_some() {
            p.font_ea = ea;
        }
    };
    name(&mut p, own);
    if let Some(ch) = p.fmt_change.as_deref_mut() {
        name(&mut ch.old, old);
    }
    Cow::Owned(p)
}

pub(super) fn styles_xml(wr: &mut Writer<'_>) -> Vec<u8> {
    let doc = wr.doc;
    let saved = wr.saved_styles.clone();
    let s: &StyleSheet = saved.as_deref().unwrap_or(&doc.styles);
    let mut w = W::new();
    let ns = xml::body_ns();
    let ns_refs: Vec<(&str, &str)> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    w.open("w:styles", &ns_refs);
    w.open("w:docDefaults", &[]);
    w.open("w:rPrDefault", &[]);
    w.open("w:rPr", &[]);
    let mut chr = s.default_chr.clone();
    // The reader reads an omitted `w:sz` as 10 pt, but the resolver falls back to
    // DEFAULT_FONT_SIZE, so writing nothing would change the size on reopening.
    chr.size.get_or_insert(craft_wordmodel::props::DEFAULT_FONT_SIZE);
    props::rpr_inner(&mut w, &chr);
    w.close("w:rPr");
    w.close("w:rPrDefault");
    if s.default_para_present || s.default_para != craft_wordmodel::props::ParaProps::default() {
        // Defaults edited over a file that left `w:pPrDefault` out still stand on Word's own
        // (8 pt after): written out whole, so reopening keeps them.
        let para = s.para_defaults();
        w.open("w:pPrDefault", &[]);
        w.open("w:pPr", &[]);
        props::ppr_inner(&mut w, &para, false, None);
        w.close("w:pPr");
        w.close("w:pPrDefault");
    }
    w.close("w:docDefaults");
    if let Some(latent) = s.latent_styles.as_deref() {
        w.raw(latent);
    }
    // Kinds whose default style the file named (`w:default`); the English ids stand in otherwise.
    let flagged: Vec<StyleKind> = s.styles.iter().filter(|st| st.default).map(|st| st.kind).collect();
    for st in &s.styles {
        style_xml(wr, &mut w, st, &flagged);
    }
    w.close("w:styles");
    w.into_bytes()
}

/// One `w:style` element. A tracked change of its paragraph/run properties is nested in the
/// corresponding property element as `w:pPrChange` / `w:rPrChange`.
/// A table style's own table properties (`w:tblPr`), in ECMA-376 order.
fn table_style_tblpr(w: &mut W, t: &TableStyleParts) {
    if t.borders.is_none() && t.cell_margins.is_none() && t.indent.is_none() && t.band_size.is_none() && t.col_band_size.is_none() {
        return;
    }
    w.open("w:tblPr", &[]);
    if let Some(n) = t.band_size {
        w.val("w:tblStyleRowBandSize", &n.min(1000).to_string());
    }
    if let Some(n) = t.col_band_size {
        w.val("w:tblStyleColBandSize", &n.min(1000).to_string());
    }
    // Kept even at 0: Word places a table whose style chain sets no indent differently.
    if let Some(i) = t.indent {
        w.empty("w:tblInd", &[("w:w", &crate::units::twips(i)), ("w:type", "dxa")]);
    }
    if let Some(b) = &t.borders {
        props::borders(w, "w:tblBorders", b, Some("w:insideH"), &[]);
    }
    if let Some(m) = &t.cell_margins {
        props::margins(w, "w:tblCellMar", m);
    }
    w.close("w:tblPr");
}

fn style_xml(wr: &mut Writer<'_>, w: &mut W, st: &Style, flagged: &[StyleKind]) {
    let ty = match st.kind {
        StyleKind::Paragraph => "paragraph",
        StyleKind::Character => "character",
        StyleKind::Table => "table",
        StyleKind::Numbering => "numbering",
    };
    let english_default = matches!(
        (st.kind, st.id.as_str()),
        (StyleKind::Paragraph, "Normal")
            | (StyleKind::Character, "DefaultParagraphFont")
            | (StyleKind::Table, "TableNormal")
            | (StyleKind::Numbering, "NoList")
    );
    let normal_is_explicitly_off = st.kind == StyleKind::Paragraph && st.id == "Normal" && !st.default && flagged.contains(&StyleKind::Paragraph);
    let is_default = st.default || (english_default && !flagged.contains(&st.kind) && !normal_is_explicitly_off);
    let mut a = vec![("w:type", ty), ("w:styleId", st.id.as_str())];
    if normal_is_explicitly_off {
        // Preserve a model where Normal is explicitly not default. Without this, the fallback
        // `english_default` marker makes the reader infer a change on round trip.
        a.push(("w:default", "0"));
    } else if is_default {
        a.push(("w:default", "1"));
    }
    if !st.builtin {
        a.push(("w:customStyle", "1"));
    }
    w.open("w:style", &a);
    w.val("w:name", &file_style_name(if st.name.is_empty() { &st.id } else { &st.name }));
    if let Some(b) = &st.based_on {
        w.val("w:basedOn", b);
    }
    if let Some(b) = &st.next {
        w.val("w:next", b);
    }
    if let Some(b) = &st.linked {
        w.val("w:link", b);
    }
    if let Some(p) = st.priority {
        w.val("w:uiPriority", &p.to_string());
    }
    if st.hidden {
        w.empty("w:semiHidden", &[]);
        w.empty("w:unhideWhenUsed", &[]);
    }
    if st.quick {
        w.empty("w:qFormat", &[]);
    }
    if st.locked {
        w.empty("w:locked", &[]);
    }
    style_property_xml(wr, w, st);
    if let Some(t) = &st.table {
        table_style_tblpr(w, t);
        // The conditional formatting parts (ECMA-376 §17.7.6): paragraph and run properties,
        // then cell borders, shading and vertical alignment. The whole table's text formatting
        // is the style's own (above).
        for part in TablePart::ALL {
            let f = t.part(part);
            let ppr = ParaProps { align: f.align, ..ParaProps::default() };
            let has_rpr = props::has_rpr(&f.chr);
            // The whole table's borders are its table borders (`w:tblPr` above).
            let borders = f.borders.filter(|_| part != TablePart::WholeTable);
            let has_tc = f.fill.is_some() || borders.is_some() || f.v_align.is_some();
            if !has_rpr && !has_tc && f.align.is_none() {
                continue;
            }
            w.open("w:tblStylePr", &[("w:type", part.ooxml())]);
            if f.align.is_some() {
                w.open("w:pPr", &[]);
                props::ppr_inner(w, &ppr, false, None);
                w.close("w:pPr");
            }
            if has_rpr {
                w.open("w:rPr", &[]);
                props::rpr_inner(w, &f.chr);
                w.close("w:rPr");
            }
            if has_tc {
                w.open("w:tcPr", &[]);
                if let Some(bs) = borders {
                    props::borders(w, "w:tcBorders", &bs, Some("w:insideH"), &[]);
                }
                if let Some(c) = f.fill {
                    w.empty("w:shd", &[("w:val", "clear"), ("w:color", "auto"), ("w:fill", &c.hex())]);
                }
                if let Some(v) = f.v_align {
                    w.val(
                        "w:vAlign",
                        match v {
                            VAlign::Top => "top",
                            VAlign::Center => "center",
                            VAlign::Bottom => "bottom",
                        },
                    );
                }
                w.close("w:tcPr");
            }
            w.close("w:tblStylePr");
        }
    }
    w.close("w:style");
}

fn style_property_xml(wr: &mut Writer<'_>, w: &mut W, st: &Style) {
    let para_change = st.para_change.as_deref().filter(|ch| ch.old.formatting() != st.para.formatting());
    if (!st.para.is_empty() || para_change.is_some()) && st.kind != StyleKind::Character {
        w.open("w:pPr", &[]);
        let mut ppr = st.para.clone();
        if ppr.align == Some(craft_wordmodel::props::Align::Justify) && ppr.kashida.is_none() {
            let mut probe = ppr.clone();
            probe.style = Some(st.id.clone());
            ppr.kashida = Some(wr.styles().resolve_para(&probe).kashida);
        }
        props::ppr_inner(w, &ppr, false, None);
        if let Some(ch) = para_change {
            let a = wr.change_attrs(ch.rev);
            open_change(w, "w:pPrChange", &a);
            w.open("w:pPr", &[]);
            props::ppr_inner(w, &ch.old, false, None);
            w.close("w:pPr");
            w.close("w:pPrChange");
        }
        w.close("w:pPr");
    }
    let run_change = st.chr_change.as_deref().filter(|ch| ch.old.formatting() != st.chr.formatting());
    if (props::has_rpr(&st.chr) || run_change.is_some()) && st.kind != StyleKind::Numbering {
        w.open("w:rPr", &[]);
        props::rpr_inner(w, &st.chr);
        if let Some(ch) = run_change {
            let a = wr.change_attrs(ch.rev);
            open_change(w, "w:rPrChange", &a);
            w.open("w:rPr", &[]);
            props::rpr_inner(w, &ch.old);
            w.close("w:rPr");
            w.close("w:rPrChange");
        }
        w.close("w:rPr");
    }
}

pub(super) fn numbering_xml(doc: &Document) -> Vec<u8> {
    let mut w = W::new();
    let ns = xml::body_ns();
    let ns_refs: Vec<(&str, &str)> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    w.open("w:numbering", &ns_refs);
    let mut nsids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for a in &doc.numbering.abstracts {
        w.open("w:abstractNum", &[("w:abstractNumId", &a.id.to_string())]);
        // CT_AbstractNum order: nsid, multiLevelType, tmpl, name… A copied definition repeats
        // its source's nsid; only the first keeps it, so Word doesn't take them for one list.
        if let Some(n) = a.nsid.as_deref().filter(|n| nsids.insert(n)) {
            w.val("w:nsid", n);
        }
        let derived = if a.levels.len() > 1 { "multilevel" } else { "singleLevel" };
        w.val("w:multiLevelType", a.multi_level_type.as_deref().unwrap_or(derived));
        if let Some(t) = &a.tmpl {
            w.val("w:tmpl", t);
        }
        if let Some(n) = &a.name {
            w.val("w:name", n);
        }
        for (i, l) in a.levels.iter().take(9).enumerate() {
            write_level(&mut w, i, l);
        }
        w.close("w:abstractNum");
    }
    for n in &doc.numbering.nums {
        w.open("w:num", &[("w:numId", &n.id.to_string())]);
        w.val("w:abstractNumId", &n.abstract_id.to_string());
        for lvl in 0..9u8 {
            let start = n.start_overrides.iter().find(|(l, _)| (*l).min(8) == lvl).map(|(_, s)| *s);
            let level = n.level_overrides.iter().find(|(l, _)| (*l).min(8) == lvl).map(|(_, l)| l);
            if start.is_none() && level.is_none() {
                continue;
            }
            w.open("w:lvlOverride", &[("w:ilvl", &lvl.to_string())]);
            if let Some(start) = start {
                w.val("w:startOverride", &start.to_string());
            }
            if let Some(level) = level {
                write_level(&mut w, lvl as usize, level);
            }
            w.close("w:lvlOverride");
        }
        w.close("w:num");
    }
    w.close("w:numbering");
    w.into_bytes()
}

/// One `w:lvl` (list level `i`).
fn write_level(w: &mut W, i: usize, l: &Level) {
    w.open("w:lvl", &[("w:ilvl", &i.to_string())]);
    w.val("w:start", &l.start.to_string());
    w.val("w:numFmt", l.format.ooxml());
    if !l.restart {
        w.val("w:lvlRestart", "0");
    } else if let Some(k) = l.restart_after.filter(|k| (1..=9).contains(k)) {
        w.val("w:lvlRestart", &k.to_string());
    }
    if let Some(s) = &l.style {
        w.val("w:pStyle", s);
    }
    if l.legal {
        w.empty("w:isLgl", &[]);
    }
    match l.suffix {
        LevelSuffix::Tab => {}
        LevelSuffix::Space => w.val("w:suff", "space"),
        LevelSuffix::Nothing => w.val("w:suff", "nothing"),
    }
    w.val("w:lvlText", &l.text);
    w.val("w:lvlJc", props::align_val(l.align));
    w.open("w:pPr", &[]);
    if let Some(t) = l.tab.filter(|t| t.is_finite()) {
        w.open("w:tabs", &[]);
        w.empty("w:tab", &[("w:val", "num"), ("w:pos", &crate::units::twips(t.clamp(-1584.0, 1584.0)))]);
        w.close("w:tabs");
    }
    let ind = crate::units::twips(l.indent);
    if l.hanging >= 0.0 {
        w.empty("w:ind", &[("w:left", &ind), ("w:hanging", &crate::units::twips(l.hanging))]);
    } else {
        w.empty("w:ind", &[("w:left", &ind), ("w:firstLine", &crate::units::twips(-l.hanging))]);
    }
    w.close("w:pPr");
    props::rpr(w, &l.chr);
    w.close("w:lvl");
}
