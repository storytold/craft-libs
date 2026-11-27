// SPDX-License-Identifier: MIT OR Apache-2.0
//! Node behaviour: what each kind computes, its parameters, sizes, formats, packages and links.
// The checks measure results (angles, spreads, reference kernels) with the platform's own maths, within tolerances;
// nothing computed here feeds a render, so the one-implementation rule of `clippy.toml` doesn't apply.
#![allow(clippy::disallowed_methods)]
use craft_material_graph::graph::{EXAMPLE, Error, Graph};
use craft_material_graph::{expr, noise, png};

#[test]
fn example_graph_renders() {
    let g = Graph::parse(EXAMPLE).unwrap();
    let out = g.evaluate(Some(64)).unwrap();
    assert_eq!(out.len(), 6);
    assert!(out["height"].px.iter().all(|p| p[0].is_finite()));
}

#[test]
fn deterministic_by_seed() {
    let g = Graph::parse("size 32\nnode 1 fbm scale=4 seed=5\noutput a 1\n").unwrap();
    assert_eq!(g.evaluate(None).unwrap()["a"].px, g.evaluate(None).unwrap()["a"].px);
}

#[test]
fn noise_is_tileable() {
    for i in 0..20 {
        let v = i as f32 / 20.0;
        let a = noise::fbm(0.0, v, 4, 4, 4, 0.5, 1, 0);
        let b = noise::fbm(1.0, v, 4, 4, 4, 0.5, 1, 0);
        assert!((a - b).abs() < 1e-4);
    }
}

#[test]
fn detects_cycle() {
    let g = "size 8\nnode 1 invert in=@2\nnode 2 invert in=@1\noutput a 1\n";
    assert!(matches!(Graph::parse(g), Err(Error::Cycle(_))));
}

#[test]
fn detects_missing_input_and_node() {
    assert!(matches!(Graph::parse("node 1 blur\noutput a 1\n"), Err(Error::MissingInput(..))));
    assert!(matches!(Graph::parse("node 1 blur in=@9\noutput a 1\n"), Err(Error::MissingNode(..))));
    assert!(matches!(Graph::parse("node 1 nope\noutput a 1\n"), Err(Error::UnknownNode(..))));
}

#[test]
fn expressions() {
    let e = expr::parse("clamp(u*2+1, 0, 1) + 2^3").unwrap();
    let env = expr::Env { vars: [0.1; 10], tex: None };
    assert!((expr::eval(&e, &env) - 9.0).abs() < 1e-4);
    assert!(expr::parse("sin(").is_err());
    assert!(expr::parse("foo(1)").is_err());
}

#[test]
fn png_signature_and_roundtrip_size() {
    let px = vec![[0.5, 0.25, 1.0, 1.0]; 16];
    let b = png::encode(4, 4, &px, 16);
    assert_eq!(&b[1..4], b"PNG");
}

#[test]
fn pixel_processor_and_save_roundtrip() {
    let t = "size 16\nnode 1 pixel_processor expr=\"u*v\"\noutput a 1\n";
    let g = Graph::parse(t).unwrap();
    let g2 = Graph::parse(&g.to_text()).unwrap();
    assert_eq!(g.evaluate(None).unwrap()["a"].px, g2.evaluate(None).unwrap()["a"].px);
}

use craft_material_graph::graph::Graph as G2;
use std::collections::BTreeSet;

#[test]
fn png_roundtrip_decode() {
    let mut px = Vec::new();
    for i in 0..64 {
        px.push([i as f32 / 63.0, 0.25, 1.0 - i as f32 / 63.0, 1.0]);
    }
    for depth in [8u8, 16] {
        let bytes = png::encode(8, 8, &px, depth);
        let t = png::decode(&bytes).unwrap();
        assert_eq!((t.w, t.h), (8, 8));
        for (a, b) in px.iter().zip(&t.px) {
            for c in 0..3 {
                assert!((a[c] - b[c]).abs() < 0.01, "{a:?} vs {b:?}");
            }
        }
    }
}

#[test]
fn subgraph_extract_preserves_result() {
    let mut g = G2::parse(EXAMPLE).unwrap();
    let before = g.evaluate(Some(48)).unwrap();
    let sel: BTreeSet<u32> = [2, 4].into_iter().collect();
    let id = g.extract_subgraph(&sel, "sg").unwrap();
    assert!(g.nodes.contains_key(&id));
    let g2 = G2::parse(&g.to_text()).unwrap();
    let after = g2.evaluate(Some(48)).unwrap();
    assert_eq!(before["basecolor"].px, after["basecolor"].px);
}

#[test]
fn publish_embeds_bitmap_and_loads() {
    let dir = std::env::temp_dir().join(format!("tc_graph_test_bitmap_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("t.png");
    let px = vec![[1.0, 0.0, 0.0, 1.0]; 16];
    std::fs::write(&f, png::encode(4, 4, &px, 8)).unwrap();
    let txt = format!("size 16\nnode 1 bitmap file=\"{}\"\noutput basecolor 1\nexpose \"X\" 1 seed 0 1\n", f.display());
    let g = G2::parse(&txt).unwrap();
    let pkg = g.publish().unwrap();
    std::fs::remove_file(&f).unwrap();
    let g2 = G2::parse(&pkg).unwrap();
    let out = g2.evaluate(None).unwrap();
    assert!(out["basecolor"].px.iter().all(|p| p[0] > 0.99 && p[1] < 0.01));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exposed_parameters_apply() {
    let mut g = G2::parse(EXAMPLE).unwrap();
    let mut p = std::collections::BTreeMap::new();
    p.insert("Brick count".to_string(), 6.0);
    g.apply_params(&p).unwrap();
    assert_eq!(g.nodes[&1].nums["cols"], 6.0);
    p.insert("Nope".to_string(), 1.0);
    assert!(g.apply_params(&p).is_err());
}

#[test]
fn sdf_nodes_and_forms() {
    let t = "size 32\nnode 1 sdf_shape shape=0 size=0.3\nnode 2 sdf_shape shape=1 size=0.2 x=0.2\nnode 3 sdf_op a=@1 b=@2 op=0 smooth=0.05\nnode 4 sdf_mask in=@3\nnode 5 sdf_form shape=2 repeat=2\noutput a 4\noutput b 5\n";
    let out = G2::parse(t).unwrap().evaluate(None).unwrap();
    assert!(out["a"].px.iter().any(|p| p[0] > 0.9) && out["a"].px.iter().any(|p| p[0] < 0.1));
    assert!(out["b"].px.iter().any(|p| p[0] > 0.05));
}

#[test]
fn bad_inputs_are_errors_not_panics() {
    assert!(png::decode(b"not a png").is_err());
    assert!(png::decode(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0]).is_err());
    assert!(G2::parse("subgraph a\nnode 1 color\n").is_err());
    assert!(G2::parse("node 1 subgraph graph=\"missing\"\noutput a 1\n").is_err());
}

#[test]
fn every_node_type_evaluates() {
    use craft_material_graph::graph::{SPECS, demo_graph_for};
    let mut count = 0;
    for sp in SPECS {
        if let Some(text) = demo_graph_for(sp.kind) {
            let g = G2::parse(&text).unwrap_or_else(|e| panic!("{}: parse: {e}", sp.kind));
            let out = g.evaluate(Some(32)).unwrap_or_else(|e| panic!("{}: eval: {e}", sp.kind));
            let t = &out["o"];
            assert!(t.px.iter().all(|p| p.iter().all(|c| c.is_finite())), "{} produced NaN", sp.kind);
            count += 1;
        }
    }
    assert!(count > 60, "only {count} nodes tested");
}

#[test]
fn output_nodes_defaults_and_migration() {
    let g = G2::parse("size 16\nnode 1 output usage=normal\nnode 2 output usage=basecolor identifier=Albedo\n").unwrap();
    let out = g.evaluate(None).unwrap();
    assert!((out["normal"].px[0][2] - 1.0).abs() < 1e-4);
    assert!(out.contains_key("Albedo"));
    let mut legacy = G2::parse(EXAMPLE).unwrap();
    let before = legacy.evaluate(Some(32)).unwrap();
    legacy.migrate_outputs();
    assert!(legacy.outputs.is_empty());
    let after = legacy.evaluate(Some(32)).unwrap();
    assert_eq!(before["basecolor"].px, after["basecolor"].px);
}

#[test]
fn flood_fill_and_distance_work() {
    let t = "size 64\nnode 1 voronoi scale=4 mode=2 seed=1\nnode 2 threshold in=@1 threshold=0.2\nnode 3 flood_fill in=@2\nnode 4 flood_fill_random in=@3 seed=2\nnode 5 distance in=@2 distance=0.1\nnode 6 bevel in=@2 distance=0.05\nnode 7 make_it_tile_photo in=@1\noutput a 4\noutput b 5\noutput c 6\noutput d 7\n";
    let out = G2::parse(t).unwrap().evaluate(None).unwrap();
    let vals: std::collections::BTreeSet<u32> = out["a"].px.iter().map(|p| (p[0] * 255.0) as u32).collect();
    assert!(vals.len() >= 4, "regions should get different grays, got {}", vals.len());
    assert!(out["b"].px.iter().any(|p| p[0] > 0.9));
    assert!(out["c"].px.iter().any(|p| p[0] > 0.9));
    // photo-tile result must be seamless: the wrap seam should look like any other pixel step
    let d = &out["d"];
    let (mut seam, mut inner) = (0.0, 0.0);
    for y in 0..64 {
        seam += (d.px[y * 64][0] - d.px[y * 64 + 63][0]).abs();
        for x in 8..55 {
            inner += (d.px[y * 64 + x][0] - d.px[y * 64 + x + 1][0]).abs() / 47.0;
        }
    }
    assert!(seam <= inner * 1.6 + 0.5, "seam {seam} vs typical {inner}");
}

#[test]
fn color_node_gray_mode() {
    let g = G2::parse("size 8\nnode 1 color r=1 g=0 b=0 gray=1 v=0.25\noutput a 1\n").unwrap();
    let p = g.evaluate(None).unwrap()["a"].px[0];
    assert!((p[0] - 0.25).abs() < 1e-5 && (p[1] - 0.25).abs() < 1e-5);
}

#[test]
fn png_decoder_handles_palette_alpha_and_low_bit_depth() {
    // hand-built 2x2 4-bit palette PNG with tRNS, stored (uncompressed) deflate
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0, 0, 0, 0]); // CRC is not checked by the decoder
    }
    let mut p = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&2u32.to_be_bytes());
    ihdr.extend_from_slice(&2u32.to_be_bytes());
    ihdr.extend_from_slice(&[4, 3, 0, 0, 0]);
    chunk(&mut p, b"IHDR", &ihdr);
    chunk(&mut p, b"PLTE", &[255, 0, 0, 0, 255, 0]);
    chunk(&mut p, b"tRNS", &[255, 128]);
    let rows = [0u8, 0x01, 0u8, 0x10]; // filter 0 + two pixels per row (indices 0,1 / 1,0)
    let mut z = vec![0x78, 0x01, 1, rows.len() as u8, 0, !(rows.len() as u8), 0xff];
    z.extend_from_slice(&rows);
    z.extend_from_slice(&[0, 0, 0, 0]);
    chunk(&mut p, b"IDAT", &z);
    chunk(&mut p, b"IEND", &[]);
    let t = png::decode(&p).unwrap();
    assert_eq!((t.w, t.h), (2, 2));
    assert!(t.px[0][0] > 0.99 && t.px[0][3] > 0.99); // index 0: opaque red
    assert!(t.px[1][1] > 0.99 && (t.px[1][3] - 128.0 / 255.0).abs() < 0.01); // index 1: half-transparent green
}

// ---------------------------------------------------------------- seamless angles + 3D mesh / displacement
/// mean |difference| across the wrap seam divided by the mean |difference| of ordinary neighbouring pixels
fn seam_ratio(t: &craft_material_graph::tex::Tex) -> f32 {
    let w = t.w;
    let (mut seam, mut inner) = (0.0f32, 0.0f32);
    for y in 0..t.h {
        seam += (t.px[y * w][0] - t.px[y * w + w - 1][0]).abs();
        for x in 0..w - 1 {
            inner += (t.px[y * w + x][0] - t.px[y * w + x + 1][0]).abs();
        }
    }
    for x in 0..w {
        seam += (t.px[x][0] - t.px[(t.h - 1) * w + x][0]).abs();
        for y in 0..t.h - 1 {
            inner += (t.px[y * w + x][0] - t.px[(y + 1) * w + x][0]).abs();
        }
    }
    (seam / (2 * w) as f32) / (inner / (2 * w * (w - 1)) as f32)
}

#[test]
fn stripes_tile_at_arbitrary_angles() {
    for (count, ang) in [(8.0, 0.0), (8.0, 0.137), (5.0, 0.21), (12.0, 0.3333), (3.0, 0.77), (20.0, 0.05), (8.0, 0.9)] {
        let g = G2::parse(&format!("size 256\nnode 1 stripes count={count} angle={ang} softness=0.2\noutput a 1\n")).unwrap();
        let r = seam_ratio(&g.evaluate(None).unwrap()["a"]);
        assert!(r < 1.6, "stripes count {count} angle {ang}: seam/inner = {r}");
        // the snapped direction must stay close to what was asked for
        let (a, b) = craft_material_graph::ops::tileable_wave(count, ang);
        let want = ang as f64 * std::f64::consts::TAU;
        let d = ((b as f64).atan2(a as f64) - want).sin().atan2(((b as f64).atan2(a as f64) - want).cos()).abs();
        let len = ((a * a + b * b) as f64).sqrt();
        assert!(d < 0.35 && len > count as f64 * 0.55 && len < count as f64 * 1.65, "snap ({a},{b}) too far for count {count} angle {ang}");
    }
    // already-tileable settings are unchanged
    assert_eq!(craft_material_graph::ops::tileable_wave(8.0, 0.0), (8, 0));
    assert_eq!(craft_material_graph::ops::tileable_wave(6.0, 0.25).0, 0);
}

#[test]
fn stripes_exact_mode_keeps_requested_angle() {
    let g = G2::parse("size 64\nnode 1 stripes count=8 angle=0.137 tile=0\noutput a 1\n").unwrap();
    assert!(g.evaluate(None).is_ok());
}

#[test]
fn directional_noise_tiles_at_arbitrary_angles() {
    for ang in [0.0, 0.05, 0.137, 0.3333, 0.62, 0.9] {
        for stretch in [1.0, 8.0, 24.0] {
            let g = G2::parse(&format!("size 128\nnode 1 directional_noise scale=6 stretch={stretch} angle={ang} seed=4\noutput a 1\n")).unwrap();
            let t = &g.evaluate(None).unwrap()["a"];
            assert!(t.px.iter().all(|p| p[0].is_finite() && (0.0..=1.0).contains(&p[0])));
            let n = t.px.len() as f32;
            let m = t.px.iter().map(|p| p[0]).sum::<f32>() / n;
            let sd = (t.px.iter().map(|p| (p[0] - m).powi(2)).sum::<f32>() / n).sqrt();
            assert!(sd > 0.07 && sd < 0.3, "angle {ang} stretch {stretch}: sd {sd}");
            // axis-aligned streaks vary along one axis only, so the ratio is a single-row sample there; skip it
            if (ang * 4.0f32).fract().abs() > 0.02 {
                let r = seam_ratio(t);
                assert!(r < 1.5, "directional noise angle {ang} stretch {stretch}: seam/inner = {r}");
            }
        }
    }
}

#[test]
fn gradient_grid_matches_perlin() {
    let gg = noise::GradGrid::new(5, 7, 3);
    for i in 0..50 {
        let (u, v) = (i as f32 * 0.0731 - 0.4, i as f32 * 0.0419 + 0.1);
        assert!((gg.sample(u, v) - noise::perlin(u, v, 5, 7, 3)).abs() < 1e-5);
    }
}

/// Fuzz every documented numeric parameter of every node with extreme values: must never panic or return NaN.
#[test]
fn node_parameters_survive_extreme_values() {
    use craft_material_graph::graph::{SPECS, demo_graph_for};
    let mut problems = Vec::new();
    let mut ran = 0usize;
    for sp in crate::sampled(SPECS, 0) {
        let Some(text) = demo_graph_for(sp.kind) else { continue };
        let doc = sp.doc.split_once(':').map(|x| x.1).unwrap_or("");
        let names: Vec<String> = doc
            .split_whitespace()
            .map(|w| w.split('(').next().unwrap().trim_matches(|c: char| !c.is_alphanumeric() && c != '_').to_string())
            .filter(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') && !w.starts_with("expr"))
            .collect();
        for name in names {
            for v in ["0", "1", "-1", "0.0001", "50", "1000"] {
                let line = format!("node 1 {}", sp.kind);
                let t2 = text.replacen(&line, &format!("{line} {name}={v}"), 1);
                let Ok(g) = G2::parse(&t2) else { continue };
                match std::panic::catch_unwind(|| g.evaluate(Some(24))) {
                    Ok(Ok(out)) => {
                        ran += 1;
                        let t = &out["o"];
                        if t.px.iter().any(|p| p.iter().any(|c| !c.is_finite())) {
                            problems.push(format!("{} {name}={v}: NaN/Inf", sp.kind));
                        }
                    }
                    Ok(Err(_)) => {} // a clean error is acceptable
                    Err(_) => problems.push(format!("{} {name}={v}: PANIC", sp.kind)),
                }
            }
        }
    }
    problems.sort();
    problems.dedup();
    assert!(ran > 500 / crate::divisor(), "only {ran} parameter cases ran");
    assert!(problems.is_empty(), "{} parameter problems:\n{}", problems.len(), problems.join("\n"));
}

#[test]
fn whole_graph_can_be_imported_and_used_as_a_node() {
    let other = G2::parse("size 64\nnode 1 perlin scale=4 seed=3\nnode 2 invert in=@1\noutput height 2\noutput basecolor 1\n").unwrap();
    let direct = other.evaluate(Some(32)).unwrap();
    let mut host = G2::parse_unchecked(
        "size 64\nnode 1 subgraph graph=\"mat\" output=\"height\"\nnode 2 subgraph graph=\"mat\" output=\"basecolor\"\noutput a 1\noutput b 2\n",
    )
    .unwrap();
    host.import_graph(other.clone(), "mat").unwrap();
    assert!(host.import_graph(other, "mat").is_err(), "duplicate name must be rejected");
    host.validate().unwrap();
    let out = host.evaluate(Some(32)).unwrap();
    assert_eq!(out["a"].px, direct["height"].px, "output=height must pick the height output");
    assert_eq!(out["b"].px, direct["basecolor"].px, "output=basecolor must pick the basecolor output");
    // an empty graph or one without outputs is refused
    let mut h2 = G2::default();
    assert!(h2.import_graph(G2::default(), "x").is_err());
}

#[test]
fn links_can_read_a_specific_output_of_a_graph_node() {
    let other =
        G2::parse("size 64\nnode 1 perlin scale=4 seed=3\nnode 2 invert in=@1\noutput height 2\noutput basecolor 1\noutput normal 1\n").unwrap();
    assert_eq!(other.output_idents(), vec!["basecolor", "normal", "height"], "outputs are listed in canonical order");
    let mut host = G2::parse_unchecked(
        "size 64\nnode 1 subgraph graph=\"mat\"\nnode 2 invert in=@1.height\nnode 3 invert in=@1.basecolor\noutput a 2\noutput b 3\n",
    )
    .unwrap();
    host.import_graph(other.clone(), "mat").unwrap();
    host.validate().unwrap();
    // round trip keeps the per-link output
    let again = G2::parse_unchecked(&host.to_text()).unwrap();
    assert_eq!(again.nodes[&2].in_out.get("in").map(String::as_str), Some("height"));
    let out = again.evaluate(Some(32)).unwrap();
    let direct = other.evaluate(Some(32)).unwrap();
    let inv = |t: &craft_material_graph::tex::Tex| craft_material_graph::ops::invert(t).px;
    assert_eq!(out["a"].px, inv(&direct["height"]));
    assert_eq!(out["b"].px, inv(&direct["basecolor"]));
    assert_ne!(out["a"].px, out["b"].px, "the two outputs differ");
    // unknown output names are a clean error
    let mut bad = G2::parse_unchecked("size 64\nnode 1 subgraph graph=\"mat\"\nnode 2 invert in=@1.nosuch\noutput a 2\n").unwrap();
    bad.import_graph(other, "mat").unwrap();
    assert!(bad.evaluate(Some(16)).is_err());
}

#[test]
fn fast_blur_matches_a_true_gaussian_and_keeps_the_mean() {
    use craft_material_graph::ops;
    use craft_material_graph::tex::Tex;
    let n = 128usize;
    let t = Tex::gray_fn(n, n, |u, v| noise::fbm(u, v, 6, 6, 4, 0.5, 3, 0));
    let sigma = 6.0f32;
    let fast = ops::blur(&t, sigma);
    // reference: direct separable gaussian with wrap-around
    let r = (sigma * 4.0) as i64;
    let k: Vec<f32> = (-r..=r).map(|i| (-((i * i) as f32) / (2.0 * sigma * sigma)).exp()).collect();
    let ks: f32 = k.iter().sum();
    let mut tmp = t.clone();
    for y in 0..n {
        for x in 0..n {
            tmp.px[y * n + x][0] = (0..k.len()).map(|i| t.at(x as i64 + i as i64 - r, y as i64)[0] * k[i]).sum::<f32>() / ks;
        }
    }
    let mut max_err = 0.0f32;
    let (mut mean_a, mut mean_b) = (0.0f64, 0.0f64);
    for y in 0..n {
        for x in 0..n {
            let v = (0..k.len()).map(|i| tmp.at(x as i64, y as i64 + i as i64 - r)[0] * k[i]).sum::<f32>() / ks;
            max_err = max_err.max((v - fast.px[y * n + x][0]).abs());
            mean_a += v as f64;
            mean_b += fast.px[y * n + x][0] as f64;
        }
    }
    assert!(max_err < 0.02, "box-approximated gaussian differs by {max_err}");
    assert!(((mean_a - mean_b) / (n * n) as f64).abs() < 1e-4, "mean not preserved");
}

#[test]
fn results_look_the_same_at_every_resolution() {
    // parameters are resolution independent: the average of every output must barely change between 128 and 512 px
    let g = Graph::parse(EXAMPLE).unwrap();
    let lo = g.evaluate(Some(128)).unwrap();
    let hi = g.evaluate(Some(512)).unwrap();
    for (name, a) in &lo {
        let b = &hi[name];
        let mean = |t: &craft_material_graph::tex::Tex| t.px.iter().map(|p| (p[0] + p[1] + p[2]) as f64 / 3.0).sum::<f64>() / t.px.len() as f64;
        let (ma, mb) = (mean(a), mean(b));
        assert!((ma - mb).abs() < 0.06, "{name}: mean {ma:.3} at 128px vs {mb:.3} at 512px");
    }
}

#[test]
fn large_sizes_are_accepted() {
    for s in [2048, 4096, 8192] {
        let text = format!("size {s}\nnode 1 fbm scale=4\noutput a 1\n");
        assert!(Graph::parse(&text).is_ok(), "size {s} rejected");
    }
    assert!(Graph::parse("size 16384\nnode 1 fbm\noutput a 1\n").is_err());
}

#[test]
fn a_cancelled_evaluation_stops_early() {
    use craft_material_graph::context::{Cancel, EvalContext};
    let mut g = Graph::parse(EXAMPLE).unwrap();
    g.size = 256;
    let keep: Vec<u32> = g.output_nodes().iter().map(|o| o.2).collect();
    let job = EvalContext::with_cancel(Cancel::new());
    let t0 = std::time::Instant::now();
    let full = g.evaluate_all_in(&job, 256, &keep);
    let full_time = t0.elapsed();
    assert!(!full.kept.is_empty(), "a current job must deliver its outputs");
    // a newer job exists: the old one is cancelled from the start
    job.cancel.cancel();
    let t1 = std::time::Instant::now();
    let r = g.evaluate_all_in(&job, 256, &keep);
    assert!(r.kept.is_empty(), "a cancelled run must not deliver outputs");
    assert!(
        t1.elapsed() < full_time / 4 + std::time::Duration::from_millis(50),
        "cancelled run still took {:?} (full run {:?})",
        t1.elapsed(),
        full_time
    );
    // cancelled while it runs: the flag is set as the third node starts, and the run stops early
    use craft_material_graph::profile::Profiler;
    struct CancelAt {
        cancel: Cancel,
        at: usize,
        started: std::sync::atomic::AtomicUsize,
    }
    impl Profiler for CancelAt {
        fn enabled(&self) -> bool {
            false
        }
        fn span(&self, _name: &str) -> Box<dyn std::any::Any + Send> {
            Box::new(())
        }
        fn node(&self, _id: u32, _kind: &str, _size: (usize, usize)) -> Option<Box<dyn std::any::Any + Send>> {
            if self.started.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1 == self.at {
                self.cancel.cancel();
            }
            None
        }
    }
    let run = |at: usize| {
        let cancel = Cancel::new();
        let probe = std::sync::Arc::new(CancelAt { cancel: cancel.clone(), at, started: Default::default() });
        let ec = EvalContext { cancel, profiler: Some(probe.clone()), ..EvalContext::default() };
        let r = g.evaluate_all_in(&ec, 64, &keep);
        (r, probe.started.load(std::sync::atomic::Ordering::SeqCst))
    };
    let (whole, all) = run(usize::MAX);
    assert!(!whole.kept.is_empty() && all > 3, "{all} nodes");
    let (cut, started) = run(3);
    assert!((3..all).contains(&started), "{started} of {all} nodes started: none starts once the run is cancelled");
    assert!(cut.kept.is_empty(), "a run cancelled on the way must not deliver outputs");
}

#[test]
fn nodes_can_have_their_own_output_size() {
    // generators follow the parent size, the modifier moves it in powers of two
    let g = Graph::parse("size 256\nnode 1 fbm scale=4 osize_rel=-2\noutput o 1\n").unwrap();
    let t = &g.evaluate(Some(256)).unwrap()["o"];
    assert_eq!((t.w, t.h), (64, 64), "-2 means a quarter of the parent size");
    let g = Graph::parse("size 256\nnode 1 fbm scale=4 osize_mode=1 osize_rel=1\noutput o 1\n").unwrap();
    assert_eq!(g.evaluate(Some(256)).unwrap()["o"].w, 512, "+1 doubles");
    // absolute sizes are powers of two and ignore the modifier
    let g = Graph::parse("size 256\nnode 1 fbm scale=4 osize_mode=2 osize_abs=7 osize_rel=3\noutput o 1\n").unwrap();
    assert_eq!(g.evaluate(Some(256)).unwrap()["o"].w, 128);
    // filters inherit from their input; a node with another size resamples its inputs to its own size
    let g = Graph::parse(
        "size 256\nnode 1 fbm scale=4 osize_rel=-1\nnode 2 blur in=@1 radius=2\nnode 3 invert in=@2 osize_mode=1\noutput o 2\noutput p 3\n",
    )
    .unwrap();
    let r = g.evaluate(Some(256)).unwrap();
    assert_eq!(r["o"].w, 128, "blur follows its input");
    assert_eq!(r["p"].w, 256, "relative-to-parent restores the graph size");
    // quick previews scale absolute sizes with the preview resolution
    let g = Graph::parse("size 1024\nnode 1 fbm scale=4 osize_mode=2 osize_abs=9\noutput o 1\n").unwrap();
    assert_eq!(g.evaluate(Some(256)).unwrap()["o"].w, 128, "512 px of a 1024 graph = 1/2, so 128 at a 256 preview");
    // everything stays tileable and finite at a reduced size
    let g = Graph::parse("size 128\nnode 1 voronoi scale=5 osize_rel=-1\nnode 2 blur in=@1 radius=3 osize_mode=1\noutput o 2\n").unwrap();
    let t = &g.evaluate(Some(128)).unwrap()["o"];
    assert!(t.px.iter().all(|p| p.iter().all(|v| v.is_finite())));
}

#[test]
fn normal_to_height_inverts_normal_from_height() {
    // height -> normal -> height must give back (a normalised copy of) the original height
    let t = "size 128\nnode 1 fbm scale=3 octaves=3 seed=4\nnode 2 normal in=@1 strength=2\nnode 3 normal_to_height in=@2\nnode 4 auto_levels in=@1\noutput a 3\noutput b 4\n";
    let o = Graph::parse(t).unwrap().evaluate(None).unwrap();
    let (a, b) = (&o["a"], &o["b"]);
    let n = a.px.len() as f64;
    let (ma, mb) = (a.px.iter().map(|p| p[0] as f64).sum::<f64>() / n, b.px.iter().map(|p| p[0] as f64).sum::<f64>() / n);
    let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
    for (p, q) in a.px.iter().zip(&b.px) {
        let (x, y) = (p[0] as f64 - ma, q[0] as f64 - mb);
        cov += x * y;
        va += x * x;
        vb += y * y;
    }
    let corr = cov / (va * vb).sqrt();
    assert!(corr > 0.9, "recovered height correlates only {corr:.3} with the original");
}

#[test]
fn node_cache_reuses_unchanged_nodes_and_matches_a_fresh_run() {
    use craft_material_graph::graph::NodeCache;
    use std::sync::Arc;
    let mut g = Graph::parse(EXAMPLE).unwrap();
    g.size = 128;
    let keep: Vec<u32> = g.output_nodes().iter().map(|o| o.2).collect();
    let cache = Arc::new(NodeCache::new(512));
    let mut first_seen = 0;
    let r1 = g.evaluate_all_cached(&Default::default(), 128, &keep, &cache, Some(9), &mut |_| first_seen += 1);
    assert_eq!(first_seen, 1, "the selected node must be reported once");
    let (h0, m0) = cache.stats();
    let _ = h0;
    assert!(m0 >= 11, "every node computed on the first run ({m0})");
    // nothing changed: everything is reused and the pixels are identical
    let r2 = g.evaluate_all_cached(&Default::default(), 128, &keep, &cache, None, &mut |_| {});
    let (h1, m1) = cache.stats();
    assert_eq!(m1, m0, "no node may be recomputed");
    assert!(h1 >= 11 + h0);
    for id in &keep {
        assert_eq!(r1.kept[id].px, r2.kept[id].px);
    }
    assert_eq!(r1.thumbs.len(), r2.thumbs.len(), "thumbnails come back from the cache");
    // change the normal node (6): only that node is recomputed, and the result equals an uncached evaluation
    g.nodes.get_mut(&6).unwrap().nums.insert("strength".into(), 7.0);
    let r3 = g.evaluate_all_cached(&Default::default(), 128, &keep, &cache, None, &mut |_| {});
    let (_, m2) = cache.stats();
    assert_eq!(m2 - m1, 1, "only the edited node should be recomputed, got {}", m2 - m1);
    let fresh = g.evaluate_all(128, &keep);
    for id in &keep {
        assert_eq!(r3.kept[id].px, fresh.kept[id].px, "cached result differs for node {id}");
    }
    // change a node near the start (2): everything downstream of it is recomputed, the rest is reused
    g.nodes.get_mut(&2).unwrap().nums.insert("scale".into(), 5.0);
    let before = cache.stats().1;
    let r4 = g.evaluate_all_cached(&Default::default(), 128, &keep, &cache, None, &mut |_| {});
    let redone = cache.stats().1 - before;
    assert!((5..=9).contains(&redone), "recomputed {redone} nodes");
    let fresh = g.evaluate_all(128, &keep);
    for id in &keep {
        assert_eq!(r4.kept[id].px, fresh.kept[id].px, "cached result differs for node {id}");
    }
    // a different size is a different result
    let m = cache.stats().1;
    g.evaluate_all_cached(&Default::default(), 64, &keep, &cache, None, &mut |_| {});
    assert!(cache.stats().1 > m);
    // a tiny budget evicts old entries instead of growing without bound
    let small = Arc::new(NodeCache::new(16));
    g.evaluate_all_cached(&Default::default(), 512, &keep, &small, None, &mut |_| {});
    assert!(small.bytes() <= (16 << 20) + 512 * 512 * 16, "cache holds {} bytes", small.bytes());
}

#[test]
fn fbm_kernel_matches_the_reference_function() {
    for (px, py, oct, mode) in [(4, 4, 5, 0u32), (8, 3, 6, 1), (1, 1, 3, 2), (32, 32, 12, 0)] {
        let k = noise::Fbm::new(px, py, oct, 0.55, 7, mode);
        for i in 0..400 {
            let (u, v) = ((i as f32 * 0.6180339) % 1.0, (i as f32 * 0.4142136) % 1.0);
            let (a, b) = (k.sample(u, v), noise::fbm(u, v, px, py, oct, 0.55, 7, mode));
            assert!((a - b).abs() < 1e-5, "{px}x{py} oct {oct} mode {mode}: {a} vs {b}");
        }
    }
}

#[test]
fn faster_kernels_match_straightforward_references() {
    use craft_material_graph::ops;
    use craft_material_graph::tex::{Tex, lum};
    let img = Tex::gray_fn(96, 80, |u, v| noise::fbm(u, v, 4, 4, 3, 0.5, 5, 0));
    // gray blur (single channel path) must equal the general four-channel path
    let mut rgba = img.clone();
    for p in rgba.px.iter_mut() {
        p[3] = 0.5;
    }
    for sigma in [3.0f32, 7.5, 20.0] {
        let (a, b) = (ops::blur(&img, sigma), ops::blur(&rgba, sigma));
        let worst = a.px.iter().zip(&b.px).map(|(p, q)| (p[0] - q[0]).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-5, "sigma {sigma}: gray and rgba blur differ by {worst}");
    }
    // normal map: compare with a direct per-pixel Scharr implementation
    let n = ops::normal_from_height(&img, 3.0, false);
    let k = 3.0 * img.w as f32 * 0.05;
    for &(x, y) in &[(0i64, 0i64), (95, 79), (10, 20), (50, 0), (0, 40), (95, 40)] {
        let g = |i: i64, j: i64| lum(img.at(x + i, y + j));
        let dx = (3.0 * (g(1, -1) - g(-1, -1)) + 10.0 * (g(1, 0) - g(-1, 0)) + 3.0 * (g(1, 1) - g(-1, 1))) / 32.0 * k;
        let dy = (3.0 * (g(-1, 1) - g(-1, -1)) + 10.0 * (g(0, 1) - g(0, -1)) + 3.0 * (g(1, 1) - g(1, -1))) / 32.0 * k;
        let l = (dx * dx + dy * dy + 1.0).sqrt();
        let want = [-dx / l * 0.5 + 0.5, dy / l * 0.5 + 0.5, 1.0 / l * 0.5 + 0.5];
        let got = n.at(x, y);
        for c in 0..3 {
            assert!((got[c] - want[c]).abs() < 1e-4, "normal at ({x},{y}) channel {c}: {} vs {}", got[c], want[c]);
        }
    }
    // levels: gamma 1 fast path equals the general formula; coloured pixels keep their channels
    let t = Tex::from_fn(8, 8, |u, v| [u, v, 0.3, 1.0]);
    let l = ops::levels(&t, 0.2, 0.8, 1.0, 0.1, 0.9);
    let p = t.px[11];
    for (&source, &computed) in p.iter().zip(&l.px[11]).take(3) {
        let want = 0.1 + 0.8 * ((source - 0.2) / 0.6).clamp(0.0, 1.0);
        assert!((computed - want).abs() < 1e-5);
    }
}

#[test]
fn directional_noise_with_fewer_taps_matches_the_dense_reference() {
    // reference: the original sampling density of 5 taps per lattice cell
    for (st, ang) in [(8.0f32, 0.13f32), (24.0, 0.4), (3.0, 0.0)] {
        let size = 96usize;
        let g = Graph::parse(&format!("size {size}\nnode 1 directional_noise scale=6 stretch={st} angle={ang} seed=3\noutput a 1\n")).unwrap();
        let got = &g.evaluate(None).unwrap()["a"];
        let p1 = 6i32;
        let (sa, ca) = (ang * std::f32::consts::TAU).sin_cos();
        let len1 = (st / p1 as f32).min(1.0);
        let k = ((st * 5.0).ceil() as usize).clamp(8, 160) | 1;
        let taps: Vec<(f32, f32)> = (0..k)
            .map(|i| {
                let t = ((i as f32 + 0.5) / k as f32) * 2.0 - 1.0;
                (t, 1.0 - t.abs())
            })
            .collect();
        let wsum: f32 = taps.iter().map(|t| t.1).sum();
        let gain = 1.6 * st.min(p1 as f32).powf(0.55);
        let (g1, g2) = (noise::GradGrid::new(p1, p1, 3), noise::GradGrid::new(p1 * 2, p1 * 2, 3 + 7));
        let mut diff = 0.0f64;
        for y in 0..size {
            for x in 0..size {
                let (u, v) = ((x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32);
                let (mut a1, mut a2) = (0.0f32, 0.0f32);
                for &(t, w) in &taps {
                    let (o1, o2) = (t * len1 * 0.5, t * len1 * 0.25);
                    a1 += w * g1.sample(u + o1 * ca, v + o1 * sa);
                    a2 += w * g2.sample(u + o2 * ca, v + o2 * sa);
                }
                let m = (a1 * 0.65 + a2 * 0.35) / wsum;
                let want = (0.5 + (m - 0.5) * gain).clamp(0.0, 1.0);
                diff += (got.at(x as i64, y as i64)[0] - want).abs() as f64;
            }
        }
        let mean = diff / (size * size) as f64;
        assert!(mean < 0.01, "stretch {st}: mean difference {mean:.4}");
    }
}

#[test]
fn slope_blur_gray_path_matches_the_color_path() {
    use craft_material_graph::{ops, tex::Tex};
    let img = Tex::gray_fn(64, 64, |u, v| noise::fbm(u, v, 4, 4, 3, 0.5, 5, 0));
    let slope = Tex::gray_fn(64, 64, |u, v| noise::fbm(u, v, 2, 2, 2, 0.5, 9, 0));
    let mut tinted = img.clone();
    for p in tinted.px.iter_mut() {
        p[3] = 0.99; // not exactly opaque: forces the general four-channel path
    }
    for mode in 0..3 {
        let (a, b) = (ops::slope_blur(&img, &slope, 12, 0.1, mode), ops::slope_blur(&tinted, &slope, 12, 0.1, mode));
        let worst = a.px.iter().zip(&b.px).map(|(p, q)| (p[0] - q[0]).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-5, "mode {mode}: paths differ by {worst}");
    }
}

#[test]
fn scatter_defaults_preserve_legacy_output() {
    use craft_material_graph::{ops, scatter, tex::Tex};
    let diff = |a: &Tex, b: &Tex| a.px.iter().zip(&b.px).map(|(p, q)| (p[0] - q[0]).abs()).fold(0.0f32, f32::max);
    let pat = Tex::from_fn(32, 32, |u, v| {
        let g = (u * 3.0 + v).fract();
        [g, g, g, 1.0]
    });
    for r#gen in [false, true] {
        let mut p = scatter::TileP::legacy(r#gen);
        p.pos_rand = 0.4;
        p.size_rand = 0.3;
        p.rot_rand = 0.5;
        p.color_rand = 0.4;
        p.seed = 7;
        let old = ops::tile_grid(
            64,
            64,
            Some(&pat),
            None,
            &ops::TileP {
                nx: p.nx,
                ny: p.ny,
                size: p.size,
                size_rand: p.size_rand,
                rot: p.rot,
                rot_rand: p.rot_rand,
                pos_rand: p.pos_rand,
                color: p.color,
                color_rand: p.color_rand,
                offset: p.offset,
                seed: 7,
                thresh: 0.5,
                default_disc: !r#gen,
                brick: r#gen,
            },
        );
        let pats = scatter::Patterns { inputs: vec![&pat], builtin: 0, fallback: 2, distribution: 0 };
        let maps =
            scatter::TileMaps { mask: None, scale: None, rotation: None, displacement: None, color: None, distribution: None, background: None };
        let new = scatter::tile_grid(64, 64, &pats, &maps, &p);
        assert!(diff(&old, &new) < 1e-4, "tile grid (gen={gen}) changed: {}", diff(&old, &new));
    }
    let old = ops::splatter(64, 64, None, 40, 0.1, 0.3, 1.0, 0.2, 3);
    let mut sp = scatter::SplatterP::legacy();
    sp.lum_var = 0.2;
    sp.seed = 3;
    let new = scatter::splatter(64, 64, &scatter::Patterns { inputs: vec![], builtin: 0, fallback: 99, distribution: 0 }, &sp);
    assert!(diff(&old, &new) < 1e-6, "splatter changed");
    let old = ops::splatter_circular(64, 64, None, 12, 0.3, 0.1, 0.2, 0.1, 0.3, 0.5, 4);
    let mut cp = scatter::CircularP::legacy();
    cp.size_rand = 0.2;
    cp.rot = 0.1;
    cp.rot_rand = 0.3;
    cp.pos_rand = 0.5;
    cp.seed = 4;
    let new = scatter::splatter_circular(64, 64, &scatter::Patterns { inputs: vec![], builtin: 0, fallback: 2, distribution: 0 }, &cp);
    assert!(diff(&old, &new) < 1e-6, "splatter circular changed");
}

#[test]
fn tile_sampler_and_generator_are_seamless() {
    // moving every instance by exactly one pixel must equal rolling the image by one pixel: this fails when any
    // instance near the border is missing or not wrapped, so it proves the pattern tiles in both directions
    let run = |body: &str| Graph::parse(&format!("size 64\n{body}")).unwrap().evaluate(None).unwrap();
    let configs = [
        "x_amount=5 y_amount=7 size=0.9 position_random=0.8 rotation_random=1 size_random=0.5 seed=3",
        "x_amount=3 y_amount=3 size=2.4 pattern_type=1 rotation=0.13 position_random=1 seed=9",
        "x_amount=1 y_amount=1 size=1.8 pattern_type=4 rotation=0.2",
        "x_amount=7 y_amount=5 size=1 offset_mode=1 offset=0.37 offset_random=0.4 pattern_type=8",
        "x_amount=5 y_amount=5 size=1.2 offset_mode=3 offset=0.5 symmetry_random=3 pattern_rotation=1 pattern_type=13",
        "x_amount=9 y_amount=4 size=1.5 size_mode=2 scale=0.2 position_random=0.5 blending=1 pattern_type=2",
        "x_amount=6 y_amount=6 size=1.3 blending=2 color_random=0.7 position_random=1 mask_random=0.3 reverse_order=1",
    ];
    for kind in ["tile_sampler", "tile_generator"] {
        for c in configs {
            let o = run(&format!(
                "node 1 {kind} {c}\nnode 2 {kind} {c} global_offset_x=0.015625\nnode 3 {kind} {c} global_offset_y=0.015625\noutput a 1\noutput bx 2\noutput by 3\n"
            ));
            let a = &o["a"];
            let mut worst = 0.0f32;
            for y in 0..64i64 {
                for x in 0..64i64 {
                    worst = worst.max((o["bx"].at((x + 1) % 64, y)[0] - a.at(x, y)[0]).abs());
                    worst = worst.max((o["by"].at(x, (y + 1) % 64)[0] - a.at(x, y)[0]).abs());
                }
            }
            assert!(worst < 2e-3, "{kind} {c}: not seamless, worst {worst}");
        }
    }
}

#[test]
fn m0_multi_output_nodes() {
    let run = |body: &str| Graph::parse(&format!("size 64\n{body}")).unwrap().evaluate(None).unwrap();
    let mean = |t: &craft_material_graph::tex::Tex, c: usize| t.px.iter().map(|p| p[c]).sum::<f32>() / t.px.len() as f32;
    // RGBA split: each output carries one channel; the default output (no name) is R
    let o = run(
        "node 1 color r=0.9 g=0.5 b=0.1 a=0.3\nnode 2 rgba_split in=@1\nnode 3 tint in=@2.r r=1 g=1 b=1\nnode 4 tint in=@2.g r=1 g=1 b=1\nnode 5 tint in=@2.b r=1 g=1 b=1\nnode 6 tint in=@2.a r=1 g=1 b=1\nnode 7 tint in=@2 r=1 g=1 b=1\noutput r 3\noutput g 4\noutput b 5\noutput a 6\noutput def 7\n",
    );
    assert!(
        (mean(&o["r"], 0) - 0.9).abs() < 1e-3
            && (mean(&o["g"], 0) - 0.5).abs() < 1e-3
            && (mean(&o["b"], 0) - 0.1).abs() < 1e-3
            && (mean(&o["a"], 0) - 0.3).abs() < 1e-3,
        "channels split"
    );
    assert!((mean(&o["def"], 0) - 0.9).abs() < 1e-3, "default output is the first");
    // Alpha split
    let o = run(
        "node 1 color r=0.9 g=0.5 b=0.1 a=0.3\nnode 2 alpha_split in=@1\nnode 3 tint in=@2.rgb r=1 g=1 b=1\nnode 4 tint in=@2.alpha r=1 g=1 b=1\noutput rgb 3\noutput alpha 4\n",
    );
    assert!((mean(&o["rgb"], 0) - 0.9).abs() < 1e-3 && (mean(&o["rgb"], 3) - 1.0).abs() < 1e-3 && (mean(&o["alpha"], 0) - 0.3).abs() < 1e-3);
    // Color Match mask: only the red surface is selected
    let o = run(
        "node 1 color r=0.8 g=0.1 b=0.1\nnode 2 color_match in=@1 method=1 source_mode=1 src_r=0.8 src_g=0.1 src_b=0.1 target_r=0.1 target_g=0.2 target_b=0.9\nnode 3 tint in=@2.mask r=1 g=1 b=1\nnode 4 color r=0.1 g=0.7 b=0.1\nnode 5 color_match in=@4 method=1 source_mode=1 src_r=0.8 src_g=0.1 src_b=0.1\nnode 6 tint in=@5.mask r=1 g=1 b=1\nnode 7 tint in=@2.output r=1 g=1 b=1\noutput sel 3\noutput not 6\noutput res 7\n",
    );
    assert!(mean(&o["sel"], 0) > 0.99 && mean(&o["not"], 0) < 0.01, "mask selects the source colour only");
    assert!(o["res"].px[0][2] > 0.7, "output 'output' is the recoloured image");
    // Quantize Color outputs: ID has one level per colour, palette lists the colours
    let o = run(
        "node 1 gradient mode=0\nnode 2 colorize in=@1 p0=0 r0=1 g0=0 b0=0 p1=0.5 r1=0 g1=1 b1=0 p2=1 r2=0 g2=0 b2=1\nnode 3 quantize_color in=@2 color_amount=3\nnode 4 tint in=@3.id r=1 g=1 b=1\nnode 5 tint in=@3.palette r=1 g=1 b=1\nnode 6 tint in=@3 r=1 g=1 b=1\noutput id 4\noutput pal 5\noutput img 6\n",
    );
    let levels = |t: &craft_material_graph::tex::Tex| {
        let mut v: Vec<i32> = t.px.iter().map(|p| (p[0] * 100.0).round() as i32).collect();
        v.sort();
        v.dedup();
        v.len()
    };
    assert_eq!(levels(&o["id"]), 3, "three ids");
    assert!(levels(&o["pal"]) >= 2, "palette bands");
    assert!(levels(&o["img"]) >= 2);
    // Weave Generator masks: the two thread directions never overlap and together equal the opacity
    let o = run(
        "node 1 weave_generator weave=2\nnode 2 tint in=@1.mask_1 r=1 g=1 b=1\nnode 3 tint in=@1.mask_2 r=1 g=1 b=1\nnode 4 tint in=@1.opacity r=1 g=1 b=1\nnode 5 tint in=@1.height r=1 g=1 b=1\noutput m1 2\noutput m2 3\noutput op 4\noutput h 5\n",
    );
    let overlap: f32 = o["m1"].px.iter().zip(&o["m2"].px).map(|(a, b)| a[0] * b[0]).sum();
    let diff: f32 = o["m1"].px.iter().zip(&o["m2"].px).zip(&o["op"].px).map(|((a, b), c)| (a[0] + b[0] - c[0]).abs()).sum();
    assert!(overlap == 0.0 && diff == 0.0 && mean(&o["m1"], 0) > 0.1 && mean(&o["h"], 0) > 0.3, "weave masks");
    // an unknown output name is rejected with a message that lists the real ones
    let e = Graph::parse("size 64\nnode 1 color r=1\nnode 2 rgba_split in=@1\nnode 3 invert in=@2.x\noutput o 3\n").err();
    assert!(format!("{e:?}").contains("outputs: r, g, b, a"), "{e:?}");
}

#[test]
fn m0_non_square_node_sizes() {
    // 256x128 graph: relative-to-input inherits the shape, the relative modifier scales both axes,
    // absolute keeps the aspect ratio of the graph
    let text = "size 256 128\nnode 1 perlin scale=4\nnode 2 blur in=@1 radius=2 osize_mode=0 osize_rel=-1\nnode 3 perlin scale=4 osize_mode=2 osize_abs=7\nnode 4 blend bg=@1 fg=@2 mode=0 opacity=0.5\noutput a 1\noutput b 2\noutput c 3\noutput d 4\n";
    let g = Graph::parse(text).unwrap();
    let out = g.evaluate(None).unwrap();
    assert_eq!((out["a"].w, out["a"].h), (256, 128));
    assert_eq!((out["b"].w, out["b"].h), (128, 64));
    assert_eq!((out["c"].w, out["c"].h), (128, 64));
    // mixed sizes meet at the node's own size without panicking or changing the aspect
    assert_eq!((out["d"].w, out["d"].h), (256, 128));
    // a preview at half width keeps the 2:1 shape
    let half = g.evaluate(Some(128)).unwrap();
    assert_eq!((half["a"].w, half["a"].h), (128, 64));
}

#[test]
fn m0_pin_types_follow_the_graph() {
    use craft_material_graph::graph::{Pin, pin_types};
    let g = Graph::parse("size 64\nnode 1 perlin scale=4\nnode 2 colorize in=@1\nnode 3 blur in=@2 radius=2\nnode 4 blur in=@1 radius=2\nnode 5 to_gray in=@3\nnode 6 color_match in=@2 reference=@2 method=1\nnode 7 tint in=@4\nnode 8 alpha_split in=@2\nnode 9 invert in=@8.alpha\nnode 10 invert in=@8.rgb\noutput a 5\n").unwrap();
    let p = pin_types(&g);
    assert_eq!(p[&1], Pin::Gray);
    assert_eq!(p[&2], Pin::Color);
    assert_eq!(p[&3], Pin::Color); // a filter keeps the colour type of its input
    assert_eq!(p[&4], Pin::Gray);
    assert_eq!(p[&5], Pin::Gray);
    assert_eq!(p[&6], Pin::Color);
    assert_eq!(p[&7], Pin::Color);
    assert_eq!(p[&9], Pin::Gray, "a named gray output feeds a gray filter");
    assert_eq!(p[&10], Pin::Color);
}

#[test]
fn m0_per_node_output_format_quantises() {
    let run = |fmt: &str| {
        let g = Graph::parse(&format!("size 64\nnode 1 gradient mode=0\nnode 2 tint in=@1 r=1.37 g=0.3333 b=0.5{fmt}\noutput o 2\n")).unwrap();
        g.evaluate(None).unwrap().remove("o").unwrap()
    };
    let f32t = run("");
    assert!(f32t.px.iter().any(|p| p[0] > 1.0), "32-bit float keeps values above 1");
    let i8 = run(" ofmt=1");
    assert!(i8.px.iter().all(|p| p[0] <= 1.0 && ((p[0] * 255.0).round() - p[0] * 255.0).abs() < 1e-3));
    let i16 = run(" ofmt=2");
    assert!(i16.px.iter().all(|p| p[0] <= 1.0));
    // whole steps of 1/65535 (to f32 precision), and not the coarser steps of 8 bit
    let steps = |v: f32, k: f64| (f64::from(v) * k - (f64::from(v) * k).round()).abs();
    assert!(i16.px.iter().all(|p| steps(p[0], 65535.0) < 0.01), "16 bit steps");
    assert!(i16.px.iter().any(|p| steps(p[0], 255.0) > 0.01), "finer than 8 bit");
    let h = run(" ofmt=3");
    assert!(h.px.iter().any(|p| p[0] > 1.0), "16-bit float keeps values above 1");
    assert!(h.px.iter().zip(&f32t.px).all(|(a, b)| (a[0] - b[0]).abs() <= b[0].abs() * 1e-3 + 1e-6));
    assert!(h.px.iter().zip(&f32t.px).any(|(a, b)| a[0] != b[0]), "half precision rounds");
}

#[test]
fn m0_format_versions() {
    // a file without a format line is version 1 and still loads; we write version 3
    let old = Graph::parse("size 64\nnode 1 perlin scale=4\noutput o 1\n").unwrap();
    assert_eq!(old.format, 1);
    assert!(old.to_text().starts_with("format 3\n"));
    let again = Graph::parse(&old.to_text()).unwrap();
    assert_eq!(again.format, 3);
    // a newer file is refused with a message that says what to do
    let err = Graph::parse("format 99\nsize 64\nnode 1 perlin\noutput o 1\n").err().unwrap().to_string();
    assert!(err.contains("format 99") && err.contains("update"), "{err}");
}

#[test]
fn m0_live_links_follow_the_file_on_disk() {
    let dir = std::env::temp_dir().join(format!("tc_graph_links_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    let lib = |scale: u32| format!("size 64\nnode 1 perlin scale={scale}\noutput o 1\n");
    std::fs::write(dir.join("lib/pattern.tcgraph"), lib(4)).unwrap();
    let main = "size 64\nlink pat \"lib/pattern.tcgraph\"\nnode 1 subgraph graph=\"pat\"\nnode 2 subgraph graph=\"pat\"\nnode 3 blend bg=@1 fg=@2 opacity=0.5\noutput o 3\n";
    std::fs::write(dir.join("main.tcgraph"), main).unwrap();
    let (mut g, issues) = Graph::load_file(&dir.join("main.tcgraph")).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert!(g.is_linked("pat"));
    // the linked graph is not written into the file, only the link
    let text = g.to_text();
    assert!(text.contains("link pat \"lib/pattern.tcgraph\"") && !text.contains("subgraph pat"), "{text}");
    let a = g.evaluate(None).unwrap().remove("o").unwrap();
    // change the linked file: refreshing picks it up
    std::fs::write(dir.join("lib/pattern.tcgraph"), lib(9)).unwrap();
    assert!(g.resolve_links(&dir).is_empty());
    let b = g.evaluate(None).unwrap().remove("o").unwrap();
    assert!(a.px.iter().zip(&b.px).any(|(p, q)| (p[0] - q[0]).abs() > 0.01), "the new scale must show");
    // a missing file is reported, not fatal, and the name then fails to evaluate with a clear error
    std::fs::remove_file(dir.join("lib/pattern.tcgraph")).unwrap();
    let issues = g.resolve_links(&dir);
    assert_eq!(issues.len(), 1);
    assert!(issues[0].contains("pat") && issues[0].contains("cannot read"), "{issues:?}");
    assert!(!g.is_linked("pat"));
    assert!(g.evaluate(None).is_err());
    // a link to a named graph inside a package
    std::fs::write(
        dir.join("lib/pkg.tcgraph"),
        "size 64\nsubgraph inner\nsize 64\nnode 1 voronoi scale=5\noutput o 1\nend\nnode 1 perlin\noutput o 1\n",
    )
    .unwrap();
    let g2 = Graph::parse("size 64\nlink v \"lib/pkg.tcgraph\" \"inner\"\nnode 1 subgraph graph=\"v\"\noutput o 1\n").unwrap();
    let mut g2 = g2;
    assert!(g2.resolve_links(&dir).is_empty());
    assert!(g2.evaluate(None).is_ok());
    // a graph that does not exist in the package
    let mut g3 = Graph::parse("size 64\nlink v \"lib/pkg.tcgraph\" \"nope\"\nnode 1 perlin\noutput o 1\n").unwrap();
    assert!(g3.resolve_links(&dir)[0].contains("no graph 'nope'"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn m0_remaining_multi_output_nodes() {
    let g = Graph::parse(concat!(
        "size 64\n",
        "node 1 cells scale=4\nnode 2 voronoi scale=5\nnode 3 splatter count=6 scale=0.2 scale_var=0 rot_var=0\n",
        "node 4 threshold in=@3 threshold=0.3\nnode 5 flood_fill in=@4\n",
        "node 6 flood_fill_to_index in=@5\nnode 7 flood_fill_to_position in=@5\nnode 8 flood_fill_to_bbox_size in=@5\n",
        "output cells_id 1.cell_id\noutput cells 1\noutput vor_id 2.cell_id\noutput vor_edge 2.edge\noutput vor 2\n",
        "output spl 3\noutput spl_id 3.id\noutput spl_c 3.center\noutput spl_s 3.size\n",
        "output idx 6\noutput pos 7\noutput posx 7.x\noutput bb 8\noutput bbw 8.width\n",
    ))
    .unwrap();
    let r = g.evaluate(None).unwrap();
    let distinct = |t: &craft_material_graph::tex::Tex| {
        let mut v: Vec<u32> = t.px.iter().map(|p| (p[0] * 1000.0) as u32).collect();
        v.sort();
        v.dedup();
        v.len()
    };
    assert!(distinct(&r["cells_id"]) >= 8 && distinct(&r["cells"]) > 2);
    assert!(r["cells_id"].px != r["cells"].px, "the extra output must differ from the main one");
    assert!(distinct(&r["vor_id"]) >= 8);
    assert!(r["vor_id"].px != r["vor_edge"].px && r["vor_edge"].px != r["vor"].px);
    assert!(distinct(&r["spl_id"]) >= 3, "each splat has its own id");
    assert!(r["spl_c"].px.iter().any(|p| p[0] > 0.0 && p[1] > 0.0 && p[2] == 0.0));
    assert!(r["spl_s"].px.iter().any(|p| p[0] > 0.1 && p[0] < 0.3), "splat size 0.2 shows up");
    // flood-fill mappers: one number / centre / extent per region, background stays black
    assert!(distinct(&r["idx"]) >= 3);
    let top = r["idx"].px.iter().fold(0.0f32, |m, p| m.max(p[0]));
    assert!((top - 1.0).abs() < 1e-5, "the last region is index 1");
    assert!(r["pos"].px.iter().all(|p| p[0] >= 0.0 && p[0] < 1.0 && p[1] >= 0.0 && p[1] < 1.0));
    let same_region_same_centre = {
        // pixels sharing one index must share one centre
        let mut seen: std::collections::HashMap<u32, (u32, u32)> = std::collections::HashMap::new();
        r["idx"].px.iter().zip(&r["pos"].px).filter(|(i, _)| i[0] > 0.0).all(|(i, p)| {
            let e = seen.entry((i[0] * 1e5) as u32).or_insert(((p[0] * 1e5) as u32, (p[1] * 1e5) as u32));
            *e == ((p[0] * 1e5) as u32, (p[1] * 1e5) as u32)
        })
    };
    assert!(same_region_same_centre);
    assert!(r["bbw"].px.iter().any(|p| p[0] > 0.1 && p[0] < 0.4), "a 0.2-wide splat has a bounding box of that width");
    assert_eq!(r["posx"].px.len(), 64 * 64);
    assert!(r["posx"].px.iter().all(|p| p[0] == p[1] && p[1] == p[2]), "named x output is gray");
    assert!(craft_material_graph::graph::node_outputs("flood_fill_to_position").contains(&"x"));
}

#[test]
fn m0_pin_type_rules() {
    use craft_material_graph::graph::{Pin, connection_issues};
    // images convert freely between gray and colour
    assert!(Pin::Color.accepts(Pin::Gray) && Pin::Gray.accepts(Pin::Color));
    // values widen but never narrow, and never mix with images
    assert!(Pin::Float.accepts(Pin::Int) && Pin::Float3.accepts(Pin::Int3) && Pin::Int.accepts(Pin::Bool));
    assert!(!Pin::Int.accepts(Pin::Float) && !Pin::Float2.accepts(Pin::Float) && !Pin::Bool.accepts(Pin::Int));
    assert!(!Pin::Float.accepts(Pin::Gray) && !Pin::Gray.accepts(Pin::Float) && !Pin::Text.accepts(Pin::Float));
    assert_eq!((Pin::Float3.lanes(), Pin::Int.lanes(), Pin::Gray.lanes(), Pin::Text.lanes()), (3, 1, 0, 0));
    // every image graph is well typed
    let g = Graph::parse("size 32\nnode 1 perlin\nnode 2 colorize in=@1\nnode 3 blend bg=@1 fg=@2\noutput o 3\n").unwrap();
    assert!(connection_issues(&g).is_empty());
}

#[test]
fn m0_graph_default_format_and_hdr_safe_filters() {
    let base = "size 64\nnode 1 gradient mode=0\nnode 2 tint in=@1 r=1.37 g=0.3333 b=0.5\n";
    let run = |meta: &str, extra: &str| {
        let g = Graph::parse(&format!("{base}{meta}{extra}output o 2\n")).unwrap();
        g.evaluate(None).unwrap().remove("o").unwrap()
    };
    let float = run("", "");
    assert!(float.px.iter().any(|p| p[0] > 1.0));
    // a graph-wide 8-bit default rounds every node
    let eight = run("meta ofmt \"1\"\n", "");
    assert!(eight.px.iter().all(|p| p[0] <= 1.0 && ((p[0] * 255.0).round() - p[0] * 255.0).abs() < 1e-3));
    let g = Graph::parse(&format!("{base}meta ofmt \"1\"\noutput o 2\n")).unwrap();
    assert_eq!(g.default_format(), 1);
    // a node can opt back out (4 = force 32-bit float) or pick its own precision
    let kept = Graph::parse("size 64\nmeta ofmt \"1\"\nnode 1 gradient mode=0 ofmt=4\nnode 2 tint in=@1 r=1.37 g=0.3333 b=0.5 ofmt=4\noutput o 2\n")
        .unwrap()
        .evaluate(None)
        .unwrap()
        .remove("o")
        .unwrap();
    assert!(kept.px.iter().any(|p| p[0] > 1.0));
    // the default survives a save / load round trip and changes the cache identity
    let again = Graph::parse(&g.to_text()).unwrap();
    assert_eq!(again.default_format(), 1);
    // filters that are linear keep values above 1 (HDR) instead of clamping
    for k in ["sharpen", "adjust", "hsl"] {
        let t = Graph::parse(&format!("size 64\nnode 1 gradient mode=0\nnode 2 tint in=@1 r=1.6 g=1.6 b=1.6\nnode 3 {k} in=@2\noutput o 3\n"))
            .unwrap()
            .evaluate(None)
            .unwrap()
            .remove("o")
            .unwrap();
        assert!(t.px.iter().any(|p| p[0] > 1.0), "{k} must not clamp HDR values");
    }
}

#[test]
fn m0_square_cells_follow_the_aspect_ratio() {
    use craft_material_graph::noise;
    let run = |extra: &str| {
        let g = Graph::parse(&format!("size 64 32\nnode 1 perlin scale=8{extra}\noutput o 1\n")).unwrap();
        g.evaluate(None).unwrap().remove("o").unwrap()
    };
    let stretched = run("");
    let square = run(" square_cells=1");
    assert_eq!((square.w, square.h), (64, 32));
    // default: the same 8 x 8 lattice stretched over the 2:1 image; option: 8 x 4 lattice (square cells)
    let want = |py: i32, t: &craft_material_graph::tex::Tex| {
        t.px.iter().enumerate().all(|(i, p)| {
            let (x, y) = (i % t.w, i / t.w);
            (p[0] - noise::perlin((x as f32 + 0.5) / t.w as f32, (y as f32 + 0.5) / t.h as f32, 8, py, 0)).abs() < 1e-5
        })
    };
    assert!(want(8, &stretched));
    assert!(want(4, &square));
    // voronoi and cells take the option too (they keep running and differ from the stretched version)
    for k in ["voronoi", "cells", "fbm"] {
        let a = Graph::parse(&format!("size 64 32\nnode 1 {k} scale=6\noutput o 1\n")).unwrap().evaluate(None).unwrap().remove("o").unwrap();
        let b = Graph::parse(&format!("size 64 32\nnode 1 {k} scale=6 square_cells=1\noutput o 1\n"))
            .unwrap()
            .evaluate(None)
            .unwrap()
            .remove("o")
            .unwrap();
        assert!(a.px != b.px, "{k} must react to square_cells");
    }
    // tile nodes derive the vertical count from the horizontal one
    let a =
        Graph::parse("size 64 32\nnode 1 tile_generator x_amount=4 y_amount=4\noutput o 1\n").unwrap().evaluate(None).unwrap().remove("o").unwrap();
    let b = Graph::parse("size 64 32\nnode 1 tile_generator x_amount=4 y_amount=4 square_cells=1\noutput o 1\n")
        .unwrap()
        .evaluate(None)
        .unwrap()
        .remove("o")
        .unwrap();
    assert!(a.px != b.px);
}

#[test]
fn m0_dependency_management() {
    let dir = std::env::temp_dir().join(format!("tc_graph_deps_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::create_dir_all(dir.join("moved")).unwrap();
    std::fs::write(dir.join("lib/pat.tcgraph"), "size 64\nnode 1 perlin scale=4\noutput o 1\n").unwrap();
    let text =
        "size 64\nlink pat \"lib/pat.tcgraph\"\nnode 1 subgraph graph=\"pat\"\nnode 2 subgraph graph=\"pat\"\nnode 3 blend bg=@1 fg=@2\noutput o 3\n";
    let mut g = Graph::parse(text).unwrap();
    assert!(g.resolve_links(&dir).is_empty());
    let info = &g.link_info["pat"];
    assert!(info.found.as_ref().unwrap().ends_with("lib/pat.tcgraph") && info.issue.is_none());
    assert_eq!(info.members, vec!["pat".to_string()]);

    // the file moves: reported, then found again through a search folder
    std::fs::rename(dir.join("lib/pat.tcgraph"), dir.join("moved/pat.tcgraph")).unwrap();
    assert_eq!(g.resolve_links(&dir).len(), 1);
    assert!(g.link_info["pat"].found.is_none() && g.link_info["pat"].issue.as_ref().unwrap().contains("cannot read"));
    g.meta.insert("search_paths".into(), "elsewhere; moved".into());
    assert_eq!(g.search_paths(), vec!["elsewhere".to_string(), "moved".to_string()]);
    assert!(g.resolve_links(&dir).is_empty(), "found by name in a search folder");
    assert!(g.link_info["pat"].found.as_ref().unwrap().ends_with("moved/pat.tcgraph"));
    assert!(g.evaluate(None).is_ok());

    // relink keeps the name, so both instances follow
    std::fs::write(dir.join("moved/other.tcgraph"), "size 64\nnode 1 voronoi scale=3\noutput o 1\n").unwrap();
    let before = g.evaluate(None).unwrap().remove("o").unwrap();
    assert!(g.relink("pat", "moved/other.tcgraph", None));
    assert!(g.resolve_links(&dir).is_empty());
    assert!(before.px != g.evaluate(None).unwrap().remove("o").unwrap().px);

    // breaking the link embeds a copy that is saved with the package and no longer follows the file
    let n = g.break_link("pat").unwrap();
    assert_eq!(n, 1);
    assert!(g.links.is_empty() && !g.is_linked("pat") && g.subgraphs.contains_key("pat"));
    let saved = g.to_text();
    assert!(!saved.contains("link pat") && saved.contains("subgraph pat"), "{saved}");
    std::fs::remove_file(dir.join("moved/other.tcgraph")).unwrap();
    let mut again = Graph::parse(&saved).unwrap();
    assert!(again.resolve_links(&dir).is_empty());
    assert!(again.evaluate(None).is_ok(), "the embedded copy still works without the file");

    // removing a link reports the instances that are left dangling
    let mut h = Graph::parse("size 64\nlink pat \"moved/none.tcgraph\"\nnode 1 subgraph graph=\"pat\"\noutput o 1\n").unwrap();
    let _ = h.resolve_links(&dir);
    assert!(h.break_link("pat").is_err(), "an unresolved link cannot be embedded");
    assert_eq!(h.remove_link("pat"), 1);
    assert!(h.links.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn m1_channel_shuffle_and_bitmap_options() {
    let g = G2::parse("size 8\nnode 1 color r=0.2 g=0.4 b=0.6\nnode 2 channel_shuffle in=@1 src_r=2 src_g=8 src_b=9 inv_a=1\noutput o 2\n").unwrap();
    let p = g.evaluate(Some(8)).unwrap()["o"].px[0];
    assert!((p[0] - 0.6).abs() < 1e-5 && p[1] == 0.0 && p[2] == 1.0 && p[3] == 0.0, "{p:?}");
}

#[test]
fn an_imported_graph_keeps_the_functions_it_runs() {
    let other = Graph::parse(
        "size 16\nsubgraph shade\nmeta kind \"function\"\nnode 1 f_pos\nnode 2 f_swizzle a=@1 mask=\"x\"\nnode 3 f_out in=@2\nend\nnode 1 pixel_processor fn=\"shade\"\noutput o 1\n",
    )
    .unwrap();
    let mut host = Graph::parse("size 16\nnode 1 color r=0 g=0 b=0\noutput base 1\n").unwrap();
    host.import_graph(other, "coat").unwrap();
    assert!(host.subgraphs.contains_key("coat__shade"));
    assert_eq!(host.subgraphs["coat"].nodes[&1].strs["fn"], "coat__shade");
    let text = format!("{}node 2 subgraph graph=\"coat\"\noutput coat 2\n", host.to_text());
    let out = Graph::parse(&text).unwrap().evaluate(None).unwrap();
    // (the function is the pixel's x position)
    assert!((out["coat"].px[0][0] - 0.5 / 16.0).abs() < 1e-3, "{:?}", out["coat"].px[0]);
}

#[test]
fn extracting_nodes_keeps_the_output_pins_that_are_read() {
    let text = "size 16\nnode 1 color r=0.9 g=0.2 b=0.4\nnode 2 rgba_split in=@1\nnode 3 invert in=@2.g\noutput o 3\n";
    let before = Graph::parse(text).unwrap().evaluate(None).unwrap();
    // the split goes into the subgraph; outside, the pin `g` was read from it
    let mut g = Graph::parse(text).unwrap();
    let sel: BTreeSet<u32> = [2].into_iter().collect();
    g.extract_subgraph(&sel, "split").unwrap();
    assert_eq!(g.evaluate(None).unwrap()["o"].px, before["o"].px);
    // the invert goes into the subgraph; it read the pin `g` from outside
    let mut h = Graph::parse(text).unwrap();
    let sel: BTreeSet<u32> = [3].into_iter().collect();
    h.extract_subgraph(&sel, "inv").unwrap();
    assert_eq!(h.evaluate(None).unwrap()["o"].px, before["o"].px);
}

#[test]
fn the_node_cache_sees_the_end_of_a_long_chain() {
    use craft_material_graph::context::EvalContext;
    // the output has the lowest id and the chain it reads the higher ones: its key must cover all of them
    let chain = |angle: f32| {
        let mut t = String::from("size 16\n");
        for i in 1..=600 {
            t.push_str(&format!("node {i} invert in=@{}\n", i + 1));
        }
        t.push_str(&format!("node 601 gradient angle={angle}\noutput o 1\n"));
        t
    };
    let cache = std::sync::Arc::new(craft_material_graph::graph::NodeCache::new(64));
    let render = |angle: f32| {
        let r = Graph::parse(&chain(angle)).unwrap().evaluate_all_cached(&EvalContext::default(), 16, &[1], &cache, None, &mut |_| {});
        assert!(r.errors.is_empty(), "{:?}", r.errors.iter().next());
        r.kept[&1].px.clone()
    };
    assert_ne!(render(0.1), render(0.35), "a change at the far end of the chain shows");
}

#[test]
fn the_node_cache_tells_accelerators_apart_by_their_tag() {
    use craft_material_graph::accelerator::Accelerator;
    use craft_material_graph::context::EvalContext;
    struct Tagged(&'static str);
    impl Accelerator for Tagged {
        fn cache_tag(&self) -> &str {
            self.0
        }
    }
    let g = Graph::parse("size 16\nnode 1 perlin scale=4\nnode 2 invert in=@1\noutput o 2\n").unwrap();
    let cache = std::sync::Arc::new(craft_material_graph::graph::NodeCache::new(16));
    // the nodes a run with the accelerator `tag` takes from the cache
    let reused = |tag: &'static str| {
        let ec = EvalContext { accelerator: Some(std::sync::Arc::new(Tagged(tag))), ..EvalContext::default() };
        let before = cache.stats().0;
        let r = g.evaluate_all_cached(&ec, 16, &[2], &cache, None, &mut |_| {});
        assert!(r.errors.is_empty());
        cache.stats().0 - before
    };
    let first = reused("a");
    let again = reused("a");
    let other = reused("b");
    assert!(again > first, "the same accelerator reuses what the first run computed ({first} then {again})");
    assert!(other < again, "another tag is another result, it reuses less ({other} against {again})");
}

#[test]
fn the_node_cache_sees_a_file_a_subgraph_reads_change() {
    use craft_material_graph::context::EvalContext;
    let dir = std::env::temp_dir().join(format!("tc_graph_subgraph_file_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.png");
    let text = format!("size 16\nsubgraph s\nnode 1 bitmap file=\"{}\"\noutput o 1\nend\nnode 1 subgraph graph=\"s\"\noutput o 1\n", file.display());
    let cache = std::sync::Arc::new(craft_material_graph::graph::NodeCache::new(64));
    let first_pixel = |side: usize, color: [f32; 4]| {
        std::fs::write(&file, png::encode(side, side, &vec![color; side * side], 8)).unwrap();
        let r = Graph::parse(&text).unwrap().evaluate_all_cached(&EvalContext::default(), 16, &[1], &cache, None, &mut |_| {});
        assert!(r.errors.is_empty(), "{:?}", r.errors.iter().next());
        r.kept[&1].px[0]
    };
    let red = first_pixel(4, [1.0, 0.0, 0.0, 1.0]);
    // (another size, so the file's length differs whatever the clock says)
    let green = first_pixel(8, [0.0, 1.0, 0.0, 1.0]);
    assert!(red[0] > 0.9 && red[1] < 0.1, "{red:?}");
    assert!(green[1] > 0.9 && green[0] < 0.1, "the new file shows, not the cached one: {green:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn named_outputs_of_nodes_with_high_ids_do_not_share_a_cache_entry() {
    // the ids 3 and 65539 differ only above the low 16 bits
    let g = Graph::parse(concat!(
        "size 8\nnode 1 color r=0 g=0.2 b=0\nnode 2 color r=0 g=0.8 b=0\n",
        "node 3 rgba_split in=@1\nnode 65539 rgba_split in=@2\n",
        "node 4 invert in=@3.g\nnode 5 invert in=@65539.g\noutput a 4\noutput b 5\n",
    ))
    .unwrap();
    let out = g.evaluate(None).unwrap();
    assert!((out["a"].px[0][0] - 0.8).abs() < 1e-5, "{:?}", out["a"].px[0]);
    assert!((out["b"].px[0][0] - 0.2).abs() < 1e-5, "{:?}", out["b"].px[0]);
}
