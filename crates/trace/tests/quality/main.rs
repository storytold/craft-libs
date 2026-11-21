//! Image Trace quality harness: traces images drawn from known vector art with every preset and
//! measures the result (see `docs/image-trace-quality.md`, which this writes).
//!
//! ```text
//! TRACE_QUALITY_REPORT=report.md cargo test --release -p craft-trace --test quality -- --ignored --nocapture
//! ```
//!
//! `TRACE_QUALITY_MP` (default `1,4,16`) sets the megapixel sizes of the cost table.
//! `TRACE_QUALITY_CLASS=centre-lines` selects only the code-drawn centre-line report.
//!
//! A yardstick: `TRACE_QUALITY_IMAGES=dir` also writes the drawings there as PNG files, and
//! `TRACE_QUALITY_YARDSTICK=dir` measures the SVG files of the same names found there (what another
//! tracer made of those PNG files, run by hand as a black box) the same way.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod centre_geom;
mod centre_lines;
mod centre_scenes;
mod inputs;
mod measure;
mod raster;

use std::fmt::Write as _;
use std::time::Instant;

use craft_trace::{Raster, TraceParams, TraceResult, TracedPath, presets, trace_within};
use inputs::{CLASSES, Class, Input, VARIANTS, Variant, inputs};
use measure::{Metrics, measure};

/// The engine's limit (`MAX_TRACE_ANCHORS`): a trace past it is refused.
const MAX_ANCHORS: usize = 8_000_000;

/// Whether `input` is what `preset` is meant for.
fn target(preset: &str, input: &Input) -> bool {
    let preset = preset.strip_suffix(", enhanced").unwrap_or(preset);
    let classes: &[Class] = match preset {
        "High Fidelity Photo"
        | "Low Fidelity Photo"
        | "Shades of Gray"
        | "Shades of Gray with Auto Grouping"
        | "High Fidelity Photo with Auto Grouping" => &[Class::Tone],
        // Three colours cannot show the badge's five.
        "3 Colors" => return input.name == "two touching halves",
        "6 Colors" | "16 Colors" | "Flat Logo" | "Auto Color" | "6 Colors with Auto Grouping" => &[Class::Flat],
        "Default with Shapes" | "6 Colors with Shapes" => &[Class::Plain],
        name if name.ends_with("Gradients") => &[Class::Ramp],
        "Sketched Art" => &[Class::Shape, Class::Lines],
        "Silhouettes" => &[Class::Shape],
        "Line Art" | "Outline" => &[Class::Lines],
        "Technical Drawing" => &[Class::Lines, Class::Poly, Class::Askew],
        _ => &[Class::Shape, Class::Poly],
    };
    classes.contains(&input.class)
}

/// One trace: which preset and input, and what was measured (`None`: the trace was refused).
struct Run {
    preset: usize,
    /// The input is one the preset is meant for.
    target: bool,
    class: Class,
    variant: Variant,
    metrics: Option<Metrics>,
}

/// Every (size, variant) an input is traced at: clean at three sizes, the others at the middle one.
const DRAWINGS: [(u32, Variant); 7] = [
    (128, Variant::Clean),
    (256, Variant::Clean),
    (512, Variant::Clean),
    (256, Variant::Hard),
    (256, Variant::Blur),
    (256, Variant::Noise),
    (256, Variant::Scan),
];

type Drawing<'a> = (&'a Input, u32, Variant, Raster);

fn draw_all(inputs: &[Input]) -> Vec<Drawing<'_>> {
    inputs.iter().flat_map(|input| DRAWINGS.map(|(size, variant)| (input, size, variant, input.draw(size, variant)))).collect()
}

/// The file name (without extension) of a drawing given to or made by another tracer.
fn stem(input: &Input, size: u32, variant: Variant) -> String {
    format!("{}-{size}-{}", input.name.replace(' ', "-"), variant.name())
}

/// The value of attribute `name` of an SVG element's text.
fn attr<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let at = element.match_indices(&key).find(|(i, _)| element[..*i].ends_with(char::is_whitespace))?.0 + key.len();
    Some(&element[at..at + element[at..].find('"')?])
}

/// The filled paths of an SVG file as a trace in black. Only what the yardstick's files use is
/// read: `path` elements with a `d`, and `matrix`, `translate` or `scale` transforms on them.
fn svg_trace(svg: &str) -> TraceResult {
    let paths = svg.split("<path").skip(1).filter_map(|rest| {
        let element = &rest[..rest.find('>')?];
        let mut path = kurbo::BezPath::from_svg(attr(element, "d")?).ok()?;
        if let Some(t) = attr(element, "transform") {
            let (kind, args) = t.trim_end_matches(')').split_once('(').unwrap();
            let v: Vec<f64> = args.split([',', ' ']).filter(|a| !a.is_empty()).map(|a| a.parse().unwrap()).collect();
            path.apply_affine(match (kind, v.as_slice()) {
                ("matrix", &[a, b, c, d, e, f]) => kurbo::Affine::new([a, b, c, d, e, f]),
                ("translate", &[x, y]) => kurbo::Affine::translate((x, y)),
                ("scale", &[x, y]) => kurbo::Affine::scale_non_uniform(x, y),
                ("scale", &[k]) => kurbo::Affine::scale(k),
                _ => panic!("unread transform {t}"),
            });
        }
        Some(TracedPath {
            path: craft_geom::PathData::from_bezpath(&path),
            color: [0; 3],
            pixels: 0,
            stroke: None,
            shape: None,
            gradient: None,
            square_caps: false,
        })
    });
    TraceResult { paths: paths.collect(), palette: vec![[0; 3]], ..Default::default() }
}

fn run_all(presets: &[(&'static str, TraceParams)], drawn: &[Drawing]) -> Vec<Run> {
    std::thread::scope(|s| {
        let workers: Vec<_> = presets
            .iter()
            .enumerate()
            .map(|(pi, (name, params))| {
                s.spawn(move || {
                    drawn
                        .iter()
                        .map(|(input, size, variant, src)| {
                            let metrics = trace_within(src, params, MAX_ANCHORS).ok().map(|res| measure(input, *variant, *size, src, params, &res));
                            Run { preset: pi, target: target(name, input), class: input.class, variant: *variant, metrics }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap()).collect()
    })
}

/// The mean (or, with `max`, the largest) of `field` over `runs`, to `digits` places; `–` when it
/// applies to none of them.
fn stat(runs: &[&Metrics], field: impl Fn(&Metrics) -> f64, max: bool, digits: usize) -> String {
    let v: Vec<f64> = runs.iter().map(|m| field(m)).filter(|v| !v.is_nan()).collect();
    if v.is_empty() {
        return "–".into();
    }
    let x = if max { v.iter().copied().fold(f64::NEG_INFINITY, f64::max) } else { v.iter().sum::<f64>() / v.len() as f64 };
    // `+ 0.0` turns a negative zero into zero.
    if x.is_finite() { format!("{:.digits$}", x + 0.0) } else { "all".into() }
}

const HEADER: &str = "| ΔE | edge ΔE | off mean | off max | missed max | paths | anchors | × art | lines % | upright % | corners % | kinks | odd colours | odd px | gap px | overlap px | specks | outside px | centre off | width off | anchors a stroke | shapes % | false shapes | grouped % | gradient paths | found % | angle off | ramp off |\n|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|\n";

fn row(runs: &[&Run]) -> String {
    let m: Vec<&Metrics> = runs.iter().filter_map(|r| r.metrics.as_ref()).collect();
    if m.is_empty() {
        return format!("| refused |{}", " |".repeat(27));
    }
    let cells = [
        stat(&m, |m| m.delta, false, 2),
        stat(&m, |m| m.edge_delta, false, 2),
        stat(&m, |m| m.geo_mean, false, 3),
        stat(&m, |m| m.geo_max, true, 2),
        stat(&m, |m| m.miss_max, true, 2),
        stat(&m, |m| m.paths, false, 1),
        stat(&m, |m| m.anchors, false, 0),
        stat(&m, |m| m.anchor_ratio, false, 2),
        stat(&m, |m| m.lines, false, 0),
        stat(&m, |m| m.upright, false, 0),
        stat(&m, |m| m.corners, false, 0),
        stat(&m, |m| m.kinks, false, 1),
        stat(&m, |m| m.slivers, false, 1),
        stat(&m, |m| m.sliver_area, false, 0),
        stat(&m, |m| m.gap, false, 1),
        stat(&m, |m| m.overlap, false, 1),
        stat(&m, |m| m.specks, false, 1),
        stat(&m, |m| m.outside, true, 1),
        stat(&m, |m| m.centre_off, false, 3),
        stat(&m, |m| m.width_off, false, 2),
        stat(&m, |m| m.stroke_anchors, false, 1),
        stat(&m, |m| m.shapes, false, 0),
        stat(&m, |m| m.false_shapes, false, 2),
        stat(&m, |m| m.grouped, false, 0),
        stat(&m, |m| m.gradients, false, 1),
        stat(&m, |m| m.ramp_found, false, 0),
        stat(&m, |m| m.ramp_angle, true, 1),
        stat(&m, |m| m.ramp_off, true, 0),
    ];
    format!("| {} |", cells.join(" | "))
}

/// A cost measurement: seconds, paths and anchors.
///
/// VectorCraft's copy of this harness also reports each trace's peak memory through a counting
/// global allocator. This workspace forbids `unsafe`, which a `GlobalAlloc` impl is, so that
/// column stays with VectorCraft's copy.
fn cost(img: &Raster, params: &TraceParams) -> String {
    let start = Instant::now();
    let res = trace_within(img, params, MAX_ANCHORS);
    let secs = start.elapsed().as_secs_f64();
    match res {
        Ok(res) => format!("{secs:.2} s, {} paths, {} anchors", res.paths.len(), res.anchor_count()),
        Err(e) => {
            let why = e.to_string();
            let why = if why.contains("anchor points") { "over 8,000,000 anchors" } else { "not flat art" };
            format!("refused after {secs:.2} s ({why})")
        }
    }
}

fn report() -> String {
    // The built-in presets, and some of them with the Shapes or the Auto Grouping option on.
    let mut presets = presets();
    for (name, base) in [("Default with Shapes", "Default"), ("6 Colors with Shapes", "6 Colors")] {
        presets.push((name, TraceParams { shapes: true, ..craft_trace::preset(base).unwrap() }));
    }
    for (name, base) in [
        ("6 Colors with Auto Grouping", "6 Colors"),
        ("Shades of Gray with Auto Grouping", "Shades of Gray"),
        ("High Fidelity Photo with Auto Grouping", "High Fidelity Photo"),
    ] {
        presets.push((name, TraceParams { auto_grouping: true, ..craft_trace::preset(base).unwrap() }));
    }
    // Gradients: on the inputs that are linear gradients, with the option and without it.
    for (name, base, gradients) in [
        ("6 Colors, no Gradients", "6 Colors", false),
        ("6 Colors with Gradients", "6 Colors", true),
        ("High Fidelity Photo, no Gradients", "High Fidelity Photo", false),
        ("High Fidelity Photo with Gradients", "High Fidelity Photo", true),
    ] {
        presets.push((name, TraceParams { gradients, ..craft_trace::preset(base).unwrap() }));
    }
    // Each preset's enhanced version, on that preset's own inputs.
    for name in craft_trace::PRESET_NAMES {
        let enhanced: &'static str = Box::leak(format!("{name}, enhanced").into_boxed_str());
        presets.extend(craft_trace::enhanced_preset(name).map(|p| (enhanced, p)));
    }
    let inputs = inputs();
    let drawn = draw_all(&inputs);
    let one_colour = |d: &&Drawing| matches!(d.0.class, Class::Shape | Class::Poly | Class::Lines | Class::Askew | Class::Plain);
    if let Ok(dir) = std::env::var("TRACE_QUALITY_IMAGES") {
        for (input, size, variant, img) in &drawn {
            std::fs::write(format!("{dir}/{}.png", stem(input, *size, *variant)), img.encode_png()).unwrap();
        }
    }
    let runs = run_all(&presets, &drawn);
    let pick = |preset: usize, f: &dyn Fn(&Run) -> bool| runs.iter().filter(|r| r.preset == preset && f(r)).collect::<Vec<_>>();
    let mut md = String::new();

    md += "## 1. Each preset on the inputs it is meant for\n\nAll sizes and variants (clean, aliased, blurred, noisy, scanned) of those inputs.\n\n| preset | inputs ";
    md += HEADER.replacen("\n|", "\n|:--|:--|", 1).as_str();
    for (pi, (name, _)) in presets.iter().enumerate() {
        let on: Vec<&str> = inputs.iter().filter(|i| target(name, i)).map(|i| i.name).collect();
        let _ = writeln!(md, "| {name} | {} {}", on.join(", "), row(&pick(pi, &|r| r.target)));
    }

    md += "\n## 2. Each preset on each kind of input (clean, three sizes)\n\n| preset | inputs ";
    md += HEADER.replacen("\n|", "\n|:--|:--|", 1).as_str();
    for (pi, (name, _)) in presets.iter().enumerate() {
        for class in CLASSES {
            let _ = writeln!(md, "| {name} | {} {}", class.name(), row(&pick(pi, &|r| r.class == class && r.variant == Variant::Clean)));
        }
    }

    md += "\n## 3. Each preset on its inputs, by variant (256 px)\n\n| preset | variant ";
    md += HEADER.replacen("\n|", "\n|:--|:--|", 1).as_str();
    for (pi, (name, _)) in presets.iter().enumerate() {
        for variant in VARIANTS {
            let _ = writeln!(md, "| {name} | {} {}", variant.name(), row(&pick(pi, &|r| r.target && r.variant == variant)));
        }
    }

    if let Ok(dir) = std::env::var("TRACE_QUALITY_YARDSTICK") {
        // Measured as a black-and-white trace without a white background.
        let params = craft_trace::preset("Black and White Logo").unwrap();
        let theirs: Vec<Run> = drawn
            .iter()
            .filter(one_colour)
            .filter_map(|(input, size, variant, src)| {
                let svg = std::fs::read_to_string(format!("{dir}/{}.svg", stem(input, *size, *variant))).ok()?;
                let metrics = Some(measure(input, *variant, *size, src, &params, &svg_trace(&svg)));
                Some(Run { preset: usize::MAX, target: true, class: input.class, variant: *variant, metrics })
            })
            .collect();
        let _ = write!(md, "\n## 4. Yardstick\n\n{} traces by another tracer, measured the same way.\n\n| inputs | variant ", theirs.len());
        md += HEADER.replacen("\n|", "\n|:--|:--|", 1).as_str();
        for class in [Class::Shape, Class::Poly, Class::Lines, Class::Askew, Class::Plain] {
            for variant in VARIANTS {
                let rows: Vec<&Run> = theirs.iter().filter(|r| r.class == class && r.variant == variant).collect();
                let _ = writeln!(md, "| {} | {} {}", class.name(), variant.name(), row(&rows));
            }
        }
    }

    // Cost, one trace at a time.
    let sizes: Vec<u32> = std::env::var("TRACE_QUALITY_MP")
        .unwrap_or_else(|_| "1,4,16".into())
        .split(',')
        .filter_map(|s| s.trim().parse::<f64>().ok())
        .map(|mp| (mp.sqrt() * 1024.0).round() as u32)
        .collect();
    let (photo, badge) = (inputs.iter().find(|i| i.name == "photo-like").unwrap(), inputs.iter().find(|i| i.name == "four-colour badge").unwrap());
    md += "\n## 5. Cost\n\nOne trace at a time on this machine (all cores). A trace of more than 8,000,000 anchors is refused, as in the app.\n\n| preset | image | megapixels | result |\n|:--|:--|--:|:--|\n";
    for size in sizes {
        let n = size as usize;
        let images = [("photo-like", photo.draw(size, Variant::Clean)), ("four-colour badge", raster::render(&badge.scaled(size), badge.bg, n, n))];
        for (name, params) in &presets {
            for (image, img) in &images {
                let _ = writeln!(md, "| {name} | {image} | {:.0} | {} |", f64::from(size).powi(2) / f64::from(1 << 20), cost(img, params));
            }
        }
    }
    md += &centre_lines::report();
    md
}

#[test]
#[ignore = "slow: measures every preset and writes the quality report (run with --release)"]
fn quality_report() {
    // One ignored test owns the allocator's cost measurements. Selecting this class
    // here avoids a second ignored test allocating concurrently with those measurements.
    if std::env::var("TRACE_QUALITY_CLASS").as_deref() == Ok("centre-lines") {
        let md = centre_lines::report();
        println!("{md}");
        if let Ok(path) = std::env::var("TRACE_QUALITY_REPORT") {
            std::fs::write(path, md).unwrap();
        }
        return;
    }
    let md = report();
    println!("{md}");
    // Gradients finds none in art that is flat colour, however it is degraded.
    let inputs = inputs();
    for (name, _) in presets().iter().filter(|(_, p)| p.mode != craft_trace::Mode::BlackAndWhite && p.mode != craft_trace::Mode::Logo) {
        let params = TraceParams { gradients: true, ..craft_trace::preset(name).unwrap() };
        for (input, size, variant, img) in draw_all(&inputs).iter().filter(|d| !matches!(d.0.class, Class::Tone | Class::Ramp)) {
            let found = trace_within(img, &params, MAX_ANCHORS).map_or(0, |res| res.paths.iter().filter(|p| p.gradient.is_some()).count());
            assert_eq!(found, 0, "{name} with Gradients on {} at {size} px, {variant:?}", input.name);
        }
    }
    if let Ok(path) = std::env::var("TRACE_QUALITY_REPORT") {
        std::fs::write(path, md).unwrap();
    }
}

/// The harness measures what it says: its rasteriser gives a disc its area, an image is at no
/// distance from itself, and a disc traced in black and white lies on the disc.
#[test]
fn harness_measures_a_disc() {
    let disc = kurbo::Shape::to_path(&kurbo::Circle::new((40.3, 39.6), 25.0), 1e-3);
    let area: f32 = raster::coverage(&disc, 80, 80).iter().sum();
    assert!((f64::from(area) - std::f64::consts::PI * 625.0).abs() < 2.0, "{area}");

    let inputs = inputs();
    let circle = &inputs[0];
    let src = circle.draw(128, Variant::Clean);
    assert_eq!(measure::fidelity(&src, &src), (0.0, 0.0));
    let params = craft_trace::preset("Black and White Logo").unwrap();
    let res = trace_within(&src, &params, MAX_ANCHORS).unwrap();
    let m = measure(circle, Variant::Clean, 128, &src, &params, &res);
    assert_eq!(m.paths, 1.0);
    assert!(m.geo_mean < 0.06 && m.geo_max < 0.3 && m.miss_max < 0.3, "{m:?}");
    assert!(m.delta < 0.2 && m.kinks == 0.0 && m.gap.is_nan(), "{m:?}");
}
