// SPDX-License-Identifier: MIT OR Apache-2.0
use craft_material_graph::graph::{Graph, SPECS, demo_graph_for};
use craft_material_graph::tex::Tex;

/// (seam jump, largest jump anywhere else): the mean difference across the wrap-around border, and the
/// steepest mean difference between any two neighbouring columns/rows inside the texture. Hard-edged patterns whose
/// cell border falls exactly on the seam still pass, because the same jump exists inside the texture.
fn seam(t: &Tex) -> (f32, f32) {
    let (w, h) = (t.w as i64, t.h as i64);
    let d = |a: [f32; 4], b: [f32; 4]| (0..3).map(|i| (a[i] - b[i]).abs()).fold(0.0f32, f32::max);
    let col = |x: i64| (0..h).map(|y| d(t.at(x, y), t.at(x + 1, y))).sum::<f32>() / h as f32;
    let row = |y: i64| (0..w).map(|x| d(t.at(x, y), t.at(x, y + 1))).sum::<f32>() / w as f32;
    let seam = col(w - 1).max(row(h - 1));
    let inner = (0..w - 1).map(col).chain((0..h - 1).map(row)).fold(0.0f32, f32::max);
    (seam, inner)
}

#[test]
fn every_node_is_seamless() {
    let mut bad = Vec::new();
    for sp in crate::sampled(SPECS, 0) {
        let Some(text) = demo_graph_for(sp.kind) else { continue };
        let g = Graph::parse(&text).unwrap();
        let out = match g.evaluate(Some(seam_size())) {
            Ok(o) => o,
            Err(e) => {
                bad.push(format!("{:<22} error {e}", sp.kind));
                continue;
            }
        };
        let t = &out["o"];
        let (s, i) = seam(t);
        if s > i * 1.5 + 0.01 {
            bad.push(format!("{:<22} seam {:.4} inner {:.4}", sp.kind, s, i));
        }
    }
    for b in &bad {
        eprintln!("{b}");
    }
    assert!(bad.is_empty(), "{} node(s) are not seamless", bad.len());
}

fn seam_size() -> usize {
    std::env::var("SEAM_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(128)
}

/// Wrap-equivariance: rolling the input by whole pixels must roll the output by the same amount. This is the exact
/// definition of "the filter wraps around the borders", so a filter that clamps or mirrors at the edge fails it.
#[test]
fn filters_commute_with_wrapped_shifts() {
    const N: usize = 128;
    let (dx, dy) = (37.0 / N as f32, 21.0 / N as f32);
    let filters: &[&str] = &[
        "blur radius=9",
        "blur_hq intensity=0.5",
        "directional_blur angle=0.1 length=0.1",
        "sharpen radius=3",
        "highpass radius=8",
        "normal strength=4",
        "ao radius=8",
        "curvature radius=4",
        "edge_detect",
        "emboss angle=0.1",
        "bevel distance=0.05",
        "distance distance=0.1",
        "histogram_range position=0.5",
        "histogram_scan position=0.5",
        "levels low=0.2 high=0.8",
        "invert",
        "threshold threshold=0.5",
        "slope_blur slope=@100 intensity=0.2",
        "warp warp=@100 intensity=0.2",
        "directional_warp warp=@100 angle=0.2 intensity=0.3",
        "vector_warp vector=@107 intensity=0.1",
        "swirl amount=0.0",
        "curvature_smooth",
        "edge_wear amount=0.5",
        "dirt",
        "dust",
        "height_blend bg=@100 fg=@107 opacity=0.5",
        "clamp low=0.2 high=0.8",
        "hsl",
        "quantize",
    ];
    let mut bad = Vec::new();
    for c in crate::sampled(filters.iter(), 0) {
        let kind = c.split(' ').next().unwrap();
        let mut line = format!("node 1 {c}");
        // (the nodes that read no `in` are given their inputs in the line)
        let reads_in = SPECS.iter().any(|s| s.kind == kind && (s.req.contains(&"in") || s.opt.contains(&"in")));
        if reads_in && !line.contains(" in=@") {
            line += " in=@100";
        }
        let pre = |shift: bool| {
            let base = if shift {
                "node 99 fbm scale=4 octaves=4 seed=2\nnode 100 transform in=@99 offset_x=DX offset_y=DY\n"
            } else {
                "node 100 fbm scale=4 octaves=4 seed=2\n"
            };
            let base = base.replace("DX", &dx.to_string()).replace("DY", &dy.to_string());
            format!("size {N}\n{base}node 107 normal in=@100 strength=3\n")
        };
        let ga = format!("{}{}\nnode 2 transform in=@1 offset_x={} offset_y={}\noutput o 2\n", pre(false), line, dx, dy);
        let gb = format!("{}{}\noutput o 1\n", pre(true), line);
        let (a, b) = match (Graph::parse(&ga), Graph::parse(&gb)) {
            (Ok(a), Ok(b)) => (a, b),
            (a, b) => {
                bad.push(format!("{c:<56} parse failed: {:?}", a.err().or(b.err()).map(|e| e.to_string())));
                continue;
            }
        };
        let (a, b) = match (a.evaluate(Some(N)), b.evaluate(Some(N))) {
            (Ok(a), Ok(b)) => (a, b),
            (a, b) => {
                bad.push(format!("{c:<56} evaluate failed: {:?}", a.err().or(b.err()).map(|e| e.to_string())));
                continue;
            }
        };
        let (a, b) = (&a["o"], &b["o"]);
        let mut worst = 0.0f32;
        let mut sum = 0.0f32;
        for (p, q) in a.px.iter().zip(&b.px) {
            let d = (0..3).map(|i| (p[i] - q[i]).abs()).fold(0.0f32, f32::max);
            worst = worst.max(d);
            sum += d;
        }
        let mean = sum / a.px.len() as f32;
        if std::env::var_os("SEAM_VERBOSE").is_some() {
            eprintln!("{c:<56} mean {mean:.5} worst {worst:.3}");
        }
        if mean > 0.004 {
            bad.push(format!("{c:<56} mean diff {mean:.4}  worst {worst:.3}"));
        }
    }
    for b in &bad {
        eprintln!("{b}");
    }
    assert!(bad.is_empty(), "{} filter(s) do not wrap at the borders", bad.len());
}

/// Non-square graphs: every node's demo graph evaluated at 2:1 and 1:2 must return textures of exactly that shape, and
/// the result must still tile on both axes.
#[test]
fn every_node_is_seamless_when_not_square() {
    let mut bad = Vec::new();
    for (wide, name) in [(true, "2:1"), (false, "1:2")] {
        for sp in crate::sampled(SPECS, 4) {
            let Some(text) = demo_graph_for(sp.kind) else { continue };
            let mut g = Graph::parse(&text).unwrap();
            let s = g.size;
            g.height = if wide { s / 2 } else { s * 2 };
            let size = seam_size();
            let h = if wide { size / 2 } else { size * 2 };
            let out = match g.evaluate(Some(size)) {
                Ok(o) => o,
                Err(e) => {
                    bad.push(format!("{} {name}: error {e}", sp.kind));
                    continue;
                }
            };
            let t = &out["o"];
            if t.w != size || t.h != h {
                bad.push(format!("{:<22} {name}: got {}x{}, want {size}x{h}", sp.kind, t.w, t.h));
                continue;
            }
            let (sm, inner) = seam(t);
            if sm > inner * 1.5 + 0.01 {
                bad.push(format!("{:<22} {name}: seam {sm:.4} inner {inner:.4}", sp.kind));
            }
        }
    }
    for b in &bad {
        eprintln!("{b}");
    }
    assert!(bad.is_empty(), "{} non-square problem(s)", bad.len());
}

#[test]
fn non_square_size_round_trips_through_the_file_format() {
    let g = Graph::parse("size 512 256\nnode 1 perlin scale=4\noutput o 1\n").unwrap();
    assert_eq!((g.size, g.height), (512, 256));
    assert!(g.to_text().contains("\nsize 512 256\n"));
    let sq = Graph::parse("size 512\nnode 1 perlin scale=4\noutput o 1\n").unwrap();
    assert!(sq.to_text().contains("\nsize 512\n"));
    let t = &g.evaluate(Some(128)).unwrap()["o"];
    assert_eq!((t.w, t.h), (128, 64));
}
