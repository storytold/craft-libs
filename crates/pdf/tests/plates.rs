//! Plate mode through the raster path: each ink's separation drawn as a gray film.
//! Adapted from storytold/pdfcraft#2770 by Guillem Hermida (`separations::tests` in pdfcraft-render at
//! 6fcf2a4aca29e943e28cb69017ff8f87d00656c6, MIT OR Apache-2.0).
#![cfg(feature = "raster")]

use craft_pdf::interpret::InterpreterSettings;
use craft_pdf::interpret::color::{CmykSimulation, Ink, Plate};
use craft_pdf::{
    open_slice,
    raster::{self, Options, RenderCache},
};

/// A 40 × 40 pt page with `content` and `resources`, and `objects` numbered from 5 (each a
/// whole object body: a dictionary, or a dictionary and its stream).
fn page_pdf(content: &str, resources: &str, objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    out.extend_from_slice(b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n");
    out.extend_from_slice(b"2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n");
    out.extend_from_slice(
        format!("3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R /Resources << {resources} >> >> endobj\n").as_bytes(),
    );
    out.extend_from_slice(format!("4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n", content.len()).as_bytes());
    for (i, body) in objects.iter().enumerate() {
        out.extend_from_slice(format!("{} 0 obj ", i + 5).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b" endobj\n");
    }
    out.extend_from_slice(b"trailer << /Root 1 0 R >>\n%%EOF");
    out
}

/// A stream object body with `dict` entries and `data`.
fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >> stream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// The page rendered once per settings, sharing one render cache as a separating caller does.
fn render_each(pdf: &[u8], settings: &[InterpreterSettings]) -> Vec<Vec<u8>> {
    let doc = open_slice(pdf, None).unwrap();
    let cache = RenderCache::new();
    let options = Options { background: [255; 4], ..Default::default() };
    settings
        .iter()
        .map(|settings| {
            let raster = raster::render(&doc.pages()[0], &cache, settings, &options).unwrap();
            assert_eq!((raster.width, raster.height), (40, 40));
            raster.bytes().to_vec()
        })
        .collect()
}

/// One film per ink, as RGBA8 over white.
struct Plates(Vec<Vec<u8>>);

impl Plates {
    /// Each ink's tint at device pixel (x, y), in whole percent.
    fn at(&self, x: usize, y: usize) -> Vec<u32> {
        self.0
            .iter()
            .map(|film| {
                let px = &film[(y * 40 + x) * 4..][..4];
                assert_eq!((px[0], px[3]), (px[2], 255), "a film is opaque gray");
                (f32::from(255 - px[0]) * 100.0 / 255.0).round() as u32
            })
            .collect()
    }
}

fn plates(pdf: &[u8], inks: &[Ink], overprint: bool, simulation: Option<&CmykSimulation>) -> Plates {
    let settings: Vec<InterpreterSettings> = inks
        .iter()
        .map(|ink| InterpreterSettings {
            plate: Some(Plate { ink: ink.clone(), overprint, simulation: simulation.cloned(), spots_as_process: false }),
            ..Default::default()
        })
        .collect();
    Plates(render_each(pdf, &settings))
}

fn spot(name: &str) -> Ink {
    Ink::Spot(name.into())
}

/// The four process plates, then `spots`.
fn inks(spots: &[&str]) -> Vec<Ink> {
    Ink::PROCESS.into_iter().chain(spots.iter().map(|s| spot(s))).collect()
}

/// The process plates and Gold, overprint honored.
fn separate(pdf: &[u8]) -> Plates {
    plates(pdf, &inks(&["Gold"]), true, None)
}

const GOLD: &str = "[/Separation /Gold /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.3 1 0] /N 1 >>]";

fn gold_resource() -> String {
    format!("/ColorSpace << /CS0 {GOLD} >>")
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_spot_color_paints_its_own_plate_and_knocks_out_the_process_plates() {
    let s = separate(&page_pdf("0 0 0 1 k 0 0 40 40 re f /CS0 cs 0.5 scn 10 10 20 20 re f", &gold_resource(), &[]));
    assert_eq!(s.at(20, 20), [0, 0, 0, 0, 50], "knocked out under the spot color");
    assert_eq!(s.at(2, 2), [0, 0, 0, 100, 0]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn overprinting_keeps_the_plates_a_color_does_not_specify() {
    let resources = format!("{} /ExtGState << /GS1 << /op true >> >>", gold_resource());
    let pdf = page_pdf("0 0 0 1 k 0 0 40 40 re f /GS1 gs /CS0 cs 0.5 scn 10 10 20 20 re f", &resources, &[]);
    assert_eq!(separate(&pdf).at(20, 20), [0, 0, 0, 100, 50], "overprinted");
    assert_eq!(plates(&pdf, &inks(&["Gold"]), false, None).at(20, 20), [0, 0, 0, 0, 50], "overprint simulation off: knocked out");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn nonzero_overprint_mode_leaves_zero_device_cmyk_components() {
    let resources = "/ExtGState << /M1 << /OP true /OPM 1 >> /M0 << /OP true /OPM 0 >> >>";
    let pdf = |gs: &str| page_pdf(&format!("1 0 0 0 k 0 0 40 40 re f /{gs} gs 0 1 0 0 k 10 10 20 20 re f"), resources, &[]);
    let process = inks(&[]);
    assert_eq!(plates(&pdf("M1"), &process, true, None).at(20, 20), [100, 100, 0, 0], "OPM 1: the zero cyan leaves the plate");
    assert_eq!(plates(&pdf("M0"), &process, true, None).at(20, 20), [0, 100, 0, 0], "OPM 0: zero components paint");
}

/// A knockout group (`/K true`, pdfcraft #2974) of two process squares at 50 % opacity separates
/// with only the top square where they overlap: its plates replace the bottom square's there.
/// Without `/K` the two blend on every plate.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_knockout_group_separates_with_only_the_top_object_in_the_overlap() {
    let group = |entries: &str| {
        let form = "/A gs 1 0 0 0 k 0 0 25 25 re f 0 1 0 0 k 15 15 25 25 re f";
        page_pdf(
            "/Fm Do",
            "/XObject << /Fm 5 0 R >>",
            &[stream(
                &format!(
                    "/Type /XObject /Subtype /Form /BBox [0 0 40 40] /Group << /S /Transparency {entries} >> /Resources << /ExtGState << /A << /ca 0.5 >> >> >>"
                ),
                form.as_bytes(),
            )],
        )
    };
    // Device rows run top down: the overlap is at (20, 20), cyan alone at (5, 35), magenta alone
    // at (35, 5).
    let knockout = plates(&group("/K true"), &inks(&[]), true, None);
    assert_eq!(knockout.at(20, 20), [0, 50, 0, 0], "only the magenta square in the overlap");
    assert_eq!(knockout.at(5, 35), [50, 0, 0, 0]);
    assert_eq!(knockout.at(35, 5), [0, 50, 0, 0]);
    let blended = plates(&group(""), &inks(&[]), true, None);
    assert_eq!(blended.at(20, 20), [25, 50, 0, 0], "both squares without /K");
}
