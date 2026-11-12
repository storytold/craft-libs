//! DrawingML reading: colours, fills, lines, effects, transforms and text.

use crate::color::{ColorTransform, Rgba, SchemeSlot};
use crate::style::{
    ColorBase, ColorRef, Compound, Dash, Effects, Fill, Glow, Gradient, GradientFlip, GradientShape, GradientStop, Line, LineAlign, LineCap, LineEnd,
    LineJoin, PatternFill, PictureFill, PictureMode, Reflection, Shadow,
};
use crate::text::{
    Action, Align, Anchor, AutoFit, BodyProps, Bullet, Caps, Hyperlink, LevelStyle, ListStyle, ParaProps, Paragraph, Run, RunKind, RunProps, Spacing,
    Strike, TabStop, TextBody, TextDir,
};
use crate::{MediaId, Xfrm, emu_to_pt};

use craft_xml::{El, Local, value};

use crate::part::Source;

pub fn pt(e: &El, name: &str) -> Option<f64> {
    e.attr_local(name).and_then(value::to_i64_lossy).map(emu_to_pt)
}

/// 1/100 000 → fraction. Strict OOXML writes a percentage as `50%`.
pub fn pct(e: &El, name: &str) -> Option<f64> {
    let v = e.attr_local(name)?.trim();
    match v.strip_suffix('%') {
        Some(p) => value::to_f64(p).map(|v| v / 100.0),
        None => value::to_f64(v).map(|v| v / 100_000.0),
    }
}

fn bullet_size_points(value: Option<&str>) -> Option<f64> {
    value.and_then(|v| v.trim().parse::<u32>().ok()).filter(|v| (100..=400_000).contains(v)).map(|v| v as f64 / 100.0)
}

fn bullet_size_percent(value: Option<&str>) -> Option<f64> {
    let value = value?.trim();
    let (value, scale) = if let Some(value) = value.strip_suffix('%') { (value.trim(), 100.0) } else { (value, 100_000.0) };
    value.parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| v / scale)
}

/// 1/60 000 degree → degrees.
pub fn ang(e: &El, name: &str) -> Option<f64> {
    e.attr_local(name).and_then(value::to_f64).map(|v| v / 60_000.0)
}

fn lin_to_srgb(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// A colour element (`srgbClr`, `schemeClr`, …) with its transforms.
pub fn color_el(c: &El) -> Option<ColorRef> {
    let base = match c.local() {
        "srgbClr" => ColorBase::Rgb { rgb: Rgba::from_hex(c.attr_local("val").unwrap_or("000000")).unwrap_or(Rgba::BLACK) },
        "schemeClr" => ColorBase::Scheme { slot: c.attr_local("val").and_then(SchemeSlot::from_xml).unwrap_or(SchemeSlot::Tx1) },
        "sysClr" => ColorBase::System {
            name: c.attr_local("val").unwrap_or("windowText").to_string(),
            last: c
                .attr_local("lastClr")
                .and_then(Rgba::from_hex)
                .or_else(|| c.attr_local("val").and_then(crate::color::system))
                .unwrap_or(Rgba::BLACK),
        },
        "prstClr" => ColorBase::Preset { name: c.attr_local("val").unwrap_or("black").to_string() },
        "scrgbClr" => {
            let f = |n: &str| lin_to_srgb(c.attr_local(n).and_then(value::to_f64).unwrap_or(0.0) / 100_000.0);
            ColorBase::Rgb { rgb: Rgba::from_f64(f("r"), f("g"), f("b"), 1.0) }
        }
        "hslClr" => {
            let h = c.attr_local("hue").and_then(value::to_f64).unwrap_or(0.0) / 60_000.0;
            let s = c.attr_local("sat").and_then(value::to_f64).unwrap_or(0.0) / 100_000.0;
            let l = c.attr_local("lum").and_then(value::to_f64).unwrap_or(0.0) / 100_000.0;
            ColorBase::Rgb { rgb: Rgba::from_hsl(h, s, l, 255) }
        }
        _ => return None,
    };
    let mods = c.elements().take(32).filter_map(|m| ColorTransform::from_xml(m.local(), m.attr_local("val").and_then(value::to_i32_lossy))).collect();
    Some(ColorRef { base, mods })
}

/// The first colour child of a container (`solidFill`, `fgClr`, `lnRef`…).
pub fn color(container: &El) -> Option<ColorRef> {
    container.elements().find_map(color_el)
}

const FILL_NAMES: [&str; 6] = ["noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill"];

/// The fill child of `parent`, if any.
pub fn fill_of(src: &mut dyn Source, parent: &El) -> Option<Fill> {
    parent.elements().find(|e| FILL_NAMES.contains(&e.local())).and_then(|f| fill_el(src, f))
}

pub fn fill_el(src: &mut dyn Source, f: &El) -> Option<Fill> {
    Some(match f.local() {
        "noFill" => Fill::None,
        "solidFill" => Fill::Solid { color: color(f).unwrap_or(ColorRef::rgb(Rgba::BLACK)) },
        "gradFill" => Fill::Gradient(gradient(f)),
        "blipFill" => match blip_fill(src, f) {
            // A picture filling a shape or background with neither `a:stretch` nor `a:tile` is
            // tiled at its own size, as PowerPoint draws it.
            Some(mut p) if f.child(Local("stretch")).is_none() && f.child(Local("tile")).is_none() => {
                p.mode = PictureMode::Tile { tx: 0.0, ty: 0.0, sx: 1.0, sy: 1.0, flip: "none".into(), align: "tl".into() };
                Fill::Picture(p)
            }
            Some(p) => Fill::Picture(p),
            None => Fill::None,
        },
        "pattFill" => Fill::Pattern(pattern(f)),
        "grpFill" => Fill::Group,
        _ => return None,
    })
}

/// An `a:pattFill`: its preset (5 % dots when unset), foreground (black) and background (white).
pub fn pattern(f: &El) -> PatternFill {
    PatternFill {
        preset: f.attr_local("prst").unwrap_or("pct5").to_string(),
        fg: f.child(Local("fgClr")).and_then(color).unwrap_or(ColorRef::rgb(Rgba::BLACK)),
        bg: f.child(Local("bgClr")).and_then(color).unwrap_or(ColorRef::rgb(Rgba::WHITE)),
    }
}

pub fn gradient(f: &El) -> Gradient {
    let stops: Vec<GradientStop> = f
        .child(Local("gsLst"))
        .map(|l| {
            l.children(Local("gs"))
                .take(256)
                .map(|g| GradientStop { pos: pct(g, "pos").unwrap_or(0.0).clamp(0.0, 1.0), color: color(g).unwrap_or(ColorRef::rgb(Rgba::BLACK)) })
                .collect()
        })
        .unwrap_or_default();
    let shape = if let Some(l) = f.child(Local("lin")) {
        Some(GradientShape::Linear { angle: ang(l, "ang").unwrap_or(0.0), scaled: l.attr_local("scaled").and_then(value::to_bool).unwrap_or(false) })
    } else if let Some(p) = f.child(Local("path")) {
        let r = p.child(Local("fillToRect"));
        let g = |n: &str| r.and_then(|r| pct(r, n)).unwrap_or(0.0);
        Some(GradientShape::Path { path: p.attr_local("path").unwrap_or("circle").to_string(), focus: [g("l"), g("t"), g("r"), g("b")] })
    } else {
        None
    };
    let tile = f.child(Local("tileRect")).map(|r| [pct(r, "l"), pct(r, "t"), pct(r, "r"), pct(r, "b")].map(|v| v.unwrap_or(0.0).clamp(-10.0, 10.0)));
    Gradient {
        tile: tile.unwrap_or_default(),
        stops,
        shape,
        flip: f.attr_local("flip").and_then(GradientFlip::from_xml),
        rotate_with_shape: f.attr_local("rotWithShape").and_then(value::to_bool),
    }
}

/// `blipFill` (either `a:` or `p:`) → picture fill, registering the image as media.
pub fn blip_fill(src: &mut dyn Source, bf: &El) -> Option<PictureFill> {
    let blip = bf.child(Local("blip"));
    let media = blip.and_then(|b| media_ref(src, b));
    let media = media?;
    let mut pf = PictureFill { media, upright: !bf.attr_local("rotWithShape").and_then(value::to_bool).unwrap_or(true), ..Default::default() };
    if let Some(layer) = blip.and_then(|b| b.child(Local("extLst"))).and_then(|x| x.find(Local("imgLayer")))
        && let Some(effect) = layer
            .elements()
            .filter(|e| e.local() == "imgEffect")
            .flat_map(|e| e.elements())
            .map(|e| e.local().to_string())
            .find(|n| n.starts_with("artistic"))
        && let Some(original) = media_ref(src, layer)
    {
        pf.artistic = Some(crate::style::Artistic { effect, original });
    }
    if let Some(b) = blip {
        for e in b.elements() {
            match e.local() {
                "alphaModFix" => pf.alpha = Some(pct(e, "amt").unwrap_or(1.0).clamp(0.0, 1.0)),
                "lum" => {
                    pf.adjust.brightness = pct(e, "bright").unwrap_or(0.0).clamp(-1.0, 1.0);
                    pf.adjust.contrast = pct(e, "contrast").unwrap_or(0.0).clamp(-1.0, 1.0);
                }
                "grayscl" => pf.adjust.grayscale = true,
                "biLevel" => pf.adjust.bilevel = Some(pct(e, "thresh").unwrap_or(0.5).clamp(0.0, 1.0)),
                "alphaInv" => pf.adjust.alpha_inv = true,
                "duotone" => {
                    let cs: Vec<ColorRef> = e.elements().filter_map(color_el).collect();
                    if let [a, b] = cs.as_slice() {
                        pf.adjust.duotone = Some((a.clone(), b.clone()));
                    }
                }
                "clrChange" => {
                    if let Some(ColorRef { base: ColorBase::Rgb { rgb }, .. }) = e.child(Local("clrFrom")).and_then(color) {
                        pf.adjust.clear_color = Some(rgb);
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(r) = bf.child(Local("srcRect")) {
        pf.crop = [pct(r, "l").unwrap_or(0.0), pct(r, "t").unwrap_or(0.0), pct(r, "r").unwrap_or(0.0), pct(r, "b").unwrap_or(0.0)]
            .map(|v| v.clamp(-10.0, 1.0));
    }
    if let Some(t) = bf.child(Local("tile")) {
        pf.mode = PictureMode::Tile {
            tx: pt(t, "tx").unwrap_or(0.0),
            ty: pt(t, "ty").unwrap_or(0.0),
            sx: pct(t, "sx").unwrap_or(1.0),
            sy: pct(t, "sy").unwrap_or(1.0),
            flip: t.attr_local("flip").unwrap_or("none").to_string(),
            align: t.attr_local("algn").unwrap_or("tl").to_string(),
        };
    } else {
        let r = bf.path(&[Local("stretch"), Local("fillRect")]);
        let g = |n: &str| r.and_then(|r| pct(r, n)).unwrap_or(0.0).clamp(-10.0, 1.0);
        pf.mode = PictureMode::Stretch { fill_rect: [g("l"), g("t"), g("r"), g("b")] };
    }
    Some(pf)
}

/// Media for an element with `r:embed` (or `r:link`).
pub fn media_ref(src: &mut dyn Source, b: &El) -> Option<MediaId> {
    if let Some(id) = b.attr_local("r:embed").filter(|s| !s.is_empty())
        && let Some(r) = src.rel(id)
        && let Some(m) = src.media(&r.target, r.external)
    {
        return Some(m);
    }
    let r = src.rel(b.attr_local("r:link").filter(|s| !s.is_empty())?)?;
    src.media(&r.target, r.external)
}

pub fn line(src: &mut dyn Source, ln: &El) -> Line {
    let mut l = Line { width: pt(ln, "w").map(|w| w.clamp(0.0, 2000.0)), ..Default::default() };
    l.cap = ln.attr_local("cap").map(|c| match c {
        "rnd" => LineCap::Round,
        "sq" => LineCap::Square,
        _ => LineCap::Flat,
    });
    l.compound = ln.attr_local("cmpd").map(Compound::from_xml);
    l.align = ln.attr_local("algn").and_then(LineAlign::from_xml);
    l.fill = fill_of(src, ln);
    for e in ln.elements() {
        match e.local() {
            "prstDash" => l.dash = Some(Dash::Preset { name: e.attr_local("val").unwrap_or("solid").to_string() }),
            "custDash" => {
                let pattern = e.children(Local("ds")).take(64).map(|d| (pct(d, "d").unwrap_or(1.0), pct(d, "sp").unwrap_or(1.0))).collect();
                l.dash = Some(Dash::Custom { pattern });
            }
            "round" => l.join = Some(LineJoin::Round),
            "bevel" => l.join = Some(LineJoin::Bevel),
            "miter" => {
                l.join = Some(LineJoin::Miter);
                l.miter_limit = e.attr_local("lim").and_then(parse_miter_limit);
            }
            "headEnd" => l.head = line_end(e),
            "tailEnd" => l.tail = line_end(e),
            _ => {}
        }
    }
    l
}

fn parse_miter_limit(value: &str) -> Option<u32> {
    const MAX: u32 = i32::MAX as u32;
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        let units = percent.trim().parse::<f64>().ok()? * 1000.0;
        (units.is_finite() && (0.0..=MAX as f64).contains(&units)).then(|| units.round() as u32)
    } else {
        value.parse::<u32>().ok().filter(|limit| *limit <= MAX)
    }
}

fn line_end(e: &El) -> Option<LineEnd> {
    let kind = e.attr_local("type").unwrap_or("none");
    Some(LineEnd { kind: kind.to_string(), w: e.attr_local("w").unwrap_or("med").to_string(), len: e.attr_local("len").unwrap_or("med").to_string() })
}

fn shadow(e: &El, inner: bool) -> Shadow {
    Shadow {
        color: color(e).unwrap_or(ColorRef::rgb(Rgba::BLACK)),
        blur: pt(e, "blurRad").unwrap_or(0.0).max(0.0),
        dist: pt(e, "dist").unwrap_or(0.0),
        dir: ang(e, "dir").unwrap_or(0.0),
        inner,
        sx: pct(e, "sx").unwrap_or(1.0),
        sy: pct(e, "sy").unwrap_or(1.0),
        kx: ang(e, "kx").unwrap_or(0.0),
        ky: ang(e, "ky").unwrap_or(0.0),
        align: e.attr_local("algn").unwrap_or(if inner { "" } else { "b" }).to_string(),
        rotate_with_shape: e.attr_local("rotWithShape").and_then(value::to_bool).unwrap_or(true),
    }
}

/// `effectLst` → effects.
pub fn effects(el: &El) -> Effects {
    let mut fx = Effects::default();
    for e in el.elements() {
        match e.local() {
            "outerShdw" => fx.outer_shadow = Some(shadow(e, false)),
            "prstShdw" => {
                let mut s = shadow(e, false);
                s.align = "b".into();
                fx.outer_shadow = Some(s);
            }
            "innerShdw" => fx.inner_shadow = Some(shadow(e, true)),
            "glow" => fx.glow = Some(Glow { color: color(e).unwrap_or(ColorRef::rgb(Rgba::BLACK)), radius: pt(e, "rad").unwrap_or(0.0).max(0.0) }),
            "softEdge" => fx.soft_edge = Some(pt(e, "rad").unwrap_or(0.0).max(0.0)),
            "fillOverlay" => {
                let fill = e.elements().find_map(|f| match f.local() {
                    "solidFill" => color(f).map(|c| Fill::Solid { color: c }),
                    "gradFill" => Some(Fill::Gradient(gradient(f))),
                    _ => None,
                });
                fx.fill_overlay = fill.map(|fill| crate::style::FillOverlay { blend: e.attr_local("blend").unwrap_or("over").to_string(), fill });
            }
            "reflection" => {
                fx.reflection = Some(Reflection {
                    blur: pt(e, "blurRad").unwrap_or(0.0),
                    start_alpha: pct(e, "stA").unwrap_or(1.0),
                    end_alpha: pct(e, "endA").unwrap_or(0.0),
                    end_pos: pct(e, "endPos").unwrap_or(1.0),
                    dist: pt(e, "dist").unwrap_or(0.0),
                    st_pos: e.attr_local("stPos").and_then(value::to_i64_lossy).and_then(|v| i32::try_from(v).ok()),
                    dir: e.attr_local("dir").and_then(value::to_i64_lossy).and_then(|v| i32::try_from(v).ok()),
                    fade_dir: e.attr_local("fadeDir").and_then(value::to_i64_lossy).and_then(|v| i32::try_from(v).ok()),
                    sx: e.attr_local("sx").and_then(value::to_i64_lossy).and_then(|v| i32::try_from(v).ok()),
                    sy: e.attr_local("sy").and_then(value::to_i64_lossy).and_then(|v| i32::try_from(v).ok()),
                    kx: e.attr_local("kx").and_then(value::to_i64_lossy).and_then(|v| i32::try_from(v).ok()),
                    ky: e.attr_local("ky").and_then(value::to_i64_lossy).and_then(|v| i32::try_from(v).ok()),
                    align: e
                        .attr_local("algn")
                        .filter(|v| matches!(*v, "tl" | "t" | "tr" | "l" | "ctr" | "r" | "bl" | "b" | "br"))
                        .map(str::to_owned),
                    rotate_with_shape: e.attr_local("rotWithShape").and_then(value::to_bool),
                })
            }
            _ => {}
        }
    }
    fx
}

/// `a:xfrm` → (box, child box for groups).
pub fn xfrm(x: &El) -> (Xfrm, Option<Xfrm>) {
    let off = x.child(Local("off"));
    let ext = x.child(Local("ext"));
    let g = |e: Option<&El>, n: &str| e.and_then(|e| pt(e, n)).unwrap_or(0.0);
    let mut xf = Xfrm::new(g(off, "x"), g(off, "y"), g(ext, "cx").max(0.0), g(ext, "cy").max(0.0));
    xf.rot = ang(x, "rot").unwrap_or(0.0);
    if !xf.rot.is_finite() {
        xf.rot = 0.0;
    }
    xf.flip_h = x.attr_local("flipH").and_then(value::to_bool).unwrap_or(false);
    xf.flip_v = x.attr_local("flipV").and_then(value::to_bool).unwrap_or(false);
    let child = match (x.child(Local("chOff")), x.child(Local("chExt"))) {
        (Some(o), Some(e)) => Some(Xfrm::new(g(Some(o), "x"), g(Some(o), "y"), g(Some(e), "cx").max(0.0), g(Some(e), "cy").max(0.0))),
        _ => None,
    };
    (xf, child)
}

// ---------------------------------------------------------------------------------------------
// Text

pub fn body_pr(b: &El) -> BodyProps {
    let mut p = BodyProps {
        inset_l: pt(b, "lIns"),
        inset_t: pt(b, "tIns"),
        inset_r: pt(b, "rIns"),
        inset_b: pt(b, "bIns"),
        anchor: b.attr_local("anchor").and_then(Anchor::from_xml),
        anchor_ctr: b.attr_local("anchorCtr").and_then(value::to_bool),
        wrap: b.attr_local("wrap").map(|w| w != "none"),
        vert: b.attr_local("vert").and_then(TextDir::from_xml),
        rot: ang(b, "rot"),
        columns: b.attr_local("numCol").and_then(value::to_u32_lossy).map(|n| n.clamp(1, 16)),
        col_spacing: pt(b, "spcCol"),
        rtl_col: b.attr_local("rtlCol").and_then(value::to_bool),
        upright: b.attr_local("upright").and_then(value::to_bool),
        space_first_last: b.attr_local("spcFirstLastPara").and_then(value::to_bool),
        compat_ln_spc: b.attr_local("compatLnSpc").and_then(value::to_bool),
        vert_overflow: b.attr_local("vertOverflow").and_then(crate::text::TextOverflow::from_xml),
        horz_overflow: b.attr_local("horzOverflow").and_then(crate::text::TextOverflow::from_horz_xml),
        ..Default::default()
    };
    for e in b.elements() {
        match e.local() {
            "normAutofit" => {
                p.autofit = Some(AutoFit::Shrink {
                    font_scale: pct(e, "fontScale").map(|v| v.clamp(0.01, 1.0)),
                    line_reduction: pct(e, "lnSpcReduction").unwrap_or(0.0).clamp(0.0, 1.0),
                })
            }
            "spAutoFit" => p.autofit = Some(AutoFit::Shape),
            "noAutofit" => p.autofit = Some(AutoFit::None),
            "prstTxWarp" => {
                p.warp = e.attr_local("prst").filter(|w| *w != "textNoShape" && !w.is_empty()).map(String::from);
                p.warp_adj = p.warp.as_deref().map(|w| warp_adjusts(w, e)).unwrap_or_default();
            }
            _ => {}
        }
    }
    p
}

/// `prstTxWarp/avLst` guides `adj` / `adj1`… as raw values by index (empty when none is given);
/// slots the file leaves out keep the preset's default.
fn warp_adjusts(preset: &str, e: &El) -> Vec<f64> {
    let mut adj: Vec<f64> = Vec::new();
    let Some(av) = e.child(Local("avLst")) else { return adj };
    let defaults = crate::warp::adjust_defaults(preset);
    for gd in av.children(Local("gd")).take(16) {
        let n = gd.attr_local("name").unwrap_or("");
        let idx = if n == "adj" {
            0
        } else if let Some(k) = n.strip_prefix("adj").and_then(|k| k.parse::<usize>().ok()).filter(|k| (1..=8).contains(k)) {
            k - 1
        } else {
            continue;
        };
        let Some(v) =
            gd.attr_local("fmla").and_then(|f| f.trim().strip_prefix("val")).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite())
        else {
            continue;
        };
        while adj.len() <= idx {
            adj.push(defaults.get(adj.len()).copied().unwrap_or(0.0));
        }
        if let Some(slot) = adj.get_mut(idx) {
            *slot = v.clamp(-1.0e9, 1.0e9);
        }
    }
    adj
}

fn spacing(e: &El) -> Option<Spacing> {
    if let Some(p) = e.child(Local("spcPct")) {
        return Some(Spacing::Pct(pct(p, "val").unwrap_or(1.0).clamp(0.0, 100.0)));
    }
    e.child(Local("spcPts")).map(|p| Spacing::Pts(p.attr_local("val").and_then(value::to_f64).unwrap_or(0.0).clamp(0.0, 1_000_000.0) / 100.0))
}

/// `a:pPr` / `a:lvlNpPr` → (level, paragraph props, default run props).
pub fn ppr(src: &mut dyn Source, p: &El) -> (Option<u8>, ParaProps, Option<RunProps>) {
    let mut pp = ParaProps {
        align: p.attr_local("algn").and_then(Align::from_xml),
        margin_left: pt(p, "marL"),
        margin_right: pt(p, "marR"),
        indent: pt(p, "indent"),
        rtl: p.attr_local("rtl").and_then(value::to_bool),
        font_align: p.attr_local("fontAlgn").map(String::from),
        east_asian_line_break: p.attr_local("eaLnBrk").and_then(value::to_bool),
        latin_line_break: p.attr_local("latinLnBrk").and_then(value::to_bool),
        hanging_punct: p.attr_local("hangingPunct").and_then(value::to_bool),
        default_tab: pt(p, "defTabSz"),
        ..Default::default()
    };
    let lvl = p.attr_local("lvl").and_then(value::to_u32_lossy).map(|l| l.min(8) as u8);
    let mut def = None;
    for e in p.elements() {
        match e.local() {
            "lnSpc" => pp.line_spacing = spacing(e),
            "spcBef" => pp.space_before = spacing(e),
            "spcAft" => pp.space_after = spacing(e),
            "buNone" => pp.bullet = Some(Bullet::None),
            "buChar" => pp.bullet = Some(Bullet::Char { char: e.attr_local("char").unwrap_or("•").chars().take(4).collect() }),
            "buAutoNum" => {
                pp.bullet = Some(Bullet::AutoNum {
                    scheme: e.attr_local("type").unwrap_or("arabicPeriod").to_string(),
                    start_at: e.attr_local("startAt").and_then(value::to_u32_lossy).unwrap_or(1).clamp(1, 32767),
                })
            }
            "buBlip" => {
                if let Some(m) = e.child(Local("blip")).and_then(|b| media_ref(src, b)) {
                    pp.bullet = Some(Bullet::Picture { media: m });
                }
            }
            "buFont" => pp.bullet_font = e.attr_local("typeface").map(String::from),
            "buFontTx" => pp.bullet_font_follow_text = Some(true),
            "buClr" => pp.bullet_color = color(e),
            "buSzPct" => {
                pp.bullet_size_points = None;
                pp.bullet_size_follow_text = None;
                pp.bullet_size = bullet_size_percent(e.attr_local("val")).map(|v| v.clamp(0.25, 4.0));
            }
            "buSzPts" => {
                pp.bullet_size = None;
                pp.bullet_size_follow_text = None;
                pp.bullet_size_points = bullet_size_points(e.attr_local("val"));
            }
            "buClrTx" => pp.bullet_color_follow_text = Some(true),
            "buSzTx" => {
                pp.bullet_size = None;
                pp.bullet_size_points = None;
                pp.bullet_size_follow_text = Some(true);
            }
            "tabLst" => {
                pp.tabs = Some(
                    e.children(Local("tab"))
                        .take(64)
                        .map(|t| TabStop { pos: pt(t, "pos").unwrap_or(0.0), align: t.attr_local("algn").unwrap_or("l").to_string() })
                        .collect(),
                )
            }
            "defRPr" => def = Some(rpr(src, e)),
            _ => {}
        }
    }
    (lvl, pp, def)
}

pub fn hyperlink(src: &mut dyn Source, h: &El) -> Option<Hyperlink> {
    let rid = h.attr_local("r:id").unwrap_or("");
    let rel = src.rel(rid);
    let action = h.attr_local("action").unwrap_or("");
    let a = if let Some(rest) = action.strip_prefix("ppaction://hlinkshowjump?jump=") {
        match rest {
            "nextslide" => Action::NextSlide,
            "previousslide" => Action::PreviousSlide,
            "firstslide" => Action::FirstSlide,
            "lastslide" => Action::LastSlide,
            "endshow" => Action::EndShow,
            "lastslideviewed" => Action::LastViewed,
            _ => return None,
        }
    } else if action.starts_with("ppaction://hlinksldjump") {
        let slide = rel.as_ref().and_then(|r| src.slide(&r.target))?;
        Action::Slide { slide }
    } else if let Some(q) = action.strip_prefix("ppaction://customshow?") {
        let id = q.split('&').find_map(|kv| kv.strip_prefix("id=")).unwrap_or("0");
        let returns = !matches!(q.split('&').find_map(|kv| kv.strip_prefix("return=")), Some("false" | "0"));
        Action::CustomShow { name: src.custom_show(id).unwrap_or_else(|| id.to_string()), returns }
    } else if action.starts_with("ppaction://program") {
        Action::Program { path: rel.map(|r| r.target).unwrap_or_default() }
    } else if action.starts_with("ppaction://media") {
        Action::PlayMedia
    } else if action.is_empty() || action.starts_with("ppaction://hlinkfile") {
        let r = rel?;
        if r.target.is_empty() {
            return None;
        }
        if !r.external
            && let Some(slide) = src.slide(&r.target)
        {
            Action::Slide { slide }
        } else {
            Action::Url { url: r.target }
        }
    } else {
        return None;
    };
    Some(Hyperlink {
        action: a,
        tooltip: h.attr_local("tooltip").unwrap_or("").to_string(),
        highlight_click: h.attr_local("highlightClick").and_then(value::to_bool).unwrap_or(false),
        stop_sound: h.attr_local("endSnd").and_then(value::to_bool).unwrap_or(false),
        sound: h.child(Local("snd")).and_then(|s| media_ref(src, s)),
        zoom_return: None,
    })
}

fn typeface(e: &El) -> Option<String> {
    let t = e.attr_local("typeface").filter(|t| !t.is_empty())?;
    Some(t.chars().take(256).collect())
}

pub fn rpr(src: &mut dyn Source, r: &El) -> RunProps {
    let mut p = RunProps {
        lang: r.attr_local("lang").map(String::from),
        alt_lang: r.attr_local("altLang").map(|lang| lang.chars().take(256).collect()),
        size: r.attr_local("sz").and_then(value::to_f64).map(|s| (s / 100.0).clamp(1.0, 4000.0)),
        bold: r.attr_local("b").and_then(value::to_bool),
        italic: r.attr_local("i").and_then(value::to_bool),
        underline: r.attr_local("u").map(String::from),
        strike: r.attr_local("strike").map(|s| match s {
            "sngStrike" => Strike::Single,
            "dblStrike" => Strike::Double,
            _ => Strike::None,
        }),
        baseline: pct(r, "baseline").map(|b| b.clamp(-10.0, 10.0)),
        caps: r.attr_local("cap").map(|c| match c {
            "small" => Caps::Small,
            "all" => Caps::All,
            _ => Caps::None,
        }),
        spacing: r.attr_local("spc").and_then(value::to_f64).map(|s| (s / 100.0).clamp(-4000.0, 4000.0)),
        kern: r.attr_local("kern").and_then(value::to_f64).map(|k| (k / 100.0).clamp(0.0, 4000.0)),
        no_proof: r.attr_local("noProof").and_then(value::to_bool),
        normalize_height: r.attr_local("normalizeH").and_then(value::to_bool),
        ..Default::default()
    };
    for e in r.elements() {
        match e.local() {
            "ln" => p.outline = Some(line(src, e)),
            n if FILL_NAMES.contains(&n) => p.fill = fill_el(src, e),
            "effectLst" => {
                let fx = effects(e);
                p.shadow = fx.outer_shadow;
                p.inner_shadow = fx.inner_shadow;
                p.glow = fx.glow;
                p.reflection = fx.reflection;
                p.soft_edge = fx.soft_edge;
            }
            "highlight" => p.highlight = color(e),
            "uFill" => p.underline_color = e.child(Local("solidFill")).and_then(color),
            "latin" => p.font = typeface(e),
            "ea" => p.font_ea = typeface(e),
            "cs" => p.font_cs = typeface(e),
            "sym" => p.font_sym = typeface(e),
            "hlinkClick" => p.link = hyperlink(src, e),
            "hlinkMouseOver" => p.hover = hyperlink(src, e),
            _ => {}
        }
    }
    p
}

/// `a:lstStyle`, `p:titleStyle`, `a:defaultTextStyle`… → list style.
pub fn list_style(src: &mut dyn Source, ls: &El) -> ListStyle {
    let mut out = ListStyle::default();
    for e in ls.elements() {
        let n = e.local();
        let Some(l) = n.strip_prefix("lvl").and_then(|r| r.strip_suffix("pPr")).and_then(|d| d.parse::<u8>().ok()) else { continue };
        if !(1..=9).contains(&l) {
            continue;
        }
        let (_, para, run) = ppr(src, e);
        out.set(l - 1, LevelStyle { para, run: run.unwrap_or_default() });
    }
    out
}

pub fn text_body(src: &mut dyn Source, tx: &El) -> TextBody {
    let mut body = tx.child(Local("bodyPr")).map(body_pr).unwrap_or_default();
    if let Some(b) = tx.child(Local("bodyPr")) {
        let mut raw3d = String::new();
        if let Some(sc) = b.child(Local("scene3d")) {
            raw3d.push_str(&src.keep_inner(sc));
        }
        if let Some(sp) = b.child(Local("sp3d")).or_else(|| b.child(Local("flatTx"))) {
            raw3d.push_str(&src.keep_inner(sp));
        }
        if !raw3d.is_empty() && raw3d.len() <= 64 * 1024 {
            body.raw3d = Some(raw3d);
        }
    }
    let list_style = tx.child(Local("lstStyle")).map(|l| list_style(src, l)).unwrap_or_default();
    let paragraphs = tx.children(Local("p")).take(100_000).map(|p| paragraph(src, p)).collect();
    TextBody { body, list_style, paragraphs }
}

const MAX_PARAGRAPH_RUNS: usize = 10_000;

/// The run properties of `e`'s `rPr` child, or the defaults.
fn run_props(src: &mut dyn Source, e: &El) -> RunProps {
    e.child(Local("rPr")).map(|r| rpr(src, r)).unwrap_or_default()
}

/// Appends the runs of an `a:r` text, where a vertical tab or line feed is a line break, as it
/// lays out and as export writes it.
fn push_text_runs(para: &mut Paragraph, props: RunProps, text: &str) {
    for (i, piece) in text.split(['\u{b}', '\n']).enumerate() {
        if para.runs.len() >= MAX_PARAGRAPH_RUNS {
            break;
        }
        if i > 0 {
            para.runs.push(Run { text: String::new(), props: props.clone(), kind: RunKind::Break });
        }
        if !piece.is_empty() {
            if para.runs.len() >= MAX_PARAGRAPH_RUNS {
                break;
            }
            para.runs.push(Run { text: piece.to_string(), props: props.clone(), kind: RunKind::Text });
        }
    }
}

/// Appends an Office Math (a14:m) run: its plain text and its OMML.
fn push_math_run(para: &mut Paragraph, src: &dyn Source, e: &El) {
    if para.runs.len() < MAX_PARAGRAPH_RUNS {
        // The text of the outermost `t` elements: a `t` nested in another is already part of
        // the outer one's text, and taking it again would copy it once per nesting level.
        fn outer_t(e: &El, depth: usize, text: &mut String) {
            if depth > crate::xmlx::MAX_DEPTH {
                return;
            }
            for c in e.elements() {
                if c.is(Local("t")) {
                    text.push_str(&c.deep_text());
                } else {
                    outer_t(c, depth + 1, text);
                }
            }
        }
        let mut text = String::new();
        outer_t(e, 0, &mut text);
        let omml = e.elements().map(|c| src.keep_inner(c)).collect::<String>();
        para.runs.push(Run { text, props: RunProps::default(), kind: RunKind::Math { omml } });
    }
}

pub fn paragraph(src: &mut dyn Source, p: &El) -> Paragraph {
    let mut para = Paragraph::default();
    for e in p.elements() {
        match e.local() {
            "pPr" => {
                let (lvl, pp, def) = ppr(src, e);
                para.level = lvl.unwrap_or(0);
                para.props = pp;
                if let Some(d) = def {
                    para.def_props = d;
                }
            }
            "r" => {
                let props = run_props(src, e);
                let text: String = e.child(Local("t")).map(|t| t.deep_text()).unwrap_or_default();
                push_text_runs(&mut para, props, &text);
            }
            "br" if para.runs.len() < MAX_PARAGRAPH_RUNS => {
                let props = run_props(src, e);
                para.runs.push(Run { text: String::new(), props, kind: RunKind::Break });
            }
            "fld" if para.runs.len() < MAX_PARAGRAPH_RUNS => {
                let props = run_props(src, e);
                let text = e.child(Local("t")).map(|t| t.deep_text()).unwrap_or_default();
                para.runs.push(Run { text, props, kind: RunKind::Field { field: e.attr_local("type").unwrap_or("").to_string() } });
            }
            "m" => push_math_run(&mut para, &*src, e),
            "endParaRPr" => para.end_props = rpr(src, e),
            _ => {}
        }
    }
    para
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xmlx::parse;

    #[test]
    fn nested_math_text_is_taken_once() {
        // 200 `m:t` nested in each other around one word: the word, not 200 copies of it.
        let x = format!(r#"<a14:m xmlns:a14="a" xmlns:m="m"><m:oMath>{}word{}</m:oMath></a14:m>"#, "<m:t>".repeat(200), "</m:t>".repeat(200));
        let d = parse(x.as_bytes()).unwrap();
        let mut para = Paragraph::default();
        push_math_run(&mut para, &crate::part::NoSource, &d);
        assert_eq!(para.runs[0].text, "word");
    }

    #[test]
    fn colors_and_transforms() {
        let d = parse(
            br#"<a:solidFill xmlns:a="x"><a:schemeClr val="accent2"><a:lumMod val="75000"/><a:alpha val="50000"/></a:schemeClr></a:solidFill>"#,
        )
        .unwrap();
        let c = color(&d).unwrap();
        assert_eq!(c.base, ColorBase::Scheme { slot: SchemeSlot::Accent2 });
        assert_eq!(c.mods, vec![ColorTransform::LumMod(75000), ColorTransform::Alpha(50000)]);
        let d = parse(br#"<x><a:sysClr val="window" lastClr="FFFFFF"/></x>"#).unwrap();
        assert!(matches!(color(&d).unwrap().base, ColorBase::System { last: Rgba::WHITE, .. }));
        let d = parse(br#"<x><a:scrgbClr r="100000" g="0" b="0"/></x>"#).unwrap();
        assert_eq!(color(&d).unwrap().base, ColorBase::Rgb { rgb: Rgba::rgb(255, 0, 0) });
    }

    #[test]
    fn bullet_point_size_trims_schema_whitespace() {
        let d = parse(br#"<a:buSzPts xmlns:a="x" val=" 1400 "/>"#).unwrap();
        assert_eq!(bullet_size_points(d.attr_local("val")), Some(14.0));
        assert_eq!(bullet_size_points(Some("1400")), Some(14.0));
    }

    #[test]
    fn bullet_percentage_size_accepts_numeric_and_percent_forms() {
        assert_eq!(bullet_size_percent(Some("100000")), Some(1.0));
        assert_eq!(bullet_size_percent(Some("100%")), Some(1.0));
        assert_eq!(bullet_size_percent(Some(" 125% ")), Some(1.25));
        assert_eq!(bullet_size_percent(Some("invalid")), None);
    }

    #[test]
    fn miter_limit_accepts_both_percentage_forms_with_schema_bounds() {
        assert_eq!(parse_miter_limit("200000"), Some(200_000));
        assert_eq!(parse_miter_limit("200%"), Some(200_000));
        assert_eq!(parse_miter_limit("1000001"), Some(1_000_001));
        assert_eq!(parse_miter_limit("malformed"), None);
        assert_eq!(parse_miter_limit("2147483648"), None);
        assert_eq!(parse_miter_limit("2147483.648%"), None);
    }

    #[test]
    fn body_props() {
        let d = parse(br#"<a:bodyPr lIns="91440" anchor="ctr" wrap="none" vert="vert270" numCol="2"><a:normAutofit fontScale="62500" lnSpcReduction="20000"/></a:bodyPr>"#).unwrap();
        let b = body_pr(&d);
        assert_eq!(b.inset_l, Some(7.2));
        assert_eq!(b.anchor, Some(Anchor::Middle));
        assert_eq!(b.wrap, Some(false));
        assert_eq!(b.vert, Some(TextDir::Vertical270));
        assert_eq!(b.columns, Some(2));
        assert_eq!(b.autofit, Some(AutoFit::Shrink { font_scale: Some(0.625), line_reduction: 0.2 }));
        assert_eq!(b.space_first_last, None);
        // A bare `<a:normAutofit/>` keeps no scale: the schema default 100 % is drawn as it is,
        // not the same as a saved 100 % (`fontScale="100000"`) (#724).
        let d = parse(br#"<a:bodyPr><a:normAutofit/></a:bodyPr>"#).unwrap();
        assert_eq!(body_pr(&d).autofit, Some(AutoFit::Shrink { font_scale: None, line_reduction: 0.0 }));
        let d = parse(br#"<a:bodyPr><a:normAutofit fontScale="100000"/></a:bodyPr>"#).unwrap();
        assert_eq!(body_pr(&d).autofit, Some(AutoFit::Shrink { font_scale: Some(1.0), line_reduction: 0.0 }));
        let d = parse(br#"<a:bodyPr spcFirstLastPara="1"/>"#).unwrap();
        assert_eq!(body_pr(&d).space_first_last, Some(true));
        // mongolianVert is its own direction (PowerPoint runs its lines left to right) and is
        // written back as it was read.
        let d = parse(br#"<a:bodyPr vert="mongolianVert"/>"#).unwrap();
        let b = body_pr(&d);
        assert_eq!(b.vert, Some(TextDir::MongolianVertical));
        assert_eq!(b.vert.map(|v| v.xml()), Some("mongolianVert"));
        let d = parse(br#"<a:bodyPr vert="wordArtVertRtl"/>"#).unwrap();
        assert_eq!(body_pr(&d).vert.map(|v| v.xml()), Some("wordArtVertRtl"));
    }

    #[test]
    fn percentages_read_in_either_spelling() {
        let d = parse(br#"<a:custDash xmlns:a="x"><a:ds d="400000" sp="300%"/><a:ds d="12.5%" sp="x"/></a:custDash>"#).unwrap();
        let ds: Vec<_> = d.children(Local("ds")).collect();
        assert_eq!((pct(ds[0], "d"), pct(ds[0], "sp")), (Some(4.0), Some(3.0)));
        assert_eq!((pct(ds[1], "d"), pct(ds[1], "sp")), (Some(0.125), None));
    }
}
