//! Auto Mask strokes on a real raw, looked at: `CRAFT_AUTOMASK_RAW=<raw>` renders it at
//! `CRAFT_AUTOMASK_SIZE` (default 1220 × 814, a dialog's preview), paints the strokes of
//! `CRAFT_AUTOMASK_STROKES` (JSON: case name → list of strokes, each a list of `[x, y]` on the
//! photo, 0..1; `size` the brush radius on the long edge, default 0.03) and writes, into
//! `CRAFT_AUTOMASK_OUT`, `<case>-overlay.ppm` (red overlay) and `<case>-exposure.ppm` (+1.5 EV on
//! the mask). Without the raw it does nothing.
//!
//! More, all optional: `CRAFT_AUTOMASK_SETTINGS` (a settings JSON to render with instead of a raw's
//! defaults, its masks dropped; an app's own defaults differ), `CRAFT_AUTOMASK_BRUSH`,
//! `CRAFT_AUTOMASK_PROCESS`, `CRAFT_AUTOMASK_LIVE` (paint as a UI does, a few points at a time
//! through one cache, and time each update), `CRAFT_AUTOMASK_DAB=<k>` (print dab k's grid of path
//! costs and memberships), `CRAFT_AUTOMASK_PIXEL=x,y` (which dabs paint that pixel, and why),
//! `CRAFT_AUTOMASK_POINTS` (JSON: name → `[x, y]`; prints the photo there, and dumps the planes).
//!
//! `cargo test -p craft-develop --lib photo_probe -- --nocapture`

use super::*;
use crate::{RenderRequest, StageCache, prepare_stages, render_cached};
use craft_develop_model::{BrushStroke, DevelopSettings, Mask, MaskComponent, MaskOp, MaskShape};

fn ppm(path: &std::path::Path, w: usize, h: usize, px: impl Fn(usize, usize) -> [u8; 3]) {
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for y in 0..h {
        for x in 0..w {
            out.extend_from_slice(&px(x, y));
        }
    }
    std::fs::write(path, out).unwrap();
}

#[test]
fn photo_probe() {
    let Ok(path) = std::env::var("CRAFT_AUTOMASK_RAW") else { return };
    let out = std::path::PathBuf::from(std::env::var("CRAFT_AUTOMASK_OUT").unwrap_or_else(|_| ".".into()));
    std::fs::create_dir_all(&out).unwrap();
    let (rw, rh) = std::env::var("CRAFT_AUTOMASK_SIZE")
        .ok()
        .and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap())))
        .unwrap_or((1220usize, 814usize));
    let size: f64 = std::env::var("CRAFT_AUTOMASK_BRUSH").ok().and_then(|s| s.parse().ok()).unwrap_or(0.03);
    let process = std::env::var("CRAFT_AUTOMASK_PROCESS").ok().and_then(|s| s.parse::<u32>().ok());
    let cases: serde_json::Value = serde_json::from_str(&std::env::var("CRAFT_AUTOMASK_STROKES").unwrap()).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let loaded = crate::load::load_raw(&bytes, &crate::load::LoadOptions::fit(rw.max(rh))).unwrap();
    let src = Arc::new(loaded.image);
    let info = loaded.info;
    let mut base = DevelopSettings::for_raw(info.as_shot_temp, info.as_shot_tint);
    if let Ok(f) = std::env::var("CRAFT_AUTOMASK_SETTINGS") {
        let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
        v["masks"] = serde_json::json!([]);
        base = DevelopSettings::from_json(&v).unwrap();
    }
    if let Some(p) = process {
        base = base.merged(&serde_json::json!({"process": p})).unwrap();
    }
    let req = RenderRequest::exact(rw, rh);
    let cache = StageCache::default();
    let plain = render_cached(&src, &info, &base, &req, &cache).image;
    let (w, h) = (plain.width, plain.height);
    eprintln!("rendered {w}×{h}, process {:?}", base.process.process());
    for (case, strokes) in cases.as_object().unwrap() {
        let strokes: Vec<BrushStroke> = strokes
            .as_array()
            .unwrap()
            .iter()
            .map(|s| BrushStroke {
                points: s.as_array().unwrap().iter().map(|p| Point::new(p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect(),
                size,
                feather: 50.0,
                flow: 100.0,
                auto_mask: true,
                ..BrushStroke::default()
            })
            .collect();
        let mut s = base.clone();
        s.masks.push(Mask {
            id: 1,
            name: "probe".into(),
            components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape: MaskShape::Brush { strokes: strokes.clone() } }],
            ..Mask::default()
        });
        let t = std::time::Instant::now();
        let alpha = crate::mask_alpha_cached(&src, &info, &s, &req, &StageCache::default(), 1).unwrap();
        eprintln!("{case}: mask in {:?}", t.elapsed());
        if std::env::var("CRAFT_AUTOMASK_LIVE").is_ok() {
            // painted as a UI paints it: the stroke growing a few points at a time, each render
            // from the same cache (the live path), the last compared with the one-shot render
            let live_cache = StageCache::default();
            let mut part = s.clone();
            let all = strokes[0].points.clone();
            let mut pts = Vec::new();
            for w in all.windows(2) {
                for k in 1..=6 {
                    let t = f64::from(k) / 6.0;
                    pts.push(Point::new(w[0].x + (w[1].x - w[0].x) * t, w[0].y + (w[1].y - w[0].y) * t));
                }
            }
            pts.insert(0, all[0]);
            let mut times = Vec::new();
            let mut last = None;
            for n in 1..=pts.len() {
                if let MaskShape::Brush { strokes } = &mut part.masks[0].components[0].shape {
                    strokes[0].points = pts[..n].to_vec();
                }
                let t = std::time::Instant::now();
                last = crate::mask_alpha_cached(&src, &info, &part, &req, &live_cache, 1);
                times.push(t.elapsed());
            }
            let live = last.unwrap();
            let fin = crate::mask_alpha_cached(&src, &info, &part, &req, &StageCache::default(), 1).unwrap();
            let d = live.data.iter().zip(&fin.data).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            let mut later: Vec<_> = times.iter().skip(1).copied().collect();
            later.sort();
            eprintln!(
                "{case}: live vs one-shot max difference {d:e}; {} moves, first {:?}, then median {:?}, slowest {:?}",
                times.len(),
                times.first(),
                later.get(later.len() / 2),
                later.last()
            );
        }
        ppm(&out.join(format!("{case}-overlay.ppm")), w, h, |x, y| {
            let p = plain.data[y * w + x];
            let a = alpha.data[y * w + x] * 0.55;
            [(p[0] as f32 * (1.0 - a) + 255.0 * a) as u8, (p[1] as f32 * (1.0 - a)) as u8, (p[2] as f32 * (1.0 - a)) as u8]
        });
        ppm(&out.join(format!("{case}-alpha.ppm")), w, h, |x, y| [(alpha.data[y * w + x] * 255.0) as u8; 3]);
        s.masks[0].adjust.exposure = 1.5;
        let lit = render_cached(&src, &info, &s, &req, &cache).image;
        ppm(&out.join(format!("{case}-exposure.ppm")), w, h, |x, y| {
            let p = lit.data[y * w + x];
            [p[0], p[1], p[2]]
        });
        // the reference each dab took, and the photo there
        let effective = s.effective();
        let plan = crate::plan(&src, &info, &effective, &req);
        let prep = prepare_stages(&crate::Src::Shared(&src), &info, &plan, &req, None, &mut |_| {});
        let frame = plan.frame.for_mask(true);
        for st in &strokes {
            let bd = crate::masks::brush_dabs(st, &frame, w, h);
            let cells = Cells::of(bd.r, bd.dabs.first().copied(), w, h);
            if let Some(k) = std::env::var("CRAFT_AUTOMASK_DAB").ok().and_then(|s| s.parse::<usize>().ok()) {
                let refc = reference(&bd, k, &cells, &prep.img, &prep.log_l).unwrap();
                let g = grow(&bd, k, bd.r, &cells, &prep.img, &prep.log_l, refc).unwrap();
                eprintln!("dab {k} at {:?} cell {} ref {refc:?} grid at cell ({}, {}) free {:?}", bd.dabs[k], cells.size, g.i0, g.j0, g.free);
                for j in 0..g.m {
                    let row: String = (0..g.n).map(|i| format!("{:>5.2}", g.cost[j * g.n + i].min(9.99))).collect();
                    let ls: String = (0..g.n).map(|i| format!("{:>5.2}", g.member[j * g.n + i] * g.uniform[j * g.n + i])).collect();
                    eprintln!("{row}  {ls}");
                }
            }
            if let Some((px, py)) = std::env::var("CRAFT_AUTOMASK_PIXEL")
                .ok()
                .and_then(|s| s.split_once(',').map(|(a, b)| (a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap())))
            {
                let k = Smoothing::of(bd.r, w, h);
                let all = PixRect { x0: 0, y0: 0, x1: w, y1: h };
                let sm = Smoothed::of(all, all, &prep.img, &prep.log_l, &k);
                let p = sm.at(px, py).unwrap();
                eprintln!("pixel ({px},{py}) smoothed {p:?} raw L {}", prep.log_l.data[py * w + px]);
                for kk in 0..bd.dabs.len() {
                    let d = bd.dabs[kk];
                    if Point::new(px as f64 + 0.5, py as f64 + 0.5).dist(d) > bd.r {
                        continue;
                    }
                    let refc = reference(&bd, kk, &cells, &prep.img, &prep.log_l).unwrap();
                    let g = grow(&bd, kk, bd.r, &cells, &prep.img, &prep.log_l, refc).unwrap();
                    let mb = pixel_member(&g, &cells, px, py, p);
                    let (ci, cj) = cells.of_pixel(px, py);
                    let mut near = Vec::new();
                    for j in cj.saturating_sub(2)..=cj + 2 {
                        for i in ci.saturating_sub(2)..=ci + 2 {
                            if let Some((m, f)) = g.get(i, j)
                                && m > 0.05
                            {
                                near.push(format!(
                                    "({},{}) m {m:.2} L {:.2} ch {:.3},{:.3}",
                                    i as i64 - ci as i64,
                                    j as i64 - cj as i64,
                                    f.0,
                                    f.1[0],
                                    f.1[2]
                                ));
                            }
                        }
                    }
                    eprintln!(
                        "  dab {kk} at ({:.0},{:.0}) member {mb:.2} chroma_tol {:.3} ref L {:.2} ch {:.3},{:.3}: {}",
                        d.x,
                        d.y,
                        g.chroma_tol,
                        refc.0,
                        refc.1[0],
                        refc.1[2],
                        near.join("; ")
                    );
                }
            }
            let step = (bd.dabs.len() / 12).max(1);
            for i in (0..bd.dabs.len()).step_by(step) {
                let d = bd.dabs[i];
                let j = crate::masks::dab_pixel(d, w, h);
                let r = reference(&bd, i, &cells, &prep.img, &prep.log_l);
                eprintln!("  dab {i:>3} at ({:>6.1},{:>6.1}) L {:>6.2} ref {:?}", d.x, d.y, prep.log_l.data[j], r.map(|r| (r.0, r.1)));
            }
        }
    }
    // probes: named points (fractions of the photo) → log luminance and chromaticity
    if let Ok(pts) = std::env::var("CRAFT_AUTOMASK_POINTS") {
        let effective = base.effective();
        let plan = crate::plan(&src, &info, &effective, &req);
        let prep = prepare_stages(&crate::Src::Shared(&src), &info, &plan, &req, None, &mut |_| {});
        let mut dump = Vec::with_capacity(w * h * 16);
        for (l, c) in prep.log_l.data.iter().zip(&prep.img.data) {
            let ch = crate::masks::chromaticity(*c);
            for v in [*l, ch[0], ch[1], ch[2]] {
                dump.extend_from_slice(&v.to_le_bytes());
            }
        }
        std::fs::write(out.join(format!("planes-{w}x{h}.f32")), dump).unwrap();
        let pts: serde_json::Value = serde_json::from_str(&pts).unwrap();
        for (name, p) in pts.as_object().unwrap() {
            let (x, y) = ((p[0].as_f64().unwrap() * w as f64) as usize, (p[1].as_f64().unwrap() * h as f64) as usize);
            let i = y * w + x;
            eprintln!("{name:<12} ({x},{y}) L {:>6.2} ch {:?}", prep.log_l.data[i], crate::masks::chromaticity(prep.img.data[i]));
        }
    }
}
