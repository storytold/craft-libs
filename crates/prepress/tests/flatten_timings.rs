//! Times the flattener in stages on every plate-test scene and on a synthetic heavy page, at
//! 300 dpi. A report, not a benchmark: run it in release with
//! `cargo test --release -p craft-prepress --features flatten --test flatten_timings -- --ignored --nocapture`.
#![cfg(feature = "flatten")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use craft_pdf::write::backend::SerializeSettings;

use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::print::{self, PrintColor};
use craft_pdf::write::{Metadata, Writer};
use craft_plate_test::reference::{self, Context};
use craft_plate_test::scene;
use craft_prepress::flatten::kurbo::{Affine, Circle, Point, Rect, Shape};
use craft_prepress::flatten::pdf::{ReadOptions, read_page};
use craft_prepress::flatten::raster::Rasterizer;
use craft_prepress::flatten::write::{page_pdf, write_flattened};
use craft_prepress::flatten::{
    BlendMode, DisplayList, FillItem, FillRule, FlattenOptions, FlattenPageOptions, GradientGeometry, GroupItem, Item, Paint, flatten, flatten_page,
};
use craft_prepress::{CmsConversion, Fallback, InkColor, InkId, InkSet, Options, Overprint, Process, plates};

const DPI: f32 = 300.0;

#[derive(Default)]
struct Stages {
    read: Duration,
    flatten: Duration,
    raster: Duration,
    write: Duration,
    total: Duration,
    regions: usize,
    whole: String,
    pixels: u64,
    fills: usize,
    complex: usize,
}

/// The page read, flattened, and written with its complex regions rasterized, each stage timed;
/// `total` is [`flatten_page`] in one call.
fn stages(pdf: &[u8]) -> Stages {
    let mut st = Stages::default();
    let start = Instant::now();
    let opts = FlattenPageOptions { flatten: FlattenOptions { raster_dpi: DPI, ..FlattenOptions::default() }, ..FlattenPageOptions::default() };
    flatten_page(pdf, 0, &opts).unwrap();
    st.total = start.elapsed();

    let start = Instant::now();
    let (list, _) = read_page(pdf, 0, &ReadOptions::default()).unwrap();
    st.read = start.elapsed();
    written(&list, &mut st);
    st
}

/// Flattens and writes `list`, timing the stages into `st`.
fn written(list: &DisplayList, st: &mut Stages) -> Vec<u8> {
    let start = Instant::now();
    let flat = flatten(list, &FlattenOptions { raster_dpi: DPI, ..FlattenOptions::default() }).unwrap();
    st.flatten = start.elapsed();
    st.fills = flat.report.fills;
    st.regions = flat.report.regions;
    st.whole = flat.report.complex_groups.iter().map(|g| format!("{:?}", g.reason)).collect::<Vec<_>>().join(", ");
    st.complex = flat.complex.len();

    let settings = settings();
    let cms = CmsConversion::fogra39();
    let start = Instant::now();
    let mut raster = Rasterizer::new(list, settings.clone(), Fallback::Fogra39, &cms, DPI);
    let mut raster_time = Duration::ZERO;
    let version = settings.configuration.version();
    let mut writer = Writer::new(settings, Metadata::default());
    let size = PageSettings::from_wh(list.page.width() as f32, list.page.height() as f32).unwrap();
    let mut page_builder = writer.document_mut().start_page_with(size);
    let mut surface = page_builder.surface();
    write_flattened(&mut surface, list, &flat, &cms, version, &mut |r| {
        let t = Instant::now();
        let image = raster.region(r);
        raster_time += t.elapsed();
        image
    })
    .unwrap();
    surface.finish();
    page_builder.finish();
    let pdf = writer.finish().unwrap();
    st.raster = raster_time;
    st.write = start.elapsed() - raster_time;
    st.pixels = raster.report().pixels;
    pdf
}

fn settings() -> SerializeSettings {
    static SETTINGS: OnceLock<SerializeSettings> = OnceLock::new();
    SETTINGS.get_or_init(|| print::settings(PrintColor::fogra39(craft_cms::builtin::COATED_FOGRA39_ICC)).unwrap()).clone()
}

fn ms(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1000.0)
}

fn row(name: &str, st: &Stages) -> String {
    format!(
        "| {name} | {} | {} | {} | {} | {} | {} | {} | {}{} | {:.1} |",
        ms(st.total),
        ms(st.read),
        ms(st.flatten),
        ms(st.raster),
        ms(st.write),
        st.regions,
        st.fills,
        st.complex,
        if st.whole.is_empty() { String::new() } else { format!(" ({})", st.whole) },
        st.pixels as f64 / 1e6
    )
}

/// About 1,000 shapes up to `size` pt on an A4 page, 100 of them transparent (opacity or
/// Multiply); `gradients` of the transparent ones are filled with a ramp. Positions come from a fixed
/// sequence, so every run draws the same page.
fn heavy(size: f64, gradients: usize) -> DisplayList {
    let mut seed = 0x2545_f491_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 10_000) as f64 / 10_000.0
    };
    let ink = |i: usize| InkId::process(Process::ALL[i % 4]);
    let mut items = Vec::new();
    let mut transparent = 0;
    for i in 0..1000 {
        let (x, y) = (20.0 + next() * 520.0, 20.0 + next() * 780.0);
        let (w, h) = (2.0 + next() * (size - 2.0), 2.0 + next() * (size - 2.0));
        let path = if i % 3 == 0 { Circle::new(Point::new(x, y), w / 2.0).to_path(0.1) } else { Rect::new(x, y, x + w, y + h).to_path(0.1) };
        let color = InkColor::Tints(vec![(ink(i), 0.2 + next() as f32 * 0.8), (ink(i + 1), next() as f32 * 0.5)]);
        let paint = if i % 10 == 9 && transparent < gradients {
            Paint::Gradient {
                stops: vec![(0.0, color), (1.0, InkColor::Tints(vec![(ink(i + 2), 1.0)]))],
                geometry: GradientGeometry::Axial {
                    start: Point::new(x, y),
                    end: Point::new(x + w, y),
                    extend: [true; 2],
                    transform: Affine::IDENTITY,
                },
            }
        } else {
            Paint::Solid(color)
        };
        let fill = Item::Fill(FillItem { path, rule: FillRule::NonZero, paint, overprint: Overprint::default() });
        if i % 10 == 9 {
            transparent += 1;
            let blend = if i % 40 == 9 { BlendMode::Multiply } else { BlendMode::Normal };
            items.push(Item::Group(GroupItem {
                opacity: 0.3 + next() as f32 * 0.5,
                blend,
                isolated: false,
                knockout: false,
                clip: None,
                mask: None,
                actual_text: None,
                children: vec![fill],
            }));
        } else {
            items.push(fill);
        }
    }
    DisplayList { page: Rect::new(0.0, 0.0, 595.0, 842.0), inks: InkSet::new(), items }
}

#[test]
#[ignore = "timing report; run in release"]
fn flatten_timings() {
    let scenes = scene::load_dir(&scene::scenes_dir()).unwrap();
    let ctx = Context::from_env(&scenes);
    // Load the profiles once, so the first scene doesn't pay for it.
    let _ = stages(&reference::write(&scenes[0], &ctx).unwrap().pdf);
    println!(
        "| Page | flatten_page ms | read | flatten | rasterize | write | regions | fills | complex | Mpx |\n|---|---|---|---|---|---|---|---|---|---|"
    );
    for s in scenes.iter().filter(|s| s.pending.is_none()) {
        let pdf = reference::write(s, &ctx).unwrap().pdf;
        println!("{}", row(&s.id, &stages(&pdf)));
    }
    let pages = [
        ("heavy, 2–65 pt, flat paints", 65.0, 0),
        ("heavy, 2–65 pt, 10 gradients", 65.0, 10),
        ("heavy, 2–20 pt, flat paints", 20.0, 0),
        ("heavy, 2–20 pt, 10 gradients", 20.0, 10),
    ];
    for (name, size, gradients) in pages {
        let list = heavy(size, gradients);
        let start = Instant::now();
        let pdf = page_pdf(&list, settings(), &CmsConversion::fogra39()).unwrap();
        let wrote = start.elapsed();
        let start = Instant::now();
        let p = plates(&pdf, 0, DPI, &Options::default()).unwrap();
        // What rasterizing a region as large as the page costs: writing its items as a PDF and
        // separating that.
        println!("<!-- {name}: page_pdf {} ms, plates {} ms for {} x {} px -->", ms(wrote), ms(start.elapsed()), p.width, p.height);
        println!("{}", row(name, &stages(&pdf)));
        let mut st = Stages::default();
        let start = Instant::now();
        written(&list, &mut st);
        st.total = start.elapsed();
        println!("{}", row(&format!("{name} (display list)"), &st));
    }
}
