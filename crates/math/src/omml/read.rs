//! OMML into the tree.

use craft_xml::El;

use super::{measure_twips, on_off};
use crate::math::{
    Arg, BrkBin, BrkBinSub, ColJc, FracKind, LimLoc, MAX_DEPTH, MNode, MRun, MScr, MSty, Math, MathJc, MathProps, ScriptKind, merge_runs,
};
use crate::rgb::Rgb;
use crate::symbol_fonts;

/// An `m:oMath` element as an equation (structure plus its source XML).
pub fn read_omath(e: &El, jc: MathJc) -> Math {
    Math { nodes: merge_runs(arg(e, 0)), omml: e.to_xml(), jc, linear: false }
}

/// A colour every math run shares, taken off the runs: it is the equation's own colour, as
/// [`write_equation`] spreads it (#1467). The source OMML
/// is kept: the writer replaces it only when the colour changes.
pub fn lift_color(math: &mut Math) -> Option<Rgb> {
    let (mut shared, mut uniform) = (None, true);
    crate::math_tree::each_run_mut(&mut math.nodes, &mut |r| match shared {
        None => shared = Some(r.color),
        Some(c) => uniform &= c == r.color,
    });
    let c = shared.flatten().filter(|_| uniform)?;
    crate::math_tree::each_run_mut(&mut math.nodes, &mut |r| r.color = None);
    Some(c)
}

/// `m:oMathParaPr/m:jc`.
pub fn para_jc(para: &El) -> MathJc {
    match para.child("m:oMathParaPr").and_then(|p| p.child("m:jc")).and_then(|j| j.attr_or_plain("m:val")) {
        Some("left") => MathJc::Left,
        Some("right") => MathJc::Right,
        Some("center") => MathJc::Center,
        _ => MathJc::CenterGroup,
    }
}

/// The settings part's `m:mathPr`: how display equations wrap.
pub fn read_math_pr(e: &El) -> MathProps {
    let mut m = MathProps::default();
    let pr = Some(e);
    m.brk_bin = match val(pr, "m:brkBin") {
        Some("after") => BrkBin::After,
        Some("repeat") => BrkBin::Repeat,
        _ => BrkBin::Before,
    };
    m.brk_bin_sub = match val(pr, "m:brkBinSub") {
        Some("-+") => BrkBinSub::MinusPlus,
        Some("+-") => BrkBinSub::PlusMinus,
        _ => BrkBinSub::MinusMinus,
    };
    if let Some(v) = e.child("m:wrapIndent").and_then(|c| measure_twips(c.attr_or_plain("m:val"))).filter(|v| v.is_finite() && *v >= 0.0) {
        m.wrap_indent = v.min(1584.0);
    }
    m.wrap_right = prop(pr, "m:wrapRight", false);
    m
}

/// OMML on/off property: absent `m:val` = on.
fn on(e: &El) -> bool {
    match e.attr_or_plain("m:val") {
        None => true,
        Some(v) => !matches!(v.trim(), "0" | "false" | "off" | "False" | "FALSE"),
    }
}

/// An on/off property `name` in the properties element `pr` (absent = `default`).
fn prop(pr: Option<&El>, name: &str, default: bool) -> bool {
    pr.and_then(|p| p.child(name)).map(on).unwrap_or(default)
}

/// A character property (`m:chr`, `m:begChr`…): `None` when absent, `Some(None)` when empty.
fn chr(pr: Option<&El>, name: &str) -> Option<Option<char>> {
    let c = pr?.child(name)?;
    match c.attr_or_plain("m:val") {
        Some(v) => Some(v.chars().next()),
        None => Some(None),
    }
}

fn val<'a>(pr: Option<&'a El>, name: &str) -> Option<&'a str> {
    pr.and_then(|p| p.child(name)).and_then(|c| c.attr_or_plain("m:val"))
}

/// The argument held by child `name` of `e`.
fn sub_arg(e: &El, name: &str, depth: usize) -> Arg {
    e.child(name).map(|c| arg(c, depth + 1)).unwrap_or_default()
}

/// Read the math content of an element (`m:oMath`, `m:e`, `m:num`…).
fn arg(e: &El, depth: usize) -> Arg {
    let mut out = Vec::new();
    if depth > MAX_DEPTH {
        return out;
    }
    for k in e.elements() {
        node(k, depth, &mut out);
    }
    out
}

fn node(k: &El, depth: usize, out: &mut Arg) {
    let d = depth + 1;
    match k.name.as_str() {
        "m:r" => {
            if let Some(r) = run(k) {
                out.push(MNode::Run(r));
            }
        }
        // Word puts ordinary runs in math zones for normal text in some files.
        "w:r" => {
            let text = run_text(k);
            if !text.is_empty() {
                let mut r = MRun { text, nor: true, sty: Some(MSty::Plain), ..Default::default() };
                wrpr(k.child("w:rPr"), &mut r);
                out.push(MNode::Run(r));
            }
        }
        "m:f" => {
            let pr = k.child("m:fPr");
            let kind = match val(pr, "m:type") {
                Some("skw") => FracKind::Skewed,
                Some("lin") => FracKind::Linear,
                Some("noBar") => FracKind::NoBar,
                _ => FracKind::Bar,
            };
            out.push(MNode::Frac { kind, num: sub_arg(k, "m:num", d), den: sub_arg(k, "m:den", d) });
        }
        "m:sSup" => out.push(MNode::Script { kind: ScriptKind::Sup, base: sub_arg(k, "m:e", d), sub: Vec::new(), sup: sub_arg(k, "m:sup", d) }),
        "m:sSub" => out.push(MNode::Script { kind: ScriptKind::Sub, base: sub_arg(k, "m:e", d), sub: sub_arg(k, "m:sub", d), sup: Vec::new() }),
        "m:sSubSup" => {
            out.push(MNode::Script { kind: ScriptKind::SubSup, base: sub_arg(k, "m:e", d), sub: sub_arg(k, "m:sub", d), sup: sub_arg(k, "m:sup", d) })
        }
        "m:sPre" => {
            out.push(MNode::Script { kind: ScriptKind::Pre, base: sub_arg(k, "m:e", d), sub: sub_arg(k, "m:sub", d), sup: sub_arg(k, "m:sup", d) })
        }
        "m:rad" => {
            let pr = k.child("m:radPr");
            out.push(MNode::Rad { deg: sub_arg(k, "m:deg", d), deg_hide: prop(pr, "m:degHide", false), e: sub_arg(k, "m:e", d) });
        }
        "m:nary" => {
            let pr = k.child("m:naryPr");
            let chr = chr(pr, "m:chr").map(|c| c.unwrap_or('∫')).unwrap_or('∫');
            let lim_loc = match val(pr, "m:limLoc") {
                Some("undOvr") => Some(LimLoc::UndOvr),
                Some("subSup") => Some(LimLoc::SubSup),
                _ => None,
            };
            out.push(MNode::Nary {
                chr,
                lim_loc,
                grow: prop(pr, "m:grow", false),
                sub_hide: prop(pr, "m:subHide", false),
                sup_hide: prop(pr, "m:supHide", false),
                sub: sub_arg(k, "m:sub", d),
                sup: sub_arg(k, "m:sup", d),
                e: sub_arg(k, "m:e", d),
            });
        }
        "m:d" => {
            let pr = k.child("m:dPr");
            let beg = chr(pr, "m:begChr").unwrap_or(Some('('));
            let end = chr(pr, "m:endChr").unwrap_or(Some(')'));
            let sep = chr(pr, "m:sepChr").unwrap_or(Some('|'));
            let elems: Vec<Arg> = k.children("m:e").map(|e| arg(e, d)).collect();
            out.push(MNode::Delim {
                beg,
                end,
                sep,
                grow: prop(pr, "m:grow", true),
                shp_match: val(pr, "m:shp") == Some("match"),
                elems: if elems.is_empty() { vec![Vec::new()] } else { elems },
            });
        }
        "m:func" => out.push(MNode::Func { name: sub_arg(k, "m:fName", d), e: sub_arg(k, "m:e", d) }),
        "m:limLow" => out.push(MNode::Lim { upper: false, e: sub_arg(k, "m:e", d), lim: sub_arg(k, "m:lim", d) }),
        "m:limUpp" => out.push(MNode::Lim { upper: true, e: sub_arg(k, "m:e", d), lim: sub_arg(k, "m:lim", d) }),
        "m:acc" => {
            let c = chr(k.child("m:accPr"), "m:chr").map(|c| c.unwrap_or('\u{302}')).unwrap_or('\u{302}');
            out.push(MNode::Acc { chr: c, e: sub_arg(k, "m:e", d) });
        }
        "m:bar" => out.push(MNode::Bar { top: val(k.child("m:barPr"), "m:pos") == Some("top"), e: sub_arg(k, "m:e", d) }),
        "m:borderBox" => {
            let pr = k.child("m:borderBoxPr");
            out.push(MNode::BorderBox {
                hide: [prop(pr, "m:hideTop", false), prop(pr, "m:hideBot", false), prop(pr, "m:hideLeft", false), prop(pr, "m:hideRight", false)],
                strike: [
                    prop(pr, "m:strikeH", false),
                    prop(pr, "m:strikeV", false),
                    prop(pr, "m:strikeBLTR", false),
                    prop(pr, "m:strikeTLBR", false),
                ],
                e: sub_arg(k, "m:e", d),
            });
        }
        "m:box" => out.push(MNode::Boxed { e: sub_arg(k, "m:e", d) }),
        "m:groupChr" => {
            let pr = k.child("m:groupChrPr");
            let c = chr(pr, "m:chr").map(|c| c.unwrap_or('\u{23DF}')).unwrap_or('\u{23DF}');
            out.push(MNode::GroupChr { chr: c, top: val(pr, "m:pos") == Some("top"), e: sub_arg(k, "m:e", d) });
        }
        "m:eqArr" => out.push(MNode::EqArr { rows: k.children("m:e").map(|e| arg(e, d)).collect() }),
        "m:m" => {
            let mut col_jc = Vec::new();
            if let Some(mcs) = k.child("m:mPr").and_then(|p| p.child("m:mcs")) {
                for mc in mcs.children("m:mc") {
                    let pr = mc.child("m:mcPr");
                    let jc = match val(pr, "m:mcJc") {
                        Some("left") => ColJc::Left,
                        Some("right") => ColJc::Right,
                        _ => ColJc::Center,
                    };
                    let count = val(pr, "m:count").and_then(|c| c.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 64);
                    for _ in 0..count {
                        if col_jc.len() < 64 {
                            col_jc.push(jc);
                        }
                    }
                }
            }
            let rows = k.children("m:mr").take(1000).map(|r| r.children("m:e").take(64).map(|e| arg(e, d)).collect()).collect();
            out.push(MNode::Matrix { rows, col_jc });
        }
        "m:phant" => {
            let pr = k.child("m:phantPr");
            out.push(MNode::Phant {
                show: prop(pr, "m:show", true),
                zero_wid: prop(pr, "m:zeroWid", false),
                zero_asc: prop(pr, "m:zeroAsc", false),
                zero_desc: prop(pr, "m:zeroDesc", false),
                e: sub_arg(k, "m:e", d),
            });
        }
        // Tracked insertions show; deletions don't. Hyperlinks, simple fields and direction
        // wrappers (EG_PContentMath, §22.1.2.77) keep their runs (#1273).
        "w:ins" | "w:moveTo" | "w:smartTag" | "w:customXml" | "w:hyperlink" | "w:fldSimple" | "w:dir" | "w:bdo" | "m:oMath" if depth <= MAX_DEPTH => {
            for c in k.elements() {
                node(c, d, out);
            }
        }
        // Markup compatibility: the math choice (we read math), else the fallback.
        "mc:AlternateContent" if depth <= MAX_DEPTH => {
            if let Some(c) = k.child("mc:Choice").or_else(|| k.child("mc:Fallback")) {
                for c in c.elements() {
                    node(c, d, out);
                }
            }
        }
        "w:sdt" => {
            if let Some(c) = k.child("w:sdtContent") {
                for c in c.elements() {
                    node(c, d, out);
                }
            }
        }
        _ => {}
    }
}

/// `m:t` (and `w:t`) text of a run, with symbol characters (`w:sym`) and special hyphens.
/// Text in a symbol font (Symbol, Wingdings…) becomes its Unicode equivalent: the math font has
/// no glyphs at those private-use codes (#1273).
fn run_text(r: &El) -> String {
    let font = r
        .child("w:rPr")
        .and_then(|p| p.child("w:rFonts"))
        .and_then(|f| f.attr_or_plain("w:ascii").or_else(|| f.attr_or_plain("w:hAnsi")))
        .unwrap_or("");
    let mut s = String::new();
    for c in r.elements() {
        match c.name.as_str() {
            "m:t" | "w:t" => s.push_str(&symbol_fonts::map_text(font, &c.text())),
            "w:tab" => s.push('\t'),
            "w:noBreakHyphen" => s.push('\u{2011}'),
            "w:sym" => {
                let ch = c
                    .attr_or_plain("w:char")
                    .and_then(|h| u32::from_str_radix(h.trim(), 16).ok())
                    .and_then(char::from_u32)
                    .filter(|c| *c as u32 >= 0x20);
                if let Some(ch) = ch {
                    let f = c.attr_or_plain("w:font").unwrap_or(font);
                    s.push(symbol_fonts::symbol_char(f, ch).unwrap_or(ch));
                }
            }
            _ => {}
        }
    }
    s
}

fn run(r: &El) -> Option<MRun> {
    let text = run_text(r);
    if text.is_empty() {
        return None;
    }
    let mut out = MRun { text, ..Default::default() };
    if let Some(pr) = r.child("m:rPr") {
        out.sty = match pr.child("m:sty").and_then(|s| s.attr_or_plain("m:val")) {
            Some("p") => Some(MSty::Plain),
            Some("b") => Some(MSty::Bold),
            Some("i") => Some(MSty::Italic),
            Some("bi") => Some(MSty::BoldItalic),
            _ => None,
        };
        out.scr = match pr.child("m:scr").and_then(|s| s.attr_or_plain("m:val")) {
            Some("script") => MScr::Script,
            Some("fraktur") => MScr::Fraktur,
            Some("double-struck") => MScr::DoubleStruck,
            Some("sans-serif") => MScr::SansSerif,
            Some("monospace") => MScr::Monospace,
            _ => MScr::Roman,
        };
        out.nor = pr.child("m:nor").is_some_and(on);
        out.lit = pr.child("m:lit").is_some_and(on);
        if let Some(b) = pr.child("m:brk") {
            let aln = b.attr_or_plain("m:alnAt").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
            out.brk = Some(aln.min(255) as u8);
        }
    }
    wrpr(r.child("w:rPr"), &mut out);
    Some(out)
}

/// The run formatting a math run takes from `w:rPr`: size, colour, normal-text font and weight.
fn wrpr(pr: Option<&El>, out: &mut MRun) {
    let Some(pr) = pr else { return };
    if let Some(sz) = pr.child("w:sz").and_then(|s| s.attr_or_plain("w:val")).and_then(|v| v.trim().parse::<f32>().ok()) {
        let pt = sz / 2.0;
        if pt.is_finite() && (1.0..=1638.0).contains(&pt) {
            out.size = Some(pt);
        }
    }
    if let Some(c) = pr.child("w:color").and_then(|c| c.attr_or_plain("w:val")).and_then(Rgb::parse) {
        out.color = Some(c);
    }
    if out.nor {
        // Symbol-font text was mapped to Unicode: it takes the text font.
        if let Some(f) = pr
            .child("w:rFonts")
            .and_then(|f| f.attr_or_plain("w:ascii").or_else(|| f.attr_or_plain("w:hAnsi")))
            .filter(|f| !symbol_fonts::is_symbol_font(f))
        {
            out.font = Some(f.to_string());
        }
        let bold = pr.child("w:b").is_some_and(on_off);
        let italic = pr.child("w:i").is_some_and(on_off);
        if bold || italic {
            out.sty = Some(match (bold, italic) {
                (true, true) => MSty::BoldItalic,
                (true, false) => MSty::Bold,
                _ => MSty::Italic,
            });
        }
    }
}
